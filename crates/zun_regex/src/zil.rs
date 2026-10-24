//! Stage 2d, slice 1: compile a regex to a synthesized ZIL function.
//!
//! This is the pipeline proof for lowering a regex to ordinary ZIL (no new
//! opcodes). It handles only the trivial subset: a pattern that is a plain
//! sequence of literal characters. The emitted method is
//!
//! ```text
//! matchAt(s: string, pos: u64): boolean
//! ```
//!
//! returning whether the literal occurs in `s` exactly at `pos` (sticky /
//! anchored semantics, like [`crate::Regex::match_at_chars`]).
//!
//! Positions are **UTF-16 code units** here (ZIL strings are indexed through
//! the `length` / `charCodeAt` builtins), whereas the interpreted engine
//! indexes by `char`. The two agree on BMP-only input.
//!
//! The string builtins are addressed by native id. Ids are assigned by
//! `zun_natives`; to keep this crate free of that dependency the caller
//! resolves them (`zun_natives::resolve("string", "length")`, ...) and
//! passes them in via [`StringNatives`].

use zun_il::{encode, Instruction, JitKind, NumKind, Opcode, Operand};
use zun_metadata::{MethodDef, StringIndex, TableKind, Token};
use zun_module::{write_body, Module};

use crate::ast::{ClassItem, is_line_terminator};
use crate::compile::{compile, Inst, Prefilter, SetMember};
use crate::flags::Flags;

/// Native ids of the string builtins the synthesized code calls.
#[derive(Debug, Clone, Copy)]
pub struct StringNatives {
    /// Id of `zun:string` `length(s): u64`.
    pub length: u32,
    /// Id of `zun:string` `charCodeAt(s, index: u64): u32`.
    pub char_code_at: u32,
}

/// The module produced by [`compile_literal`] and the method to call.
#[derive(Debug)]
pub struct CompiledLiteral {
    /// A module holding exactly one method, `matchAt`.
    pub module: Module,
    /// Method token of `matchAt` (`rid` is the 1-based method index).
    pub method: Token,
}

/// Characters with special meaning in a pattern; their presence means the
/// pattern is not literal-only and is not compilable in this slice.
const META: &[char] = &['\\', '^', '$', '.', '|', '?', '*', '+', '(', ')', '[', ']', '{', '}'];

/// The UTF-16 code units of a literal-only `pattern`, or `None` when the
/// pattern uses any regex syntax (so the caller falls back to the interpreter).
pub fn literal_units(pattern: &str) -> Option<Vec<u16>> {
    if pattern.chars().any(|c| META.contains(&c) || c == '/') {
        return None;
    }
    Some(pattern.encode_utf16().collect())
}

/// Compile a literal-only `pattern` (no flags) to a ZIL `matchAt` method.
///
/// Returns `None` if the pattern is not literal-only.
pub fn compile_literal(pattern: &str, natives: StringNatives) -> Option<CompiledLiteral> {
    let units = literal_units(pattern)?;
    let mut module = Module::new();
    let mut code = Vec::new();
    let mut emit = |op: Opcode, operand: Operand| encode(&Instruction::new(op, operand), &mut code);
    let u64_imm = NumKind::U64.to_imm();
    let u32_imm = NumKind::U32.to_imm();
    // Offset of the `const.false; ret` early-exit tail (two 1-byte opcodes).
    const FAIL_TAIL: i32 = 2;

    // Bounds check: pos + len > s.length() -> false.
    emit(Opcode::LoadLocal, Operand::Local(1));
    emit(Opcode::ConstI64, Operand::Imm64(units.len() as i64));
    emit(Opcode::Conv, Operand::Imm32(u64_imm));
    emit(Opcode::AddI32, Operand::None);
    emit(Opcode::LoadLocal, Operand::Local(0));
    emit(Opcode::CallBuiltin, Operand::Imm32(natives.length as i32));
    emit(Opcode::CmpGt, Operand::None);
    emit(Opcode::BranchFalse, Operand::Branch(FAIL_TAIL));
    emit(Opcode::ConstFalse, Operand::None);
    emit(Opcode::Ret, Operand::None);

    // One compare per code unit: s.charCodeAt(pos + i) != unit -> false.
    for (i, unit) in units.iter().enumerate() {
        emit(Opcode::LoadLocal, Operand::Local(0));
        emit(Opcode::LoadLocal, Operand::Local(1));
        if i > 0 {
            emit(Opcode::ConstI64, Operand::Imm64(i as i64));
            emit(Opcode::Conv, Operand::Imm32(u64_imm));
            emit(Opcode::AddI32, Operand::None);
        }
        emit(Opcode::CallBuiltin, Operand::Imm32(natives.char_code_at as i32));
        emit(Opcode::ConstI32, Operand::Imm32(*unit as i32));
        emit(Opcode::Conv, Operand::Imm32(u32_imm));
        emit(Opcode::CmpNe, Operand::None);
        emit(Opcode::BranchFalse, Operand::Branch(FAIL_TAIL));
        emit(Opcode::ConstFalse, Operand::None);
        emit(Opcode::Ret, Operand::None);
    }
    emit(Opcode::ConstTrue, Operand::None);
    emit(Opcode::Ret, Operand::None);

    let name: StringIndex = module.strings.intern("matchAt");
    let local_kinds = [
        JitKind::String.to_byte(),
        JitKind::Num(NumKind::U64).to_byte(),
    ];
    let body_offset = write_body(
        &mut module.code,
        2,
        2,
        true,
        JitKind::Bool.to_byte(),
        &local_kinds,
        &code,
    );
    let rid = module.tables.add_method_def(MethodDef {
        name,
        signature: zun_metadata::BlobIndex(0),
        body_offset,
        is_static: true,
    });
    let _ = rid;
    let method = Token::new(TableKind::MethodDef, 1);
    Some(CompiledLiteral { module, method })
}

// ---------------------------------------------------------------------------
// Slice 2: backtracking matcher ({Char, Any, Class, Split, Jmp, Save, Match}).
// ---------------------------------------------------------------------------

/// Native ids the backtracking matcher calls, on top of the string builtins.
///
/// The backtrack stack and the capture array both live in a caller-supplied
/// managed `ArrayBuffer` (so no `NewArray`/`newobj` appears in the body and the
/// method stays JIT-eligible on the general numeric path). We index it with the
/// `bufferReadU32`/`bufferWriteU32` intrinsics.
#[derive(Debug, Clone, Copy)]
pub struct MatcherNatives {
    /// `zun:string` `length(s): u64`.
    pub length: u32,
    /// `zun:string` `charCodeAt(s, index: u64): u32`.
    pub char_code_at: u32,
    /// `zun:buffer` `bufferReadU32(buf, byteOffset: u64): u32`.
    pub buffer_read_u32: u32,
    /// `zun:buffer` `bufferWriteU32(buf, byteOffset: u64, value: u32): boolean`.
    pub buffer_write_u32: u32,
    /// `zun:buffer` `bufferNew(length: u64): ArrayBuffer`.
    pub buffer_new: u32,
}

/// Bits used to pack each capture cell into the `i64` result.
const PACK_BITS: u32 = 10;
/// Sentinel value (in a packed cell) for an unset capture endpoint.
const PACK_NONE: u64 = (1 << PACK_BITS) - 1; // 1023
/// Largest number of capture cells that fit in a packed `i64` result.
const PACK_MAX_CELLS: usize = 6; // 6 * 10 = 60 bits (< 63)

/// The backtracking matcher produced by [`compile_backtracking`].
///
/// The module holds two methods:
///
/// ```text
/// matchAt(s: string, pos: u64, work: ArrayBuffer): i64   // JIT-eligible
/// matchAtAlloc(s: string, pos: u64): i64                 // Tier-0 wrapper
/// ```
///
/// `matchAt` runs the backtracking match anchored at `pos`, using `work` as the
/// backtrack stack and capture scratch, and returns the match result **packed**
/// into the `i64`: `-1` for no match, otherwise the capture cells packed
/// [`PACK_BITS`] bits each (cell `2*g` / `2*g+1` are group `g`'s start / end,
/// with [`PACK_NONE`] meaning "unset"). Returning by value avoids reading the
/// heap after the call — which matters because the interpreter runs each method
/// in its own transient heap.
///
/// `matchAtAlloc` allocates a `work` buffer with `bufferNew` and calls
/// `matchAt`; it is the Tier-0 entry (not JIT-eligible because of `bufferNew`).
/// Tier-1 callers invoke `matchAt` directly with a buffer allocated in the
/// ambient heap.
///
/// Positions are UTF-16 code units (same convention as [`compile_literal`]);
/// char and UTF-16 index coincide on BMP-only input. Packing limits this to at
/// most [`PACK_MAX_CELLS`] capture cells and positions below [`PACK_NONE`];
/// patterns needing more fall back (return `None`).
#[derive(Debug)]
pub struct CompiledMatcher {
    /// A module holding `matchAt` (rid 1) and `matchAtAlloc` (rid 2).
    pub module: Module,
    /// Method token of the JIT-eligible `matchAt(s, pos, work)`.
    pub method: Token,
    /// Method token of the Tier-0 wrapper `matchAtAlloc(s, pos)`.
    pub alloc_method: Token,
    /// Required minimum size of the `work` buffer, in bytes.
    pub buffer_bytes: usize,
    /// Number of capture groups excluding group 0.
    pub group_count: usize,
    /// Number of capture cells (`2 * (group_count + 1)`).
    pub cap_cells: usize,
    /// The program's conservative first-position prefilter, reused by
    /// [`compile_is_match`] to emit a candidate-skipping unanchored scan (so the
    /// generated matcher and the runtime engine agree on prefilter decisions).
    pub(crate) prefilter: Prefilter,
}

/// Decode a packed `matchAt` result into capture-group spans (`None` = no
/// match). Mirrors the packing emitted by [`compile_backtracking`].
pub fn unpack_result(packed: i64, cap_cells: usize) -> Option<Vec<Option<(usize, usize)>>> {
    if packed < 0 {
        return None;
    }
    let bits = packed as u64;
    let cells: Vec<u64> = (0..cap_cells)
        .map(|j| (bits >> (PACK_BITS * j as u32)) & ((1 << PACK_BITS) - 1))
        .collect();
    Some(
        cells
            .chunks(2)
            .map(|c| {
                (c[0] != PACK_NONE && c[1] != PACK_NONE).then_some((c[0] as usize, c[1] as usize))
            })
            .collect(),
    )
}

/// A label-based ZIL assembler with branch fixups.
struct Asm {
    code: Vec<u8>,
    labels: Vec<Option<usize>>,
    patches: Vec<(usize, usize)>,
}

impl Asm {
    fn new() -> Asm {
        Asm { code: Vec::new(), labels: Vec::new(), patches: Vec::new() }
    }

    fn label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }

    fn bind(&mut self, label: usize) {
        self.labels[label] = Some(self.code.len());
    }

    fn emit(&mut self, op: Opcode, operand: Operand) {
        encode(&Instruction::new(op, operand), &mut self.code);
    }

    /// Emit a branch to `label` with a placeholder displacement.
    fn br(&mut self, op: Opcode, label: usize) {
        encode(&Instruction::new(op, Operand::Branch(0)), &mut self.code);
        self.patches.push((self.code.len() - 4, label));
    }

    fn load(&mut self, slot: u16) {
        self.emit(Opcode::LoadLocal, Operand::Local(slot));
    }

    fn store(&mut self, slot: u16) {
        self.emit(Opcode::StoreLocal, Operand::Local(slot));
    }

    /// Push an unsigned 64-bit value (used for positions and byte offsets).
    fn const_u64(&mut self, v: u64) {
        self.emit(Opcode::ConstI64, Operand::Imm64(v as i64));
        self.emit(Opcode::Conv, Operand::Imm32(NumKind::U64.to_imm()));
    }

    /// Push a 32-bit value (used for char codes and buffer cell contents).
    fn const_u32(&mut self, v: u32) {
        self.emit(Opcode::ConstI32, Operand::Imm32(v as i32));
        self.emit(Opcode::Conv, Operand::Imm32(NumKind::U32.to_imm()));
    }

    fn finish(mut self) -> Vec<u8> {
        for (at, label) in &self.patches {
            let target = self.labels[*label].expect("label bound");
            let disp = (target as i64 - (*at as i64 + 4)) as i32;
            self.code[*at..*at + 4].copy_from_slice(&disp.to_le_bytes());
        }
        self.code
    }
}

// Local slots of the synthesized method.
const L_S: u16 = 0; // string parameter
const L_POS: u16 = 1; // current position (also the start parameter)
const L_BUF: u16 = 2; // work buffer parameter
const L_START: u16 = 3; // original start position
const L_N: u16 = 4; // input length
const L_SP: u16 = 5; // backtrack stack pointer (byte offset)
const L_C: u16 = 6; // current char code
const L_M: u16 = 7; // class membership accumulator (0/1)
const L_M2: u16 = 8; // per-item membership temp
const L_K: u16 = 9; // popped entry kind
const L_BX: u16 = 10; // popped entry field x
const L_BY: u16 = 11; // popped entry field y
const L_OLD: u16 = 12; // saved capture old value
const L_R: u16 = 13; // packed result accumulator
const L_PV: u16 = 14; // packed value of one cell
const LOCAL_COUNT: u16 = 15;

// Backtrack-entry kinds.
const BT_BRANCH: u32 = 0; // resume at pc=x with pos=y
const BT_CAP: u32 = 1; // restore caps cell at byte-offset x to y

const NONE_CELL: u32 = 0xFFFF_FFFF;

/// Read capture/stack cell `work[off]` (off is a byte offset); leaves a `u32`.
fn read_cell_off(a: &mut Asm, nat: MatcherNatives, emit_off: impl FnOnce(&mut Asm)) {
    a.load(L_BUF);
    emit_off(a);
    a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.buffer_read_u32 as i32));
}

/// Write `work[off] = value` (both emitted by the closures); discards the bool.
fn write_cell(
    a: &mut Asm,
    nat: MatcherNatives,
    emit_off: impl FnOnce(&mut Asm),
    emit_val: impl FnOnce(&mut Asm),
) {
    a.load(L_BUF);
    emit_off(a);
    emit_val(a);
    a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.buffer_write_u32 as i32));
    a.emit(Opcode::Pop, Operand::None);
}

/// Push `sp + delta` as the byte offset for a stack cell access.
fn sp_plus(a: &mut Asm, delta: u64) {
    a.load(L_SP);
    if delta != 0 {
        a.const_u64(delta);
        a.emit(Opcode::AddI32, Operand::None);
    }
}

/// Push a 3-cell entry `(kind, x, y)` onto the backtrack stack and bump `sp`.
fn push_entry(
    a: &mut Asm,
    nat: MatcherNatives,
    kind: u32,
    emit_x: impl FnOnce(&mut Asm),
    emit_y: impl FnOnce(&mut Asm),
) {
    write_cell(a, nat, |a| sp_plus(a, 0), |a| a.const_u32(kind));
    write_cell(a, nat, |a| sp_plus(a, 4), emit_x);
    write_cell(a, nat, |a| sp_plus(a, 8), emit_y);
    a.load(L_SP);
    a.const_u64(12);
    a.emit(Opcode::AddI32, Operand::None);
    a.store(L_SP);
}

/// Inclusive code-point ranges for a class escape's base set.
fn escape_ranges(item: ClassItem) -> (Vec<(u32, u32)>, bool) {
    match item {
        ClassItem::Range(lo, hi) => (vec![(lo as u32, hi as u32)], false),
        ClassItem::Digit => (vec![('0' as u32, '9' as u32)], false),
        ClassItem::NotDigit => (vec![('0' as u32, '9' as u32)], true),
        ClassItem::Word | ClassItem::NotWord => (
            vec![
                ('0' as u32, '9' as u32),
                ('a' as u32, 'z' as u32),
                ('A' as u32, 'Z' as u32),
                ('_' as u32, '_' as u32),
            ],
            matches!(item, ClassItem::NotWord),
        ),
        ClassItem::Space | ClassItem::NotSpace => (
            vec![
                (0x09, 0x0d),
                (0x20, 0x20),
                (0xa0, 0xa0),
                (0x1680, 0x1680),
                (0x2000, 0x200a),
                (0x2028, 0x2029),
                (0x202f, 0x202f),
                (0x205f, 0x205f),
                (0x3000, 0x3000),
                (0xfeff, 0xfeff),
            ],
            matches!(item, ClassItem::NotSpace),
        ),
    }
}

/// Emit `out = (char in slot `c_slot` is in any of `ranges`)` as a 0/1 value in
/// local `out_slot`.
fn emit_in_ranges_slots(a: &mut Asm, ranges: &[(u32, u32)], c_slot: u16, out_slot: u16) {
    a.const_u64(0);
    a.store(out_slot);
    for &(lo, hi) in ranges {
        let skip = a.label();
        if lo == hi {
            a.load(c_slot);
            a.const_u64(lo as u64);
            a.emit(Opcode::CmpNe, Operand::None);
            a.br(Opcode::BranchTrue, skip);
        } else {
            a.load(c_slot);
            a.const_u64(lo as u64);
            a.emit(Opcode::CmpLt, Operand::None);
            a.br(Opcode::BranchTrue, skip);
            a.load(c_slot);
            a.const_u64(hi as u64);
            a.emit(Opcode::CmpGt, Operand::None);
            a.br(Opcode::BranchTrue, skip);
        }
        a.const_u64(1);
        a.store(out_slot);
        a.bind(skip);
    }
}

/// Emit `m2 = (c in any of `ranges`)` as a 0/1 value in [`L_M2`] (the matcher's
/// class-membership slots).
fn emit_in_ranges(a: &mut Asm, ranges: &[(u32, u32)]) {
    emit_in_ranges_slots(a, ranges, L_C, L_M2);
}

/// Compile a pattern whose program uses only the slice-2 subset to a
/// backtracking `matchAt`. Returns `None` for any out-of-subset program (loops,
/// backrefs, lookaround, assertions), for backward matching, or for the `i`
/// flag — so the caller falls back to the interpreter.
pub fn compile_backtracking(
    pattern: &str,
    flag_str: &str,
    nat: MatcherNatives,
) -> Option<CompiledMatcher> {
    let flags = Flags::parse(flag_str).ok()?;
    if flags.ignore_case {
        return None; // case folding in ZIL is a later slice
    }
    let parsed = crate::parser::parse(pattern).ok()?;
    let prog = compile(&parsed.node, parsed.group_count, flags);
    let insts = &prog.insts;
    let n = insts.len();

    // Subset gate: reject anything outside {Char, Any, Class, Split, Jmp, Save,
    // Match} and any backward (lookbehind) matching.
    for (k, inst) in insts.iter().enumerate() {
        match inst {
            Inst::Char { back: false, .. }
            | Inst::Any { back: false }
            | Inst::Class { back: false, .. }
            | Inst::Jmp(_)
            | Inst::Save(_)
            | Inst::Match => {}
            Inst::Split { first, .. } => {
                // The compiler always lays alternation branches out so the
                // first arm is the fall-through; the lowering relies on it.
                if *first != k + 1 {
                    return None;
                }
            }
            _ => return None,
        }
    }

    let prefilter = prog.prefilter.clone();
    let cap_cells = 2 * (prog.group_count + 1);
    if cap_cells > PACK_MAX_CELLS {
        return None; // too many groups to pack into the i64 result
    }
    let base = (cap_cells * 4) as u64; // stack region starts after the caps
    // Resume targets: the distinct `second` arms of every Split.
    let mut seconds: Vec<usize> = insts
        .iter()
        .filter_map(|i| match i {
            Inst::Split { second, .. } => Some(*second),
            _ => None,
        })
        .collect();
    seconds.sort_unstable();
    seconds.dedup();

    let dot_all = prog.dot_all;

    let mut a = Asm::new();
    let blk: Vec<usize> = (0..n).map(|_| a.label()).collect();
    let backtrack = a.label();
    let ret_fail = a.label();

    // --- Prologue ---------------------------------------------------------
    a.load(L_POS);
    a.store(L_START);
    a.load(L_S);
    a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.length as i32));
    a.store(L_N);
    for cell in 0..cap_cells {
        write_cell(&mut a, nat, |a| a.const_u64((cell * 4) as u64), |a| a.const_u32(NONE_CELL));
    }
    a.const_u64(base);
    a.store(L_SP);
    // Fall through into block 0.

    // --- Instruction blocks ----------------------------------------------
    for (k, inst) in insts.iter().enumerate() {
        a.bind(blk[k]);
        match inst {
            Inst::Char { c, .. } => {
                // if pos >= n: backtrack
                a.load(L_POS);
                a.load(L_N);
                a.emit(Opcode::CmpGe, Operand::None);
                a.br(Opcode::BranchTrue, backtrack);
                // if charCodeAt(s, pos) != c: backtrack
                a.load(L_S);
                a.load(L_POS);
                a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.char_code_at as i32));
                a.const_u32(*c as u32);
                a.emit(Opcode::CmpNe, Operand::None);
                a.br(Opcode::BranchTrue, backtrack);
                // pos += 1 (fall through to next block)
                a.load(L_POS);
                a.const_u64(1);
                a.emit(Opcode::AddI32, Operand::None);
                a.store(L_POS);
            }
            Inst::Any { .. } => {
                a.load(L_POS);
                a.load(L_N);
                a.emit(Opcode::CmpGe, Operand::None);
                a.br(Opcode::BranchTrue, backtrack);
                if !dot_all {
                    // c = charCodeAt(s, pos); if is_line_terminator(c): backtrack
                    a.load(L_S);
                    a.load(L_POS);
                    a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.char_code_at as i32));
                    a.emit(Opcode::Conv, Operand::Imm32(NumKind::U64.to_imm()));
                    a.store(L_C);
                    let lt_ranges: Vec<(u32, u32)> = line_terminator_ranges();
                    emit_in_ranges(&mut a, &lt_ranges);
                    a.load(L_M2);
                    a.const_u64(0);
                    a.emit(Opcode::CmpNe, Operand::None);
                    a.br(Opcode::BranchTrue, backtrack);
                }
                a.load(L_POS);
                a.const_u64(1);
                a.emit(Opcode::AddI32, Operand::None);
                a.store(L_POS);
            }
            Inst::Class { idx, .. } => {
                let class = &prog.classes[*idx];
                a.load(L_POS);
                a.load(L_N);
                a.emit(Opcode::CmpGe, Operand::None);
                a.br(Opcode::BranchTrue, backtrack);
                a.load(L_S);
                a.load(L_POS);
                a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.char_code_at as i32));
                a.emit(Opcode::Conv, Operand::Imm32(NumKind::U64.to_imm()));
                a.store(L_C);
                // hit = OR over items (respecting per-item negation)
                a.const_u64(0);
                a.store(L_M);
                for &item in &class.items {
                    let (ranges, negate_item) = escape_ranges(item);
                    emit_in_ranges(&mut a, &ranges);
                    let after = a.label();
                    a.load(L_M2);
                    a.const_u64(0);
                    if negate_item {
                        a.emit(Opcode::CmpEq, Operand::None); // matches when NOT in set
                    } else {
                        a.emit(Opcode::CmpNe, Operand::None); // matches when in set
                    }
                    a.br(Opcode::BranchFalse, after);
                    a.const_u64(1);
                    a.store(L_M);
                    a.bind(after);
                }
                // matched = hit XOR negated; backtrack if not matched.
                a.load(L_M);
                a.const_u64(0);
                if class.negated {
                    a.emit(Opcode::CmpNe, Operand::None); // in set -> negated fails
                } else {
                    a.emit(Opcode::CmpEq, Operand::None); // not in set -> fails
                }
                a.br(Opcode::BranchTrue, backtrack);
                a.load(L_POS);
                a.const_u64(1);
                a.emit(Opcode::AddI32, Operand::None);
                a.store(L_POS);
            }
            Inst::Split { second, .. } => {
                // Push a resume point at `second` with the current pos, then
                // fall through to `first` (== k + 1).
                let second = *second as u32;
                push_entry(
                    &mut a,
                    nat,
                    BT_BRANCH,
                    |a| a.const_u32(second),
                    |a| {
                        a.load(L_POS);
                        a.emit(Opcode::Conv, Operand::Imm32(NumKind::U32.to_imm()));
                    },
                );
            }
            Inst::Jmp(t) => {
                a.br(Opcode::Branch, blk[*t]);
            }
            Inst::Save(slot) => {
                let off = (*slot * 4) as u32;
                // old = caps[slot]
                read_cell_off(&mut a, nat, |a| a.const_u64(off as u64));
                a.emit(Opcode::Conv, Operand::Imm32(NumKind::U64.to_imm()));
                a.store(L_OLD);
                // push Cap{ x = byte offset, y = old }
                push_entry(
                    &mut a,
                    nat,
                    BT_CAP,
                    |a| a.const_u32(off),
                    |a| {
                        a.load(L_OLD);
                        a.emit(Opcode::Conv, Operand::Imm32(NumKind::U32.to_imm()));
                    },
                );
                // caps[slot] = pos
                write_cell(
                    &mut a,
                    nat,
                    |a| a.const_u64(off as u64),
                    |a| {
                        a.load(L_POS);
                        a.emit(Opcode::Conv, Operand::Imm32(NumKind::U32.to_imm()));
                    },
                );
            }
            Inst::Match => {
                // caps[0] = start, caps[1] = end (= pos).
                write_cell(
                    &mut a,
                    nat,
                    |a| a.const_u64(0),
                    |a| {
                        a.load(L_START);
                        a.emit(Opcode::Conv, Operand::Imm32(NumKind::U32.to_imm()));
                    },
                );
                write_cell(
                    &mut a,
                    nat,
                    |a| a.const_u64(4),
                    |a| {
                        a.load(L_POS);
                        a.emit(Opcode::Conv, Operand::Imm32(NumKind::U32.to_imm()));
                    },
                );
                // Pack every capture cell into the i64 result.
                a.const_u64(0);
                a.store(L_R);
                for cell in 0..cap_cells {
                    // c = caps[cell]
                    read_cell_off(&mut a, nat, |a| a.const_u64((cell * 4) as u64));
                    a.emit(Opcode::Conv, Operand::Imm32(NumKind::U64.to_imm()));
                    a.store(L_C);
                    // pv = (c > PACK_NONE - 1) ? PACK_NONE : c
                    let use_c = a.label();
                    let done = a.label();
                    a.load(L_C);
                    a.const_u64(PACK_NONE - 1);
                    a.emit(Opcode::CmpGt, Operand::None);
                    a.br(Opcode::BranchFalse, use_c);
                    a.const_u64(PACK_NONE);
                    a.store(L_PV);
                    a.br(Opcode::Branch, done);
                    a.bind(use_c);
                    a.load(L_C);
                    a.store(L_PV);
                    a.bind(done);
                    // R += pv * (1 << (PACK_BITS * cell))
                    a.load(L_R);
                    a.load(L_PV);
                    a.const_u64(1u64 << (PACK_BITS * cell as u32));
                    a.emit(Opcode::MulI32, Operand::None);
                    a.emit(Opcode::AddI32, Operand::None);
                    a.store(L_R);
                }
                a.load(L_R);
                a.emit(Opcode::Conv, Operand::Imm32(NumKind::I64.to_imm()));
                a.emit(Opcode::Ret, Operand::None);
            }
            _ => unreachable!("subset gate rejects non-slice-2 instructions"),
        }
    }

    // --- Backtrack / resume dispatch -------------------------------------
    a.bind(backtrack);
    // if sp == base: no more choices -> fail
    a.load(L_SP);
    a.const_u64(base);
    a.emit(Opcode::CmpEq, Operand::None);
    a.br(Opcode::BranchTrue, ret_fail);
    // sp -= 12
    a.load(L_SP);
    a.const_u64(12);
    a.emit(Opcode::SubI32, Operand::None);
    a.store(L_SP);
    // k = work[sp]; bx = work[sp+4]; by = work[sp+8]
    read_cell_off(&mut a, nat, |a| sp_plus(a, 0));
    a.emit(Opcode::Conv, Operand::Imm32(NumKind::U64.to_imm()));
    a.store(L_K);
    read_cell_off(&mut a, nat, |a| sp_plus(a, 4));
    a.emit(Opcode::Conv, Operand::Imm32(NumKind::U64.to_imm()));
    a.store(L_BX);
    read_cell_off(&mut a, nat, |a| sp_plus(a, 8));
    a.emit(Opcode::Conv, Operand::Imm32(NumKind::U64.to_imm()));
    a.store(L_BY);
    // if k != CAP: it is a branch entry -> resume
    let is_branch = a.label();
    a.load(L_K);
    a.const_u64(BT_CAP as u64);
    a.emit(Opcode::CmpEq, Operand::None);
    a.br(Opcode::BranchFalse, is_branch);
    // Cap: caps[bx] = by, keep unwinding.
    write_cell(
        &mut a,
        nat,
        |a| a.load(L_BX),
        |a| {
            a.load(L_BY);
            a.emit(Opcode::Conv, Operand::Imm32(NumKind::U32.to_imm()));
        },
    );
    a.br(Opcode::Branch, backtrack);
    // Branch: pos = by, dispatch pc=bx to the matching block.
    a.bind(is_branch);
    a.load(L_BY);
    a.store(L_POS);
    for &t in &seconds {
        a.load(L_BX);
        a.const_u64(t as u64);
        a.emit(Opcode::CmpEq, Operand::None);
        a.br(Opcode::BranchTrue, blk[t]);
    }
    a.br(Opcode::Branch, ret_fail); // unreachable when programs are well-formed

    // --- Failure return ---------------------------------------------------
    a.bind(ret_fail);
    a.emit(Opcode::ConstI64, Operand::Imm64(-1));
    a.emit(Opcode::Ret, Operand::None);

    let code = a.finish();

    let mut module = Module::new();
    let u64b = JitKind::Num(NumKind::U64).to_byte();
    let i64b = JitKind::Num(NumKind::I64).to_byte();
    let match_name: StringIndex = module.strings.intern("matchAt");
    let local_kinds = [
        JitKind::String.to_byte(),
        u64b, // pos
        JitKind::Object.to_byte(),
        u64b, // start
        u64b, // n
        u64b, // sp
        u64b, // c
        u64b, // m
        u64b, // m2
        u64b, // k
        u64b, // bx
        u64b, // by
        u64b, // old
        u64b, // R
        u64b, // PV
    ];
    debug_assert_eq!(local_kinds.len(), LOCAL_COUNT as usize);
    let match_body = write_body(&mut module.code, 3, LOCAL_COUNT, true, i64b, &local_kinds, &code);
    let _ = module.tables.add_method_def(MethodDef {
        name: match_name,
        signature: zun_metadata::BlobIndex(0),
        body_offset: match_body,
        is_static: true,
    });
    let method = Token::new(TableKind::MethodDef, 1);

    // Caps region + up to one live entry per instruction on any single path.
    let buffer_bytes = base as usize + (n + 2) * 12;

    // Tier-0 wrapper: allocate the work buffer, then call matchAt.
    let mut w = Asm::new();
    w.load(L_S); // s
    w.load(L_POS); // pos
    w.const_u64(buffer_bytes as u64);
    w.emit(Opcode::CallBuiltin, Operand::Imm32(nat.buffer_new as i32)); // -> buffer
    w.emit(Opcode::Call, Operand::Token(method));
    w.emit(Opcode::Ret, Operand::None);
    let wrapper = w.finish();
    let alloc_name: StringIndex = module.strings.intern("matchAtAlloc");
    let wrapper_kinds = [JitKind::String.to_byte(), u64b];
    let alloc_body = write_body(&mut module.code, 2, 2, true, i64b, &wrapper_kinds, &wrapper);
    let _ = module.tables.add_method_def(MethodDef {
        name: alloc_name,
        signature: zun_metadata::BlobIndex(0),
        body_offset: alloc_body,
        is_static: true,
    });
    let alloc_method = Token::new(TableKind::MethodDef, 2);

    Some(CompiledMatcher {
        module,
        method,
        alloc_method,
        buffer_bytes,
        group_count: prog.group_count,
        cap_cells,
        prefilter,
    })
}

// ---------------------------------------------------------------------------
// Slice 3: unanchored boolean `isMatch` (RegExp.prototype.test semantics).
// ---------------------------------------------------------------------------

/// The unanchored search entry produced by [`compile_is_match`].
#[derive(Debug)]
pub struct CompiledIsMatch {
    /// Module holding `matchAt` (rid 1), `matchAtAlloc` (rid 2), and
    /// `isMatch` (rid 3).
    pub module: Module,
    /// Token of `isMatch(s: string): boolean`, the unanchored search entry.
    pub is_match_method: Token,
    /// Work-buffer size in bytes (allocated inside `isMatch`).
    pub buffer_bytes: usize,
    /// Number of capture groups excluding group 0.
    pub group_count: usize,
}

/// Emit `cand_slot = (the char in `c_slot` is a viable first character per
/// `members`)`. Mirrors the matcher's `Class` membership (and literal compare),
/// so the generated prefilter accepts exactly the chars the runtime `FirstSet`
/// does. Over-approximation is safe (a false candidate only costs a `matchAt`);
/// under-approximation would skip a real match, so this never narrows.
fn emit_first_set_candidate(
    a: &mut Asm,
    members: &[SetMember],
    c_slot: u16,
    cand_slot: u16,
    item_slot: u16,
    hit_slot: u16,
) {
    a.const_u64(0);
    a.store(cand_slot);
    for member in members {
        match member {
            SetMember::Char(ch) => {
                let after = a.label();
                a.load(c_slot);
                a.const_u64(*ch as u64);
                a.emit(Opcode::CmpNe, Operand::None);
                a.br(Opcode::BranchTrue, after);
                a.const_u64(1);
                a.store(cand_slot);
                a.bind(after);
            }
            SetMember::Class(cls) => {
                // hit = OR over items (respecting per-item negation).
                a.const_u64(0);
                a.store(hit_slot);
                for &item in &cls.items {
                    let (ranges, negate_item) = escape_ranges(item);
                    emit_in_ranges_slots(a, &ranges, c_slot, item_slot);
                    let after = a.label();
                    a.load(item_slot);
                    a.const_u64(0);
                    if negate_item {
                        a.emit(Opcode::CmpEq, Operand::None); // matches when NOT in set
                    } else {
                        a.emit(Opcode::CmpNe, Operand::None); // matches when in set
                    }
                    a.br(Opcode::BranchFalse, after);
                    a.const_u64(1);
                    a.store(hit_slot);
                    a.bind(after);
                }
                // matched = hit XOR cls.negated -> candidate.
                let after = a.label();
                a.load(hit_slot);
                a.const_u64(0);
                if cls.negated {
                    a.emit(Opcode::CmpNe, Operand::None); // in set -> negated fails
                } else {
                    a.emit(Opcode::CmpEq, Operand::None); // not in set -> fails
                }
                a.br(Opcode::BranchTrue, after);
                a.const_u64(1);
                a.store(cand_slot);
                a.bind(after);
            }
        }
    }
}

/// Compile `pattern`/`flag_str` to an unanchored boolean `isMatch(s)` method
/// (JS `RegExp.prototype.test` / `isMatch` semantics) built on top of the
/// anchored [`compile_backtracking`] matcher. Returns `None` whenever the
/// matcher does (out-of-subset program, ignore-case, too many groups).
///
/// `isMatch` performs an unanchored search by advancing the start position
/// through a **FindFirstChar prefilter** derived from the program's conservative
/// first-position analysis (the same [`Prefilter`] the runtime engine computes,
/// carried out of [`compile_backtracking`]), calling the JIT-eligible `matchAt`
/// only at candidate positions and returning `true` on the first anchored match.
/// The prefilter cases emitted:
///
/// * [`Prefilter::Byte`] (a required leading ASCII literal `c`): a `charCodeAt`
///   compare loop advances the start index while the code unit `!= c`, so
///   `matchAt` launches only at matching positions (the JIT-native analog of
///   `memchr`; ZIL has no memchr).
/// * [`Prefilter::Set`] (a required leading first-character set, e.g. a class or
///   an all-consuming alternation): a candidate test mirroring the matcher's
///   `Class` membership gates each `matchAt`.
/// * [`Prefilter::Anchored`] (non-multiline leading `^`): a single `matchAt` at
///   position 0. (Unreachable for the compilable subset — assertions are gated
///   out of [`compile_backtracking`] — but kept for completeness.)
/// * [`Prefilter::None`] / [`Prefilter::Multiline`]: the original naive scan of
///   every start position `0..=length(s)` (a bail reproduces today's exact
///   results).
///
/// This mirrors [`crate::Regex::is_match`], which loops `find` (now prefiltered)
/// over start positions. The work buffer is allocated once with `bufferNew` (so
/// `isMatch` stays a Tier-0 orchestrator, like `matchAtAlloc`), while the hot
/// `matchAt` tiers up to Tier 1. Positions are UTF-16 code units (BMP-only
/// agrees with the interpreted engine; see [`compile_backtracking`]).
pub fn compile_is_match(
    pattern: &str,
    flag_str: &str,
    nat: MatcherNatives,
) -> Option<CompiledIsMatch> {
    // Only honor flags whose semantics the matcher fully reproduces for a
    // boolean unanchored search: `s` (dotAll, handled in the matcher) and `m`
    // (multiline — affects only `^`/`$` assertions, which are outside the
    // compilable subset, so it is a no-op here). `i` is rejected by
    // `compile_backtracking`; `g`/`y`/`u`/`d` change search or indexing
    // semantics (and the runtime engine rejects some of them), so any of them
    // means fall back to the runtime engine.
    let flags = Flags::parse(flag_str).ok()?;
    if flags.global || flags.sticky || flags.unicode || flags.has_indices {
        return None;
    }
    let CompiledMatcher { mut module, method, buffer_bytes, group_count, prefilter, .. } =
        compile_backtracking(pattern, flag_str, nat)?;

    // Locals: 0 s:string, 1 n:u64, 2 pos:u64, 3 buf:object, 4 r:i64, then the
    // prefilter scratch (used only by the Set path): 5 c:u64, 6 cand:u64,
    // 7 item:u64, 8 hit:u64.
    const S: u16 = 0;
    const N: u16 = 1;
    const POS: u16 = 2;
    const BUF: u16 = 3;
    const R: u16 = 4;
    const C: u16 = 5;
    const CAND: u16 = 6;
    const ITEM: u16 = 7;
    const HIT: u16 = 8;

    let mut a = Asm::new();
    let on_match = a.label();
    let on_fail = a.label();

    // n = length(s)
    a.load(S);
    a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.length as i32));
    a.store(N);
    // buf = bufferNew(buffer_bytes)
    a.const_u64(buffer_bytes as u64);
    a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.buffer_new as i32));
    a.store(BUF);
    // pos = 0
    a.const_u64(0);
    a.store(POS);

    // Emit `r = matchAt(s, pos, buf); if r >= 0 goto on_match`. matchAt
    // re-initializes caps/sp in its prologue, so reusing one buffer across
    // every position is safe.
    let emit_attempt = |a: &mut Asm| {
        a.load(S);
        a.load(POS);
        a.load(BUF);
        a.emit(Opcode::Call, Operand::Token(method));
        a.store(R);
        a.load(R);
        a.emit(Opcode::ConstI64, Operand::Imm64(0));
        a.emit(Opcode::CmpGe, Operand::None);
        a.br(Opcode::BranchTrue, on_match);
    };

    match &prefilter {
        // Leading `^` (non-multiline): only position 0 is viable.
        Prefilter::Anchored => {
            emit_attempt(&mut a);
            a.br(Opcode::Branch, on_fail);
        }
        // Required leading literal / first-char set: skip non-candidate starts.
        // Candidates are all < n (a required first char can never match at the
        // end), so `pos >= n` ends the search, exactly like the runtime
        // `Prefilter::{Byte,Set}::next`, which never returns `len`.
        // A required leading literal (single char or a 2+ char prefix): the
        // generated matcher keeps the simple first-char scan (the runtime engine
        // uses full-literal `memmem`; ZIL has no memmem, so scanning the first
        // char preserves today's behavior and correctness — the anchored
        // `matchAt` rejects false candidates). `LoopLiteral` falls to the naive
        // arm below (it can only arise with a loop, which the ZIL subset rejects
        // before reaching this point, so it is never actually emitted).
        Prefilter::Byte(..) | Prefilter::Literal(_) => {
            let first = literal_first(&prefilter).expect("Byte/Literal carry a first char");
            let scan = a.label();
            let advance = a.label();
            a.bind(scan);
            a.load(POS);
            a.load(N);
            a.emit(Opcode::CmpGe, Operand::None);
            a.br(Opcode::BranchTrue, on_fail);
            // if charCodeAt(s, pos) != first: advance
            a.load(S);
            a.load(POS);
            a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.char_code_at as i32));
            a.const_u32(first as u32);
            a.emit(Opcode::CmpNe, Operand::None);
            a.br(Opcode::BranchTrue, advance);
            emit_attempt(&mut a);
            a.bind(advance);
            a.load(POS);
            a.const_u64(1);
            a.emit(Opcode::AddI32, Operand::None);
            a.store(POS);
            a.br(Opcode::Branch, scan);
        }
        Prefilter::Set(fs) => {
            let scan = a.label();
            let advance = a.label();
            a.bind(scan);
            a.load(POS);
            a.load(N);
            a.emit(Opcode::CmpGe, Operand::None);
            a.br(Opcode::BranchTrue, on_fail);
            // c = charCodeAt(s, pos); cand = (c is a viable first char)
            a.load(S);
            a.load(POS);
            a.emit(Opcode::CallBuiltin, Operand::Imm32(nat.char_code_at as i32));
            a.emit(Opcode::Conv, Operand::Imm32(NumKind::U64.to_imm()));
            a.store(C);
            emit_first_set_candidate(&mut a, fs.members(), C, CAND, ITEM, HIT);
            // if cand == 0: advance
            a.load(CAND);
            a.const_u64(0);
            a.emit(Opcode::CmpEq, Operand::None);
            a.br(Opcode::BranchTrue, advance);
            emit_attempt(&mut a);
            a.bind(advance);
            a.load(POS);
            a.const_u64(1);
            a.emit(Opcode::AddI32, Operand::None);
            a.store(POS);
            a.br(Opcode::Branch, scan);
        }
        // No usable first-char information (or multiline `^`, which the subset
        // never reaches): try every start position `0..=n` inclusive, so an
        // empty match at the end of the string is found (matching `find`'s
        // `s <= input.len()` bound). Identical to the pre-prefilter behavior.
        // `LoopLiteral` cannot arise here (its loop is rejected by the ZIL
        // subset gate), but route it to the naive scan for exhaustiveness.
        Prefilter::None | Prefilter::Multiline | Prefilter::LoopLiteral { .. } => {
            let loop_head = a.label();
            a.bind(loop_head);
            a.load(POS);
            a.load(N);
            a.emit(Opcode::CmpGt, Operand::None);
            a.br(Opcode::BranchTrue, on_fail);
            emit_attempt(&mut a);
            a.load(POS);
            a.const_u64(1);
            a.emit(Opcode::AddI32, Operand::None);
            a.store(POS);
            a.br(Opcode::Branch, loop_head);
        }
    }

    a.bind(on_match);
    a.emit(Opcode::ConstTrue, Operand::None);
    a.emit(Opcode::Ret, Operand::None);
    a.bind(on_fail);
    a.emit(Opcode::ConstFalse, Operand::None);
    a.emit(Opcode::Ret, Operand::None);

    let code = a.finish();
    let name: StringIndex = module.strings.intern("isMatch");
    let u64b = JitKind::Num(NumKind::U64).to_byte();
    let local_kinds = [
        JitKind::String.to_byte(),
        u64b, // n
        u64b, // pos
        JitKind::Object.to_byte(),
        JitKind::Num(NumKind::I64).to_byte(), // r
        u64b,                                 // c
        u64b,                                 // cand
        u64b,                                 // item
        u64b,                                 // hit
    ];
    let body = write_body(&mut module.code, 1, 9, true, JitKind::Bool.to_byte(), &local_kinds, &code);
    let is_match_method = module.tables.add_method_def(MethodDef {
        name,
        signature: zun_metadata::BlobIndex(0),
        body_offset: body,
        is_static: true,
    });
    Some(CompiledIsMatch { module, is_match_method, buffer_bytes, group_count })
}

/// The first required char of a `Byte` / `Literal` prefilter, if any.
fn literal_first(pf: &Prefilter) -> Option<char> {
    match pf {
        Prefilter::Byte(c) => Some(*c),
        Prefilter::Literal(lit) => lit.first().copied(),
        _ => None,
    }
}

/// Inclusive ranges of JS line terminators (matching [`is_line_terminator`]).
fn line_terminator_ranges() -> Vec<(u32, u32)> {
    debug_assert!(['\n', '\r', '\u{2028}', '\u{2029}'].iter().all(|&c| is_line_terminator(c)));
    vec![(0x0a, 0x0a), (0x0d, 0x0d), (0x2028, 0x2029)]
}
