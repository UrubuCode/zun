//! Explicit-stack backtracking VM.
//!
//! Every state mutation (capture slot, loop register) pushes an undo entry on
//! the backtrack stack, so popping the stack restores state exactly. Each
//! executed instruction costs one step against a shared budget.

use crate::ast::{canonicalize, is_line_terminator, is_word_char, AssertKind};
use crate::compile::{Inst, Program};
use crate::error::RegexError;

pub(crate) const NONE: usize = usize::MAX;

enum Bt {
    Branch { pc: usize, pos: usize },
    Cap { slot: usize, old: usize },
    Reg { idx: usize, old: usize },
}

pub(crate) struct Vm<'a> {
    pub prog: &'a Program,
    pub input: &'a [char],
    pub steps: u64,
    pub limit: u64,
}

impl Vm<'_> {
    fn eq(&self, a: char, b: char) -> bool {
        if self.prog.icase {
            canonicalize(a) == b
        } else {
            a == b
        }
    }

    fn word_at(&self, i: usize) -> bool {
        self.input.get(i).is_some_and(|&c| is_word_char(c))
    }

    /// Runs from `pc`/`pos` until `Match` / `LookEnd`; returns the end position.
    /// On failure all state changes are rolled back.
    pub fn run(
        &mut self,
        mut pc: usize,
        mut pos: usize,
        caps: &mut [usize],
        regs: &mut [usize],
    ) -> Result<Option<usize>, RegexError> {
        let prog = self.prog;
        let n = self.input.len();
        let mut stack: Vec<Bt> = Vec::new();
        loop {
            self.steps += 1;
            if self.steps > self.limit {
                return Err(RegexError::StepLimitExceeded);
            }
            let ok = match &prog.insts[pc] {
                Inst::Char { c, back } => {
                    if *back {
                        if pos > 0 && self.eq(self.input[pos - 1], *c) {
                            pos -= 1;
                            pc += 1;
                            true
                        } else {
                            false
                        }
                    } else if pos < n && self.eq(self.input[pos], *c) {
                        pos += 1;
                        pc += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::Any { back } => {
                    let at = if *back { pos.checked_sub(1) } else { (pos < n).then_some(pos) };
                    match at {
                        Some(i) if prog.dot_all || !is_line_terminator(self.input[i]) => {
                            pos = if *back { i } else { i + 1 };
                            pc += 1;
                            true
                        }
                        _ => false,
                    }
                }
                Inst::Class { idx, back } => {
                    let at = if *back { pos.checked_sub(1) } else { (pos < n).then_some(pos) };
                    match at {
                        Some(i) if prog.classes[*idx].matches(self.input[i], prog.icase) => {
                            pos = if *back { i } else { i + 1 };
                            pc += 1;
                            true
                        }
                        _ => false,
                    }
                }
                Inst::Split { first, second } => {
                    stack.push(Bt::Branch { pc: *second, pos });
                    pc = *first;
                    true
                }
                Inst::Jmp(t) => {
                    pc = *t;
                    true
                }
                Inst::Save(slot) => {
                    stack.push(Bt::Cap { slot: *slot, old: caps[*slot] });
                    caps[*slot] = pos;
                    pc += 1;
                    true
                }
                Inst::Assert(kind) => {
                    let ok = match kind {
                        AssertKind::Start => {
                            pos == 0 || (prog.multiline && is_line_terminator(self.input[pos - 1]))
                        }
                        AssertKind::End => {
                            pos == n || (prog.multiline && is_line_terminator(self.input[pos]))
                        }
                        AssertKind::WordBoundary => {
                            (pos > 0 && self.word_at(pos - 1)) != self.word_at(pos)
                        }
                        AssertKind::NotWordBoundary => {
                            (pos > 0 && self.word_at(pos - 1)) == self.word_at(pos)
                        }
                    };
                    if ok {
                        pc += 1;
                    }
                    ok
                }
                Inst::BackRef { group, back } => {
                    let (s, e) = (caps[2 * group], caps[2 * group + 1]);
                    if s == NONE || e == NONE || e < s {
                        pc += 1; // unset group matches empty
                        true
                    } else {
                        let len = e - s;
                        let start = if *back { pos.checked_sub(len) } else { Some(pos) };
                        match start {
                            Some(st) if st + len <= n => {
                                let same = (0..len).all(|k| {
                                    let (a, b) = (self.input[s + k], self.input[st + k]);
                                    a == b || (prog.icase && canonicalize(a) == canonicalize(b))
                                });
                                if same {
                                    pos = if *back { st } else { st + len };
                                    pc += 1;
                                }
                                same
                            }
                            _ => false,
                        }
                    }
                }
                Inst::Look { negate, end } => {
                    let snapshot = caps.to_vec();
                    let sub = self.run(pc + 1, pos, caps, regs)?;
                    match (sub.is_some(), *negate) {
                        (true, false) => {
                            for (slot, &old) in snapshot.iter().enumerate() {
                                if caps[slot] != old {
                                    stack.push(Bt::Cap { slot, old });
                                }
                            }
                            pc = *end;
                            true
                        }
                        (false, true) => {
                            pc = *end;
                            true
                        }
                        (true, true) => {
                            caps.copy_from_slice(&snapshot);
                            false
                        }
                        (false, false) => false,
                    }
                }
                Inst::LookEnd | Inst::Match => return Ok(Some(pos)),
                Inst::LoopInit { ctr } => {
                    stack.push(Bt::Reg { idx: *ctr, old: regs[*ctr] });
                    regs[*ctr] = 0;
                    pc += 1;
                    true
                }
                Inst::LoopTest { ctr, min, max, greedy, exit } => {
                    let count = regs[*ctr] as u64;
                    if count < *min as u64 {
                        pc += 1;
                    } else if count >= *max as u64 {
                        pc = *exit;
                    } else if *greedy {
                        stack.push(Bt::Branch { pc: *exit, pos });
                        pc += 1;
                    } else {
                        stack.push(Bt::Branch { pc: pc + 1, pos });
                        pc = *exit;
                    }
                    true
                }
                Inst::LoopBody { pos_reg, clear } => {
                    stack.push(Bt::Reg { idx: *pos_reg, old: regs[*pos_reg] });
                    regs[*pos_reg] = pos;
                    if let Some((lo, hi)) = clear {
                        for (k, cell) in caps[2 * lo..2 * hi + 2].iter_mut().enumerate() {
                            if *cell != NONE {
                                stack.push(Bt::Cap { slot: 2 * lo + k, old: *cell });
                                *cell = NONE;
                            }
                        }
                    }
                    pc += 1;
                    true
                }
                Inst::LoopEnd { ctr, pos_reg, min, test } => {
                    let count = regs[*ctr];
                    if pos == regs[*pos_reg] && count as u64 >= *min as u64 {
                        false // empty iteration beyond the minimum is rejected
                    } else {
                        stack.push(Bt::Reg { idx: *ctr, old: count });
                        regs[*ctr] = count + 1;
                        pc = *test;
                        true
                    }
                }
            };
            if ok {
                continue;
            }
            loop {
                match stack.pop() {
                    None => return Ok(None),
                    Some(Bt::Cap { slot, old }) => caps[slot] = old,
                    Some(Bt::Reg { idx, old }) => regs[idx] = old,
                    Some(Bt::Branch { pc: p, pos: q }) => {
                        pc = p;
                        pos = q;
                        break;
                    }
                }
            }
        }
    }
}
