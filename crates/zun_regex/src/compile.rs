//! Lowers a [`RegexNode`] tree to a flat backtracking program.
//!
//! The matcher runs this program with an explicit backtrack stack, so native
//! recursion depth is bounded by lookaround nesting only (never by input).

use crate::ast::{canonicalize, is_line_terminator, AssertKind, CharClass, RegexNode};
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
    /// Conservative first-position prefilter (FindFirstChar model).
    pub prefilter: Prefilter,
}

/// One member of a first-character set.
#[derive(Clone, Debug)]
pub(crate) enum SetMember {
    /// A required literal char (already canonicalized when `icase`).
    Char(char),
    /// A character class the first char may belong to.
    Class(CharClass),
}

/// A first-character set: a fast ASCII bitmask plus the exact members for a
/// correctness fallback on non-ASCII input.
#[derive(Clone, Debug)]
pub(crate) struct FirstSet {
    /// Bit `i` set iff ASCII char `i` is a member (`i` in `0..128`).
    ascii_mask: u128,
    members: Vec<SetMember>,
    icase: bool,
}

impl FirstSet {
    /// The exact first-set members (used to synthesize a candidate test in
    /// [`crate::zil`], so the generated matcher's prefilter matches this one).
    pub(crate) fn members(&self) -> &[SetMember] {
        &self.members
    }

    fn test(&self, x: char) -> bool {
        self.members.iter().any(|m| match m {
            SetMember::Char(c) => {
                if self.icase {
                    canonicalize(x) == *c
                } else {
                    x == *c
                }
            }
            SetMember::Class(cls) => cls.matches(x, self.icase),
        })
    }

    #[inline]
    fn contains(&self, x: char) -> bool {
        let u = x as u32;
        if u < 128 {
            (self.ascii_mask >> u) & 1 != 0
        } else {
            self.test(x)
        }
    }
}

/// A conservative first-position filter. It must never under-approximate: a
/// `None` result reproduces the naive "try every position" behavior exactly.
#[derive(Clone, Debug)]
pub(crate) enum Prefilter {
    /// Try every position in `from..=len` (no information; current behavior).
    None,
    /// Only position 0 is viable (non-multiline leading `^`).
    Anchored,
    /// Position 0 and every index just after a line terminator (multiline `^`).
    Multiline,
    /// A required leading ASCII literal (non-icase); scanned with `memchr`.
    Byte(u8, char),
    /// A required leading first-character set.
    Set(FirstSet),
}

impl Prefilter {
    /// Smallest candidate start position `>= from`, or `None` if no viable
    /// position remains. The returned index is always `<= input.len()`.
    pub(crate) fn next(&self, input: &[char], from: usize) -> Option<usize> {
        let len = input.len();
        match self {
            Prefilter::None => (from <= len).then_some(from),
            Prefilter::Anchored => (from == 0).then_some(0),
            Prefilter::Multiline => {
                let mut i = from;
                loop {
                    if i > len {
                        return None;
                    }
                    if i == 0 || is_line_terminator(input[i - 1]) {
                        return Some(i);
                    }
                    i += 1;
                }
            }
            Prefilter::Byte(b, c) => {
                // View the char slice as raw bytes and let `memchr` find the
                // low byte of the target; verify against the full char to reject
                // false low-byte collisions (little-endian scalar layout).
                let bytes = unsafe {
                    std::slice::from_raw_parts(input.as_ptr() as *const u8, len * 4)
                };
                let mut i = from;
                while i < len {
                    match memchr::memchr(*b, &bytes[i * 4..]) {
                        None => return None,
                        Some(off) => {
                            let abs = i * 4 + off;
                            let idx = abs / 4;
                            if abs % 4 == 0 && input[idx] == *c {
                                return Some(idx);
                            }
                            i = idx + 1;
                        }
                    }
                }
                None
            }
            Prefilter::Set(fs) => {
                let mut i = from;
                while i < len {
                    if fs.contains(input[i]) {
                        return Some(i);
                    }
                    i += 1;
                }
                None
            }
        }
    }
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
    let prefilter = compute_prefilter(&c.insts, &c.classes, flags);
    Program {
        insts: c.insts,
        classes: c.classes,
        group_count,
        reg_count: c.nregs,
        icase: flags.ignore_case,
        multiline: flags.multiline,
        dot_all: flags.dot_all,
        prefilter,
    }
}

/// Result of analyzing the first required atom of a (sub)program.
enum FirstReq {
    /// Non-multiline leading `^`.
    Anchored,
    /// Multiline leading `^`.
    Multiline,
    /// The first consumed char must satisfy one of these members.
    Set(Vec<SetMember>),
    /// Cannot prove a required first char — fall back to no prefilter.
    Bail,
}

/// Computes a conservative first-position prefilter by walking the program from
/// pc 0. Correctness first: it must never skip a valid leftmost match, so any
/// uncertainty becomes [`Prefilter::None`] (try every position).
fn compute_prefilter(insts: &[Inst], classes: &[CharClass], flags: Flags) -> Prefilter {
    match analyze(insts, classes, flags, 0, 0) {
        FirstReq::Anchored => Prefilter::Anchored,
        FirstReq::Multiline => Prefilter::Multiline,
        FirstReq::Bail => Prefilter::None,
        FirstReq::Set(members) if members.is_empty() => Prefilter::None,
        FirstReq::Set(members) => {
            // A single non-icase ASCII literal gets the fast `memchr` path.
            if !flags.ignore_case && members.len() == 1 {
                if let SetMember::Char(c) = members[0] {
                    if c.is_ascii() {
                        return Prefilter::Byte(c as u8, c);
                    }
                }
            }
            let mut ascii_mask: u128 = 0;
            let set = FirstSet { ascii_mask: 0, members, icase: flags.ignore_case };
            for i in 0u32..128 {
                if set.test(char::from(i as u8)) {
                    ascii_mask |= 1u128 << i;
                }
            }
            Prefilter::Set(FirstSet { ascii_mask, ..set })
        }
    }
}

/// Walks instructions starting at `pc`, skipping zero-width atoms, until the
/// first consuming atom. `depth` bounds recursion through `Split`/`Jmp`.
fn analyze(insts: &[Inst], classes: &[CharClass], flags: Flags, pc: usize, depth: u32) -> FirstReq {
    if depth > 64 || pc >= insts.len() {
        return FirstReq::Bail;
    }
    match &insts[pc] {
        // Leading `^`: strongest possible filter.
        Inst::Assert(AssertKind::Start) => {
            if flags.multiline {
                FirstReq::Multiline
            } else {
                FirstReq::Anchored
            }
        }
        // Other zero-width assertions only further restrict a match; skip them.
        Inst::Assert(_) => analyze(insts, classes, flags, pc + 1, depth + 1),
        // Capture open/close are zero-width.
        Inst::Save(_) => analyze(insts, classes, flags, pc + 1, depth + 1),
        Inst::Jmp(t) => analyze(insts, classes, flags, *t, depth + 1),
        // First consumed atom: forward only.
        Inst::Char { c, back: false } => FirstReq::Set(vec![SetMember::Char(*c)]),
        Inst::Class { idx, back: false } => {
            FirstReq::Set(vec![SetMember::Class(classes[*idx].clone())])
        }
        // Alternation: union the first sets of both branches. If either branch
        // may match empty or carries an anchor, we cannot form a char set.
        Inst::Split { first, second } => {
            let a = analyze(insts, classes, flags, *first, depth + 1);
            let b = analyze(insts, classes, flags, *second, depth + 1);
            match (a, b) {
                (FirstReq::Set(mut x), FirstReq::Set(y)) => {
                    x.extend(y);
                    FirstReq::Set(x)
                }
                _ => FirstReq::Bail,
            }
        }
        // Loop entry: step to the test.
        Inst::LoopInit { .. } => analyze(insts, classes, flags, pc + 1, depth + 1),
        // A loop whose minimum is 0 can match empty here -> no required char.
        Inst::LoopTest { min, .. } => {
            if *min >= 1 {
                analyze(insts, classes, flags, pc + 1, depth + 1)
            } else {
                FirstReq::Bail
            }
        }
        Inst::LoopBody { .. } => analyze(insts, classes, flags, pc + 1, depth + 1),
        // Everything else (Any, BackRef, Look/LookEnd, backward atoms, Match,
        // LoopEnd) either matches too broadly or permits an empty prefix.
        _ => FirstReq::Bail,
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
