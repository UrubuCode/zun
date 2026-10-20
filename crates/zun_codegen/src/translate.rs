//! Typed ZIL → Cranelift IR translation for one method body.
//!
//! Each evaluation-stack value carries its [`JitKind`], so the translator emits
//! the right machine type and picks signed vs unsigned instructions. Integers
//! use their exact Cranelift integer type (I8..I64), floats F32/F64, booleans
//! I32. Control flow is rebuilt into a CFG (leaders at branch targets and after
//! branches/ret); the front-end leaves the stack empty at block boundaries, so
//! blocks need no stack parameters and locals become typed Variables.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, Block, InstBuilder, MemFlags, Type, Value};
use cranelift_codegen::ir::AbiParam;
use cranelift_module::Linkage;
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{FuncId, Module as ClifModule};
use zun_il::{decode_all, JitKind, NumKind, Opcode, Operand};
use zun_module::{read_body, MethodBody, Module as ZmModule};
use zun_types::TypeSystem;

/// The Cranelift type for a JIT value kind.
pub fn clif_ty(kind: JitKind) -> Type {
    match kind {
        JitKind::Bool => types::I32,
        JitKind::Object => types::I64, // a machine pointer (64-bit target)
        JitKind::Other => types::I64,  // never used; eligibility excludes it
        JitKind::Num(n) => match n {
            NumKind::I8 | NumKind::U8 => types::I8,
            NumKind::I16 | NumKind::U16 => types::I16,
            NumKind::I32 | NumKind::U32 => types::I32,
            NumKind::I64 | NumKind::U64 => types::I64,
            NumKind::F32 => types::F32,
            NumKind::F64 => types::F64,
        },
    }
}

fn is_signed(kind: JitKind) -> bool {
    match kind {
        JitKind::Num(n) => n.int_parts().map(|(_, s)| s).unwrap_or(false),
        JitKind::Bool | JitKind::Object | JitKind::Other => false,
    }
}

fn is_float(kind: JitKind) -> bool {
    matches!(kind, JitKind::Num(NumKind::F32) | JitKind::Num(NumKind::F64))
}

/// Translate `body` into `ctx.func`. `zm` provides callee frames (arity and
/// return kind); `func_ids` maps method RIDs to declared Cranelift functions.
pub fn translate_body(
    module: &mut dyn ClifModule,
    zm: &ZmModule,
    ctx: &mut Context,
    fb_ctx: &mut FunctionBuilderContext,
    func_ids: &HashMap<u32, FuncId>,
    types: &TypeSystem,
    body: &MethodBody<'_>,
) -> Result<(), String> {
    let instrs = decode_all(body.code).map_err(|e| e.to_string())?;

    let mut offset_of_index = Vec::with_capacity(instrs.len());
    let mut index_of_offset = BTreeMap::new();
    {
        let mut off = 0usize;
        for (i, instr) in instrs.iter().enumerate() {
            index_of_offset.insert(off, i);
            offset_of_index.push(off);
            off += instr.encoded_len();
        }
        index_of_offset.insert(off, instrs.len());
    }

    let leaders = find_leaders(&instrs, &offset_of_index, &index_of_offset)?;

    let mut builder = FunctionBuilder::new(&mut ctx.func, fb_ctx);
    let mut blocks: BTreeMap<usize, Block> = BTreeMap::new();
    for &leader in &leaders {
        blocks.insert(leader, builder.create_block());
    }
    let entry = blocks[&0];
    builder.append_block_params_for_function_params(entry);

    // Local kinds (params first); declare a typed Variable per local.
    let local_kinds: Vec<JitKind> = body.local_kinds.iter().map(|&b| JitKind::from_byte(b)).collect();
    let mut vars = Vec::with_capacity(local_kinds.len());
    for &k in &local_kinds {
        vars.push(builder.declare_var(clif_ty(k)));
    }

    builder.switch_to_block(entry);
    let entry_params: Vec<Value> = builder.block_params(entry).to_vec();
    for (i, &p) in entry_params.iter().enumerate() {
        builder.def_var(vars[i], p);
    }
    // Zero-initialize non-parameter locals to a typed zero.
    for i in body.param_count as usize..local_kinds.len() {
        let z = typed_zero(&mut builder, local_kinds[i]);
        builder.def_var(vars[i], z);
    }

    let mut t = Translator {
        builder,
        vars,
        local_kinds,
        blocks,
        func_ids,
        types,
        module,
        zm,
    };
    t.run(&instrs, &offset_of_index, &index_of_offset)?;
    t.builder.finalize();
    Ok(())
}

fn typed_zero(builder: &mut FunctionBuilder, kind: JitKind) -> Value {
    match clif_ty(kind) {
        types::F32 => builder.ins().f32const(0.0),
        types::F64 => builder.ins().f64const(0.0),
        ty => builder.ins().iconst(ty, 0),
    }
}

fn find_leaders(
    instrs: &[zun_il::Instruction],
    offset_of_index: &[usize],
    index_of_offset: &BTreeMap<usize, usize>,
) -> Result<BTreeSet<usize>, String> {
    let mut leaders = BTreeSet::new();
    leaders.insert(0);
    for (i, instr) in instrs.iter().enumerate() {
        match instr.opcode {
            Opcode::Branch | Opcode::BranchTrue | Opcode::BranchFalse => {
                if let Operand::Branch(disp) = instr.operand {
                    let end = offset_of_index[i] + instr.encoded_len();
                    let target = (end as i64 + disp as i64) as usize;
                    let &idx = index_of_offset
                        .get(&target)
                        .ok_or("branch target not on an instruction boundary")?;
                    leaders.insert(idx);
                }
                if i + 1 < instrs.len() {
                    leaders.insert(i + 1);
                }
            }
            Opcode::Ret => {
                if i + 1 < instrs.len() {
                    leaders.insert(i + 1);
                }
            }
            _ => {}
        }
    }
    Ok(leaders)
}

/// A typed stack entry.
type Slot = (Value, JitKind);

struct Translator<'a> {
    builder: FunctionBuilder<'a>,
    vars: Vec<Variable>,
    local_kinds: Vec<JitKind>,
    blocks: BTreeMap<usize, Block>,
    func_ids: &'a HashMap<u32, FuncId>,
    module: &'a mut dyn ClifModule,
    zm: &'a ZmModule,
    types: &'a TypeSystem,
}

impl Translator<'_> {
    fn run(
        &mut self,
        instrs: &[zun_il::Instruction],
        offset_of_index: &[usize],
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<(), String> {
        let mut stack: Vec<Slot> = Vec::new();

        for (i, instr) in instrs.iter().enumerate() {
            if i != 0 {
                if let Some(&block) = self.blocks.get(&i) {
                    if !terminated(instrs, i) {
                        self.builder.ins().jump(block, &[]);
                    }
                    self.builder.switch_to_block(block);
                    stack.clear();
                }
            }

            match instr.opcode {
                Opcode::Nop => {}
                Opcode::ConstI32 => {
                    let v = imm32(&instr.operand);
                    let c = self.builder.ins().iconst(types::I32, v as i64);
                    stack.push((c, JitKind::Num(NumKind::I32)));
                }
                Opcode::ConstI64 => {
                    let v = imm64(&instr.operand);
                    let c = self.builder.ins().iconst(types::I64, v);
                    stack.push((c, JitKind::Num(NumKind::I64)));
                }
                Opcode::ConstF64 => {
                    let v = f64v(&instr.operand);
                    let c = self.builder.ins().f64const(v);
                    stack.push((c, JitKind::Num(NumKind::F64)));
                }
                Opcode::ConstTrue => {
                    let c = self.builder.ins().iconst(types::I32, 1);
                    stack.push((c, JitKind::Bool));
                }
                Opcode::ConstFalse => {
                    let c = self.builder.ins().iconst(types::I32, 0);
                    stack.push((c, JitKind::Bool));
                }
                Opcode::Conv => {
                    let target = NumKind::from_imm(imm32(&instr.operand))
                        .ok_or("bad conv kind")?;
                    let (v, from) = stack.pop().ok_or("stack underflow")?;
                    let out = self.convert(v, from, JitKind::Num(target));
                    stack.push((out, JitKind::Num(target)));
                }
                Opcode::LoadLocal => {
                    let slot = local_index(&instr.operand)?;
                    let v = self.builder.use_var(self.vars[slot]);
                    stack.push((v, self.local_kinds[slot]));
                }
                Opcode::StoreLocal => {
                    let slot = local_index(&instr.operand)?;
                    let (v, _) = stack.pop().ok_or("stack underflow")?;
                    self.builder.def_var(self.vars[slot], v);
                }
                Opcode::Pop => {
                    stack.pop().ok_or("stack underflow")?;
                }
                Opcode::Dup => {
                    let top = *stack.last().ok_or("stack underflow")?;
                    stack.push(top);
                }
                Opcode::AddI32 => self.int_bin(&mut stack, IntOp::Add)?,
                Opcode::SubI32 => self.int_bin(&mut stack, IntOp::Sub)?,
                Opcode::MulI32 => self.int_bin(&mut stack, IntOp::Mul)?,
                Opcode::DivI32 => self.int_bin(&mut stack, IntOp::Div)?,
                Opcode::RemI32 => self.int_bin(&mut stack, IntOp::Rem)?,
                Opcode::NegI32 => {
                    let (v, k) = stack.pop().ok_or("stack underflow")?;
                    let r = self.builder.ins().ineg(v);
                    stack.push((r, k));
                }
                Opcode::AddF64 => self.float_bin(&mut stack, FloatOp::Add)?,
                Opcode::SubF64 => self.float_bin(&mut stack, FloatOp::Sub)?,
                Opcode::MulF64 => self.float_bin(&mut stack, FloatOp::Mul)?,
                Opcode::DivF64 => self.float_bin(&mut stack, FloatOp::Div)?,
                Opcode::CmpEq => self.compare(&mut stack, Cmp::Eq)?,
                Opcode::CmpNe => self.compare(&mut stack, Cmp::Ne)?,
                Opcode::CmpLt => self.compare(&mut stack, Cmp::Lt)?,
                Opcode::CmpLe => self.compare(&mut stack, Cmp::Le)?,
                Opcode::CmpGt => self.compare(&mut stack, Cmp::Gt)?,
                Opcode::CmpGe => self.compare(&mut stack, Cmp::Ge)?,
                Opcode::Branch => {
                    let target = self.target(instr, i, offset_of_index, index_of_offset)?;
                    self.builder.ins().jump(target, &[]);
                }
                Opcode::BranchTrue => {
                    let target = self.target(instr, i, offset_of_index, index_of_offset)?;
                    let (cond, _) = stack.pop().ok_or("stack underflow")?;
                    let ft = self.fallthrough(i)?;
                    self.builder.ins().brif(cond, target, &[], ft, &[]);
                }
                Opcode::BranchFalse => {
                    let target = self.target(instr, i, offset_of_index, index_of_offset)?;
                    let (cond, _) = stack.pop().ok_or("stack underflow")?;
                    let ft = self.fallthrough(i)?;
                    // Taken when false: the true edge is the fall-through.
                    self.builder.ins().brif(cond, ft, &[], target, &[]);
                }
                Opcode::Ret => {
                    if let Some((v, _)) = stack.pop() {
                        self.builder.ins().return_(&[v]);
                    } else {
                        self.builder.ins().return_(&[]);
                    }
                }
                Opcode::Call | Opcode::CallVirt => {
                    let rid = match instr.operand {
                        Operand::Token(t) => t.rid(),
                        _ => return Err("call operand".into()),
                    };
                    let fid = *self.func_ids.get(&rid).ok_or("call to non-compiled method")?;
                    let callee = read_body(
                        &self.zm.code,
                        self.zm.tables.method_defs[rid as usize - 1].body_offset,
                    )
                    .map_err(|e| e.to_string())?;
                    let nargs = callee.param_count as usize;
                    if stack.len() < nargs {
                        return Err("stack underflow at call".into());
                    }
                    let args: Vec<Value> = stack
                        .split_off(stack.len() - nargs)
                        .into_iter()
                        .map(|(v, _)| v)
                        .collect();
                    let fref = self.module.declare_func_in_func(fid, self.builder.func);
                    let inst = self.builder.ins().call(fref, &args);
                    let results = self.builder.inst_results(inst);
                    if callee.returns_value {
                        let r = results[0];
                        stack.push((r, JitKind::from_byte(callee.return_kind)));
                    }
                }
                Opcode::NewObj => {
                    let type_rid = match instr.operand {
                        Operand::Token(t) => t.rid(),
                        _ => return Err("newobj operand".into()),
                    };
                    let (_field_start, count) = type_field_range(self.zm, type_rid)?;
                    // Pop field initializers (last pushed = last field).
                    if stack.len() < count {
                        return Err("stack underflow at newobj".into());
                    }
                    let args: Vec<Slot> = stack.split_off(stack.len() - count);
                    // ptr = zun_rt_alloc(type_rid, count)
                    let alloc = self.alloc_fref();
                    let trid = self.builder.ins().iconst(types::I32, type_rid as i64);
                    let nfields = self.builder.ins().iconst(types::I64, count as i64);
                    let inst = self.builder.ins().call(alloc, &[trid, nfields]);
                    let ptr = self.builder.inst_results(inst)[0];
                    // Store each field as a full i64 slot (same encoding the
                    // interpreter uses), at offset (slot + 1) * 8.
                    for (i, (val, kind)) in args.into_iter().enumerate() {
                        let raw = crate::shim::pack_result(&mut self.builder, val, kind);
                        let off = ((i + 1) * 8) as i32;
                        self.builder.ins().store(MemFlags::trusted(), raw, ptr, off);
                    }
                    stack.push((ptr, JitKind::Object));
                }
                Opcode::LoadField => {
                    let (kind, slot) = self.field_info(&instr.operand)?;
                    let (ptr, _) = stack.pop().ok_or("stack underflow")?;
                    let off = ((slot + 1) * 8) as i32;
                    let raw = self.builder.ins().load(types::I64, MemFlags::trusted(), ptr, off);
                    let v = crate::shim::unpack_arg(&mut self.builder, raw, kind);
                    stack.push((v, kind));
                }
                Opcode::StoreField => {
                    let (kind, slot) = self.field_info(&instr.operand)?;
                    let (val, _) = stack.pop().ok_or("stack underflow")?;
                    let (ptr, _) = stack.pop().ok_or("stack underflow")?;
                    let raw = crate::shim::pack_result(&mut self.builder, val, kind);
                    let off = ((slot + 1) * 8) as i32;
                    self.builder.ins().store(MemFlags::trusted(), raw, ptr, off);
                    stack.push((val, kind)); // stfld leaves the stored value
                }
                other => return Err(format!("opcode {other:?} not supported by the typed JIT")),
            }
        }

        self.builder.seal_all_blocks();
        Ok(())
    }

    fn int_bin(&mut self, stack: &mut Vec<Slot>, op: IntOp) -> Result<(), String> {
        let (b, _) = stack.pop().ok_or("stack underflow")?;
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let signed = is_signed(k);
        let ins = self.builder.ins();
        let r = match op {
            IntOp::Add => ins.iadd(a, b),
            IntOp::Sub => ins.isub(a, b),
            IntOp::Mul => ins.imul(a, b),
            IntOp::Div if signed => ins.sdiv(a, b),
            IntOp::Div => ins.udiv(a, b),
            IntOp::Rem if signed => ins.srem(a, b),
            IntOp::Rem => ins.urem(a, b),
        };
        stack.push((r, k));
        Ok(())
    }

    fn float_bin(&mut self, stack: &mut Vec<Slot>, op: FloatOp) -> Result<(), String> {
        let (b, _) = stack.pop().ok_or("stack underflow")?;
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let ins = self.builder.ins();
        let r = match op {
            FloatOp::Add => ins.fadd(a, b),
            FloatOp::Sub => ins.fsub(a, b),
            FloatOp::Mul => ins.fmul(a, b),
            FloatOp::Div => ins.fdiv(a, b),
        };
        stack.push((r, k));
        Ok(())
    }

    fn compare(&mut self, stack: &mut Vec<Slot>, cmp: Cmp) -> Result<(), String> {
        let (b, _) = stack.pop().ok_or("stack underflow")?;
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let flag = if is_float(k) {
            let cc = match cmp {
                Cmp::Eq => FloatCC::Equal,
                Cmp::Ne => FloatCC::NotEqual,
                Cmp::Lt => FloatCC::LessThan,
                Cmp::Le => FloatCC::LessThanOrEqual,
                Cmp::Gt => FloatCC::GreaterThan,
                Cmp::Ge => FloatCC::GreaterThanOrEqual,
            };
            self.builder.ins().fcmp(cc, a, b)
        } else {
            let signed = is_signed(k);
            let cc = match cmp {
                Cmp::Eq => IntCC::Equal,
                Cmp::Ne => IntCC::NotEqual,
                Cmp::Lt if signed => IntCC::SignedLessThan,
                Cmp::Lt => IntCC::UnsignedLessThan,
                Cmp::Le if signed => IntCC::SignedLessThanOrEqual,
                Cmp::Le => IntCC::UnsignedLessThanOrEqual,
                Cmp::Gt if signed => IntCC::SignedGreaterThan,
                Cmp::Gt => IntCC::UnsignedGreaterThan,
                Cmp::Ge if signed => IntCC::SignedGreaterThanOrEqual,
                Cmp::Ge => IntCC::UnsignedGreaterThanOrEqual,
            };
            self.builder.ins().icmp(cc, a, b)
        };
        // icmp/fcmp yield I8; widen to the I32 boolean representation.
        let as_i32 = self.builder.ins().uextend(types::I32, flag);
        stack.push((as_i32, JitKind::Bool));
        Ok(())
    }

    /// Convert `v` of kind `from` to kind `to`, emitting the right widening,
    /// narrowing, and int/float conversions.
    fn convert(&mut self, v: Value, from: JitKind, to: JitKind) -> Value {
        let from_ty = clif_ty(from);
        let to_ty = clif_ty(to);
        let from_float = is_float(from);
        let to_float = is_float(to);

        match (from_float, to_float) {
            (false, false) => {
                // int -> int: match widths, extending by source signedness.
                let fb = from_ty.bits();
                let tb = to_ty.bits();
                if tb == fb {
                    v
                } else if tb < fb {
                    self.builder.ins().ireduce(to_ty, v)
                } else if is_signed(from) {
                    self.builder.ins().sextend(to_ty, v)
                } else {
                    self.builder.ins().uextend(to_ty, v)
                }
            }
            (false, true) => {
                // int -> float. Widen narrow ints to I32 first for the convert.
                let wide = self.widen_for_convert(v, from);
                if is_signed(from) {
                    self.builder.ins().fcvt_from_sint(to_ty, wide)
                } else {
                    self.builder.ins().fcvt_from_uint(to_ty, wide)
                }
            }
            (true, false) => {
                // float -> int, saturating (never traps; JS-like), then narrow.
                let wide_ty = if to_ty.bits() < 32 { types::I32 } else { to_ty };
                let wide = if is_signed(to) {
                    self.builder.ins().fcvt_to_sint_sat(wide_ty, v)
                } else {
                    self.builder.ins().fcvt_to_uint_sat(wide_ty, v)
                };
                if wide_ty == to_ty {
                    wide
                } else {
                    self.builder.ins().ireduce(to_ty, wide)
                }
            }
            (true, true) => {
                if from_ty == to_ty {
                    v
                } else if to_ty == types::F64 {
                    self.builder.ins().fpromote(to_ty, v)
                } else {
                    self.builder.ins().fdemote(to_ty, v)
                }
            }
        }
    }

    /// Widen a narrow integer to I32 (for feeding int→float conversions).
    fn widen_for_convert(&mut self, v: Value, from: JitKind) -> Value {
        let ty = clif_ty(from);
        if ty.bits() >= 32 {
            v
        } else if is_signed(from) {
            self.builder.ins().sextend(types::I32, v)
        } else {
            self.builder.ins().uextend(types::I32, v)
        }
    }

    fn target(
        &self,
        instr: &zun_il::Instruction,
        i: usize,
        offset_of_index: &[usize],
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<Block, String> {
        let disp = match instr.operand {
            Operand::Branch(d) => d,
            _ => return Err("branch operand".into()),
        };
        let end = offset_of_index[i] + instr.encoded_len();
        let target = (end as i64 + disp as i64) as usize;
        let &idx = index_of_offset.get(&target).ok_or("bad branch target")?;
        self.blocks.get(&idx).copied().ok_or_else(|| "target not a leader".into())
    }

    fn fallthrough(&self, i: usize) -> Result<Block, String> {
        self.blocks
            .get(&(i + 1))
            .copied()
            .ok_or_else(|| "missing fall-through block".into())
    }

    /// Import `zun_rt_alloc(type_rid: i32, field_count: i64) -> ptr` into the
    /// current function.
    fn alloc_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let conv = self.module.isa().default_call_conv();
        let mut sig = cranelift_codegen::ir::Signature::new(conv);
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_rt_alloc", Linkage::Import, &sig)
            .expect("declare zun_rt_alloc");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    /// Resolve a field token to its `(kind, slot)` using module metadata.
    fn field_info(&self, operand: &Operand) -> Result<(JitKind, usize), String> {
        let rid = match operand {
            Operand::Token(t) => t.rid(),
            _ => return Err("field operand".into()),
        };
        let field = self.types.get_field(rid).ok_or("bad field token")?;
        Ok((JitKind::from_byte(field.kind), field.slot))
    }
}

/// For a TypeDef RID, return `(field_start_rid, field_count)`.
fn type_field_range(zm: &ZmModule, type_rid: u32) -> Result<(u32, usize), String> {
    let ty = zm
        .tables
        .type_defs
        .get(type_rid as usize - 1)
        .ok_or("bad type token")?;
    Ok((ty.field_start, (ty.field_end - ty.field_start) as usize))
}

#[derive(Clone, Copy)]
enum IntOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}
#[derive(Clone, Copy)]
enum FloatOp {
    Add,
    Sub,
    Mul,
    Div,
}
#[derive(Clone, Copy)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

fn terminated(instrs: &[zun_il::Instruction], i: usize) -> bool {
    matches!(
        instrs[i - 1].opcode,
        Opcode::Branch | Opcode::Ret | Opcode::BranchTrue | Opcode::BranchFalse
    )
}

fn local_index(operand: &Operand) -> Result<usize, String> {
    match operand {
        Operand::Local(v) => Ok(*v as usize),
        _ => Err("expected a local operand".into()),
    }
}

fn imm32(o: &Operand) -> i32 {
    match o {
        Operand::Imm32(v) => *v,
        _ => 0,
    }
}
fn imm64(o: &Operand) -> i64 {
    match o {
        Operand::Imm64(v) => *v,
        _ => 0,
    }
}
fn f64v(o: &Operand) -> f64 {
    match o {
        Operand::Float64(v) => *v,
        _ => 0.0,
    }
}
