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
mod exceptions;
mod objects;

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
    for (i, &kind) in local_kinds.iter().take(body.param_count as usize).enumerate() {
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
                Opcode::ConstStr => {
                    let Operand::StringRef(index) = instr.operand else {
                        return Err("const.str operand".into());
                    };
                    if self.literal_data_ids.is_null() && self.literal_ptrs.is_null() {
                        return Err("const.str requires runtime literal storage".into());
                    }
                    let len = self
                        .zm
                        .strings
                        .get(zun_metadata::StringIndex(index))
                        .len();
                    let ptr = if self.literal_data_ids.is_null() {
                        let address = unsafe { *self.literal_ptrs.add(index as usize) };
                        self.builder.ins().iconst(types::I64, address as i64)
                    } else {
                        let data_id = unsafe { *self.literal_data_ids.add(index as usize) };
                        let global = self
                            .module
                            .declare_data_in_func(DataId::from_u32(data_id), self.builder.func);
                        self.builder.ins().global_value(types::I64, global)
                    };
                    let len = self.builder.ins().iconst(types::I64, len as i64);
                    let literal = self.string_literal_fref();
                    let call = self.builder.ins().call(literal, &[ptr, len]);
                    let result = self.builder.inst_results(call)[0];
                    // Interning may allocate and trigger a collection. Keep
                    // the managed handle visible to the precise stack map
                    // for subsequent safepoints in this method.
                    self.builder.declare_value_needs_stack_map(result);
                    stack.push((result, JitKind::String));
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
                Opcode::Concat => {
                    // String `+`: pop two managed string handles, call the shared
                    // runtime concat helper (the same one the `.concat` method
                    // lowers to), and push the new string. Both operands and the
                    // result are GC references, so each is kept in a stack map
                    // across the allocating call.
                    let (right, right_kind) = stack.pop().ok_or("concat right")?;
                    let (left, left_kind) = stack.pop().ok_or("concat left")?;
                    if left_kind != JitKind::String || right_kind != JitKind::String {
                        return Err("concat expects string operands".into());
                    }
                    self.builder.declare_value_needs_stack_map(left);
                    self.builder.declare_value_needs_stack_map(right);
                    let helper = self.string_concat_fref();
                    let call = self.builder.ins().call(helper, &[left, right]);
                    let result = self.builder.inst_results(call)[0];
                    self.builder.declare_value_needs_stack_map(result);
                    stack.push((result, JitKind::String));
                }
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
                                self.builder.declare_value_needs_stack_map(receiver);
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
                                self.builder.declare_value_needs_stack_map(receiver);
                                self.builder.declare_value_needs_stack_map(needle);
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
                                self.builder.declare_value_needs_stack_map(receiver);
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
                                self.builder.declare_value_needs_stack_map(receiver);
                                self.builder.declare_value_needs_stack_map(needle);
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
                                self.builder.declare_value_needs_stack_map(receiver);
                                self.builder.declare_value_needs_stack_map(needle);
                                let helper = self.string_index_of_fref();
                                let call = self.builder.ins().call(helper, &[receiver, needle]);
                                let result = self.builder.inst_results(call)[0];
                                stack.push((result, JitKind::Num(NumKind::I32)));
                                continue;
                            }
                            Some("lastIndexOf") => {
                                let (needle, needle_kind) = stack.pop().ok_or("string lastIndexOf needle")?;
                                let (receiver, receiver_kind) = stack.pop().ok_or("string lastIndexOf receiver")?;
                                if receiver_kind != JitKind::String || needle_kind != JitKind::String {
                                    return Err("string lastIndexOf expects string arguments".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                self.builder.declare_value_needs_stack_map(needle);
                                let helper = self.string_last_index_of_fref();
                                let call = self.builder.ins().call(helper, &[receiver, needle]);
                                let result = self.builder.inst_results(call)[0];
                                stack.push((result, JitKind::Num(NumKind::I32)));
                                continue;
                            }
                            Some("count") => {
                                let (needle, needle_kind) = stack.pop().ok_or("string count needle")?;
                                let (receiver, receiver_kind) = stack.pop().ok_or("string count receiver")?;
                                if receiver_kind != JitKind::String || needle_kind != JitKind::String {
                                    return Err("string count expects string arguments".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                self.builder.declare_value_needs_stack_map(needle);
                                let helper = self.string_count_fref();
                                let call = self.builder.ins().call(helper, &[receiver, needle]);
                                let result = self.builder.inst_results(call)[0];
                                stack.push((result, JitKind::Num(NumKind::I32)));
                                continue;
                            }
                            Some("compare") => {
                                let (right, right_kind) = stack.pop().ok_or("string compare right")?;
                                let (left, left_kind) = stack.pop().ok_or("string compare left")?;
                                if left_kind != JitKind::String || right_kind != JitKind::String {
                                    return Err("string compare expects string arguments".into());
                                }
                                self.builder.declare_value_needs_stack_map(left);
                                self.builder.declare_value_needs_stack_map(right);
                                let helper = self.string_compare_fref();
                                let call = self.builder.ins().call(helper, &[left, right]);
                                let result = self.builder.inst_results(call)[0];
                                stack.push((result, JitKind::Num(NumKind::I32)));
                                continue;
                            }
                            Some("concat") => {
                                let (right, right_kind) = stack.pop().ok_or("string concat right")?;
                                let (left, left_kind) = stack.pop().ok_or("string concat left")?;
                                if left_kind != JitKind::String || right_kind != JitKind::String {
                                    return Err("string concat expects string arguments".into());
                                }
                                self.builder.declare_value_needs_stack_map(left);
                                self.builder.declare_value_needs_stack_map(right);
                                let helper = self.string_concat_fref();
                                let call = self.builder.ins().call(helper, &[left, right]);
                                let result = self.builder.inst_results(call)[0];
                                self.builder.declare_value_needs_stack_map(result);
                                stack.push((result, JitKind::String));
                                continue;
                            }
                            Some("substring") => {
                                let (end, end_kind) = stack.pop().ok_or("substring end")?;
                                let (start, start_kind) = stack.pop().ok_or("substring start")?;
                                let (receiver, receiver_kind) = stack.pop().ok_or("substring receiver")?;
                                if receiver_kind != JitKind::String
                                    || !matches!(start_kind, JitKind::Num(kind) if kind.int_parts().is_some())
                                    || !matches!(end_kind, JitKind::Num(kind) if kind.int_parts().is_some())
                                {
                                    return Err("substring expects string and integer arguments".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                let start = match clif_ty(start_kind) {
                                    types::I64 => start,
                                    ty if ty.is_int() => self.builder.ins().uextend(types::I64, start),
                                    _ => return Err("substring start must be an integer".into()),
                                };
                                let end = match clif_ty(end_kind) {
                                    types::I64 => end,
                                    ty if ty.is_int() => self.builder.ins().uextend(types::I64, end),
                                    _ => return Err("substring end must be an integer".into()),
                                };
                                let helper = self.string_substring_fref();
                                let call = self.builder.ins().call(helper, &[receiver, start, end]);
                                let result = self.builder.inst_results(call)[0];
                                self.builder.declare_value_needs_stack_map(result);
                                stack.push((result, JitKind::String));
                                continue;
                            }
                            Some("trim") => {
                                let (receiver, receiver_kind) = stack.pop().ok_or("trim receiver")?;
                                if receiver_kind != JitKind::String {
                                    return Err("trim expects a string receiver".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                let helper = self.string_trim_fref("trim");
                                let call = self.builder.ins().call(helper, &[receiver]);
                                let result = self.builder.inst_results(call)[0];
                                self.builder.declare_value_needs_stack_map(result);
                                stack.push((result, JitKind::String));
                                continue;
                            }
                            Some("trimStart" | "trimEnd") => {
                                let (receiver, receiver_kind) = stack.pop().ok_or("trim receiver")?;
                                if receiver_kind != JitKind::String {
                                    return Err("trim expects a string receiver".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                let helper = self.string_trim_fref(method_name.unwrap_or("trim"));
                                let call = self.builder.ins().call(helper, &[receiver]);
                                let result = self.builder.inst_results(call)[0];
                                self.builder.declare_value_needs_stack_map(result);
                                stack.push((result, JitKind::String));
                                continue;
                            }
                            Some("toLowerCase" | "toUpperCase") => {
                                let (receiver, receiver_kind) = stack.pop().ok_or("case receiver")?;
                                if receiver_kind != JitKind::String {
                                    return Err("case conversion expects a string receiver".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                let helper = self.string_transform_fref(method_name.unwrap_or("toLowerCase"));
                                let call = self.builder.ins().call(helper, &[receiver]);
                                let result = self.builder.inst_results(call)[0];
                                self.builder.declare_value_needs_stack_map(result);
                                stack.push((result, JitKind::String));
                                continue;
                            }
                            Some("repeat") => {
                                let (count, count_kind) = stack.pop().ok_or("repeat count")?;
                                let (receiver, receiver_kind) = stack.pop().ok_or("repeat receiver")?;
                                if receiver_kind != JitKind::String
                                    || !matches!(count_kind, JitKind::Num(kind) if kind.int_parts().is_some())
                                {
                                    return Err("repeat expects a string and integer count".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                let count = match clif_ty(count_kind) {
                                    types::I64 => count,
                                    ty if ty.is_int() => self.builder.ins().uextend(types::I64, count),
                                    _ => return Err("repeat count must be an integer".into()),
                                };
                                let helper = self.string_repeat_fref();
                                let call = self.builder.ins().call(helper, &[receiver, count]);
                                let result = self.builder.inst_results(call)[0];
                                self.builder.declare_value_needs_stack_map(result);
                                stack.push((result, JitKind::String));
                                continue;
                            }
                            Some("reverse") => {
                                let (receiver, receiver_kind) = stack.pop().ok_or("reverse receiver")?;
                                if receiver_kind != JitKind::String {
                                    return Err("reverse expects a string receiver".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                let helper = self.string_reverse_fref();
                                let call = self.builder.ins().call(helper, &[receiver]);
                                let result = self.builder.inst_results(call)[0];
                                self.builder.declare_value_needs_stack_map(result);
                                stack.push((result, JitKind::String));
                                continue;
                            }
                            Some("padStart" | "padEnd") => {
                                let (fill, fill_kind) = stack.pop().ok_or("pad fill")?;
                                let (target, target_kind) = stack.pop().ok_or("pad target")?;
                                let (receiver, receiver_kind) = stack.pop().ok_or("pad receiver")?;
                                if receiver_kind != JitKind::String
                                    || fill_kind != JitKind::String
                                    || !matches!(target_kind, JitKind::Num(kind) if kind.int_parts().is_some())
                                {
                                    return Err("pad expects string, integer, and string arguments".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                self.builder.declare_value_needs_stack_map(fill);
                                let target = match clif_ty(target_kind) {
                                    types::I64 => target,
                                    ty if ty.is_int() => self.builder.ins().uextend(types::I64, target),
                                    _ => return Err("pad target must be an integer".into()),
                                };
                                let helper = self.string_pad_fref(method_name.unwrap_or("padStart"));
                                let call = self.builder.ins().call(helper, &[receiver, target, fill]);
                                let result = self.builder.inst_results(call)[0];
                                self.builder.declare_value_needs_stack_map(result);
                                stack.push((result, JitKind::String));
                                continue;
                            }
                            Some("replace") => {
                                let (replacement, replacement_kind) = stack.pop().ok_or("replace replacement")?;
                                let (needle, needle_kind) = stack.pop().ok_or("replace needle")?;
                                let (receiver, receiver_kind) = stack.pop().ok_or("replace receiver")?;
                                if receiver_kind != JitKind::String
                                    || needle_kind != JitKind::String
                                    || replacement_kind != JitKind::String
                                {
                                    return Err("replace expects three strings".into());
                                }
                                self.builder.declare_value_needs_stack_map(receiver);
                                self.builder.declare_value_needs_stack_map(needle);
                                self.builder.declare_value_needs_stack_map(replacement);
                                let helper = self.string_replace_fref();
                                let call = self.builder.ins().call(helper, &[receiver, needle, replacement]);
                                let result = self.builder.inst_results(call)[0];
                                self.builder.declare_value_needs_stack_map(result);
                                stack.push((result, JitKind::String));
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
                    let arg_values: Vec<(Value, JitKind)> = stack
                        .split_off(stack.len() - nargs)
                        .into_iter()
                        .map(|(v, kind)| {
                            if matches!(kind, JitKind::Object | JitKind::String) {
                                self.builder.declare_value_needs_stack_map(v);
                            }
                            (v, kind)
                        })
                        .collect();
                    // A direct call to an ineligible (non-compiled) callee is
                    // routed to the interpreter through the bridge. Eligibility
                    // only admits this for a non-throwing, marshalable callee.
                    if instr.opcode == Opcode::Call
                        && self.bridge_enabled
                        && !self.func_ids.contains_key(&rid)
                    {
                        self.emit_bridge_call(rid, &arg_values, &callee, &mut stack)?;
                        self.after_call_pending_check(
                            &mut block_sigs,
                            &mut stack,
                            offset_of_index[i],
                            index_of_offset,
                        )?;
                        continue;
                    }
                    let direct_string_call = instr.opcode == Opcode::CallVirt
                        && arg_values
                            .first()
                            .is_some_and(|(_, kind)| *kind == JitKind::String);
                    let inst = if instr.opcode == Opcode::CallVirt && !direct_string_call {
                        let receiver = arg_values.first().map(|(value, _)| *value).ok_or("virtual call receiver")?;
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
                        let mut call_args = Vec::new();
                        for ((value, _kind), &param_kind_byte) in arg_values.iter().zip(
                            &callee.local_kinds[..callee.param_count as usize],
                        ) {
                            let param_kind = JitKind::from_byte(param_kind_byte);
                            if param_kind == JitKind::Other {
                                sig.params.push(AbiParam::new(types::I64));
                                sig.params.push(AbiParam::new(types::I64));
                                call_args.push(self.builder.ins().load(
                                    types::I64,
                                    MemFlags::trusted(),
                                    *value,
                                    crate::any_abi::TAG_OFFSET,
                                ));
                                call_args.push(self.builder.ins().load(
                                    types::I64,
                                    MemFlags::trusted(),
                                    *value,
                                    crate::any_abi::PAYLOAD_OFFSET,
                                ));
                            } else {
                                sig.params.push(AbiParam::new(clif_ty(param_kind)));
                                call_args.push(*value);
                            }
                        }
                        if callee.returns_value {
                            let return_kind = JitKind::from_byte(callee.return_kind);
                            if return_kind == JitKind::Other {
                                sig.returns.push(AbiParam::new(types::I64));
                                sig.returns.push(AbiParam::new(types::I64));
                            } else {
                                sig.returns.push(AbiParam::new(clif_ty(return_kind)));
                            }
                        }
                        let sig_ref = self.builder.import_signature(sig);
                        self.builder.ins().call_indirect(sig_ref, target, &call_args)
                    } else {
                        let fid = *self
                            .func_ids
                            .get(&rid)
                            .ok_or("call to non-compiled method")?;
                        let fref = self.module.declare_func_in_func(fid, self.builder.func);
                        let mut call_args = Vec::new();
                        for ((value, _kind), &param_kind_byte) in arg_values.iter().zip(
                            &callee.local_kinds[..callee.param_count as usize],
                        ) {
                            if JitKind::from_byte(param_kind_byte) == JitKind::Other {
                                call_args.push(self.builder.ins().load(
                                    types::I64,
                                    MemFlags::trusted(),
                                    *value,
                                    crate::any_abi::TAG_OFFSET,
                                ));
                                call_args.push(self.builder.ins().load(
                                    types::I64,
                                    MemFlags::trusted(),
                                    *value,
                                    crate::any_abi::PAYLOAD_OFFSET,
                                ));
                            } else {
                                call_args.push(*value);
                            }
                        }
                        self.builder.ins().call(fref, &call_args)
                    };
                    let results = self.builder.inst_results(inst).to_vec();
                    if callee.returns_value {
                        let kind = JitKind::from_byte(callee.return_kind);
                        if kind == JitKind::Other {
                            let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                                StackSlotKind::ExplicitSlot,
                                crate::any_abi::CARRIER_SIZE,
                                crate::any_abi::CARRIER_ALIGN_SHIFT,
                            ));
                            let carrier = self.builder.ins().stack_addr(types::I64, slot, 0);
                            self.builder.ins().store(
                                MemFlags::trusted(),
                                results[0],
                                carrier,
                                crate::any_abi::TAG_OFFSET,
                            );
                            self.builder.ins().store(
                                MemFlags::trusted(),
                                results[1],
                                carrier,
                                crate::any_abi::PAYLOAD_OFFSET,
                            );
                            stack.push((carrier, kind));
                        } else {
                            let r = results[0];
                            if matches!(kind, JitKind::Object | JitKind::String) {
                                self.builder.declare_value_needs_stack_map(r);
                            }
                            stack.push((r, kind));
                        }
                    }
                    // A direct/virtual callee may itself have propagated a throw
                    // (through a bridged call); check and unwind/route if so.
                    self.after_call_pending_check(
                        &mut block_sigs,
                        &mut stack,
                        offset_of_index[i],
                        index_of_offset,
                    )?;
                }
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
                        || instrs.get(i + 1).is_some_and(|next| next.opcode == Opcode::UnboxAny);
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
                        || (!store_object && !store_u64 && !store_i64 && !store_i32 && !store_narrow && !store_bool && value_kind != JitKind::Num(NumKind::F64))
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
                            JitKind::Num(NumKind::I8 | NumKind::U8) => self.builder.ins().uextend(types::I32, value),
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
                Opcode::Throw => self.op_throw(i, &mut stack, &mut block_sigs, offset_of_index, index_of_offset)?,
                Opcode::LoadFunction => {
                    // A no-closure function value is carried as its method rid.
                    let rid = match instr.operand {
                        Operand::Token(token) => token.rid(),
                        _ => return Err("ldftn operand".into()),
                    };
                    let value = self.builder.ins().iconst(types::I64, rid as i64);
                    stack.push((value, JitKind::Function));
                }
                Opcode::CallIndirect => {
                    // The signature method gives the call's arity and return kind.
                    let token = match instr.operand {
                        Operand::Token(token) => token,
                        _ => return Err("callindirect operand".into()),
                    };
                    let signature = read_body(
                        &self.zm.code,
                        self.zm.tables.method_defs[token.rid() as usize - 1].body_offset,
                    )
                    .map_err(|e| e.to_string())?;
                    let argc = signature.param_count as usize;
                    if stack.len() < argc + 1 {
                        return Err("stack underflow at callindirect".into());
                    }
                    let arg_values: Vec<Slot> = stack.split_off(stack.len() - argc);
                    let (fn_value, fn_kind) = stack.pop().ok_or("callindirect function value")?;
                    if fn_kind != JitKind::Function {
                        return Err("callindirect expects a function value".into());
                    }
                    let return_kind = signature
                        .returns_value
                        .then(|| JitKind::from_byte(signature.return_kind));
                    // The function value is carried as its method rid (the
                    // no-closure case: environment word 0). Dispatch through the
                    // handle-reconstructing indirect bridge, which reaches a
                    // captured environment once the producer slice can place one
                    // in the carrier; today the environment is always 0.
                    let environment = self.builder.ins().iconst(types::I64, 0);
                    self.emit_bridged_indirect_dispatch(
                        fn_value,
                        environment,
                        &arg_values,
                        return_kind,
                        &mut stack,
                    )?;
                    self.after_call_pending_check(
                        &mut block_sigs,
                        &mut stack,
                        offset_of_index[i],
                        index_of_offset,
                    )?;
                }
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

    fn bridge_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64)); // method rid
        sig.params.push(AbiParam::new(types::I64)); // packed args pointer
        sig.params.push(AbiParam::new(types::I64)); // argument count
        sig.returns.push(AbiParam::new(types::I64)); // packed result
        let id = self
            .module
            .declare_function("zun_jit_call_interp", Linkage::Import, &sig)
            .expect("declare zun_jit_call_interp");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn call_indirect_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64)); // handle identity (rid | reserved<<32)
        sig.params.push(AbiParam::new(types::I64)); // closure environment address, or 0
        sig.params.push(AbiParam::new(types::I64)); // packed args pointer
        sig.params.push(AbiParam::new(types::I64)); // argument count
        sig.returns.push(AbiParam::new(types::I64)); // packed result
        let id = self
            .module
            .declare_function("zun_jit_call_indirect", Linkage::Import, &sig)
            .expect("declare zun_jit_call_indirect");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    /// Lower a direct call to an ineligible, non-throwing callee as a bridged
    /// call: pack the arguments into a stack buffer in the shim ABI, call the
    /// host `zun_jit_call_interp`, and unpack the result. Object arguments and
    /// the result are kept in the call's stack map (the bridge runs the
    /// interpreter, a GC safepoint).
    fn emit_bridge_call(
        &mut self,
        rid: u32,
        arg_values: &[Slot],
        callee: &MethodBody<'_>,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let target = self.builder.ins().iconst(types::I64, rid as i64);
        let return_kind = callee
            .returns_value
            .then(|| JitKind::from_byte(callee.return_kind));
        self.emit_bridged_dispatch(target, arg_values, return_kind, stack)
    }

    /// Shared lowering for a bridged call to a method `target` (a Cranelift value
    /// holding the method rid — a constant for a direct bridged call, or the
    /// popped function value for `CallIndirect`): pack the args in the shim ABI,
    /// call `zun_jit_call_interp`, and unpack the result by `return_kind`.
    fn emit_bridged_dispatch(
        &mut self,
        target: Value,
        arg_values: &[Slot],
        return_kind: Option<JitKind>,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let argc = arg_values.len();
        let buf = if argc == 0 {
            self.builder.ins().iconst(types::I64, 0)
        } else {
            let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                (argc * 8) as u32,
                3,
            ));
            self.builder.ins().stack_addr(types::I64, slot, 0)
        };
        for (i, &(value, kind)) in arg_values.iter().enumerate() {
            if kind == JitKind::Other {
                return Err("bridge argument must not be an Any carrier".into());
            }
            if matches!(kind, JitKind::Object | JitKind::Array | JitKind::String) {
                self.builder.declare_value_needs_stack_map(value);
            }
            let packed = crate::shim::pack_result(&mut self.builder, value, kind);
            self.builder
                .ins()
                .store(MemFlags::trusted(), packed, buf, (i * 8) as i32);
        }
        let fref = self.bridge_fref();
        let argc_c = self.builder.ins().iconst(types::I64, argc as i64);
        let call = self.builder.ins().call(fref, &[target, buf, argc_c]);
        let result = self.builder.inst_results(call)[0];
        if let Some(kind) = return_kind {
            if kind == JitKind::Other {
                return Err("bridge return must not be an Any carrier".into());
            }
            let v = crate::shim::unpack_arg(&mut self.builder, result, kind);
            if matches!(kind, JitKind::Object | JitKind::Array) {
                self.builder.declare_value_needs_stack_map(v);
            }
            stack.push((v, kind));
        }
        Ok(())
    }

    /// Lowering for a `CallIndirect` on a concrete function value: pack the
    /// explicit arguments in the shim ABI and call `zun_jit_call_indirect` with
    /// the function value's two carrier words (`identity` = packed rid/bound
    /// discriminant, `environment` = closure-environment address or 0). The host
    /// reconstructs the `FunctionHandle` and dispatches on the current heap,
    /// carrying any captured environment the one-word rid path could not.
    ///
    /// The environment word is a managed reference live across this call (a GC
    /// safepoint), so it joins the call's stack map like any object argument.
    fn emit_bridged_indirect_dispatch(
        &mut self,
        identity: Value,
        environment: Value,
        arg_values: &[Slot],
        return_kind: Option<JitKind>,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let argc = arg_values.len();
        let buf = if argc == 0 {
            self.builder.ins().iconst(types::I64, 0)
        } else {
            let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                (argc * 8) as u32,
                3,
            ));
            self.builder.ins().stack_addr(types::I64, slot, 0)
        };
        for (i, &(value, kind)) in arg_values.iter().enumerate() {
            if kind == JitKind::Other {
                return Err("bridge argument must not be an Any carrier".into());
            }
            if matches!(kind, JitKind::Object | JitKind::Array | JitKind::String) {
                self.builder.declare_value_needs_stack_map(value);
            }
            let packed = crate::shim::pack_result(&mut self.builder, value, kind);
            self.builder
                .ins()
                .store(MemFlags::trusted(), packed, buf, (i * 8) as i32);
        }
        // The closure environment is a managed root across the bridged call.
        self.builder.declare_value_needs_stack_map(environment);
        let fref = self.call_indirect_fref();
        let argc_c = self.builder.ins().iconst(types::I64, argc as i64);
        let call = self.builder.ins().call(fref, &[identity, environment, buf, argc_c]);
        let result = self.builder.inst_results(call)[0];
        if let Some(kind) = return_kind {
            if kind == JitKind::Other {
                return Err("bridge return must not be an Any carrier".into());
            }
            let v = crate::shim::unpack_arg(&mut self.builder, result, kind);
            if matches!(kind, JitKind::Object | JitKind::Array) {
                self.builder.declare_value_needs_stack_map(v);
            }
            stack.push((v, kind));
        }
        Ok(())
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

    fn string_literal_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_literal", Linkage::Import, &sig)
            .expect("declare zun_jit_string_literal");
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

    fn string_last_index_of_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_last_index_of", Linkage::Import, &sig)
            .expect("declare zun_jit_string_last_index_of");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_count_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_count", Linkage::Import, &sig)
            .expect("declare zun_jit_string_count");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_compare_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_compare", Linkage::Import, &sig)
            .expect("declare zun_jit_string_compare");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_concat_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_concat", Linkage::Import, &sig)
            .expect("declare zun_jit_string_concat");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_substring_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_substring", Linkage::Import, &sig)
            .expect("declare zun_jit_string_substring");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_trim_fref(&mut self, name: &str) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function(
                match name {
                    "trimStart" => "zun_jit_string_trim_start",
                    "trimEnd" => "zun_jit_string_trim_end",
                    _ => "zun_jit_string_trim",
                },
                Linkage::Import,
                &sig,
            )
            .expect("declare string trim helper");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_transform_fref(&mut self, name: &str) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let symbol = match name {
            "toUpperCase" => "zun_jit_string_to_upper_case",
            _ => "zun_jit_string_to_lower_case",
        };
        let id = self
            .module
            .declare_function(symbol, Linkage::Import, &sig)
            .expect("declare string case helper");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_repeat_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_repeat", Linkage::Import, &sig)
            .expect("declare zun_jit_string_repeat");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_reverse_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_reverse", Linkage::Import, &sig)
            .expect("declare zun_jit_string_reverse");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_pad_fref(&mut self, name: &str) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        for _ in 0..3 { sig.params.push(AbiParam::new(types::I64)); }
        sig.returns.push(AbiParam::new(types::I64));
        let symbol = if name == "padEnd" { "zun_jit_string_pad_end" } else { "zun_jit_string_pad_start" };
        let id = self.module.declare_function(symbol, Linkage::Import, &sig).expect("declare string pad helper");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    fn string_replace_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        for _ in 0..3 { sig.params.push(AbiParam::new(types::I64)); }
        sig.returns.push(AbiParam::new(types::I64));
        let id = self.module.declare_function("zun_jit_string_replace", Linkage::Import, &sig).expect("declare zun_jit_string_replace");
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
