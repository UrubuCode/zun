//! Typed ZIL → Cranelift IR translation for one method body.
//!
//! Each evaluation-stack value carries its [`JitKind`], so the translator emits
//! the right machine type and picks signed vs unsigned instructions. Integers
//! use their exact Cranelift integer type (I8..I64), floats F32/F64, booleans
//! I32. Control flow is rebuilt into a CFG (leaders at branch targets and after
//! branches/ret); the front-end leaves the stack empty at block boundaries, so
//! blocks need no stack parameters and locals become typed Variables.

mod arith;
mod arrays;
mod builtins;
mod builtins_table;
mod calls;
mod exceptions;
mod objects;
mod strings;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    types, AbiParam, Block, BlockArg, InstBuilder, MemFlags, Signature, StackSlotData,
    StackSlotKind, Type, Value,
};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::Linkage;
use cranelift_module::{DataId, FuncId, Module as ClifModule};
use zun_il::{decode_all, JitKind, NumKind, Opcode, Operand};
use zun_module::{read_body, MethodBody, Module as ZmModule};
use zun_types::TypeSystem;

/// The Cranelift type for a JIT value kind.
pub fn clif_ty(kind: JitKind) -> Type {
    match kind {
        JitKind::Bool => types::I32,
        JitKind::Object | JitKind::Array | JitKind::String => types::I64,
        // A transient function value carries its method rid (no-closure case).
        JitKind::Function => types::I64,
        JitKind::Other => types::I64, // never used; eligibility excludes it
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

pub(crate) fn is_signed(kind: JitKind) -> bool {
    match kind {
        JitKind::Num(n) => n.int_parts().map(|(_, s)| s).unwrap_or(false),
        JitKind::Bool
        | JitKind::Object
        | JitKind::Array
        | JitKind::String
        | JitKind::Function
        | JitKind::Other => false,
    }
}

pub(crate) fn is_float(kind: JitKind) -> bool {
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
    literal_ptrs: *const u64,
    literal_data_ids: *const u32,
    body: &MethodBody<'_>,
    bridge_enabled: bool,
    pending_slot_addr: u64,
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

    let mut leaders = find_leaders(&instrs, &offset_of_index, &index_of_offset)?;
    // Each catch handler's entry is a branch target (the `throw` jumps to it), so
    // it must be a block leader even though no textual branch points at it.
    for handler in &body.handlers {
        let &idx = index_of_offset
            .get(&(handler.handler_start as usize))
            .ok_or("handler start not on an instruction boundary")?;
        leaders.insert(idx);
    }

    let mut builder = FunctionBuilder::new(&mut ctx.func, fb_ctx);
    let mut blocks: BTreeMap<usize, Block> = BTreeMap::new();
    for &leader in &leaders {
        blocks.insert(leader, builder.create_block());
    }
    // The prologue gets its own entry block: a method whose first instruction is a
    // loop header makes `blocks[&0]` a branch target, and Cranelift forbids
    // jumping to the entry block.
    let first = blocks[&0];
    let entry = builder.create_block();
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
        if matches!(k, JitKind::Object | JitKind::Array) {
            builder.declare_var_needs_stack_map(var);
        }
        vars.push(var);
    }

    builder.switch_to_block(entry);
    let entry_params: Vec<Value> = builder.block_params(entry).to_vec();
    let mut param_value = 0usize;
    for (i, &kind) in local_kinds
        .iter()
        .take(body.param_count as usize)
        .enumerate()
    {
        if kind == JitKind::Other {
            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                crate::any_abi::CARRIER_SIZE,
                crate::any_abi::CARRIER_ALIGN_SHIFT,
            ));
            let address = builder.ins().stack_addr(types::I64, slot, 0);
            builder.ins().store(
                MemFlags::trusted(),
                entry_params[param_value],
                address,
                crate::any_abi::TAG_OFFSET,
            );
            builder.ins().store(
                MemFlags::trusted(),
                entry_params[param_value + 1],
                address,
                crate::any_abi::PAYLOAD_OFFSET,
            );
            builder.def_var(vars[i], address);
            param_value += 2;
        } else {
            let p = entry_params[param_value];
            builder.def_var(vars[i], p);
            if matches!(kind, JitKind::Object | JitKind::Array) {
                builder.declare_value_needs_stack_map(p);
            }
            param_value += 1;
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
    builder.ins().jump(first, &[]);
    builder.switch_to_block(first);

    let mut t = Translator {
        builder,
        vars,
        local_kinds,
        blocks,
        func_ids,
        types,
        virtual_table_ptrs,
        literal_ptrs,
        literal_data_ids,
        module,
        zm,
        osr_counter,
        method_return_kind: JitKind::from_byte(body.return_kind),
        method_returns_value: body.returns_value,
        handlers: body.handlers.clone(),
        bridge_enabled,
        pending_slot_addr,
        exc_exit: None,
    };
    t.run(&instrs, &offset_of_index, &index_of_offset)?;
    t.builder.finalize();
    Ok(())
}

pub(crate) fn typed_zero(builder: &mut FunctionBuilder, kind: JitKind) -> Value {
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
            Opcode::Ret | Opcode::Throw => {
                // `throw` lowers to a branch to the catch block, so it ends its
                // block like `ret`: the next instruction begins a new one.
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
pub(crate) type Slot = (Value, JitKind);

pub(crate) struct Translator<'a> {
    pub(crate) builder: FunctionBuilder<'a>,
    pub(crate) vars: Vec<Variable>,
    pub(crate) local_kinds: Vec<JitKind>,
    pub(crate) blocks: BTreeMap<usize, Block>,
    pub(crate) func_ids: &'a HashMap<u32, FuncId>,
    pub(crate) module: &'a mut dyn ClifModule,
    pub(crate) zm: &'a ZmModule,
    pub(crate) types: &'a TypeSystem,
    pub(crate) virtual_table_ptrs: *const u64,
    pub(crate) literal_ptrs: *const u64,
    pub(crate) literal_data_ids: *const u32,
    pub(crate) osr_counter: Variable,
    pub(crate) method_return_kind: JitKind,
    /// Exception regions of the method body (same-frame catch-all slice only).
    pub(crate) handlers: Vec<zun_module::ExceptionHandler>,
    /// Whether a call to an ineligible (non-`func_ids`) method is lowered to a
    /// bridged call to the interpreter instead of an error.
    pub(crate) bridge_enabled: bool,
    /// Whether the method returns a value (for the exceptional-exit block's ABI).
    pub(crate) method_returns_value: bool,
    /// Address of the pending-exception slot loaded after each call when the
    /// bridge is enabled (0 otherwise).
    pub(crate) pending_slot_addr: u64,
    /// The method's single exceptional-exit block (returns an ABI-shaped dummy,
    /// leaving the pending slot set), created lazily on the first post-call check.
    pub(crate) exc_exit: Option<Block>,
}

impl Translator<'_> {
    /// Thread the operand stack across a block edge. The front end may leave a
    /// value on the stack at a block boundary (a `&&`/`||`/ternary result, or a
    /// value live across a branch), which Cranelift models as a block parameter.
    /// This ensures `target` has one parameter per stack slot — created once, on
    /// the first edge into it — and returns the stack values as the jump
    /// arguments so the value survives into the successor block. GC references
    /// get a stack map on the parameter.
    pub(crate) fn thread_edge(
        &mut self,
        block_sigs: &mut HashMap<Block, Vec<JitKind>>,
        target: Block,
        stack: &[Slot],
    ) -> Vec<BlockArg> {
        if !block_sigs.contains_key(&target) {
            for &(_, kind) in stack {
                let param = self.builder.append_block_param(target, clif_ty(kind));
                if matches!(kind, JitKind::Object | JitKind::Array | JitKind::String) {
                    self.builder.declare_value_needs_stack_map(param);
                }
            }
            block_sigs.insert(target, stack.iter().map(|&(_, k)| k).collect());
        }
        stack.iter().map(|&(v, _)| BlockArg::from(v)).collect()
    }

    fn run(
        &mut self,
        instrs: &[zun_il::Instruction],
        offset_of_index: &[usize],
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<(), String> {
        let mut stack: Vec<Slot> = Vec::new();
        // Per-block operand-stack signature (the kinds of its entry parameters),
        // filled lazily by `thread_edge` on the first edge into a block.
        let mut block_sigs: HashMap<Block, Vec<JitKind>> = HashMap::new();
        let mut builtin_ids: HashMap<&'static str, FuncId> = HashMap::new();

        for (i, instr) in instrs.iter().enumerate() {
            if i != 0 {
                if let Some(&block) = self.blocks.get(&i) {
                    if !terminated(instrs, i) {
                        let args = self.thread_edge(&mut block_sigs, block, &stack);
                        self.builder.ins().jump(block, &args);
                    }
                    self.builder.switch_to_block(block);
                    // Rebuild the operand stack from the block's parameters (the
                    // threaded edge values) rather than discarding it.
                    stack = match block_sigs.get(&block) {
                        Some(kinds) => self
                            .builder
                            .block_params(block)
                            .iter()
                            .zip(kinds)
                            .map(|(&value, &kind)| (value, kind))
                            .collect(),
                        None => Vec::new(),
                    };
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
                Opcode::ConstStr => self.op_const_str(instr, &mut stack)?,
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
                Opcode::AndI32 => self.int_bin(&mut stack, IntOp::And)?,
                Opcode::OrI32 => self.int_bin(&mut stack, IntOp::Or)?,
                Opcode::XorI32 => self.int_bin(&mut stack, IntOp::Xor)?,
                Opcode::ShlI32 => self.int_bin(&mut stack, IntOp::Shl)?,
                Opcode::ShrI32 => self.int_bin(&mut stack, IntOp::Shr)?,
                Opcode::UShrI32 => self.int_bin(&mut stack, IntOp::UShr)?,
                Opcode::NotI32 => {
                    let (v, k) = stack.pop().ok_or("stack underflow")?;
                    let r = self.builder.ins().bnot(v);
                    stack.push((r, k));
                }
                Opcode::PowI32 => self.int_pow(&mut stack)?,
                Opcode::AddF64 => self.float_bin(&mut stack, FloatOp::Add)?,
                Opcode::SubF64 => self.float_bin(&mut stack, FloatOp::Sub)?,
                Opcode::MulF64 => self.float_bin(&mut stack, FloatOp::Mul)?,
                Opcode::DivF64 => self.float_bin(&mut stack, FloatOp::Div)?,
                Opcode::RemF64 => self.float_rem(&mut stack)?,
                Opcode::Concat => self.op_concat(&mut stack)?,
                // Float math machine ops: one Cranelift instruction each, so
                // these are instructions in the generated code rather than the
                // libm calls the native path still uses for transcendentals.
                Opcode::SqrtF64 => self.float_un(&mut stack, FloatUnOp::Sqrt)?,
                Opcode::AbsF64 => self.float_un(&mut stack, FloatUnOp::Abs)?,
                Opcode::NegF64 => self.float_un(&mut stack, FloatUnOp::Neg)?,
                Opcode::FloorF64 => self.float_un(&mut stack, FloatUnOp::Floor)?,
                Opcode::CeilF64 => self.float_un(&mut stack, FloatUnOp::Ceil)?,
                Opcode::TruncF64 => self.float_un(&mut stack, FloatUnOp::Trunc)?,
                Opcode::MinF64 => self.float_bin(&mut stack, FloatOp::Min)?,
                Opcode::MaxF64 => self.float_bin(&mut stack, FloatOp::Max)?,
                // Integer machine ops. The Cranelift value already has the
                // operand kind's width (clif_ty maps u8 to I8), so the
                // bit-counting instructions answer for that width natively,
                // matching Tier 0's explicit masking.
                Opcode::RoundF64 => self.round_f64(&mut stack)?,
                Opcode::SignF64 => self.sign_f64(&mut stack)?,
                Opcode::MinI32 => self.int_minmax(&mut stack, true)?,
                Opcode::MaxI32 => self.int_minmax(&mut stack, false)?,
                Opcode::ClzI32 => self.int_un(&mut stack, IntUnOp::Clz)?,
                Opcode::CtzI32 => self.int_un(&mut stack, IntUnOp::Ctz)?,
                Opcode::PopcntI32 => self.int_un(&mut stack, IntUnOp::Popcnt)?,
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
                    let args = self.thread_edge(&mut block_sigs, target, &stack);
                    self.builder.ins().jump(target, &args);
                }
                Opcode::BranchTrue => {
                    let target = self.target(instr, i, offset_of_index, index_of_offset)?;
                    if self.is_backedge(instr, i, offset_of_index, index_of_offset)? {
                        self.poll_backedge();
                    }
                    let (cond, _) = stack.pop().ok_or("stack underflow")?;
                    let ft = self.fallthrough(i)?;
                    // The remaining stack (after the condition) is live on both
                    // edges, so it is threaded to each target.
                    let args = self.thread_edge(&mut block_sigs, target, &stack);
                    let ft_args = self.thread_edge(&mut block_sigs, ft, &stack);
                    self.builder.ins().brif(cond, target, &args, ft, &ft_args);
                }
                Opcode::BranchFalse => {
                    let target = self.target(instr, i, offset_of_index, index_of_offset)?;
                    if self.is_backedge(instr, i, offset_of_index, index_of_offset)? {
                        self.poll_backedge();
                    }
                    let (cond, _) = stack.pop().ok_or("stack underflow")?;
                    let ft = self.fallthrough(i)?;
                    // Taken when false: the true edge is the fall-through. The
                    // remaining stack is live on both edges.
                    let args = self.thread_edge(&mut block_sigs, target, &stack);
                    let ft_args = self.thread_edge(&mut block_sigs, ft, &stack);
                    self.builder.ins().brif(cond, ft, &ft_args, target, &args);
                }
                Opcode::Ret => {
                    if let Some((v, kind)) = stack.pop() {
                        if kind == JitKind::Other {
                            let tag = self.builder.ins().load(
                                types::I64,
                                MemFlags::trusted(),
                                v,
                                crate::any_abi::TAG_OFFSET,
                            );
                            let payload = self.builder.ins().load(
                                types::I64,
                                MemFlags::trusted(),
                                v,
                                crate::any_abi::PAYLOAD_OFFSET,
                            );
                            self.builder.ins().return_(&[tag, payload]);
                        } else {
                            self.builder.ins().return_(&[v]);
                        }
                    } else {
                        self.builder.ins().return_(&[]);
                    }
                }
                Opcode::Call | Opcode::CallVirt => self.op_call(
                    instr,
                    i,
                    &mut stack,
                    &mut block_sigs,
                    offset_of_index,
                    index_of_offset,
                )?,
                Opcode::CallBuiltin => self.op_call_builtin(instr, &mut stack, &mut builtin_ids)?,
                Opcode::NewObj => self.op_new_obj(instr, &mut stack)?,
                Opcode::ArrayLength => self.op_array_length(&mut stack)?,
                Opcode::NewArray => self.op_new_array(&mut stack)?,
                Opcode::UnboxAny => self.op_unbox_any(&mut stack)?,
                Opcode::CastClass => self.op_cast_class(&mut stack)?,
                Opcode::BoxAny => self.op_box_any(&mut stack)?,
                Opcode::LoadArray => {
                    let (index, index_kind) = stack.pop().ok_or("stack underflow")?;
                    let (array, array_kind) = stack.pop().ok_or("stack underflow")?;
                    if !matches!(array_kind, JitKind::Object | JitKind::Array) {
                        return Err("array load requires an object receiver".into());
                    }
                    self.builder.declare_value_needs_stack_map(array);
                    let index = match clif_ty(index_kind) {
                        types::I64 => index,
                        ty if ty.is_int() => self.builder.ins().uextend(types::I64, index),
                        _ => return Err("array index must be an integer".into()),
                    };
                    let carrier_load = self.method_return_kind == JitKind::Other
                        || instrs
                            .get(i + 1)
                            .is_some_and(|next| next.opcode == Opcode::UnboxAny);
                    let load_u64 = self.method_return_kind == JitKind::Num(NumKind::U64);
                    let load_i64 = self.method_return_kind == JitKind::Num(NumKind::I64);
                    let load_i32 = self.method_return_kind == JitKind::Num(NumKind::I32);
                    let load_narrow = matches!(
                        self.method_return_kind,
                        JitKind::Num(NumKind::I8 | NumKind::U8 | NumKind::I16 | NumKind::U16)
                    );
                    let load_bool = self.method_return_kind == JitKind::Bool;
                    let load_object = self.method_return_kind == JitKind::Object;
                    if carrier_load {
                        let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                            StackSlotKind::ExplicitSlot,
                            crate::any_abi::CARRIER_SIZE,
                            crate::any_abi::CARRIER_ALIGN_SHIFT,
                        ));
                        let carrier = self.builder.ins().stack_addr(types::I64, slot, 0);
                        let load = self.array_load_any_fref();
                        self.builder.ins().call(load, &[array, index, carrier]);
                        stack.push((carrier, JitKind::Other));
                        continue;
                    }
                    let load = if load_object {
                        self.array_load_object_fref()
                    } else if load_bool {
                        self.array_load_bool_fref()
                    } else if load_i32 || load_narrow {
                        self.array_load_i32_fref()
                    } else if load_u64 || load_i64 {
                        self.array_load_u64_fref()
                    } else {
                        self.array_load_f64_fref()
                    };
                    let result = self.builder.ins().call(load, &[array, index]);
                    let raw_result = self.builder.inst_results(result)[0];
                    let kind = if load_object {
                        JitKind::Object
                    } else if load_u64 {
                        JitKind::Num(NumKind::U64)
                    } else if load_i64 {
                        JitKind::Num(NumKind::I64)
                    } else if load_bool {
                        JitKind::Bool
                    } else if load_i32 {
                        JitKind::Num(NumKind::I32)
                    } else if load_narrow {
                        let width = match self.method_return_kind {
                            JitKind::Num(NumKind::I8 | NumKind::U8) => types::I8,
                            _ => types::I16,
                        };
                        let narrowed = self.builder.ins().ireduce(width, raw_result);
                        stack.push((narrowed, self.method_return_kind));
                        continue;
                    } else {
                        JitKind::Num(NumKind::F64)
                    };
                    stack.push((raw_result, kind));
                }
                Opcode::StoreArray => {
                    let (value, value_kind) = stack.pop().ok_or("stack underflow")?;
                    let (index, index_kind) = stack.pop().ok_or("stack underflow")?;
                    let (array, array_kind) = stack.pop().ok_or("stack underflow")?;
                    let store_object = matches!(value_kind, JitKind::Object | JitKind::Array);
                    let store_u64 = value_kind == JitKind::Num(NumKind::U64);
                    let store_i64 = value_kind == JitKind::Num(NumKind::I64);
                    let store_i32 = value_kind == JitKind::Num(NumKind::I32);
                    let store_narrow = matches!(
                        value_kind,
                        JitKind::Num(NumKind::I8 | NumKind::U8 | NumKind::I16 | NumKind::U16)
                    );
                    let store_bool = value_kind == JitKind::Bool;
                    if !matches!(array_kind, JitKind::Object | JitKind::Array)
                        || (!store_object
                            && !store_u64
                            && !store_i64
                            && !store_i32
                            && !store_narrow
                            && !store_bool
                            && value_kind != JitKind::Num(NumKind::F64))
                    {
                        return Err("numeric array store requires object and f64/u64 value".into());
                    }
                    self.builder.declare_value_needs_stack_map(array);
                    let index = match clif_ty(index_kind) {
                        types::I64 => index,
                        ty if ty.is_int() => self.builder.ins().uextend(types::I64, index),
                        _ => return Err("array index must be an integer".into()),
                    };
                    let store_value = if store_narrow {
                        match value_kind {
                            JitKind::Num(NumKind::I8 | NumKind::U8) => {
                                self.builder.ins().uextend(types::I32, value)
                            }
                            _ => self.builder.ins().sextend(types::I32, value),
                        }
                    } else {
                        value
                    };
                    let store = if store_object {
                        self.array_store_object_fref()
                    } else if store_u64 || store_i64 {
                        self.array_store_u64_fref()
                    } else if store_bool {
                        self.array_store_bool_fref()
                    } else if store_i32 || store_narrow {
                        self.array_store_i32_fref()
                    } else {
                        self.array_store_f64_fref()
                    };
                    self.builder.ins().call(store, &[array, index, store_value]);
                    stack.push((value, value_kind));
                }
                Opcode::ArrayPop => self.op_array_pop(&mut stack)?,
                Opcode::ArrayPush => self.op_array_push(&mut stack)?,
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
                Opcode::LoadStatic => self.op_load_static(instr, &mut stack)?,
                Opcode::StoreStatic => self.op_store_static(instr, &mut stack)?,
                Opcode::Throw => self.op_throw(
                    i,
                    &mut stack,
                    &mut block_sigs,
                    offset_of_index,
                    index_of_offset,
                )?,
                Opcode::LoadFunction => {
                    // A no-closure function value is carried as its method rid.
                    let rid = match instr.operand {
                        Operand::Token(token) => token.rid(),
                        _ => return Err("ldftn operand".into()),
                    };
                    let value = self.builder.ins().iconst(types::I64, rid as i64);
                    stack.push((value, JitKind::Function));
                }
                Opcode::CallIndirect => self.op_call_indirect(
                    instr,
                    i,
                    &mut stack,
                    &mut block_sigs,
                    offset_of_index,
                    index_of_offset,
                )?,
                other => return Err(format!("opcode {other:?} not supported by the typed JIT")),
            }
        }

        self.finish_exc_exit();
        self.builder.seal_all_blocks();
        Ok(())
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
}

#[derive(Clone, Copy)]
pub(crate) enum IntOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    UShr,
}
#[derive(Clone, Copy)]
pub(crate) enum FloatOp {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

/// Integer operations that take a single operand and lower to one Cranelift
/// instruction.
#[derive(Clone, Copy)]
pub(crate) enum IntUnOp {
    Clz,
    Ctz,
    Popcnt,
}

/// Float operations that take a single operand and lower to one Cranelift
/// instruction.
#[derive(Clone, Copy)]
pub(crate) enum FloatUnOp {
    Sqrt,
    Abs,
    Neg,
    Floor,
    Ceil,
    Trunc,
}
#[derive(Clone, Copy)]
pub(crate) enum Cmp {
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
        Opcode::Branch | Opcode::Ret | Opcode::BranchTrue | Opcode::BranchFalse | Opcode::Throw
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
