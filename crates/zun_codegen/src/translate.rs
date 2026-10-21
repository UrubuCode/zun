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
use cranelift_codegen::ir::{
    types, AbiParam, Block, InstBuilder, MemFlags, Signature, Type, Value,
};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::Linkage;
use cranelift_module::{FuncId, Module as ClifModule};
use zun_il::{decode_all, JitKind, NumKind, Opcode, Operand};
use zun_module::{read_body, MethodBody, Module as ZmModule};
use zun_types::TypeSystem;

/// The Cranelift type for a JIT value kind.
pub fn clif_ty(kind: JitKind) -> Type {
    match kind {
        JitKind::Bool => types::I32,
        JitKind::Object | JitKind::String => types::I64,
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
        JitKind::Bool | JitKind::Object | JitKind::String | JitKind::Other => false,
    }
}

fn is_float(kind: JitKind) -> bool {
    matches!(
        kind,
        JitKind::Num(NumKind::F32) | JitKind::Num(NumKind::F64)
    )
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
    virtual_table_ptrs: *const u64,
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
    let local_kinds: Vec<JitKind> = body
        .local_kinds
        .iter()
        .map(|&b| JitKind::from_byte(b))
        .collect();
    let mut vars = Vec::with_capacity(local_kinds.len());
    for &k in &local_kinds {
        let var = builder.declare_var(clif_ty(k));
        if k == JitKind::Object {
            builder.declare_var_needs_stack_map(var);
        }
        vars.push(var);
    }

    builder.switch_to_block(entry);
    let entry_params: Vec<Value> = builder.block_params(entry).to_vec();
    for (i, &p) in entry_params.iter().enumerate() {
        builder.def_var(vars[i], p);
        if local_kinds[i] == JitKind::Object {
            builder.declare_value_needs_stack_map(p);
        }
    }
    // Zero-initialize non-parameter locals to a typed zero.
    for i in body.param_count as usize..local_kinds.len() {
        let z = typed_zero(&mut builder, local_kinds[i]);
        builder.def_var(vars[i], z);
    }
    let osr_counter = builder.declare_var(types::I64);
    let initial_osr_counter = builder.ins().iconst(types::I64, 1000);
    builder.def_var(osr_counter, initial_osr_counter);

    let mut t = Translator {
        builder,
        vars,
        local_kinds,
        blocks,
        func_ids,
        types,
        virtual_table_ptrs,
        module,
        zm,
        osr_counter,
        method_return_kind: JitKind::from_byte(body.return_kind),
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
    virtual_table_ptrs: *const u64,
    osr_counter: Variable,
    method_return_kind: JitKind,
}

impl Translator<'_> {
    fn run(
        &mut self,
        instrs: &[zun_il::Instruction],
        offset_of_index: &[usize],
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<(), String> {
        let mut stack: Vec<Slot> = Vec::new();
        let mut builtin_ids: HashMap<&'static str, FuncId> = HashMap::new();

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
                    let target = NumKind::from_imm(imm32(&instr.operand)).ok_or("bad conv kind")?;
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
                    if self.is_backedge(instr, i, offset_of_index, index_of_offset)? {
                        self.poll_backedge();
                    }
                    self.builder.ins().jump(target, &[]);
                }
                Opcode::BranchTrue => {
                    let target = self.target(instr, i, offset_of_index, index_of_offset)?;
                    if self.is_backedge(instr, i, offset_of_index, index_of_offset)? {
                        self.poll_backedge();
                    }
                    let (cond, _) = stack.pop().ok_or("stack underflow")?;
                    let ft = self.fallthrough(i)?;
                    self.builder.ins().brif(cond, target, &[], ft, &[]);
                }
                Opcode::BranchFalse => {
                    let target = self.target(instr, i, offset_of_index, index_of_offset)?;
                    if self.is_backedge(instr, i, offset_of_index, index_of_offset)? {
                        self.poll_backedge();
                    }
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
                    let method_name = self
                        .zm
                        .tables
                        .method_defs
                        .get(rid as usize - 1)
                        .map(|method| self.zm.strings.get(method.name));
                    if instr.opcode == Opcode::CallVirt
                        && stack.iter().any(|(_, kind)| *kind == JitKind::String)
                    {
                        match method_name {
                            Some("length") => {
                                let (receiver, _) =
                                    stack.pop().ok_or("string length receiver")?;
                                let length = self.string_length_fref();
                                let call = self.builder.ins().call(length, &[receiver]);
                                let result = self.builder.inst_results(call)[0];
                                stack.push((result, JitKind::Num(NumKind::U64)));
                                continue;
                            }
                            Some("contains") | Some("includes") => {
                                let (needle, needle_kind) =
                                    stack.pop().ok_or("string contains needle")?;
                                let (receiver, receiver_kind) =
                                    stack.pop().ok_or("string contains receiver")?;
                                if receiver_kind != JitKind::String
                                    || needle_kind != JitKind::String
                                {
                                    return Err("string contains expects string arguments".into());
                                }
                                let contains = self.string_contains_fref();
                                let call = self.builder.ins().call(contains, &[receiver, needle]);
                                let result = self.builder.inst_results(call)[0];
                                stack.push((result, JitKind::Bool));
                                continue;
                            }
                            Some("charCodeAt") => {
                                let (index, index_kind) =
                                    stack.pop().ok_or("string charCodeAt index")?;
                                let (receiver, receiver_kind) =
                                    stack.pop().ok_or("string charCodeAt receiver")?;
                                if receiver_kind != JitKind::String
                                    || !matches!(index_kind, JitKind::Num(kind) if kind.int_parts().is_some())
                                {
                                    return Err("string charCodeAt expects string and integer arguments".into());
                                }
                                let index = match clif_ty(index_kind) {
                                    types::I64 => index,
                                    ty if ty.is_int() => self.builder.ins().uextend(types::I64, index),
                                    _ => return Err("string charCodeAt index must be an integer".into()),
                                };
                                let char_code = self.string_char_code_at_fref();
                                let call = self.builder.ins().call(char_code, &[receiver, index]);
                                let result = self.builder.inst_results(call)[0];
                                stack.push((result, JitKind::Num(NumKind::U32)));
                                continue;
                            }
                            Some("startsWith") | Some("endsWith") => {
                                let (needle, needle_kind) =
                                    stack.pop().ok_or("string prefix/suffix")?;
                                let (receiver, receiver_kind) =
                                    stack.pop().ok_or("string prefix/suffix receiver")?;
                                if receiver_kind != JitKind::String
                                    || needle_kind != JitKind::String
                                {
                                    return Err("string prefix/suffix expects string arguments".into());
                                }
                                let helper = if method_name == Some("startsWith") {
                                    self.string_starts_with_fref()
                                } else {
                                    self.string_ends_with_fref()
                                };
                                let call = self.builder.ins().call(helper, &[receiver, needle]);
                                let result = self.builder.inst_results(call)[0];
                                stack.push((result, JitKind::Bool));
                                continue;
                            }
                            Some("indexOf") => {
                                let (needle, needle_kind) = stack.pop().ok_or("string indexOf needle")?;
                                let (receiver, receiver_kind) = stack.pop().ok_or("string indexOf receiver")?;
                                if receiver_kind != JitKind::String || needle_kind != JitKind::String {
                                    return Err("string indexOf expects string arguments".into());
                                }
                                let helper = self.string_index_of_fref();
                                let call = self.builder.ins().call(helper, &[receiver, needle]);
                                let result = self.builder.inst_results(call)[0];
                                stack.push((result, JitKind::Num(NumKind::I32)));
                                continue;
                            }
                            _ => {}
                        }
                    }
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
                        .map(|(v, kind)| {
                            if kind == JitKind::Object {
                                self.builder.declare_value_needs_stack_map(v);
                            }
                            v
                        })
                        .collect();
                    let inst = if instr.opcode == Opcode::CallVirt {
                        let receiver = args.first().copied().ok_or("virtual call receiver")?;
                        let slot = self.types.virtual_slot(rid).ok_or("virtual method slot")?;
                        let type_rid =
                            self.builder
                                .ins()
                                .load(types::I32, MemFlags::trusted(), receiver, 0);
                        let type_rid = self.builder.ins().uextend(types::I64, type_rid);
                        let ptrs = self
                            .builder
                            .ins()
                            .iconst(types::I64, self.virtual_table_ptrs as i64);
                        let ptr_offset = self.builder.ins().imul_imm(type_rid, 8);
                        let table_ptr_addr = self.builder.ins().iadd(ptrs, ptr_offset);
                        let table = self.builder.ins().load(
                            types::I64,
                            MemFlags::trusted(),
                            table_ptr_addr,
                            0,
                        );
                        let address = self.builder.ins().iadd_imm(table, (slot * 8) as i64);
                        let target =
                            self.builder
                                .ins()
                                .load(types::I64, MemFlags::trusted(), address, 0);
                        let mut sig = Signature::new(self.builder.func.signature.call_conv);
                        for &kind in &callee.local_kinds[..callee.param_count as usize] {
                            sig.params
                                .push(AbiParam::new(clif_ty(JitKind::from_byte(kind))));
                        }
                        if callee.returns_value {
                            sig.returns.push(AbiParam::new(clif_ty(JitKind::from_byte(
                                callee.return_kind,
                            ))));
                        }
                        let sig_ref = self.builder.import_signature(sig);
                        self.builder.ins().call_indirect(sig_ref, target, &args)
                    } else {
                        let fid = *self
                            .func_ids
                            .get(&rid)
                            .ok_or("call to non-compiled method")?;
                        let fref = self.module.declare_func_in_func(fid, self.builder.func);
                        self.builder.ins().call(fref, &args)
                    };
                    let results = self.builder.inst_results(inst);
                    if callee.returns_value {
                        let r = results[0];
                        let kind = JitKind::from_byte(callee.return_kind);
                        if kind == JitKind::Object {
                            self.builder.declare_value_needs_stack_map(r);
                        }
                        stack.push((r, kind));
                    }
                }
                Opcode::CallBuiltin => {
                    let id = match instr.operand {
                        Operand::Imm32(id) => id as u32,
                        _ => return Err("builtin operand".into()),
                    };
                    let (name, arg_count, arg_type, result_kind, result_type) =
                        match zun_natives::by_id(id).map(|f| f.name) {
                            Some("length") => (
                                "zun_jit_string_length",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::U64),
                                types::I64,
                            ),
                            Some("contains") | Some("includes") => (
                                "zun_jit_string_contains",
                                2,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("charCodeAt") => (
                                "zun_jit_string_char_code_at",
                                2,
                                types::I64,
                                JitKind::Num(NumKind::U32),
                                types::I32,
                            ),
                            Some("startsWith") => (
                                "zun_jit_string_starts_with",
                                2,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("endsWith") => (
                                "zun_jit_string_ends_with",
                                2,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("indexOf") => (
                                "zun_jit_string_index_of",
                                2,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("isValid") => (
                                "zun_jit_date_is_valid",
                                1,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("year") => (
                                "zun_jit_date_year",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("month") => (
                                "zun_jit_date_month",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("day") => (
                                "zun_jit_date_day",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("hour") => (
                                "zun_jit_date_hour",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("minute") => (
                                "zun_jit_date_minute",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("second") => (
                                "zun_jit_date_second",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("millisecond") => (
                                "zun_jit_date_millisecond",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("weekday") => (
                                "zun_jit_date_weekday",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("dayOfYear") => (
                                "zun_jit_date_day_of_year",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::I32),
                                types::I32,
                            ),
                            Some("abs") => (
                                "zun_jit_math_abs",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("signNumber") => (
                                "zun_jit_math_sign",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("sqrt" | "sqrtNumber") => (
                                "zun_jit_math_sqrt",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("cbrtNumber") => (
                                "zun_jit_math_cbrt",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("floor" | "floorNumber") => (
                                "zun_jit_math_floor",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("ceil" | "ceilNumber") => (
                                "zun_jit_math_ceil",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("round" | "roundNumber") => (
                                "zun_jit_math_round",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("truncNumber") => (
                                "zun_jit_math_trunc",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("sinNumber") => (
                                "zun_jit_math_sin",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("cosNumber") => (
                                "zun_jit_math_cos",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("expNumber") => (
                                "zun_jit_math_exp",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("logNumber") => (
                                "zun_jit_math_log",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("log10Number") => (
                                "zun_jit_math_log10",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("log2Number") => (
                                "zun_jit_math_log2",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("expm1Number") => (
                                "zun_jit_math_expm1",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("log1pNumber") => (
                                "zun_jit_math_log1p",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("sinhNumber") => (
                                "zun_jit_math_sinh",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("coshNumber") => (
                                "zun_jit_math_cosh",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("tanhNumber") => (
                                "zun_jit_math_tanh",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("asinhNumber") => (
                                "zun_jit_math_asinh",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("acoshNumber") => (
                                "zun_jit_math_acosh",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("atanhNumber") => (
                                "zun_jit_math_atanh",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("tanNumber") => (
                                "zun_jit_math_tan",
                                1,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("min" | "minNumber") => (
                                "zun_jit_math_min",
                                2,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("max" | "maxNumber") => (
                                "zun_jit_math_max",
                                2,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("powNumber") => (
                                "zun_jit_math_pow",
                                2,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("atan2Number") => (
                                "zun_jit_math_atan2",
                                2,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("hypotNumber") => (
                                "zun_jit_math_hypot",
                                2,
                                types::F64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("isFinite") => (
                                "zun_jit_number_is_finite",
                                1,
                                types::F64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("isNaN") => (
                                "zun_jit_number_is_nan",
                                1,
                                types::F64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("isNegativeZero") => (
                                "zun_jit_number_is_negative_zero",
                                1,
                                types::F64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("isSafeInteger") => (
                                "zun_jit_number_is_safe_integer",
                                1,
                                types::F64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("mapSize") => (
                                "zun_jit_map_size",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::U64),
                                types::I64,
                            ),
                            Some("setSize") => (
                                "zun_jit_set_size",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::U64),
                                types::I64,
                            ),
                            Some("bufferLength") => (
                                "zun_jit_buffer_length",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::U64),
                                types::I64,
                            ),
                            Some("dataViewLength") => (
                                "zun_jit_data_view_length",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::U64),
                                types::I64,
                            ),
                            Some("dataViewByteLength") => (
                                "zun_jit_data_view_byte_length", 1, types::I64,
                                JitKind::Num(NumKind::U64), types::I64,
                            ),
                            Some("u8ArrayLength") | Some("u16ArrayLength") | Some("u32ArrayLength") | Some("f64ArrayLength") => (
                                "zun_jit_typed_array_length",
                                1,
                                types::I64,
                                JitKind::Num(NumKind::U64),
                                types::I64,
                            ),
                            Some("u8ArrayByteLength") | Some("u16ArrayByteLength") | Some("u32ArrayByteLength") | Some("f64ArrayByteLength") => (
                                "zun_jit_typed_array_byte_length", 1, types::I64,
                                JitKind::Num(NumKind::U64), types::I64,
                            ),
                            Some("bufferGetU8") => (
                                "zun_jit_buffer_get_u8",
                                2,
                                types::I64,
                                JitKind::Num(NumKind::U8),
                                types::I8,
                            ),
                            Some("bufferReadU32") => (
                                "zun_jit_buffer_read_u32",
                                2,
                                types::I64,
                                JitKind::Num(NumKind::U32),
                                types::I32,
                            ),
                            Some("bufferReadF64") => (
                                "zun_jit_buffer_read_f64",
                                2,
                                types::I64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("bufferSetU8") => (
                                "zun_jit_buffer_set_u8",
                                3,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("bufferWriteU32") => (
                                "zun_jit_buffer_write_u32",
                                3,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("bufferWriteF64") => (
                                "zun_jit_buffer_write_f64",
                                3,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("dataViewGetU32") => (
                                "zun_jit_data_view_get_u32",
                                3,
                                types::I64,
                                JitKind::Num(NumKind::U32),
                                types::I32,
                            ),
                            Some("dataViewGetU16") => (
                                "zun_jit_data_view_get_u16",
                                3,
                                types::I64,
                                JitKind::Num(NumKind::U16),
                                types::I16,
                            ),
                            Some("dataViewGetF64") => (
                                "zun_jit_data_view_get_f64",
                                3,
                                types::I64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("dataViewSetU32") => (
                                "zun_jit_data_view_set_u32",
                                4,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("dataViewSetU16") => (
                                "zun_jit_data_view_set_u16",
                                4,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("dataViewSetF64") => (
                                "zun_jit_data_view_set_f64",
                                4,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("u8ArrayGet") => (
                                "zun_jit_u8_array_get",
                                2,
                                types::I64,
                                JitKind::Num(NumKind::U8),
                                types::I8,
                            ),
                            Some("u8ArraySet") => (
                                "zun_jit_u8_array_set",
                                3,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("u16ArrayGet") => (
                                "zun_jit_u16_array_get",
                                2,
                                types::I64,
                                JitKind::Num(NumKind::U16),
                                types::I16,
                            ),
                            Some("u16ArraySet") => (
                                "zun_jit_u16_array_set",
                                3,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("u32ArrayGet") => (
                                "zun_jit_u32_array_get",
                                2,
                                types::I64,
                                JitKind::Num(NumKind::U32),
                                types::I32,
                            ),
                            Some("u32ArraySet") => (
                                "zun_jit_u32_array_set",
                                3,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            Some("f64ArrayGet") => (
                                "zun_jit_f64_array_get",
                                2,
                                types::I64,
                                JitKind::Num(NumKind::F64),
                                types::F64,
                            ),
                            Some("f64ArraySet") => (
                                "zun_jit_f64_array_set",
                                3,
                                types::I64,
                                JitKind::Bool,
                                types::I32,
                            ),
                            _ => return Err("unsupported builtin".into()),
                        };
                    if stack.len() < arg_count {
                        return Err("builtin stack underflow".into());
                    }
                    let args: Vec<Slot> = stack.split_off(stack.len() - arg_count);
                    // Every imported helper is a potential GC boundary. Keep
                    // managed receivers/arguments visible before the call,
                    // regardless of whether the helper currently allocates.
                    for &(value, kind) in &args {
                        if kind == JitKind::Object {
                            self.builder.declare_value_needs_stack_map(value);
                        }
                    }
                    let arg_types = match name {
                        "zun_jit_string_char_code_at" => vec![types::I64, types::I64],
                        "zun_jit_buffer_set_u8" => vec![types::I64, types::I64, types::I8],
                        "zun_jit_buffer_write_u32" => vec![types::I64, types::I64, types::I32],
                        "zun_jit_buffer_write_f64" => vec![types::I64, types::I64, types::F64],
                        "zun_jit_data_view_get_u32" => vec![types::I64, types::I64, types::I32],
                        "zun_jit_data_view_get_u16" => vec![types::I64, types::I64, types::I32],
                        "zun_jit_data_view_get_f64" => vec![types::I64, types::I64, types::I32],
                        "zun_jit_data_view_set_u32" => vec![types::I64, types::I64, types::I32, types::I32],
                        "zun_jit_data_view_set_u16" => vec![types::I64, types::I64, types::I16, types::I32],
                        "zun_jit_data_view_set_f64" => vec![types::I64, types::I64, types::F64, types::I32],
                        "zun_jit_u8_array_get" => vec![types::I64, types::I64],
                        "zun_jit_u8_array_set" => vec![types::I64, types::I64, types::I8],
                        "zun_jit_u16_array_get" => vec![types::I64, types::I64],
                        "zun_jit_u16_array_set" => vec![types::I64, types::I64, types::I16],
                        "zun_jit_u32_array_get" => vec![types::I64, types::I64],
                        "zun_jit_u32_array_set" => vec![types::I64, types::I64, types::I32],
                        "zun_jit_f64_array_get" => vec![types::I64, types::I64],
                        "zun_jit_f64_array_set" => vec![types::I64, types::I64, types::F64],
                        _ => vec![arg_type; arg_count],
                    };
                    let fid = if let Some(&fid) = builtin_ids.get(name) {
                        fid
                    } else {
                        let mut sig = Signature::new(self.builder.func.signature.call_conv);
                        for arg_type in arg_types {
                            sig.params.push(AbiParam::new(arg_type));
                        }
                        sig.returns.push(AbiParam::new(result_type));
                        let fid = self
                            .module
                            .declare_function(name, Linkage::Import, &sig)
                            .map_err(|e| e.to_string())?;
                        builtin_ids.insert(name, fid);
                        fid
                    };
                    let fref = self.module.declare_func_in_func(fid, self.builder.func);
                    let values: Vec<Value> = args.iter().map(|(value, _)| *value).collect();
                    let inst = self.builder.ins().call(fref, &values);
                    let result = self.builder.inst_results(inst)[0];
                    if result_kind == JitKind::Object {
                        self.builder.declare_value_needs_stack_map(result);
                    }
                    stack.push((result, result_kind));
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
                    // Allocation may reach the GC. Keep reference-valued
                    // initializers live while the new object is being built.
                    for &(value, kind) in &args {
                        if kind == JitKind::Object {
                            self.builder.declare_value_needs_stack_map(value);
                        }
                    }
                    // ptr = zun_rt_alloc(type_rid, count, caller_sp)
                    let alloc = self.alloc_fref();
                    let trid = self.builder.ins().iconst(types::I32, type_rid as i64);
                    let nfields = self.builder.ins().iconst(types::I64, count as i64);
                    let caller_sp = self.builder.ins().get_stack_pointer(types::I64);
                    let inst = self.builder.ins().call(alloc, &[trid, nfields, caller_sp]);
                    let ptr = self.builder.inst_results(inst)[0];
                    self.builder.declare_value_needs_stack_map(ptr);
                    // Store each field as a full i64 slot (same encoding the
                    // interpreter uses), at the shared core layout offset.
                    for (i, (val, kind)) in args.into_iter().enumerate() {
                        if kind == JitKind::Object {
                            let store = self.store_ref_fref();
                            let field = self.builder.ins().iconst(types::I64, i as i64);
                            self.builder.ins().call(store, &[ptr, field, val]);
                        } else {
                            let raw = crate::shim::pack_result(&mut self.builder, val, kind);
                            let off = zun_core::field_offset(i) as i32;
                            self.builder.ins().store(MemFlags::trusted(), raw, ptr, off);
                        }
                    }
                    stack.push((ptr, JitKind::Object));
                }
                Opcode::ArrayLength => {
                    let (array, kind) = stack.pop().ok_or("stack underflow")?;
                    if kind == JitKind::Object {
                        self.builder.declare_value_needs_stack_map(array);
                    }
                    let length = self.array_length_fref();
                    let inst = self.builder.ins().call(length, &[array]);
                    let result = self.builder.inst_results(inst)[0];
                    stack.push((result, JitKind::Num(NumKind::U64)));
                }
                Opcode::NewArray => {
                    let (length, length_kind) = stack.pop().ok_or("stack underflow")?;
                    if !matches!(length_kind, JitKind::Num(kind) if kind.int_parts().is_some()) {
                        return Err("array length must be an integer".into());
                    }
                    let length = match clif_ty(length_kind) {
                        types::I64 => length,
                        ty if ty.is_int() => self.builder.ins().uextend(types::I64, length),
                        _ => return Err("array length must be an integer".into()),
                    };
                    let alloc = self.array_new_fref();
                    let inst = self.builder.ins().call(alloc, &[length]);
                    let result = self.builder.inst_results(inst)[0];
                    self.builder.declare_value_needs_stack_map(result);
                    stack.push((result, JitKind::Object));
                }
                // A numeric array load already decodes the dynamic carrier at
                // the helper boundary; the source-level cast is therefore a
                // representation-preserving check for this narrow JIT path.
                Opcode::UnboxAny => {
                    let value = stack.pop().ok_or("stack underflow")?;
                    if value.1 == JitKind::Other {
                        return Err("unbox.any requires a typed JIT value".into());
                    }
                    stack.push(value);
                }
                // Numeric values are already represented in their native JIT
                // form; the managed Any carrier is materialized only by the
                // runtime array helper at the store boundary.
                Opcode::BoxAny => {
                    let value = stack.pop().ok_or("stack underflow")?;
                    if !matches!(value.1, JitKind::Num(_) | JitKind::Bool) {
                        return Err("box.any requires a scalar JIT value".into());
                    }
                    stack.push(value);
                }
                Opcode::LoadArray => {
                    let (index, index_kind) = stack.pop().ok_or("stack underflow")?;
                    let (array, array_kind) = stack.pop().ok_or("stack underflow")?;
                    if array_kind != JitKind::Object {
                        return Err("array load requires an object receiver".into());
                    }
                    self.builder.declare_value_needs_stack_map(array);
                    let index = match clif_ty(index_kind) {
                        types::I64 => index,
                        ty if ty.is_int() => self.builder.ins().uextend(types::I64, index),
                        _ => return Err("array index must be an integer".into()),
                    };
                    let load_u64 = self.method_return_kind == JitKind::Num(NumKind::U64);
                    let load_object = self.method_return_kind == JitKind::Object;
                    let load = if load_object {
                        self.array_load_object_fref()
                    } else if load_u64 {
                        self.array_load_u64_fref()
                    } else {
                        self.array_load_f64_fref()
                    };
                    let result = self.builder.ins().call(load, &[array, index]);
                    let kind = if load_object {
                        JitKind::Object
                    } else if load_u64 {
                        JitKind::Num(NumKind::U64)
                    } else {
                        JitKind::Num(NumKind::F64)
                    };
                    stack.push((self.builder.inst_results(result)[0], kind));
                }
                Opcode::StoreArray => {
                    let (value, value_kind) = stack.pop().ok_or("stack underflow")?;
                    let (index, index_kind) = stack.pop().ok_or("stack underflow")?;
                    let (array, array_kind) = stack.pop().ok_or("stack underflow")?;
                    let store_object = value_kind == JitKind::Object;
                    let store_u64 = value_kind == JitKind::Num(NumKind::U64);
                    if array_kind != JitKind::Object
                        || (!store_object && !store_u64 && value_kind != JitKind::Num(NumKind::F64))
                    {
                        return Err("numeric array store requires object and f64/u64 value".into());
                    }
                    self.builder.declare_value_needs_stack_map(array);
                    let index = match clif_ty(index_kind) {
                        types::I64 => index,
                        ty if ty.is_int() => self.builder.ins().uextend(types::I64, index),
                        _ => return Err("array index must be an integer".into()),
                    };
                    let store = if store_object {
                        self.array_store_object_fref()
                    } else if store_u64 {
                        self.array_store_u64_fref()
                    } else {
                        self.array_store_f64_fref()
                    };
                    self.builder.ins().call(store, &[array, index, value]);
                    stack.push((value, value_kind));
                }
                Opcode::ArrayPop => {
                    let (array, kind) = stack.pop().ok_or("stack underflow")?;
                    if kind != JitKind::Object {
                        return Err("array pop requires an object receiver".into());
                    }
                    self.builder.declare_value_needs_stack_map(array);
                    let pop_u64 = self.method_return_kind == JitKind::Num(NumKind::U64);
                    let pop = if pop_u64 {
                        self.array_pop_u64_fref()
                    } else {
                        self.array_pop_f64_fref()
                    };
                    let inst = self.builder.ins().call(pop, &[array]);
                    stack.push((
                        self.builder.inst_results(inst)[0],
                        if pop_u64 { JitKind::Num(NumKind::U64) } else { JitKind::Num(NumKind::F64) },
                    ));
                }
                Opcode::ArrayPush => {
                    let (value, value_kind) = stack.pop().ok_or("stack underflow")?;
                    let (array, array_kind) = stack.pop().ok_or("stack underflow")?;
                    let push_u64 = value_kind == JitKind::Num(NumKind::U64);
                    if array_kind != JitKind::Object
                        || (!push_u64 && value_kind != JitKind::Num(NumKind::F64))
                    {
                        return Err("numeric array push requires object and f64/u64 value".into());
                    }
                    self.builder.declare_value_needs_stack_map(array);
                    let push = if push_u64 {
                        self.array_push_u64_fref()
                    } else {
                        self.array_push_f64_fref()
                    };
                    let inst = self.builder.ins().call(push, &[array, value]);
                    stack.push((self.builder.inst_results(inst)[0], JitKind::Num(NumKind::I64)));
                }
                Opcode::LoadField => {
                    let (kind, slot) = self.field_info(&instr.operand)?;
                    let (ptr, _) = stack.pop().ok_or("stack underflow")?;
                    self.builder.declare_value_needs_stack_map(ptr);
                    let off = zun_core::field_offset(slot) as i32;
                    let raw = self
                        .builder
                        .ins()
                        .load(types::I64, MemFlags::trusted(), ptr, off);
                    let v = crate::shim::unpack_arg(&mut self.builder, raw, kind);
                    if kind == JitKind::Object {
                        self.builder.declare_value_needs_stack_map(v);
                    }
                    stack.push((v, kind));
                }
                Opcode::StoreField => {
                    let (kind, slot) = self.field_info(&instr.operand)?;
                    let (val, _) = stack.pop().ok_or("stack underflow")?;
                    let (ptr, _) = stack.pop().ok_or("stack underflow")?;
                    if kind == JitKind::Object {
                        self.builder.declare_value_needs_stack_map(ptr);
                        self.builder.declare_value_needs_stack_map(val);
                        let store = self.store_ref_fref();
                        let field = self.builder.ins().iconst(types::I64, slot as i64);
                        self.builder.ins().call(store, &[ptr, field, val]);
                    } else {
                        let raw = crate::shim::pack_result(&mut self.builder, val, kind);
                        let off = zun_core::field_offset(slot) as i32;
                        self.builder.ins().store(MemFlags::trusted(), raw, ptr, off);
                    }
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
        self.blocks
            .get(&idx)
            .copied()
            .ok_or_else(|| "target not a leader".into())
    }

    fn is_backedge(
        &self,
        instr: &zun_il::Instruction,
        i: usize,
        offset_of_index: &[usize],
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<bool, String> {
        let disp = match instr.operand {
            Operand::Branch(d) => d,
            _ => return Err("branch operand".into()),
        };
        let end = offset_of_index[i] + instr.encoded_len();
        let target = (end as i64 + disp as i64) as usize;
        let &idx = index_of_offset.get(&target).ok_or("bad branch target")?;
        Ok(idx <= i)
    }

    fn poll_backedge(&mut self) {
        let counter = self.builder.use_var(self.osr_counter);
        let one = self.builder.ins().iconst(types::I64, 1);
        let decremented = self.builder.ins().isub(counter, one);
        let fref = self.osr_poll_fref();
        let call = self.builder.ins().call(fref, &[decremented]);
        let replenished = self.builder.inst_results(call)[0];
        self.builder.def_var(self.osr_counter, replenished);
    }

    fn fallthrough(&self, i: usize) -> Result<Block, String> {
        self.blocks
            .get(&(i + 1))
            .copied()
            .ok_or_else(|| "missing fall-through block".into())
    }

    /// Import `zun_rt_alloc(type_rid: i32, field_count: i64, caller_sp: i64) ->
    /// ptr` into the current function. The caller SP lets the allocation
    /// safepoint publish the stopped JIT frame without guessing its layout.
    fn alloc_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let conv = self.module.isa().default_call_conv();
        let mut sig = cranelift_codegen::ir::Signature::new(conv);
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_rt_alloc", Linkage::Import, &sig)
            .expect("declare zun_rt_alloc");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn osr_poll_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_osr_poll", Linkage::Import, &sig)
            .expect("declare zun_jit_osr_poll");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn array_length_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let conv = self.module.isa().default_call_conv();
        let mut sig = Signature::new(conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_length", Linkage::Import, &sig)
            .expect("declare zun_jit_array_length");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_length_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_length", Linkage::Import, &sig)
            .expect("declare zun_jit_string_length");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_contains_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_contains", Linkage::Import, &sig)
            .expect("declare zun_jit_string_contains");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_char_code_at_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_char_code_at", Linkage::Import, &sig)
            .expect("declare zun_jit_string_char_code_at");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_starts_with_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        self.string_predicate_fref("zun_jit_string_starts_with")
    }

    fn string_ends_with_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        self.string_predicate_fref("zun_jit_string_ends_with")
    }

    fn string_index_of_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_index_of", Linkage::Import, &sig)
            .expect("declare zun_jit_string_index_of");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_predicate_fref(&mut self, name: &str) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function(name, Linkage::Import, &sig)
            .expect("declare string predicate");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn array_new_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_new", Linkage::Import, &sig)
            .expect("declare zun_jit_array_new");
        self.module
            .declare_func_in_func(id, self.builder.func)
    }

    fn array_load_f64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));
        let id = self
            .module
            .declare_function("zun_jit_array_load_f64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_load_f64");
        self.module
            .declare_func_in_func(id, self.builder.func)
    }

    fn array_load_u64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func = self
            .module
            .declare_function("zun_jit_array_load_u64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_load_u64");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    fn array_load_object_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_load_object", Linkage::Import, &sig)
            .expect("declare zun_jit_array_load_object");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn array_store_f64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_array_store_f64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_store_f64");
        self.module
            .declare_func_in_func(id, self.builder.func)
    }

    fn array_store_u64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let func = self
            .module
            .declare_function("zun_jit_array_store_u64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_store_u64");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    fn array_store_object_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_array_store_object", Linkage::Import, &sig)
            .expect("declare zun_jit_array_store_object");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn store_ref_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_store_ref", Linkage::Import, &sig)
            .expect("declare zun_jit_store_ref");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn array_pop_f64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));
        let id = self
            .module
            .declare_function("zun_jit_array_pop_f64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_pop_f64");
        self.module
            .declare_func_in_func(id, self.builder.func)
    }

    fn array_pop_u64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func = self
            .module
            .declare_function("zun_jit_array_pop_u64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_pop_u64");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    fn array_push_f64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_push_f64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_push_f64");
        self.module
            .declare_func_in_func(id, self.builder.func)
    }

    fn array_push_u64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func = self
            .module
            .declare_function("zun_jit_array_push_u64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_push_u64");
        self.module.declare_func_in_func(func, self.builder.func)
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
