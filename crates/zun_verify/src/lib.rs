//! The ZIL verifier.
//!
//! Before a method body is interpreted or JIT-compiled, the verifier proves a
//! set of structural invariants so neither the interpreter nor codegen has to
//! guard against malformed code:
//!
//! - every opcode decodes and every branch lands on an instruction boundary in
//!   range;
//! - the evaluation stack never underflows, and its height is identical on
//!   every path that reaches a given instruction (consistency at join points);
//! - the stack is empty when control leaves via `ret` (phase-1 rule; values are
//!   returned through a dedicated slot later) — except a single return value,
//!   which `ret` may leave;
//! - local-slot accesses are within the frame's declared `local_count`.
//!
//! This is a stack-*height* analysis; full type tracking (StackValueKind) lands
//! with the type system. It grows alongside the IL: every opcode must declare a
//! [`StackEffect`], so a new instruction cannot be added without teaching the
//! verifier about it.
//!
//! [`StackEffect`]: zun_il::StackEffect

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use zun_il::{decode_one, Opcode, Operand, StackEffect};

/// Describes the method being verified.
#[derive(Debug, Clone, Copy)]
pub struct MethodInfo {
    /// Number of local slots (parameters included).
    pub local_count: u16,
    /// Whether the method returns a value (so `ret` expects height 1, not 0).
    pub returns_value: bool,
}

/// A verification failure, naming the byte offset where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError {
    /// Byte offset of the offending instruction (or branch target).
    pub offset: usize,
    /// What went wrong.
    pub kind: ErrorKind,
}

/// The kinds of verification failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// An opcode byte did not decode, or an operand was truncated.
    BadEncoding,
    /// A branch target is out of range or not on an instruction boundary.
    BadBranchTarget(usize),
    /// An instruction would pop more values than are on the stack.
    StackUnderflow,
    /// Two paths reach the same instruction with different stack heights.
    StackMismatch {
        /// Height seen on the first path.
        expected: usize,
        /// Height seen on a later path.
        found: usize,
    },
    /// A local slot index is >= `local_count`.
    LocalOutOfRange(u16),
    /// A call's resolved arity is unknown (no signature provided).
    UnknownCallArity,
    /// `ret` left the stack at the wrong height.
    BadReturnHeight {
        /// Height the method's return convention requires.
        expected: usize,
        /// Height actually present.
        found: usize,
    },
    /// Control reached the end of the code without a terminator.
    FallsOffEnd,
    /// A handler region is malformed or crosses another region.
    InvalidHandlerRegion,
    /// A catch token is not an `Error` type or one of its derived types.
    InvalidCatchType,
}

/// Verify a framed method body, including its exception regions.
///
/// `is_error_type` is supplied by the metadata/type system because the
/// verifier deliberately does not own class inheritance information.
pub fn verify_body(
    body: &zun_module::MethodBody<'_>,
    info: MethodInfo,
    resolve: &CallResolver<'_>,
    is_error_type: &dyn Fn(zun_metadata::Token) -> bool,
) -> Result<(), VerifyError> {
    verify(body.code, info, resolve)?;
    let mut boundaries = std::collections::BTreeSet::new();
    let mut pos = 0usize;
    while pos < body.code.len() {
        boundaries.insert(pos);
        let (_, size) = decode_one(&body.code[pos..]).map_err(|_| VerifyError {
            offset: pos,
            kind: ErrorKind::BadEncoding,
        })?;
        pos += size;
    }
    boundaries.insert(body.code.len());

    for handler in &body.handlers {
        let valid_bounds = boundaries.contains(&(handler.try_start as usize))
            && boundaries.contains(&(handler.try_end as usize))
            && boundaries.contains(&(handler.handler_start as usize))
            && boundaries.contains(&(handler.handler_end as usize));
        if !valid_bounds
            || handler.try_start >= handler.try_end
            || handler.handler_start >= handler.handler_end
            || handler.try_end as usize > body.code.len()
            || handler.handler_end as usize > body.code.len()
        {
            return Err(VerifyError {
                offset: handler.try_start as usize,
                kind: ErrorKind::InvalidHandlerRegion,
            });
        }
        if let Some(token) = handler.catch_type {
            if !is_error_type(token) {
                return Err(VerifyError {
                    offset: handler.handler_start as usize,
                    kind: ErrorKind::InvalidCatchType,
                });
            }
        }
    }
    // A region crosses another when it starts strictly inside it and ends
    // strictly outside it. Sort by start and sweep the active end offsets; a
    // proper crossing is an active end strictly inside the current region.
    // This replaces the previous pairwise O(H²) scan with O(H log H).
    let try_crossing = first_crossing(
        body.handlers
            .iter()
            .enumerate()
            .map(|(index, handler)| (index, handler.try_start, handler.try_end)),
    );
    let handler_crossing = first_crossing(
        body.handlers
            .iter()
            .enumerate()
            .map(|(index, handler)| (index, handler.handler_start, handler.handler_end)),
    );
    if let Some(other) = try_crossing.or(handler_crossing) {
        return Err(VerifyError {
            offset: body.handlers[other].try_start as usize,
            kind: ErrorKind::InvalidHandlerRegion,
        });
    }
    Ok(())
}

/// Return the index of the later-declared handler in a pair of crossing
/// half-open `(start, end)` regions, if any. Regions are identified by their
/// declaration index so the reported handler matches the old scan's
/// `other.try_start` (the later-declared one).
fn first_crossing(regions: impl Iterator<Item = (usize, u32, u32)>) -> Option<usize> {
    let mut sorted: Vec<(usize, u32, u32)> = regions.collect();
    sorted.sort_by_key(|&(_, start, _)| start);
    let mut active: BTreeSet<(u32, usize)> = BTreeSet::new();
    let mut index = 0;
    while index < sorted.len() {
        let group_start = sorted[index].1;
        // Drop regions that end at or before this start: they cannot cross
        // this or any later region.
        while let Some(&entry) = active.first() {
            if entry.0 <= group_start {
                active.remove(&entry);
            } else {
                break;
            }
        }
        let mut next = index;
        while next < sorted.len() && sorted[next].1 == group_start {
            let (orig, _, end) = sorted[next];
            let lower = Bound::Excluded((group_start, usize::MAX));
            let upper = Bound::Excluded((end, 0));
            if let Some(&(_, earlier)) = active.range((lower, upper)).next() {
                return Some(earlier.max(orig));
            }
            next += 1;
        }
        // Equal-start regions cannot cross each other, so insert the whole
        // group only after every member has been queried.
        for &(orig, _, end) in &sorted[index..next] {
            active.insert((end, orig));
        }
        index = next;
    }
    None
}

/// Resolves a call instruction's arity: `(args_popped, results_pushed)`.
///
/// Returns `None` when the callee is unknown, which the verifier reports as
/// [`ErrorKind::UnknownCallArity`]. The signature is `(opcode, operand)` so one
/// resolver can serve `call`, `callvirt`, `newobj`, and `call.builtin`.
pub type CallResolver<'a> = dyn Fn(Opcode, &Operand) -> Option<(u8, u8)> + 'a;

/// Verify `code` under `info`, resolving call arities with `resolve`.
pub fn verify(
    code: &[u8],
    info: MethodInfo,
    resolve: &CallResolver<'_>,
) -> Result<(), VerifyError> {
    // Pass 1: decode every instruction, recording the stack effect and the set
    // of valid instruction-start offsets.
    let mut instrs: Vec<DecodedInstr> = Vec::new();
    let mut offset_to_index: BTreeMap<usize, usize> = BTreeMap::new();
    let mut pos = 0usize;
    while pos < code.len() {
        offset_to_index.insert(pos, instrs.len());
        let (instr, size) = decode_one(&code[pos..]).map_err(|_| VerifyError {
            offset: pos,
            kind: ErrorKind::BadEncoding,
        })?;
        instrs.push(DecodedInstr {
            offset: pos,
            end: pos + size,
            opcode: instr.opcode,
            operand: instr.operand,
        });
        pos += size;
    }
    let code_len = code.len();

    // Pass 2: resolve branch targets to instruction indices.
    let targets = resolve_targets(&instrs, &offset_to_index, code_len)?;

    // Pass 3: abstract stack-height propagation from the entry.
    verify_stack(&instrs, &targets, info, resolve)
}

struct DecodedInstr {
    offset: usize,
    end: usize,
    opcode: Opcode,
    operand: Operand,
}

/// For each instruction index, its successor index on branch (if any).
struct Targets {
    /// `branch_to[i]` = Some(target index) for a branch instruction.
    branch_to: Vec<Option<usize>>,
}

fn resolve_targets(
    instrs: &[DecodedInstr],
    offset_to_index: &BTreeMap<usize, usize>,
    code_len: usize,
) -> Result<Targets, VerifyError> {
    let mut branch_to = vec![None; instrs.len()];
    for (i, instr) in instrs.iter().enumerate() {
        if let Operand::Branch(disp) = instr.operand {
            let target = instr.end as i64 + disp as i64;
            if target < 0 || target as usize > code_len {
                return Err(VerifyError {
                    offset: instr.offset,
                    kind: ErrorKind::BadBranchTarget(target.max(0) as usize),
                });
            }
            let target = target as usize;
            let &idx = offset_to_index.get(&target).ok_or(VerifyError {
                offset: instr.offset,
                kind: ErrorKind::BadBranchTarget(target),
            })?;
            branch_to[i] = Some(idx);
        }
    }
    Ok(Targets { branch_to })
}

fn verify_stack(
    instrs: &[DecodedInstr],
    targets: &Targets,
    info: MethodInfo,
    resolve: &CallResolver<'_>,
) -> Result<(), VerifyError> {
    // Stack height on entry to each instruction; None = not yet reached.
    let mut entry_height: Vec<Option<usize>> = vec![None; instrs.len()];
    if instrs.is_empty() {
        return Err(VerifyError {
            offset: 0,
            kind: ErrorKind::FallsOffEnd,
        });
    }
    // Worklist of (instruction index, stack height on entry).
    let mut work = vec![(0usize, 0usize)];

    while let Some((idx, height_in)) = work.pop() {
        // Consistency at join points: a second visit must agree.
        if let Some(existing) = entry_height[idx] {
            if existing != height_in {
                return Err(VerifyError {
                    offset: instrs[idx].offset,
                    kind: ErrorKind::StackMismatch {
                        expected: existing,
                        found: height_in,
                    },
                });
            }
            continue;
        }
        entry_height[idx] = Some(height_in);

        let instr = &instrs[idx];
        let mut height = height_in;

        // Validate local-slot operands.
        if let Operand::Local(slot) = instr.operand {
            if slot >= info.local_count {
                return Err(VerifyError {
                    offset: instr.offset,
                    kind: ErrorKind::LocalOutOfRange(slot),
                });
            }
        }

        // Apply the stack effect.
        let (pops, pushes) = match instr.opcode.stack_effect() {
            StackEffect::Fixed { pops, pushes } => (pops as usize, pushes as usize),
            StackEffect::Call => {
                let (pops, pushes) = resolve(instr.opcode, &instr.operand).ok_or(VerifyError {
                    offset: instr.offset,
                    kind: ErrorKind::UnknownCallArity,
                })?;
                (pops as usize, pushes as usize)
            }
            StackEffect::Terminator => {
                match instr.opcode {
                    Opcode::Ret => {
                        let expected = if info.returns_value { 1 } else { 0 };
                        if height != expected {
                            return Err(VerifyError {
                                offset: instr.offset,
                                kind: ErrorKind::BadReturnHeight {
                                    expected,
                                    found: height,
                                },
                            });
                        }
                    }
                    Opcode::Branch => {
                        let target = targets.branch_to[idx].expect("branch has a target");
                        work.push((target, height));
                    }
                    Opcode::Throw | Opcode::Rethrow => {
                        if height == 0 {
                            return Err(VerifyError {
                                offset: instr.offset,
                                kind: ErrorKind::StackUnderflow,
                            });
                        }
                    }
                    _ => unreachable!("unexpected terminator {:?}", instr.opcode),
                }
                // Terminators do not fall through.
                continue;
            }
        };

        if height < pops {
            return Err(VerifyError {
                offset: instr.offset,
                kind: ErrorKind::StackUnderflow,
            });
        }
        height = height - pops + pushes;

        // Conditional branch: both the taken and fall-through edges continue
        // with the post-effect height.
        if let Some(target) = targets.branch_to[idx] {
            work.push((target, height));
        }

        // Fall-through edge.
        let next = idx + 1;
        if next < instrs.len() {
            work.push((next, height));
        } else {
            // Ran past the last instruction without a terminator.
            return Err(VerifyError {
                offset: instr.end,
                kind: ErrorKind::FallsOffEnd,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
