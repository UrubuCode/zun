//! ZIL → Cranelift IR translation for one numeric method body.
//!
//! The translator rebuilds a CFG from the linear ZIL: it finds basic-block
//! leaders (offset 0, every branch target, and the instruction after any
//! branch or `ret`), makes one Cranelift block per leader, and walks the code
//! emitting into the current block. The front-end guarantees the evaluation
//! stack is empty at every block boundary, so blocks carry no stack parameters
//! and an operand stack of Cranelift values is rebuilt within each block.
//!
//! Every value is `f64`; booleans are `1.0`/`0.0`. Comparisons select between
//! those constants, and `br.true`/`br.false` test against `0.0`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, Block, InstBuilder, Value};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{FuncId, Module as ClifModule};
use zun_il::{decode_all, Opcode, Operand};
use zun_module::MethodBody;

/// Translate `body` into `ctx.func`. Returns an error string on malformed input
/// (which should not happen for verified, eligible bodies).
pub fn translate_body(
    module: &mut dyn ClifModule,
    ctx: &mut Context,
    fb_ctx: &mut FunctionBuilderContext,
    func_ids: &HashMap<u32, FuncId>,
    body: &MethodBody<'_>,
) -> Result<(), String> {
    let instrs = decode_all(body.code).map_err(|e| e.to_string())?;

    // Map each instruction's byte offset to its index, and record each
    // instruction's end offset, so branch displacements resolve to indices.
    let mut offset_of_index = Vec::with_capacity(instrs.len());
    let mut index_of_offset = BTreeMap::new();
    {
        let mut off = 0usize;
        for (i, instr) in instrs.iter().enumerate() {
            index_of_offset.insert(off, i);
            offset_of_index.push(off);
            off += instr.encoded_len();
        }
        index_of_offset.insert(off, instrs.len()); // end sentinel
    }

    // Find block leaders.
    let leaders = find_leaders(&instrs, &offset_of_index, &index_of_offset)?;

    let mut builder = FunctionBuilder::new(&mut ctx.func, fb_ctx);

    // One Cranelift block per leader index.
    let mut blocks: BTreeMap<usize, Block> = BTreeMap::new();
    for &leader in &leaders {
        blocks.insert(leader, builder.create_block());
    }
    let entry = blocks[&0];
    builder.append_block_params_for_function_params(entry);

    // Locals are f64 variables; parameters initialize the leading slots.
    let mut vars = Vec::with_capacity(body.local_count as usize);
    for _ in 0..body.local_count as usize {
        vars.push(builder.declare_var(types::F64));
    }
    builder.switch_to_block(entry);
    let entry_params: Vec<Value> = builder.block_params(entry).to_vec();
    for (i, &p) in entry_params.iter().enumerate() {
        builder.def_var(vars[i], p);
    }
    // Zero-initialize non-parameter locals.
    let zero = builder.ins().f64const(0.0);
    for i in body.param_count as usize..body.local_count as usize {
        builder.def_var(vars[i], zero);
    }

    let mut t = Translator {
        builder,
        vars,
        blocks,
        func_ids,
        module,
    };
    t.run(&instrs, &offset_of_index, &index_of_offset)?;
    t.builder.finalize();
    Ok(())
}

/// Collect the instruction indices that begin basic blocks.
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
                // The instruction after any branch also starts a block.
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

struct Translator<'a> {
    builder: FunctionBuilder<'a>,
    vars: Vec<Variable>,
    blocks: BTreeMap<usize, Block>,
    func_ids: &'a HashMap<u32, FuncId>,
    module: &'a mut dyn ClifModule,
}

impl Translator<'_> {
    fn run(
        &mut self,
        instrs: &[zun_il::Instruction],
        offset_of_index: &[usize],
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<(), String> {
        let mut stack: Vec<Value> = Vec::new();

        for (i, instr) in instrs.iter().enumerate() {
            // Entering a new block: if the previous instruction fell through,
            // wire a jump; then switch. The operand stack is empty at bounds.
            // Index 0 is the entry block, already switched to before the loop.
            if i != 0 {
                if let Some(&block) = self.blocks.get(&i) {
                    // Wire the fall-through edge from the previous instruction
                    // unless it already terminated its block.
                    if !stack_terminated(instrs, i) {
                        self.builder.ins().jump(block, &[]);
                    }
                    self.builder.switch_to_block(block);
                    stack.clear();
                }
            }

            match instr.opcode {
                Opcode::Nop => {}
                Opcode::ConstF64 => {
                    let v = match instr.operand {
                        Operand::Float64(v) => v,
                        _ => return Err("const.f64 operand".into()),
                    };
                    let c = self.builder.ins().f64const(v);
                    stack.push(c);
                }
                Opcode::ConstTrue => {
                    let c = self.builder.ins().f64const(1.0);
                    stack.push(c);
                }
                Opcode::ConstFalse => {
                    let c = self.builder.ins().f64const(0.0);
                    stack.push(c);
                }
                Opcode::LoadLocal => {
                    let slot = local_index(&instr.operand)?;
                    let v = self.builder.use_var(self.vars[slot]);
                    stack.push(v);
                }
                Opcode::StoreLocal => {
                    let slot = local_index(&instr.operand)?;
                    let v = stack.pop().ok_or("stack underflow")?;
                    self.builder.def_var(self.vars[slot], v);
                }
                Opcode::Pop => {
                    stack.pop().ok_or("stack underflow")?;
                }
                Opcode::Dup => {
                    let v = *stack.last().ok_or("stack underflow")?;
                    stack.push(v);
                }
                Opcode::AddF64 => self.binop(&mut stack, |b, x, y| b.ins().fadd(x, y))?,
                Opcode::SubF64 => self.binop(&mut stack, |b, x, y| b.ins().fsub(x, y))?,
                Opcode::MulF64 => self.binop(&mut stack, |b, x, y| b.ins().fmul(x, y))?,
                Opcode::DivF64 => self.binop(&mut stack, |b, x, y| b.ins().fdiv(x, y))?,
                Opcode::CmpEq => self.cmp(&mut stack, FloatCC::Equal)?,
                Opcode::CmpNe => self.cmp(&mut stack, FloatCC::NotEqual)?,
                Opcode::CmpLt => self.cmp(&mut stack, FloatCC::LessThan)?,
                Opcode::CmpLe => self.cmp(&mut stack, FloatCC::LessThanOrEqual)?,
                Opcode::CmpGt => self.cmp(&mut stack, FloatCC::GreaterThan)?,
                Opcode::CmpGe => self.cmp(&mut stack, FloatCC::GreaterThanOrEqual)?,
                Opcode::Branch => {
                    let target = self.branch_target(instr, i, offset_of_index, index_of_offset)?;
                    self.builder.ins().jump(target, &[]);
                }
                Opcode::BranchTrue => {
                    let target = self.branch_target(instr, i, offset_of_index, index_of_offset)?;
                    let cond = stack.pop().ok_or("stack underflow")?;
                    let fallthrough = self.fallthrough_block(i)?;
                    let zero = self.builder.ins().f64const(0.0);
                    let is_nonzero = self.builder.ins().fcmp(FloatCC::NotEqual, cond, zero);
                    self.builder.ins().brif(is_nonzero, target, &[], fallthrough, &[]);
                }
                Opcode::BranchFalse => {
                    let target = self.branch_target(instr, i, offset_of_index, index_of_offset)?;
                    let cond = stack.pop().ok_or("stack underflow")?;
                    let fallthrough = self.fallthrough_block(i)?;
                    let zero = self.builder.ins().f64const(0.0);
                    let is_nonzero = self.builder.ins().fcmp(FloatCC::NotEqual, cond, zero);
                    // Branch is taken when the value is zero (false): so the
                    // non-zero edge is the fall-through.
                    self.builder.ins().brif(is_nonzero, fallthrough, &[], target, &[]);
                }
                Opcode::Ret => {
                    let v = stack.pop().unwrap_or_else(|| self.builder.ins().f64const(0.0));
                    self.builder.ins().return_(&[v]);
                }
                Opcode::Call => {
                    let callee_rid = match instr.operand {
                        Operand::Token(t) => t.rid(),
                        _ => return Err("call operand".into()),
                    };
                    let fid = *self
                        .func_ids
                        .get(&callee_rid)
                        .ok_or("call to a non-compiled method")?;
                    let fref = self.module.declare_func_in_func(fid, self.builder.func);
                    // Pop the arguments (the callee's param count equals the
                    // declared signature arity).
                    let arity = self.builder.func.dfg.ext_funcs[fref]
                        .signature;
                    let nargs = self.builder.func.dfg.signatures[arity].params.len();
                    if stack.len() < nargs {
                        return Err("stack underflow at call".into());
                    }
                    let args = stack.split_off(stack.len() - nargs);
                    let inst = self.builder.ins().call(fref, &args);
                    let result = self.builder.inst_results(inst)[0];
                    stack.push(result);
                }
                other => return Err(format!("opcode {other:?} is not in the numeric subset")),
            }
        }

        // Seal every block now that all edges are wired.
        self.builder.seal_all_blocks();
        Ok(())
    }

    fn binop(
        &mut self,
        stack: &mut Vec<Value>,
        f: impl Fn(&mut FunctionBuilder, Value, Value) -> Value,
    ) -> Result<(), String> {
        let b = stack.pop().ok_or("stack underflow")?;
        let a = stack.pop().ok_or("stack underflow")?;
        let r = f(&mut self.builder, a, b);
        stack.push(r);
        Ok(())
    }

    fn cmp(&mut self, stack: &mut Vec<Value>, cc: FloatCC) -> Result<(), String> {
        let b = stack.pop().ok_or("stack underflow")?;
        let a = stack.pop().ok_or("stack underflow")?;
        let flag = self.builder.ins().fcmp(cc, a, b);
        let one = self.builder.ins().f64const(1.0);
        let zero = self.builder.ins().f64const(0.0);
        let r = self.builder.ins().select(flag, one, zero);
        stack.push(r);
        Ok(())
    }

    fn branch_target(
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
        let &idx = index_of_offset
            .get(&target)
            .ok_or("branch target not on an instruction boundary")?;
        self.blocks
            .get(&idx)
            .copied()
            .ok_or_else(|| "branch target is not a block leader".into())
    }

    fn fallthrough_block(&self, i: usize) -> Result<Block, String> {
        self.blocks
            .get(&(i + 1))
            .copied()
            .ok_or_else(|| "conditional branch without a fall-through block".into())
    }
}

/// Whether the instruction before index `i` was a terminator (so the new block
/// at `i` has no implicit fall-through edge from it).
fn stack_terminated(instrs: &[zun_il::Instruction], i: usize) -> bool {
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
