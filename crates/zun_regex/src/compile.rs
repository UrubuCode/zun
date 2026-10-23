//! Lowers a [`RegexNode`] tree to a flat backtracking program.
//!
//! The matcher runs this program with an explicit backtrack stack, so native
//! recursion depth is bounded by lookaround nesting only (never by input).

use crate::ast::{canonicalize, AssertKind, CharClass, RegexNode};
use crate::flags::Flags;

/// One program instruction. `back` marks backward (lookbehind) matching.
#[derive(Clone, Debug)]
pub(crate) enum Inst {
    /// Literal char (already canonicalized when `ignore_case`).
    Char { c: char, back: bool },
    Any { back: bool },
    Class { idx: usize, back: bool },
    /// Try `first`, on failure `second`.
    Split { first: usize, second: usize },
    Jmp(usize),
    Save(usize),
    Assert(AssertKind),
    BackRef { group: usize, back: bool },
    /// Zero-width assertion; body follows at `pc + 1` and ends with `LookEnd`.
    Look { negate: bool, end: usize },
    LookEnd,
    /// Reset loop counter register `ctr` to 0.
    LoopInit { ctr: usize },
    /// Decide between another iteration (`pc + 1`) and `exit`.
    LoopTest { ctr: usize, min: u32, max: u32, greedy: bool, exit: usize },
    /// Start of an iteration: remember position, clear inner captures.
    LoopBody { pos_reg: usize, clear: Option<(usize, usize)> },
    /// End of an iteration: empty-iteration check, count, jump back to `test`.
    LoopEnd { ctr: usize, pos_reg: usize, min: u32, test: usize },
    Match,
}

/// A compiled program.
#[derive(Debug)]
pub(crate) struct Program {
    pub insts: Vec<Inst>,
    pub classes: Vec<CharClass>,
    pub group_count: usize,
    pub reg_count: usize,
    pub icase: bool,
    pub multiline: bool,
    pub dot_all: bool,
}

pub(crate) fn compile(node: &RegexNode, group_count: usize, flags: Flags) -> Program {
    let mut c = Compiler {
        insts: Vec::new(),
        classes: Vec::new(),
        nregs: 0,
        icase: flags.ignore_case,
    };
    c.node(node, false);
    c.insts.push(Inst::Match);
    Program {
        insts: c.insts,
        classes: c.classes,
        group_count,
        reg_count: c.nregs,
        icase: flags.ignore_case,
        multiline: flags.multiline,
        dot_all: flags.dot_all,
    }
}

struct Compiler {
    insts: Vec<Inst>,
    classes: Vec<CharClass>,
    nregs: usize,
    icase: bool,
}

impl Compiler {
    fn emit(&mut self, i: Inst) -> usize {
        self.insts.push(i);
        self.insts.len() - 1
    }

    fn node(&mut self, n: &RegexNode, back: bool) {
        match n {
            RegexNode::Empty => {}
            RegexNode::Char(c) => {
                let c = if self.icase { canonicalize(*c) } else { *c };
                self.emit(Inst::Char { c, back });
            }
            RegexNode::Any => {
                self.emit(Inst::Any { back });
            }
            RegexNode::Class(cls) => {
                self.classes.push(cls.clone());
                let idx = self.classes.len() - 1;
                self.emit(Inst::Class { idx, back });
            }
            RegexNode::Seq(items) => {
                if back {
                    items.iter().rev().for_each(|x| self.node(x, back));
                } else {
                    items.iter().for_each(|x| self.node(x, back));
                }
            }
            RegexNode::Alt(alts) => {
                let mut jumps = Vec::new();
                for (k, a) in alts.iter().enumerate() {
                    if k + 1 < alts.len() {
                        let split = self.emit(Inst::Split { first: 0, second: 0 });
                        self.node(a, back);
                        jumps.push(self.emit(Inst::Jmp(0)));
                        let next = self.insts.len();
                        self.insts[split] = Inst::Split { first: split + 1, second: next };
                    } else {
                        self.node(a, back);
                    }
                }
                let end = self.insts.len();
                for j in jumps {
                    self.insts[j] = Inst::Jmp(end);
                }
            }
            RegexNode::Group { index, node, .. } => {
                // Backward matching reaches the end of the group first.
                let (first, last) = if back { (2 * index + 1, 2 * index) } else { (2 * index, 2 * index + 1) };
                self.emit(Inst::Save(first));
                self.node(node, back);
                self.emit(Inst::Save(last));
            }
            RegexNode::Assertion(k) => {
                self.emit(Inst::Assert(*k));
            }
            RegexNode::BackRef(g) => {
                self.emit(Inst::BackRef { group: *g, back });
            }
            RegexNode::Look { behind, negate, node } => {
                let look = self.emit(Inst::Look { negate: *negate, end: 0 });
                self.node(node, *behind);
                self.emit(Inst::LookEnd);
                let end = self.insts.len();
                self.insts[look] = Inst::Look { negate: *negate, end };
            }
            RegexNode::Repeat { node, min, max, greedy } => {
                if *max == Some(0) {
                    return;
                }
                let ctr = self.nregs;
                self.nregs += 2;
                self.emit(Inst::LoopInit { ctr });
                let test = self.emit(Inst::LoopTest {
                    ctr,
                    min: *min,
                    max: max.unwrap_or(u32::MAX),
                    greedy: *greedy,
                    exit: 0,
                });
                self.emit(Inst::LoopBody { pos_reg: ctr + 1, clear: node.group_range() });
                self.node(node, back);
                self.emit(Inst::LoopEnd { ctr, pos_reg: ctr + 1, min: *min, test });
                let exit = self.insts.len();
                self.insts[test] = Inst::LoopTest {
                    ctr,
                    min: *min,
                    max: max.unwrap_or(u32::MAX),
                    greedy: *greedy,
                    exit,
                };
            }
        }
    }
}
