//! Explicit-stack backtracking VM.
//!
//! Every state mutation (capture slot, loop register) pushes an undo entry on
//! the backtrack stack, so popping the stack restores state exactly. Each
//! executed instruction costs one step against a shared budget.

use crate::ast::{canonicalize, is_line_terminator, is_word_char, AssertKind};
use crate::compile::{Inst, Program, RunAtom};
use crate::error::RegexError;

pub(crate) const NONE: usize = usize::MAX;

enum Bt {
    Branch { pc: usize, pos: usize },
    Cap { slot: usize, old: usize },
    Reg { idx: usize, old: usize },
    /// Greedy fast-run give-back: on pop, resume the loop exit having consumed
    /// one fewer atom (`count - 1`), re-pushing while still above `min`.
    Run { init: usize, start: usize, count: u32 },
    /// Lazy fast-run take-more: on pop, resume having consumed one more atom
    /// (`count + 1`), re-pushing while another atom is available below `max`.
    RunLazy { init: usize, start: usize, count: u32 },
}

pub(crate) struct Vm<'a> {
    pub prog: &'a Program,
    pub input: &'a [char],
    pub steps: u64,
    pub limit: u64,
    /// Whether the bare single-atom loop fast path is enabled. Always true in
    /// production; the test-only reference scan disables it to validate the
    /// fast path against the per-instruction backtracker.
    fast: bool,
    /// Backtrack stack, reused across start positions and reentrant runs.
    stack: Vec<Bt>,
    /// Capture snapshots for lookaround, used as a reusable stack of slices
    /// (avoids a fresh `Vec` allocation per lookaround evaluation).
    snap: Vec<usize>,
}

impl<'a> Vm<'a> {
    pub fn new(prog: &'a Program, input: &'a [char], limit: u64) -> Vm<'a> {
        Vm::with_fast(prog, input, limit, true)
    }

    pub fn with_fast(prog: &'a Program, input: &'a [char], limit: u64, fast: bool) -> Vm<'a> {
        Vm { prog, input, steps: 0, limit, fast, stack: Vec::new(), snap: Vec::new() }
    }

    /// Resets per-attempt scratch (counters stay; buffers keep their capacity).
    pub fn reset(&mut self) {
        self.stack.clear();
        self.snap.clear();
    }

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

    /// Whether the fast-run `atom` consumes the (forward) char at index `i`.
    #[inline]
    fn atom_at(&self, atom: RunAtom, i: usize) -> bool {
        let Some(&c) = self.input.get(i) else { return false };
        match atom {
            RunAtom::Char(ch) => self.eq(c, ch),
            RunAtom::Any => self.prog.dot_all || !is_line_terminator(c),
            RunAtom::Class(idx) => self.prog.classes[idx].matches(c, self.prog.icase),
        }
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
        // Base of this (possibly reentrant) run's region on the shared stack.
        let base = self.stack.len();
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
                    self.stack.push(Bt::Branch { pc: *second, pos });
                    pc = *first;
                    true
                }
                Inst::Jmp(t) => {
                    pc = *t;
                    true
                }
                Inst::Save(slot) => {
                    self.stack.push(Bt::Cap { slot: *slot, old: caps[*slot] });
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
                    // Snapshot captures onto the reusable `snap` buffer instead
                    // of a fresh Vec. The recursive run pushes/pops its own
                    // region above `snap_base`, leaving ours intact.
                    let snap_base = self.snap.len();
                    self.snap.extend_from_slice(caps);
                    let sub = self.run(pc + 1, pos, caps, regs)?;
                    let matched = sub.is_some();
                    let result = match (matched, *negate) {
                        (true, false) => {
                            // Keep the lookahead's captures; record undo entries.
                            for slot in 0..caps.len() {
                                let old = self.snap[snap_base + slot];
                                if caps[slot] != old {
                                    self.stack.push(Bt::Cap { slot, old });
                                }
                            }
                            pc = *end;
                            true
                        }
                        (false, true) => {
                            // Sub failed: it already rolled captures back to the
                            // snapshot via its own backtracking.
                            pc = *end;
                            true
                        }
                        (true, true) => {
                            // Negative lookahead matched: undo its captures.
                            for slot in 0..caps.len() {
                                caps[slot] = self.snap[snap_base + slot];
                            }
                            false
                        }
                        (false, false) => false,
                    };
                    self.snap.truncate(snap_base);
                    result
                }
                Inst::LookEnd | Inst::Match => {
                    // Success: drop any leftover branch points from this run's
                    // region (captures stay as the match result).
                    self.stack.truncate(base);
                    return Ok(Some(pos));
                }
                Inst::LoopInit { ctr } => {
                    let fast = self.fast.then(|| prog.fast_runs[pc]).flatten();
                    if let Some(fr) = fast {
                        // Fast path: a bare single-atom loop. Scan the run in a
                        // tight loop and leave a single backtrack entry carrying
                        // the run bounds, instead of 4 instructions per char.
                        let start = pos;
                        if fr.greedy {
                            // Consume as many as possible, up to `max`.
                            let mut k = 0u32;
                            while k < fr.max && self.atom_at(fr.atom, start + k as usize) {
                                k += 1;
                            }
                            // Charge steps proportional to the scanned run so the
                            // budget still bounds pathological inputs.
                            self.steps += k as u64;
                            if self.steps > self.limit {
                                return Err(RegexError::StepLimitExceeded);
                            }
                            if k < fr.min {
                                false // cannot satisfy the minimum
                            } else {
                                pos = start + k as usize;
                                if k > fr.min {
                                    self.stack.push(Bt::Run { init: pc, start, count: k });
                                }
                                pc = fr.exit;
                                true
                            }
                        } else {
                            // Lazy: consume exactly `min`, defer the rest.
                            let mut k = 0u32;
                            while k < fr.min && self.atom_at(fr.atom, start + k as usize) {
                                k += 1;
                            }
                            self.steps += fr.min as u64;
                            if self.steps > self.limit {
                                return Err(RegexError::StepLimitExceeded);
                            }
                            if k < fr.min {
                                false // not enough atoms for the minimum
                            } else {
                                pos = start + fr.min as usize;
                                if fr.min < fr.max && self.atom_at(fr.atom, pos) {
                                    self.stack.push(Bt::RunLazy {
                                        init: pc,
                                        start,
                                        count: fr.min,
                                    });
                                }
                                pc = fr.exit;
                                true
                            }
                        }
                    } else {
                        self.stack.push(Bt::Reg { idx: *ctr, old: regs[*ctr] });
                        regs[*ctr] = 0;
                        pc += 1;
                        true
                    }
                }
                Inst::LoopTest { ctr, min, max, greedy, exit } => {
                    let count = regs[*ctr] as u64;
                    if count < *min as u64 {
                        pc += 1;
                    } else if count >= *max as u64 {
                        pc = *exit;
                    } else if *greedy {
                        self.stack.push(Bt::Branch { pc: *exit, pos });
                        pc += 1;
                    } else {
                        self.stack.push(Bt::Branch { pc: pc + 1, pos });
                        pc = *exit;
                    }
                    true
                }
                Inst::LoopBody { pos_reg, clear } => {
                    self.stack.push(Bt::Reg { idx: *pos_reg, old: regs[*pos_reg] });
                    regs[*pos_reg] = pos;
                    if let Some((lo, hi)) = clear {
                        for (k, cell) in caps[2 * lo..2 * hi + 2].iter_mut().enumerate() {
                            if *cell != NONE {
                                self.stack.push(Bt::Cap { slot: 2 * lo + k, old: *cell });
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
                        self.stack.push(Bt::Reg { idx: *ctr, old: count });
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
                if self.stack.len() == base {
                    // Exhausted this run's region: all state above `base` has
                    // been rolled back; captures are restored to run entry.
                    return Ok(None);
                }
                match self.stack.pop().unwrap() {
                    Bt::Cap { slot, old } => caps[slot] = old,
                    Bt::Reg { idx, old } => regs[idx] = old,
                    Bt::Branch { pc: p, pos: q } => {
                        pc = p;
                        pos = q;
                        break;
                    }
                    Bt::Run { init, start, count } => {
                        // Greedy give-back: try one fewer atom (still >= min, as
                        // this entry was only pushed while count > min).
                        let fr = prog.fast_runs[init].unwrap();
                        let new = count - 1;
                        if new > fr.min {
                            self.stack.push(Bt::Run { init, start, count: new });
                        }
                        pos = start + new as usize;
                        pc = fr.exit;
                        break;
                    }
                    Bt::RunLazy { init, start, count } => {
                        // Lazy take-more: consume one further atom (the push
                        // guaranteed it matches), then offer the next if any.
                        let fr = prog.fast_runs[init].unwrap();
                        let new = count + 1;
                        if new < fr.max && self.atom_at(fr.atom, start + new as usize) {
                            self.stack.push(Bt::RunLazy { init, start, count: new });
                        }
                        pos = start + new as usize;
                        pc = fr.exit;
                        break;
                    }
                }
            }
        }
    }
}
