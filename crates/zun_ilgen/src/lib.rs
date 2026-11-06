//! HIR → ZIL lowering.
//!
//! Walks the typed HIR from `zun_sema` and emits a complete `.zm`
//! [`Module`]: one `MethodDef` per function (RID = HIR index + 1, so call
//! tokens are known up front), framed bodies in the code section, and string
//! literals interned into the module string heap.
//!
//! Lowering invariants:
//! - `number` is f64 end to end: literals use `const.f64`, arithmetic the
//!   `.f64` opcodes; negation uses `neg.f64`.
//! - Booleans lower to `const.true`/`const.false`; `!x` lowers as
//!   `x == false`.
//! - An assignment expression leaves its value on the stack (`dup` before
//!   `stloc`); expression statements `pop` any non-void result.
//! - Branches patch forward displacements after the target is known
//!   (displacements are relative to the end of the branch instruction).

use zun_il::{decode_all, encode, Instruction, NumKind, Opcode, Operand};
use zun_common::Span;
use zun_metadata::{
    BlobIndex, FieldDef, MemberRef, MethodDef, ModuleRef, TableKind, Token, TypeDef,
};
use zun_module::{read_body, write_body, write_body_with_handlers, ExceptionHandler, HandlerKind, Module};
use zun_regex::zil::MatcherNatives;
use zun_sema::hir::{
    self, BinaryOp, CaptureMode, CaptureSource, ExprKind, FloatKind, IntKind, Stmt, Ty, UnaryOp,
};

#[path = "helpers.rs"]
mod helpers;
use helpers::{field_kind_byte, int_or_float, jitkind_byte};

/// `zun:math` natives the machine can perform in a single instruction, and the
/// ZIL opcode that replaces the call (PLAN.md §3, criterion (d)).
///
/// The native stays registered and keeps working: this is a lowering
/// substitution, so an entry that stops matching costs speed, never
/// correctness. `MATH_MACHINE_OPS` is checked against the registry by test, to
/// catch the drift .docs-dotnet/07-actions-operations/05-jit-intrinsics.md
/// warns of, where renaming a native silently disables the expansion.
///
/// Absent on purpose: the transcendentals, which are libm's job. `round` and
/// `sign` are here despite not being single instructions — neither has an ISA
/// equivalent with JS semantics, so each lowers to a short branchless
/// sequence instead of a call.
pub const MATH_MACHINE_OPS: &[(&str, Opcode)] = &[
    ("sqrt", Opcode::SqrtF64),
    ("sqrtNumber", Opcode::SqrtF64),
    ("abs", Opcode::AbsF64),
    ("floor", Opcode::FloorF64),
    ("floorNumber", Opcode::FloorF64),
    ("ceil", Opcode::CeilF64),
    ("ceilNumber", Opcode::CeilF64),
    ("truncNumber", Opcode::TruncF64),
    ("min", Opcode::MinF64),
    ("minNumber", Opcode::MinF64),
    ("max", Opcode::MaxF64),
    ("maxNumber", Opcode::MaxF64),
    ("roundNumber", Opcode::RoundF64),
    ("signNumber", Opcode::SignF64),
];

/// The machine op that replaces native `id` called with `argc` arguments, if
/// there is one.
///
/// Two guards keep the substitution honest. The id must be the `zun:math`
/// function of that name, so a same-named native in another module is never
/// rewritten; and the argument count must equal the opcode's own `pops`, read
/// from the master table rather than restated here, so a table row and this
/// mapping cannot disagree about arity.
fn math_machine_op(id: u32, argc: usize) -> Option<Opcode> {
    let native = zun_natives::by_id(id)?;
    let (_, op) = MATH_MACHINE_OPS
        .iter()
        .find(|(name, _)| *name == native.name)?;
    if zun_natives::resolve("math", native.name)?.0 != id {
        return None;
    }
    let zun_il::StackEffect::Fixed { pops, .. } = op.stack_effect() else {
        return None;
    };
    (argc == pops as usize && native.params.len() == pops as usize).then_some(*op)
}

/// Map a checked numeric type to the runtime [`NumKind`], or `None` if the type
/// is not numeric.
pub(crate) fn numkind_of(ty: Ty) -> Option<NumKind> {
    Some(match ty {
        Ty::Int(IntKind {
            bits: 8,
            signed: true,
        }) => NumKind::I8,
        Ty::Int(IntKind {
            bits: 16,
            signed: true,
        }) => NumKind::I16,
        Ty::Int(IntKind {
            bits: 32,
            signed: true,
        }) => NumKind::I32,
        Ty::Int(IntKind {
            bits: 64,
            signed: true,
        }) => NumKind::I64,
        Ty::Int(IntKind {
            bits: 8,
            signed: false,
        }) => NumKind::U8,
        Ty::Int(IntKind {
            bits: 16,
            signed: false,
        }) => NumKind::U16,
        Ty::Int(IntKind {
            bits: 32,
            signed: false,
        }) => NumKind::U32,
        Ty::Int(IntKind {
            bits: 64,
            signed: false,
        }) => NumKind::U64,
        Ty::Int(_) => return None,
        Ty::Float(FloatKind::F32) => NumKind::F32,
        Ty::Float(FloatKind::F64) => NumKind::F64,
        _ => return None,
    })
}

fn ffi_type_tag(ty: Ty) -> u8 {
    match ty {
        Ty::Int(IntKind { bits: 32, signed: true }) => 5,
        Ty::Int(IntKind { bits: 32, signed: false }) => 6,
        Ty::Int(IntKind { bits: 64, .. }) => 1,
        Ty::Float(FloatKind::F64) => 2,
        Ty::Boolean => 3,
        Ty::String => 4,
        _ => 0,
    }
}


/// Tokens a lowered program needs to reference its own metadata.
struct Layout {
    /// TypeDef token per class index.
    type_tokens: Vec<Token>,
    /// FieldDef token per (class index, field slot).
    field_tokens: Vec<Vec<Token>>,
}

/// How a [`hir::ExprKind::RegexMatch`] is lowered: either to a call to the
/// synthesized ZIL matcher (JIT-eligible, tiers up like any loop) or, when the
/// pattern is outside the compilable subset, to the runtime regex engine.
enum RegexPlan {
    /// Compiled to a synthesized `isMatch(s): bool` method; call its RID.
    Compiled { is_match_rid: u32 },
    /// Outside the compilable subset (lookaround/backref/`i` flag/too many
    /// groups): build the runtime `RegExp` and call `regexpIsMatch`, exactly as
    /// the non-static path would. Observably identical to the old behavior.
    Fallback {
        regexp_new_id: u32,
        is_match_id: u32,
        pattern: String,
        flags: String,
    },
}


/// A pending cleanup that an abrupt exit (`break`/`continue`/`return`) must run
/// before leaving its scope: a `finally` body, or the iterator-close protocol of
/// a `for-of` loop (`loop_index` identifies the owning loop).
#[derive(Clone, Copy)]
enum Cleanup<'a> {
    Block(&'a [Stmt]),
    IterClose { slot: u16, loop_index: usize },
}

/// Handle to a forward branch whose displacement is patched once the target
/// offset is known.
struct BranchPatch {
    /// Byte offset of the 4-byte displacement inside the code buffer.
    operand_at: usize,
}

struct FnLowerer<'a, 'b> {
    code: Vec<u8>,
    layout: &'a Layout,
    module: &'b mut Module,
    break_patches: Vec<Vec<BranchPatch>>,
    loop_labels: Vec<Option<String>>,
    pending_label: Option<String>,
    continue_targets: Vec<Vec<usize>>,
    continue_patches: Vec<Vec<BranchPatch>>,
    handlers: Vec<ExceptionHandler>,
    finally_bodies: Vec<Cleanup<'a>>,
    /// `finally_bodies.len()` when each enclosing break target was entered.
    break_finally_depths: Vec<usize>,
    /// Code ranges that are inlined copies of a `finally` body (start, end,
    /// index of the owning cleanup); excluded from exception handler ranges.
    finally_holes: Vec<(u32, u32, usize)>,
    /// Index of the closure being lowered (for [`ExprKind::SelfClosure`]).
    self_closure: Option<u32>,
    closure_method_base: u32,
    points: Vec<(u32, Span)>,
    /// Lowering plan for each static regex (parallel to
    /// `Program::regex_matchers`); see [`RegexPlan`].
    regex_plan: &'a [RegexPlan],
    /// Whether the function being lowered is `async` (awaits suspend like yields).
    is_async: bool,
    /// Whether it is an async generator (awaits are tagged so the driver can tell
    /// them from yields).
    is_async_gen: bool,
}

mod emit;
mod expr;
mod function;
mod program;
mod stmt;

pub use program::lower;

#[cfg(test)]
mod tests;
