//! Pattern string -> [`RegexNode`] parser (ECMAScript syntax incl. Annex B leniencies).

use crate::ast::{AssertKind, CharClass, ClassItem, RegexNode};
use crate::error::RegexParseError;

const MAX_DEPTH: usize = 200;

/// Result of parsing a pattern.
#[derive(Clone, Debug)]
pub struct ParsedPattern {
    /// The tree.
    pub node: RegexNode,
    /// Number of capturing groups (excluding group 0).
    pub group_count: usize,
    /// Named groups as `(name, index)` in source order.
    pub names: Vec<(String, usize)>,
}

/// Parses `pattern`.
pub fn parse(pattern: &str) -> Result<ParsedPattern, RegexParseError> {
    let s: Vec<char> = pattern.chars().collect();
    let (group_total, scanned_names) = prescan(&s);
    let mut p = Parser { s, i: 0, group_total, scanned_names, names: Vec::new(), next_group: 0 };
    let node = p.disjunction(0)?;
    if p.i < p.s.len() {
        return Err(p.err("unmatched ')'"));
    }
    Ok(ParsedPattern { node, group_count: p.next_group, names: p.names })
}

/// Counts capturing groups and collects group names without building a tree
/// (needed up front: `\N` meaning depends on the total group count, and
/// `\k<name>` may be a forward reference).
fn prescan(s: &[char]) -> (usize, Vec<(String, usize)>) {
    let mut count = 0;
    let mut names = Vec::new();
    let mut i = 0;
    let mut in_class = false;
    while i < s.len() {
        match s[i] {
            '\\' => i += 1,
            '[' => in_class = true,
            ']' => in_class = false,
            '(' if !in_class => {
                if s.get(i + 1) == Some(&'?') {
                    if s.get(i + 2) == Some(&'<') && !matches!(s.get(i + 3), Some('=') | Some('!')) {
                        count += 1;
                        let mut j = i + 3;
                        let mut name = String::new();
                        while j < s.len() && s[j] != '>' {
                            name.push(s[j]);
                            j += 1;
                        }
                        names.push((name, count));
                    }
                } else {
                    count += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (count, names)
}

struct Parser {
    s: Vec<char>,
    i: usize,
    group_total: usize,
    scanned_names: Vec<(String, usize)>,
    names: Vec<(String, usize)>,
    next_group: usize,
}

enum ClassAtom {
    Char(char),
    Set(ClassItem),
}

fn hex_val(c: char) -> Option<u32> {
    c.to_digit(16)
}

impl Parser {
    fn err(&self, msg: &str) -> RegexParseError {
        RegexParseError::new(msg, self.i)
    }

    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.s.get(self.i + off).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn disjunction(&mut self, depth: usize) -> Result<RegexNode, RegexParseError> {
        if depth > MAX_DEPTH {
            return Err(self.err("pattern nested too deeply"));
        }
        let mut alts = vec![self.alternative(depth)?];
        while self.eat('|') {
            alts.push(self.alternative(depth)?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap() } else { RegexNode::Alt(alts) })
    }

    fn alternative(&mut self, depth: usize) -> Result<RegexNode, RegexParseError> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            items.push(self.term(depth)?);
        }
        Ok(match items.len() {
            0 => RegexNode::Empty,
            1 => items.pop().unwrap(),
            _ => RegexNode::Seq(items),
        })
    }

    fn term(&mut self, depth: usize) -> Result<RegexNode, RegexParseError> {
        let c = self.peek().unwrap();
        let (atom, quantifiable) = match c {
            '^' => {
                self.i += 1;
                (RegexNode::Assertion(AssertKind::Start), false)
            }
            '$' => {
                self.i += 1;
                (RegexNode::Assertion(AssertKind::End), false)
            }
            '(' => self.group(depth)?,
            '.' => {
                self.i += 1;
                (RegexNode::Any, true)
            }
            '[' => (self.class()?, true),
            '\\' => self.atom_escape()?,
            '*' | '+' | '?' => return Err(self.err("nothing to repeat")),
            '{' => {
                let save = self.i;
                let is_q = self.braces_quantifier()?.is_some();
                self.i = save;
                if is_q {
                    return Err(self.err("nothing to repeat"));
                }
                self.i += 1;
                (RegexNode::Char('{'), true)
            }
            _ => {
                self.i += 1;
                (RegexNode::Char(c), true)
            }
        };
        match self.quantifier()? {
            None => Ok(atom),
            Some((min, max)) => {
                if !quantifiable {
                    return Err(self.err("nothing to repeat"));
                }
                let greedy = !self.eat('?');
                Ok(RegexNode::Repeat { node: Box::new(atom), min, max, greedy })
            }
        }
    }

    fn quantifier(&mut self) -> Result<Option<(u32, Option<u32>)>, RegexParseError> {
        match self.peek() {
            Some('*') => {
                self.i += 1;
                Ok(Some((0, None)))
            }
            Some('+') => {
                self.i += 1;
                Ok(Some((1, None)))
            }
            Some('?') => {
                self.i += 1;
                Ok(Some((0, Some(1))))
            }
            Some('{') => self.braces_quantifier(),
            _ => Ok(None),
        }
    }

    /// Parses `{m}`, `{m,}`, `{m,n}` at `{`. Returns `None` (cursor restored)
    /// when the text is not a valid quantifier, so `{` is a literal (Annex B).
    fn braces_quantifier(&mut self) -> Result<Option<(u32, Option<u32>)>, RegexParseError> {
        let save = self.i;
        self.i += 1;
        let min = match self.decimal() {
            Some(n) => n,
            None => {
                self.i = save;
                return Ok(None);
            }
        };
        let max = if self.eat(',') {
            if self.peek() == Some('}') {
                None
            } else {
                match self.decimal() {
                    Some(n) => Some(n),
                    None => {
                        self.i = save;
                        return Ok(None);
                    }
                }
            }
        } else {
            Some(min)
        };
        if !self.eat('}') {
            self.i = save;
            return Ok(None);
        }
        if let Some(m) = max {
            if m < min {
                return Err(self.err("numbers out of order in {} quantifier"));
            }
        }
        Ok(Some((min, max)))
    }

    fn decimal(&mut self) -> Option<u32> {
        let start = self.i;
        let mut n: u32 = 0;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            n = n.saturating_mul(10).saturating_add(d);
            self.i += 1;
        }
        if self.i == start {
            None
        } else {
            Some(n)
        }
    }

    fn group(&mut self, depth: usize) -> Result<(RegexNode, bool), RegexParseError> {
        self.i += 1; // '('
        let mut name = None;
        let mut look = None; // (behind, negate)
        let mut capturing = true;
        if self.eat('?') {
            match self.peek() {
                Some(':') => {
                    self.i += 1;
                    capturing = false;
                }
                Some('=') => {
                    self.i += 1;
                    look = Some((false, false));
                }
                Some('!') => {
                    self.i += 1;
                    look = Some((false, true));
                }
                Some('<') => match self.peek_at(1) {
                    Some('=') => {
                        self.i += 2;
                        look = Some((true, false));
                    }
                    Some('!') => {
                        self.i += 2;
                        look = Some((true, true));
                    }
                    _ => {
                        self.i += 1;
                        name = Some(self.group_name()?);
                    }
                },
                _ => return Err(self.err("invalid group")),
            }
        }
        if look.is_some() {
            capturing = false;
        }
        let index = if capturing {
            self.next_group += 1;
            if let Some(n) = &name {
                if self.names.iter().any(|(m, _)| m == n) {
                    return Err(self.err("duplicate capture group name"));
                }
                self.names.push((n.clone(), self.next_group));
            }
            self.next_group
        } else {
            0
        };
        let body = self.disjunction(depth + 1)?;
        if !self.eat(')') {
            return Err(self.err("unterminated group"));
        }
        Ok(match look {
            Some((behind, negate)) => {
                (RegexNode::Look { behind, negate, node: Box::new(body) }, !behind)
            }
            None if capturing => (RegexNode::Group { index, name, node: Box::new(body) }, true),
            None => (body, true),
        })
    }

    /// Reads `name>` (after `<`).
    fn group_name(&mut self) -> Result<String, RegexParseError> {
        let mut name = String::new();
        loop {
            match self.peek() {
                Some('>') => {
                    self.i += 1;
                    break;
                }
                Some(c) if c.is_alphabetic() || c == '_' || c == '$' || (!name.is_empty() && c.is_alphanumeric()) => {
                    name.push(c);
                    self.i += 1;
                }
                _ => return Err(self.err("invalid capture group name")),
            }
        }
        if name.is_empty() {
            return Err(self.err("empty capture group name"));
        }
        Ok(name)
    }

    fn atom_escape(&mut self) -> Result<(RegexNode, bool), RegexParseError> {
        self.i += 1; // backslash
        let c = self.peek().ok_or_else(|| self.err("\\ at end of pattern"))?;
        let set = |item| RegexNode::Class(CharClass { items: vec![item], negated: false });
        let r = match c {
            'b' => {
                self.i += 1;
                (RegexNode::Assertion(AssertKind::WordBoundary), false)
            }
            'B' => {
                self.i += 1;
                (RegexNode::Assertion(AssertKind::NotWordBoundary), false)
            }
            'd' | 'D' | 'w' | 'W' | 's' | 'S' => {
                self.i += 1;
                (set(class_escape(c)), true)
            }
            '1'..='9' => {
                let save = self.i;
                let n = self.decimal().unwrap() as usize;
                if n <= self.group_total {
                    (RegexNode::BackRef(n), true)
                } else {
                    self.i = save;
                    if c >= '8' {
                        self.i += 1;
                        (RegexNode::Char(c), true)
                    } else {
                        (RegexNode::Char(self.legacy_octal()), true)
                    }
                }
            }
            '0' if !matches!(self.peek_at(1), Some('0'..='9')) => {
                self.i += 1;
                (RegexNode::Char('\0'), true)
            }
            '0' => (RegexNode::Char(self.legacy_octal()), true),
            'k' if !self.scanned_names.is_empty() => {
                self.i += 1;
                if !self.eat('<') {
                    return Err(self.err("invalid named reference"));
                }
                let name = self.group_name()?;
                match self.scanned_names.iter().find(|(n, _)| *n == name) {
                    Some((_, idx)) => (RegexNode::BackRef(*idx), true),
                    None => return Err(self.err("reference to undefined group name")),
                }
            }
            _ => (RegexNode::Char(self.char_escape(false)?), true),
        };
        Ok(r)
    }

    /// Annex B legacy octal escape starting at an octal digit.
    fn legacy_octal(&mut self) -> char {
        let mut v = self.peek().unwrap().to_digit(8).unwrap();
        self.i += 1;
        let first = v;
        for k in 0..2 {
            match self.peek().and_then(|c| c.to_digit(8)) {
                Some(d) if (k == 0 || first <= 3) && v * 8 + d <= 0o377 => {
                    v = v * 8 + d;
                    self.i += 1;
                }
                _ => break,
            }
        }
        char::from_u32(v).unwrap()
    }

    /// Parses the escape after a backslash (cursor on the char following it).
    fn char_escape(&mut self, in_class: bool) -> Result<char, RegexParseError> {
        let c = self.peek().ok_or_else(|| self.err("\\ at end of pattern"))?;
        self.i += 1;
        Ok(match c {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'f' => '\u{c}',
            'v' => '\u{b}',
            'b' if in_class => '\u{8}',
            '0'..='7' if in_class => {
                self.i -= 1;
                self.legacy_octal()
            }
            'x' => match (self.peek().and_then(hex_val), self.peek_at(1).and_then(hex_val)) {
                (Some(a), Some(b)) => {
                    self.i += 2;
                    char::from_u32(a * 16 + b).unwrap()
                }
                _ => 'x',
            },
            'u' => match self.hex4(0) {
                Some(hi) => {
                    self.i += 4;
                    if (0xD800..0xDC00).contains(&hi)
                        && self.peek() == Some('\\')
                        && self.peek_at(1) == Some('u')
                    {
                        if let Some(lo) = self.hex4(2) {
                            if (0xDC00..0xE000).contains(&lo) {
                                self.i += 6;
                                let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                return Ok(char::from_u32(cp).unwrap_or('\u{fffd}'));
                            }
                        }
                    }
                    // Lone surrogates are not representable as `char`.
                    char::from_u32(hi).unwrap_or('\u{fffd}')
                }
                None => 'u',
            },
            'c' => match self.peek() {
                Some(l) if l.is_ascii_alphabetic() => {
                    self.i += 1;
                    char::from_u32(l as u32 % 32).unwrap()
                }
                _ => {
                    self.i -= 1; // literal backslash; 'c' is parsed next
                    '\\'
                }
            },
            other => other,
        })
    }

    fn hex4(&self, off: usize) -> Option<u32> {
        let mut v = 0;
        for k in 0..4 {
            v = v * 16 + self.peek_at(off + k).and_then(hex_val)?;
        }
        Some(v)
    }

    fn class(&mut self) -> Result<RegexNode, RegexParseError> {
        self.i += 1; // '['
        let negated = self.eat('^');
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated character class")),
                Some(']') => {
                    self.i += 1;
                    break;
                }
                _ => {}
            }
            let a = self.class_atom()?;
            if self.peek() == Some('-') && self.peek_at(1).is_some_and(|c| c != ']') {
                self.i += 1;
                let b = self.class_atom()?;
                match (a, b) {
                    (ClassAtom::Char(lo), ClassAtom::Char(hi)) => {
                        if lo > hi {
                            return Err(self.err("range out of order in character class"));
                        }
                        items.push(ClassItem::Range(lo, hi));
                    }
                    (a, b) => {
                        for x in [a, ClassAtom::Char('-'), b] {
                            items.push(match x {
                                ClassAtom::Char(c) => ClassItem::Range(c, c),
                                ClassAtom::Set(s) => s,
                            });
                        }
                    }
                }
            } else {
                items.push(match a {
                    ClassAtom::Char(c) => ClassItem::Range(c, c),
                    ClassAtom::Set(s) => s,
                });
            }
        }
        Ok(RegexNode::Class(CharClass { items, negated }))
    }

    fn class_atom(&mut self) -> Result<ClassAtom, RegexParseError> {
        let c = self.peek().unwrap();
        self.i += 1;
        if c != '\\' {
            return Ok(ClassAtom::Char(c));
        }
        match self.peek() {
            Some(e @ ('d' | 'D' | 'w' | 'W' | 's' | 'S')) => {
                self.i += 1;
                Ok(ClassAtom::Set(class_escape(e)))
            }
            Some(d @ ('8' | '9')) => {
                self.i += 1;
                Ok(ClassAtom::Char(d))
            }
            _ => Ok(ClassAtom::Char(self.char_escape(true)?)),
        }
    }
}

fn class_escape(c: char) -> ClassItem {
    match c {
        'd' => ClassItem::Digit,
        'D' => ClassItem::NotDigit,
        'w' => ClassItem::Word,
        'W' => ClassItem::NotWord,
        's' => ClassItem::Space,
        _ => ClassItem::NotSpace,
    }
}
