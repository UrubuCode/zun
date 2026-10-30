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

/// Lower a checked program into a `.zm` module.
pub fn lower(program: &hir::Program) -> Module {
    let mut module = Module::new();
    module.requires_codegen = !program.codegen_sites.is_empty();
    module.codegen_sites = program.codegen_sites.clone();
    let layout = emit_classes(program, &mut module);

    // Reserve one MethodDef per function first so call sites can name any
    // function by token regardless of order. body_offset is patched after
    // each body is emitted.
    for f in &program.functions {
        let name = module.strings.intern(&f.name);
        module.tables.add_method_def(MethodDef {
            name,
            signature: BlobIndex(0), // signatures land with the verifier
            body_offset: u32::MAX,
            is_static: f.is_static,
        });
    }
    for closure in &program.closures {
        let name = module.strings.intern(&closure.name);
        module.tables.add_method_def(MethodDef {
            name,
            signature: BlobIndex(0),
            body_offset: u32::MAX,
            is_static: false,
        });
    }

    // Synthesize ZIL matchers for the static regexes (the `[GeneratedRegex]`
    // analog) and splice them in as extra methods *before* lowering bodies, so
    // their RIDs are allocated when call sites reference them. Patterns outside
    // the compilable subset fall back to the runtime engine (see `RegexPlan`).
    let regex_plan = splice_regex_matchers(program, &mut module);

    for (index, f) in program.functions.iter().enumerate() {
        let (code, handlers, points) =
            lower_function(f, &layout, &mut module, program.functions.len() as u32, &regex_plan);
        module.sequence_points.extend(points.into_iter().map(|(pc, span)| {
            zun_module::SequencePoint { method_rid: index as u32 + 1, pc, source_start: span.start, source_end: span.end }
        }));
        let local_kinds: Vec<u8> = f.locals.iter().map(|t| jitkind_byte(*t)).collect();
        let offset = write_body_with_handlers(
            &mut module.code,
            f.param_count,
            f.locals.len() as u16,
            f.return_ty != Ty::Void,
            jitkind_byte(f.return_ty),
            &local_kinds,
            &code,
            &handlers,
        );
        module.tables.method_defs[index].body_offset = offset;
    }
    let closure_base = program.functions.len();
    for (index, closure) in program.closures.iter().enumerate() {
        let (code, handlers, points) = lower_closure_function(
            closure,
            &layout,
            &mut module,
            program.functions.len() as u32,
            &regex_plan,
        );
        module.sequence_points.extend(points.into_iter().map(|(pc, span)| {
            zun_module::SequencePoint { method_rid: (closure_base + index) as u32 + 1, pc, source_start: span.start, source_end: span.end }
        }));
        let local_kinds: Vec<u8> = closure.locals.iter().map(|t| jitkind_byte(*t)).collect();
        let offset = write_body_with_handlers(
            &mut module.code,
            closure.param_count + 1,
            local_kinds.len() as u16,
            closure.return_ty != Ty::Void,
            jitkind_byte(closure.return_ty),
            &local_kinds,
            &code,
            &handlers,
        );
        module.tables.method_defs[closure_base + index].body_offset = offset;
    }

    module.entry_point = program
        .main
        .map(|index| Token::new(TableKind::MethodDef, index + 1));
    // Record the module initializer's MethodDef token so the runtime can run it
    // once before the entry point (PLAN §3.1, doc 02 area 10).
    module.module_initializer = program
        .module_init
        .map(|index| Token::new(TableKind::MethodDef, index + 1));
    module
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

/// Resolve the native ids the synthesized matcher calls. `None` only if the
/// frozen string/buffer builtins are missing (then every regex falls back).
fn matcher_natives() -> Option<MatcherNatives> {
    Some(MatcherNatives {
        length: zun_natives::resolve("string", "length")?.0,
        char_code_at: zun_natives::resolve("string", "charCodeAt")?.0,
        buffer_read_u32: zun_natives::resolve("buffer", "bufferReadU32")?.0,
        buffer_write_u32: zun_natives::resolve("buffer", "bufferWriteU32")?.0,
        buffer_new: zun_natives::resolve("buffer", "bufferNew")?.0,
    })
}

/// Compile each static regex to a ZIL matcher and splice it into `module`,
/// returning a lowering plan per matcher (parallel to `program.regex_matchers`).
fn splice_regex_matchers(program: &hir::Program, module: &mut Module) -> Vec<RegexPlan> {
    if program.regex_matchers.is_empty() {
        return Vec::new();
    }
    let nat = matcher_natives();
    let fallback = |pattern: &str, flags: &str| -> RegexPlan {
        // These natives back the ordinary (non-static) regex path, so sema has
        // already required them to exist by the time any regex is lowered.
        let regexp_new_id = zun_natives::resolve("collections", "regexpNew")
            .expect("regexpNew intrinsic")
            .0;
        let is_match_id = zun_natives::resolve("collections", "regexpIsMatch")
            .expect("regexpIsMatch intrinsic")
            .0;
        RegexPlan::Fallback {
            regexp_new_id,
            is_match_id,
            pattern: pattern.to_owned(),
            flags: flags.to_owned(),
        }
    };
    let mut plans = Vec::with_capacity(program.regex_matchers.len());
    for cr in &program.regex_matchers {
        let compiled = nat
            .and_then(|nat| zun_regex::zil::compile_is_match(&cr.pattern, &cr.flags, nat));
        match compiled {
            Some(ci) => {
                let base = splice_module(module, &ci.module);
                let is_match_rid = base + (ci.is_match_method.rid() - 1);
                plans.push(RegexPlan::Compiled { is_match_rid });
            }
            None => plans.push(fallback(&cr.pattern, &cr.flags)),
        }
    }
    plans
}

/// Copy every method of a synthesized `src` module into `dst`, returning the
/// RID that `src`'s first method (RID 1) receives in `dst`. Internal
/// `Call <MethodDef>` tokens are shifted by the same delta so the spliced
/// methods still call each other; every other reference in these bodies is a
/// `call.builtin` to a global native id, which needs no remap.
fn splice_module(dst: &mut Module, src: &Module) -> u32 {
    let base = dst.tables.method_defs.len() as u32 + 1;
    let delta = base - 1;
    for md in &src.tables.method_defs {
        let body = read_body(&src.code, md.body_offset).expect("synthesized body reads back");
        let instrs = decode_all(body.code).expect("synthesized code decodes");
        let mut code = Vec::new();
        for ins in &instrs {
            // Re-encoding is length-preserving (fixed-width operands), so
            // relative branch displacements stay valid after the token rewrite.
            let remapped = match (ins.opcode, ins.operand) {
                (Opcode::Call, Operand::Token(t)) if t.table() == Some(TableKind::MethodDef) => {
                    Instruction::new(
                        Opcode::Call,
                        Operand::Token(Token::new(TableKind::MethodDef, t.rid() + delta)),
                    )
                }
                _ => *ins,
            };
            encode(&remapped, &mut code);
        }
        // Names must be module-unique (the loader rejects duplicate exports),
        // so disambiguate every synthesized method with its splice base RID.
        // These methods are only ever reached by `Call <token>`, never by name.
        let name = dst.strings.intern(&format!("{}${}", src.strings.get(md.name), base));
        let offset = write_body(
            &mut dst.code,
            body.param_count,
            body.local_count,
            body.returns_value,
            body.return_kind,
            body.local_kinds,
            &code,
        );
        dst.tables.add_method_def(MethodDef {
            name,
            signature: BlobIndex(0),
            body_offset: offset,
            is_static: md.is_static,
        });
    }
    base
}

/// The method-body value-type code ([`zun_il::JitKind`]) for a checked type.
/// Total number of methods across all classes.
fn total_methods(program: &hir::Program) -> usize {
    program.classes.iter().map(|c| c.methods.len()).sum()
}

/// Emit TypeDef and FieldDef rows for every class and return their tokens.
fn emit_classes(program: &hir::Program, module: &mut Module) -> Layout {
    let mut type_tokens = Vec::with_capacity(program.classes.len());
    let mut field_tokens = Vec::with_capacity(program.classes.len());

    // The number of top-level functions: method MethodDef RIDs follow them.
    let top_level_fns = program.functions.len() - total_methods(program);

    for (class_idx, class) in program.classes.iter().enumerate() {
        let name = module.strings.intern(&class.name);

        // Field layout is the full inherited-first list; a FieldDef is emitted
        // per full-layout slot so ldfld/stfld tokens address the right slot.
        let mut this_class_fields = Vec::with_capacity(class.fields.len());
        let field_start = module.tables.field_defs.len() as u32 + 1; // RIDs are 1-based
        for field in &class.fields {
            let fname = module.strings.intern(&field.name);
            let tok = module.tables.add_field_def(FieldDef {
                name: fname,
                signature: BlobIndex(0),
                kind: field_kind_byte(field.ty),
                is_static: field.is_static,
                is_private: field.is_private,
            });
            this_class_fields.push(tok);
        }
        let field_end = module.tables.field_defs.len() as u32 + 1;

        // This class's own methods are a contiguous MethodDef range. Their RIDs
        // are func_index + 1, and func indices are top-level functions first,
        // then methods in class order, so the range is derived from them.
        let (method_start, method_end) = match (class.methods.first(), class.methods.last()) {
            (Some(&first), Some(&last)) => (first + 1, last + 2),
            _ => (0, 0),
        };
        let _ = top_level_fns;

        let base = match class.base {
            Some(b) => Token::new(TableKind::TypeDef, b + 1),
            None => Token::null(TableKind::TypeDef),
        };

        let tok = module.tables.add_type_def(TypeDef {
            name,
            namespace: zun_metadata::StringIndex(0),
            base,
            field_start,
            field_end,
            method_start,
            method_end,
        });
        debug_assert_eq!(tok.rid(), class_idx as u32 + 1);
        type_tokens.push(tok);
        field_tokens.push(this_class_fields);
    }

    Layout {
        type_tokens,
        field_tokens,
    }
}

/// Lower one function body to ZIL bytes.
fn lower_function<'a>(
    f: &'a hir::Function,
    layout: &'a Layout,
    module: &mut Module,
    closure_method_base: u32,
    regex_plan: &'a [RegexPlan],
) -> (Vec<u8>, Vec<ExceptionHandler>, Vec<(u32, Span)>) {
    let mut gen = FnLowerer {
        code: Vec::new(),
        layout,
        module,
        break_patches: Vec::new(),
        loop_labels: Vec::new(),
        pending_label: None,
        continue_targets: Vec::new(),
        continue_patches: Vec::new(),
        handlers: Vec::new(),
        finally_bodies: Vec::new(),
        closure_method_base,
        points: Vec::new(),
        regex_plan,
    };
    gen.emit_defaults(&f.defaults, f.param_count - f.defaults.len() as u16);
    gen.stmts(&f.body);
    // Guarantee the body cannot run off its end: a void function gets an
    // implicit return. (Non-void paths are the verifier's job later.)
    if f.return_ty == Ty::Void || !matches!(f.body.last(), Some(Stmt::Return(_))) {
        gen.emit(Instruction::simple(Opcode::Ret));
    }
    (gen.code, gen.handlers, gen.points)
}

/// Lower a synthetic closure function. Its hidden environment is local slot 0
/// and its user locals are shifted by one slot in the emitted method body.
fn lower_closure_function<'a>(
    f: &'a hir::ClosureFunction,
    layout: &'a Layout,
    module: &mut Module,
    closure_method_base: u32,
    regex_plan: &'a [RegexPlan],
) -> (Vec<u8>, Vec<ExceptionHandler>, Vec<(u32, Span)>) {
    let mut gen = FnLowerer {
        code: Vec::new(),
        layout,
        module,
        break_patches: Vec::new(),
        loop_labels: Vec::new(),
        pending_label: None,
        continue_targets: Vec::new(),
        continue_patches: Vec::new(),
        handlers: Vec::new(),
        finally_bodies: Vec::new(),
        closure_method_base,
        points: Vec::new(),
        regex_plan,
    };
    gen.emit_defaults(&f.defaults, 1);
    gen.stmts(&f.body);
    if f.return_ty == Ty::Void || !matches!(f.body.last(), Some(Stmt::Return(_))) {
        gen.emit(Instruction::simple(Opcode::Ret));
    }
    (gen.code, gen.handlers, gen.points)
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
    finally_bodies: Vec<&'a [Stmt]>,
    closure_method_base: u32,
    points: Vec<(u32, Span)>,
    /// Lowering plan for each static regex (parallel to
    /// `Program::regex_matchers`); see [`RegexPlan`].
    regex_plan: &'a [RegexPlan],
}

impl<'a, 'b> FnLowerer<'a, 'b> {
    fn emit(&mut self, instr: Instruction) {
        encode(&instr, &mut self.code);
    }

    /// Materialize omitted arguments before entering the function body. Calls
    /// use null sentinels so the framed ABI remains fixed-width.
    fn emit_defaults(&mut self, defaults: &[Option<hir::Expr>], slot_offset: u16) {
        for (index, default) in defaults.iter().enumerate() {
            let Some(default) = default else { continue };
            let slot = slot_offset + index as u16;
            self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(slot)));
            self.emit(Instruction::simple(Opcode::BoxAny));
            self.emit(Instruction::simple(Opcode::ConstNull));
            self.emit(Instruction::simple(Opcode::CmpEq));
            let skip = self.emit_branch(Opcode::BranchFalse);
            self.expr(default);
            self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(slot)));
            self.patch_to_here(skip);
        }
    }

    fn emit_dynamic_load(&mut self, name: &str) {
        let property = self.module.strings.intern(name);
        self.emit(Instruction::new(
            Opcode::LoadDynamic,
            Operand::StringRef(property.0),
        ));
    }

    fn emit_dynamic_store(&mut self, name: &str) {
        let property = self.module.strings.intern(name);
        self.emit(Instruction::new(
            Opcode::StoreDynamic,
            Operand::StringRef(property.0),
        ));
    }

    /// Push a numeric constant of the given type: a base `const.i32`/`const.i64`
    /// or `const.f64`, then a `conv` when the target kind is narrower/unsigned
    /// or single-precision.
    fn emit_numeric_const(&mut self, v: f64, ty: Ty) {
        let kind = numkind_of(ty).unwrap_or(NumKind::F64);
        match kind {
            NumKind::F64 => {
                self.emit(Instruction::new(Opcode::ConstF64, Operand::Float64(v)));
            }
            NumKind::F32 => {
                self.emit(Instruction::new(Opcode::ConstF64, Operand::Float64(v)));
                self.emit(Instruction::new(
                    Opcode::Conv,
                    Operand::Imm32(NumKind::F32.to_imm()),
                ));
            }
            NumKind::I64 | NumKind::U64 => {
                self.emit(Instruction::new(Opcode::ConstI64, Operand::Imm64(v as i64)));
                if kind != NumKind::I64 {
                    self.emit(Instruction::new(
                        Opcode::Conv,
                        Operand::Imm32(kind.to_imm()),
                    ));
                }
            }
            _ => {
                // i8/i16/i32/u8/u16/u32: build as i32, then narrow if needed.
                //
                // Via i64, because `f64 as i32` saturates in Rust: a `u32`
                // literal above i32::MAX became i32::MAX, so
                // `let a: u32 = 4294967295` silently held 2147483647. Going
                // through i64 and truncating keeps the bit pattern, which the
                // following `conv` reinterprets as the target width. Sema has
                // already checked the literal fits that width.
                self.emit(Instruction::new(
                    Opcode::ConstI32,
                    Operand::Imm32(v as i64 as i32),
                ));
                if kind != NumKind::I32 {
                    self.emit(Instruction::new(
                        Opcode::Conv,
                        Operand::Imm32(kind.to_imm()),
                    ));
                }
            }
        }
    }

    /// Emit a `conv` to an explicit numeric kind.
    fn emit_conv_kind(&mut self, kind: NumKind) {
        self.emit(Instruction::new(Opcode::Conv, Operand::Imm32(kind.to_imm())));
    }

    /// Emit the JS `ToInt32`/`ToUint32` coercion of the `number` on top of the
    /// stack. Routed through an i64 intermediate (`f64 -> i64 -> i32/u32`), not a
    /// direct `f64 -> u32`, because a direct unsigned saturating convert maps a
    /// negative operand to 0 while JS wraps it modulo 2^32 (`(-1) >>> 0` must be
    /// 4294967295). Both tiers share this IL, so they agree by construction.
    /// Values beyond the i64 range saturate rather than reduce modulo 2^32 — a
    /// documented edge, consistent with the saturating-convert policy.
    fn emit_to_int32(&mut self, unsigned: bool) {
        // fmod by 2^32 first: the f64->i64 conversion saturates, so values at or
        // beyond 2^63 would otherwise lose their low 32 bits. The fmod is exact
        // and sends NaN/Infinity to NaN (which converts to 0).
        self.emit(Instruction::new(Opcode::ConstF64, Operand::Float64(4294967296.0)));
        self.emit(Instruction::simple(Opcode::RemF64));
        self.emit_conv_kind(NumKind::I64);
        self.emit_conv_kind(if unsigned { NumKind::U32 } else { NumKind::I32 });
    }

    /// Emit a `conv` to `ty` (used by the `as` operator).
    fn emit_conv_to(&mut self, ty: Ty) {
        if let Some(kind) = numkind_of(ty) {
            self.emit(Instruction::new(
                Opcode::Conv,
                Operand::Imm32(kind.to_imm()),
            ));
        }
    }

    /// Emit a branch with a placeholder displacement; returns a patch handle.
    fn emit_branch(&mut self, opcode: Opcode) -> BranchPatch {
        self.emit(Instruction::new(opcode, Operand::Branch(0)));
        BranchPatch {
            // The 4 displacement bytes sit at the end of what we just wrote.
            operand_at: self.code.len() - 4,
        }
    }

    /// Point `patch` at the current end of code.
    fn patch_to_here(&mut self, patch: BranchPatch) {
        let disp = (self.code.len() - (patch.operand_at + 4)) as i32;
        self.code[patch.operand_at..patch.operand_at + 4].copy_from_slice(&disp.to_le_bytes());
    }

    /// Point `patch` at an arbitrary code offset.
    fn patch_to_target(&mut self, patch: BranchPatch, target: usize) {
        let disp = (target as i64 - (patch.operand_at + 4) as i64) as i32;
        self.code[patch.operand_at..patch.operand_at + 4].copy_from_slice(&disp.to_le_bytes());
    }

    /// Emit a backward branch to `target` (a previously recorded code offset).
    fn emit_branch_back(&mut self, opcode: Opcode, target: usize) {
        // Displacement is relative to the end of this instruction: 1 opcode
        // byte + 4 operand bytes past the current position.
        let disp = target as i64 - (self.code.len() + 5) as i64;
        self.emit(Instruction::new(opcode, Operand::Branch(disp as i32)));
    }

    fn stmts(&mut self, stmts: &'a [Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Let { slot, init } => {
                self.expr(init);
                self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
            }
            Stmt::Group(statements) => {
                for statement in statements {
                    self.stmt(statement);
                }
            }
            Stmt::BoxedLet { slot, init } => {
                self.emit(Instruction::simple(Opcode::NewDynamic));
                self.emit(Instruction::simple(Opcode::Dup));
                self.expr(init);
                self.emit(Instruction::simple(Opcode::BoxAny));
                self.emit_dynamic_store("$value");
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
            }
            Stmt::Expr(e) => {
                // Every expression leaves exactly one value on the stack in this
                // model — including void calls, which push a void value — so an
                // expression statement always discards one.
                self.expr(e);
                if !matches!(e.kind, ExprKind::Yield { .. }) {
                    self.emit(Instruction::simple(Opcode::Pop));
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                self.expr(cond);
                let to_else = self.emit_branch(Opcode::BranchFalse);
                self.stmts(then_body);
                if else_body.is_empty() {
                    self.patch_to_here(to_else);
                } else {
                    let to_end = self.emit_branch(Opcode::Branch);
                    self.patch_to_here(to_else);
                    self.stmts(else_body);
                    self.patch_to_here(to_end);
                }
            }
            Stmt::While { cond, body } => {
                let loop_start = self.code.len();
                self.expr(cond);
                let to_exit = self.emit_branch(Opcode::BranchFalse);
                self.break_patches.push(Vec::new());
                self.loop_labels.push(self.pending_label.take());
                self.continue_targets.push(vec![loop_start]);
                self.continue_patches.push(Vec::new());
                self.stmts(body);
                self.emit_branch_back(Opcode::Branch, loop_start);
                self.patch_to_here(to_exit);
                self.continue_targets.pop();
                self.continue_patches.pop();
                self.loop_labels.pop();
                for patch in self.break_patches.pop().unwrap_or_default() {
                    self.patch_to_here(patch);
                }
            }
            Stmt::DoWhile { body, cond } => {
                let loop_start = self.code.len();
                self.break_patches.push(Vec::new());
                self.loop_labels.push(self.pending_label.take());
                self.continue_targets.push(vec![usize::MAX]);
                self.continue_patches.push(Vec::new());
                self.stmts(body);
                let continue_target = self.code.len();
                self.continue_targets.pop();
                self.loop_labels.pop();
                for patch in self.continue_patches.pop().unwrap_or_default() {
                    self.patch_to_target(patch, continue_target);
                }
                self.expr(cond);
                self.emit_branch_back(Opcode::BranchTrue, loop_start);
                for patch in self.break_patches.pop().unwrap_or_default() {
                    self.patch_to_here(patch);
                }
            }
            Stmt::For {
                init,
                cond,
                update,
                body,
            } => {
                if let Some(init) = init {
                    self.stmt(init);
                }
                let loop_start = self.code.len();
                self.expr(cond);
                let to_exit = self.emit_branch(Opcode::BranchFalse);
                self.break_patches.push(Vec::new());
                self.loop_labels.push(self.pending_label.take());
                self.continue_targets.push(vec![usize::MAX]);
                self.continue_patches.push(Vec::new());
                self.stmts(body);
                let update_target = self.code.len();
                self.continue_targets.pop();
                self.loop_labels.pop();
                for patch in self.continue_patches.pop().unwrap_or_default() {
                    self.patch_to_target(patch, update_target);
                }
                if let Some(update) = update {
                    self.expr(update);
                    self.emit(Instruction::simple(Opcode::Pop));
                }
                self.emit_branch_back(Opcode::Branch, loop_start);
                self.patch_to_here(to_exit);
                for patch in self.break_patches.pop().unwrap_or_default() {
                    self.patch_to_here(patch);
                }
            }
            Stmt::ForOf {
                iterator,
                iterator_slot,
                done,
                next,
                binding,
                body,
            } => {
                self.expr(iterator);
                self.emit(Instruction::new(
                    Opcode::StoreLocal,
                    Operand::Local(*iterator_slot),
                ));
                let loop_start = self.code.len();
                self.expr(done);
                let to_exit = self.emit_branch(Opcode::BranchTrue);
                self.break_patches.push(Vec::new());
                self.loop_labels.push(self.pending_label.take());
                self.continue_targets.push(vec![loop_start]);
                self.continue_patches.push(Vec::new());
                self.expr(next);
                self.emit(Instruction::new(
                    Opcode::StoreLocal,
                    Operand::Local(*binding),
                ));
                self.stmts(body);
                self.emit_branch_back(Opcode::Branch, loop_start);
                self.patch_to_here(to_exit);
                self.continue_targets.pop();
                self.continue_patches.pop();
                self.loop_labels.pop();
                for patch in self.break_patches.pop().unwrap_or_default() {
                    self.patch_to_here(patch);
                }
            }
            Stmt::Labeled { label, body } => {
                let previous = self.pending_label.replace(label.clone());
                self.stmt(body);
                self.pending_label = previous;
            }
            Stmt::Break(label) => {
                let patch = self.emit_branch(Opcode::Branch);
                let target = label.as_ref().and_then(|name| {
                    self.loop_labels.iter().rposition(|candidate| candidate.as_ref() == Some(name))
                }).or_else(|| self.break_patches.len().checked_sub(1));
                if let Some(index) = target {
                    if let Some(patches) = self.break_patches.get_mut(index) {
                        patches.push(patch);
                    }
                }
            }
            Stmt::Continue(label) => {
                let target_index = label.as_ref().and_then(|name| {
                    self.loop_labels.iter().rposition(|candidate| candidate.as_ref() == Some(name))
                }).or_else(|| self.continue_targets.len().checked_sub(1));
                if let Some(index) = target_index {
                    let target = self.continue_targets[index].last().copied().unwrap_or(usize::MAX);
                    if target == usize::MAX {
                        let patch = self.emit_branch(Opcode::Branch);
                        if let Some(patches) = self.continue_patches.get_mut(index) {
                            patches.push(patch);
                        }
                    } else {
                        self.emit_branch_back(Opcode::Branch, target);
                    }
                }
            }
            Stmt::Return(value) => {
                let cleanups: Vec<&[Stmt]> = self.finally_bodies.iter().rev().copied().collect();
                for cleanup in cleanups {
                    self.stmts(cleanup);
                }
                if let Some(v) = value {
                    self.expr(v);
                }
                self.emit(Instruction::simple(Opcode::Ret));
            }
            Stmt::Throw(value) => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::Throw));
            }
            Stmt::Try {
                body,
                catches,
                finally_body,
            } => {
                let try_start = self.code.len() as u32;
                if let Some(finally_body) = finally_body {
                    self.finally_bodies.push(finally_body);
                }
                self.stmts(body);
                if finally_body.is_some() {
                    self.finally_bodies.pop();
                }
                let try_end = self.code.len() as u32;
                let mut exits = Vec::new();
                if let Some(finally_body) = finally_body {
                    self.stmts(finally_body);
                }
                exits.push(self.emit_branch(Opcode::Branch));

                for catch in catches {
                    let handler_start = self.code.len() as u32;
                    // A matched handler receives the exception object on the
                    // evaluation stack; bind it before entering user code.
                    self.emit(Instruction::new(
                        Opcode::StoreLocal,
                        Operand::Local(catch.slot),
                    ));
                    if let Some(finally_body) = finally_body {
                        self.finally_bodies.push(finally_body);
                    }
                    self.stmts(&catch.body);
                    if finally_body.is_some() {
                        self.finally_bodies.pop();
                    }
                    if let Some(finally_body) = finally_body {
                        self.stmts(finally_body);
                    }
                    let handler_end = self.code.len() as u32;
                    exits.push(self.emit_branch(Opcode::Branch));
                    self.handlers.push(ExceptionHandler {
                        try_start,
                        try_end,
                        handler_start,
                        handler_end,
                        kind: HandlerKind::Catch,
                        catch_type: match catch.ty {
                            Ty::Instance(index) => Some(Token::new(TableKind::TypeDef, index + 1)),
                            _ => None,
                        },
                    });
                }

                if let Some(finally_body) = finally_body {
                    let handler_start = self.code.len() as u32;
                    self.stmts(finally_body);
                    self.emit(Instruction::simple(Opcode::Rethrow));
                    let handler_end = self.code.len() as u32;
                    self.handlers.push(ExceptionHandler {
                        try_start,
                        try_end,
                        handler_start,
                        handler_end,
                        kind: HandlerKind::Finally,
                        catch_type: None,
                    });
                }
                for exit in exits {
                    self.patch_to_here(exit);
                }
            }
        }
    }

    fn expr(&mut self, e: &hir::Expr) {
        if !e.span.is_empty() {
            self.points.push((self.code.len() as u32, e.span));
        }
        match &e.kind {
            ExprKind::Number(v) => self.emit_numeric_const(*v, e.ty),
            ExprKind::String(s) => {
                let idx = self.module.strings.intern(s);
                self.emit(Instruction::new(
                    Opcode::ConstStr,
                    Operand::StringRef(idx.0),
                ));
            }
            ExprKind::Boolean(true) => self.emit(Instruction::simple(Opcode::ConstTrue)),
            ExprKind::Boolean(false) => self.emit(Instruction::simple(Opcode::ConstFalse)),
            ExprKind::Null => self.emit(Instruction::simple(Opcode::ConstNull)),
            ExprKind::Local(slot) => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
            }
            ExprKind::CellLoad { slot } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
                self.emit_dynamic_load("$value");
                self.emit(Instruction::simple(Opcode::UnboxAny));
            }
            ExprKind::CellRef { slot } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
            }
            ExprKind::CaptureRef { environment_slot } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                self.emit_dynamic_load(&capture_name(*environment_slot));
                self.emit(Instruction::simple(Opcode::UnboxAny));
            }
            ExprKind::CellStore { slot, value } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
                self.expr(value);
                self.emit(Instruction::simple(Opcode::BoxAny));
                self.emit_dynamic_store("$value");
                self.emit(Instruction::simple(Opcode::UnboxAny));
            }
            ExprKind::FunctionRef { func } => {
                self.emit(Instruction::new(
                    Opcode::LoadFunction,
                    Operand::Token(Token::new(TableKind::MethodDef, *func + 1)),
                ));
            }
            ExprKind::Closure { function, captures } => {
                self.emit(Instruction::simple(Opcode::NewDynamic));
                for capture in captures {
                    self.emit(Instruction::simple(Opcode::Dup));
                    if capture.capture.mode == CaptureMode::ByCell {
                        match capture.capture.source {
                            CaptureSource::Local(slot) => {
                                self.emit(Instruction::new(
                                    Opcode::LoadLocal,
                                    Operand::Local(slot),
                                ));
                                self.emit(Instruction::simple(Opcode::BoxAny));
                            }
                            CaptureSource::Environment {
                                environment_slot, ..
                            } => {
                                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                                self.emit_dynamic_load(&capture_name(environment_slot));
                            }
                        }
                    } else {
                        self.expr(&capture.value);
                        // A function value cannot box to `Any`; it stays raw and
                        // `StoreDynamic` routes it to the environment's function
                        // slot. (ByCell/mutated function captures stay deferred:
                        // the `$value` cell slot is an `Any` and cannot hold one.)
                        if !matches!(capture.capture.ty, Ty::Function(_)) {
                            self.emit(Instruction::simple(Opcode::BoxAny));
                        }
                    }
                    self.emit_dynamic_store(&capture_name(capture.capture.environment_slot));
                    self.emit(Instruction::simple(Opcode::Pop));
                }
                let token = Token::new(
                    TableKind::MethodDef,
                    self.closure_method_base + *function + 1,
                );
                self.emit(Instruction::new(Opcode::MakeClosure, Operand::Token(token)));
            }
            ExprKind::CaptureLoad {
                environment_slot,
                mode,
                ty,
            } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                self.emit_dynamic_load(&capture_name(*environment_slot));
                if *mode == CaptureMode::ByCell {
                    self.emit(Instruction::simple(Opcode::UnboxAny));
                    self.emit_dynamic_load("$value");
                }
                // A by-value function capture lives in the function slot and
                // `LoadDynamic` already yields `Value::Function`: no unbox.
                if *mode == CaptureMode::ByCell || !matches!(ty, Ty::Function(_)) {
                    self.emit(Instruction::simple(Opcode::UnboxAny));
                }
            }
            ExprKind::CaptureStore {
                environment_slot,
                mode,
                value,
                ..
            } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                if *mode == CaptureMode::ByCell {
                    self.emit_dynamic_load(&capture_name(*environment_slot));
                    self.emit(Instruction::simple(Opcode::UnboxAny));
                }
                self.expr(value);
                self.emit(Instruction::simple(Opcode::BoxAny));
                if *mode == CaptureMode::ByCell {
                    self.emit_dynamic_store("$value");
                } else {
                    self.emit_dynamic_store(&capture_name(*environment_slot));
                }
                self.emit(Instruction::simple(Opcode::UnboxAny));
            }
            ExprKind::Sequence { effect, value } => {
                self.expr(effect);
                self.emit(Instruction::simple(Opcode::Pop));
                self.expr(value);
            }
            ExprKind::Assign { slot, value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
            }
            ExprKind::Await { operand } => {
                self.expr(operand);
                self.emit(Instruction::simple(Opcode::Await));
            }
            ExprKind::ImportDynamic { specifier } => {
                self.expr(specifier);
                self.emit(Instruction::simple(Opcode::ImportDynamic));
            }
            ExprKind::Yield { value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::Yield));
            }
            ExprKind::Binary { op, lhs, rhs } => {
                if *op == BinaryOp::Nullish {
                    self.expr(lhs);
                    self.emit(Instruction::simple(Opcode::Dup));
                    self.emit(Instruction::simple(Opcode::ConstNull));
                    self.emit(Instruction::simple(Opcode::CmpEq));
                    let use_rhs = self.emit_branch(Opcode::BranchTrue);
                    let end = self.emit_branch(Opcode::Branch);
                    self.patch_to_here(use_rhs);
                    self.emit(Instruction::simple(Opcode::Pop));
                    self.expr(rhs);
                    self.patch_to_here(end);
                    return;
                }
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    // Value-preserving short-circuit (JS semantics): the result
                    // is the operand that decided the outcome, not a boolean.
                    //   a || b: keep a when a is truthy, else evaluate b.
                    //   a && b: keep a when a is falsy, else evaluate b.
                    // Branch opcodes test truthiness, so this is correct for both
                    // boolean and dynamic operands.
                    self.expr(lhs);
                    self.emit(Instruction::simple(Opcode::Dup));
                    let keep = self.emit_branch(if *op == BinaryOp::And {
                        Opcode::BranchFalse
                    } else {
                        Opcode::BranchTrue
                    });
                    self.emit(Instruction::simple(Opcode::Pop));
                    self.expr(rhs);
                    self.patch_to_here(keep);
                    return;
                }
                // JS-number bitwise/shift: both operands are `number` (f64).
                // Coerce to int32 (uint32 for the `>>>` left operand), run the
                // integer op, then widen the result back to a `number`.
                let is_int = lhs.ty.is_int();
                if !is_int
                    && matches!(
                        op,
                        BinaryOp::BitAnd
                            | BinaryOp::BitOr
                            | BinaryOp::BitXor
                            | BinaryOp::Shl
                            | BinaryOp::Shr
                            | BinaryOp::UShr
                    )
                {
                    let is_ushr = *op == BinaryOp::UShr;
                    self.expr(lhs);
                    self.emit_to_int32(is_ushr);
                    self.expr(rhs);
                    self.emit_to_int32(false);
                    let opcode = match op {
                        BinaryOp::BitAnd => Opcode::AndI32,
                        BinaryOp::BitOr => Opcode::OrI32,
                        BinaryOp::BitXor => Opcode::XorI32,
                        BinaryOp::Shl => Opcode::ShlI32,
                        BinaryOp::Shr => Opcode::ShrI32,
                        BinaryOp::UShr => Opcode::UShrI32,
                        _ => unreachable!(),
                    };
                    self.emit(Instruction::simple(opcode));
                    // The int result (int32, or uint32 for `>>>`) widens back to
                    // a JS `number`.
                    self.emit_conv_kind(NumKind::F64);
                    return;
                }
                self.expr(lhs);
                self.expr(rhs);
                // Pick the opcode family from the operand type: int operands use
                // the i32 ops, floats the f64 ops, strings concatenate.
                let opcode = match op {
                    BinaryOp::Add if lhs.ty == Ty::String => Opcode::Concat,
                    BinaryOp::Add => int_or_float(is_int, Opcode::AddI32, Opcode::AddF64),
                    BinaryOp::Sub => int_or_float(is_int, Opcode::SubI32, Opcode::SubF64),
                    BinaryOp::Mul => int_or_float(is_int, Opcode::MulI32, Opcode::MulF64),
                    BinaryOp::Div => int_or_float(is_int, Opcode::DivI32, Opcode::DivF64),
                    BinaryOp::Rem => int_or_float(is_int, Opcode::RemI32, Opcode::RemF64),
                    // Integer `**` (number `**` is routed to `Math.pow` in sema).
                    BinaryOp::Pow => Opcode::PowI32,
                    // Integer bitwise/shift (the f64 forms returned above).
                    BinaryOp::BitAnd => Opcode::AndI32,
                    BinaryOp::BitOr => Opcode::OrI32,
                    BinaryOp::BitXor => Opcode::XorI32,
                    BinaryOp::Shl => Opcode::ShlI32,
                    BinaryOp::Shr => Opcode::ShrI32,
                    BinaryOp::UShr => Opcode::UShrI32,
                    // Comparisons share one opcode set; the runtime compares by
                    // the operands' actual kind.
                    BinaryOp::Eq | BinaryOp::LooseEq => Opcode::CmpEq,
                    BinaryOp::Ne | BinaryOp::LooseNe => Opcode::CmpNe,
                    BinaryOp::Lt => Opcode::CmpLt,
                    BinaryOp::Le => Opcode::CmpLe,
                    BinaryOp::Gt => Opcode::CmpGt,
                    BinaryOp::Ge => Opcode::CmpGe,
                    BinaryOp::And | BinaryOp::Or | BinaryOp::Nullish => {
                        unreachable!("logical operators lower through branches")
                    }
                    BinaryOp::In | BinaryOp::Instanceof => {
                        unreachable!("`in`/`instanceof` are lowered in sema")
                    }
                };
                self.emit(Instruction::simple(opcode));
            }
            ExprKind::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                self.expr(condition);
                let to_false = self.emit_branch(Opcode::BranchFalse);
                self.expr(when_true);
                let to_end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(to_false);
                self.expr(when_false);
                self.patch_to_here(to_end);
            }
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Neg if operand.ty.is_int() => {
                    self.expr(operand);
                    self.emit(Instruction::simple(Opcode::NegI32));
                }
                UnaryOp::Neg => {
                    // `neg.f64` flips the sign bit. The previous `0 - x` was
                    // wrong at zero: `0.0 - 0.0` is `+0.0`, while `-0.0` must
                    // stay `-0.0` (observable as `1 / -x === -Infinity`).
                    self.expr(operand);
                    self.emit(Instruction::simple(Opcode::NegF64));
                }
                UnaryOp::Not => {
                    // x == false
                    self.expr(operand);
                    self.emit(Instruction::simple(Opcode::ConstFalse));
                    self.emit(Instruction::simple(Opcode::CmpEq));
                }
                UnaryOp::BitNot if operand.ty.is_int() => {
                    self.expr(operand);
                    self.emit(Instruction::simple(Opcode::NotI32));
                }
                UnaryOp::BitNot => {
                    // `~` on a JS `number`: ToInt32, complement, widen back.
                    self.expr(operand);
                    self.emit_to_int32(false);
                    self.emit(Instruction::simple(Opcode::NotI32));
                    self.emit_conv_kind(NumKind::F64);
                }
                UnaryOp::Void => {
                    // Evaluate for side effects, discard, yield `undefined`.
                    self.expr(operand);
                    self.emit(Instruction::simple(Opcode::Pop));
                    self.emit(Instruction::simple(Opcode::ConstUndef));
                }
                UnaryOp::Typeof | UnaryOp::Delete => {
                    unreachable!("`typeof`/`delete` are lowered in sema")
                }
            },
            ExprKind::Call { func, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::Call, Operand::Token(token)));
            }
            ExprKind::CallRest { func, args } => {
                for arg in args {
                    self.expr(arg);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallRest,
                    Operand::TokenImm32(token, args.len() as i32),
                ));
            }
            ExprKind::CallRestArray { func, args } => {
                self.expr(args);
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallRestArray,
                    Operand::Token(token),
                ));
            }
            ExprKind::CallRestArrayFixed {
                func,
                fixed_args,
                args,
            } => {
                for arg in fixed_args {
                    self.expr(arg);
                }
                self.expr(args);
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallRestArrayFixed,
                    Operand::TokenImm32(token, fixed_args.len() as i32),
                ));
            }
            ExprKind::GeneratorCall { func, args, native_id } => {
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(*func as i32 + 1)));
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(args.len() as i32)));
                self.emit(Instruction::simple(Opcode::NewArray));
                for (index, arg) in args.iter().enumerate() {
                    self.emit(Instruction::simple(Opcode::Dup));
                    self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(index as i32)));
                    self.expr(arg);
                    self.emit(Instruction::simple(Opcode::StoreArray));
                    self.emit(Instruction::simple(Opcode::Pop));
                }
                self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(*native_id as i32)));
            }
            ExprKind::FunctionBind { callee, args, native_id } => {
                // Push the target function, then build the managed array of
                // prepended arguments, then invoke the `functionBind` builtin
                // (params `[Function, Array]`).
                self.expr(callee);
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(args.len() as i32)));
                self.emit(Instruction::simple(Opcode::NewArray));
                for (index, arg) in args.iter().enumerate() {
                    self.emit(Instruction::simple(Opcode::Dup));
                    self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(index as i32)));
                    self.expr(arg);
                    self.emit(Instruction::simple(Opcode::StoreArray));
                    self.emit(Instruction::simple(Opcode::Pop));
                }
                self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(*native_id as i32)));
            }
            ExprKind::IndirectCall { callee, args, func } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
                // A dynamic function value called with other than one argument
                // (e.g. a multi-argument bound call) carries its count.
                if *func == hir::DYNAMIC_FUNCTION && args.len() != 1 {
                    self.emit(Instruction::new(
                        Opcode::CallIndirectN,
                        Operand::Imm32(args.len() as i32),
                    ));
                } else {
                    let token = if *func == hir::DYNAMIC_FUNCTION || *func == hir::DYNAMIC_PREDICATE {
                        Token::null(TableKind::MethodDef)
                    } else if *func == hir::DYNAMIC_REDUCER {
                        Token::null(TableKind::MemberRef)
                    } else {
                        Token::new(TableKind::MethodDef, *func + 1)
                    };
                    self.emit(Instruction::new(
                        Opcode::CallIndirect,
                        Operand::Token(token),
                    ));
                }
            }
            ExprKind::IndirectCallRest { callee, args, func } => {
                self.expr(callee);
                for arg in args {
                    self.expr(arg);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallIndirectRest,
                    Operand::TokenImm32(token, args.len() as i32),
                ));
            }
            ExprKind::OptionalIndirectCall { callee, args, func } => {
                self.expr(callee);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstNull));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_null = self.emit_branch(Opcode::BranchTrue);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_undef = self.emit_branch(Opcode::BranchTrue);
                for arg in args { self.expr(arg); }
                if *func == hir::DYNAMIC_FUNCTION && args.len() != 1 {
                    self.emit(Instruction::new(Opcode::CallIndirectN, Operand::Imm32(args.len() as i32)));
                } else {
                    let token = if *func == hir::DYNAMIC_FUNCTION || *func == hir::DYNAMIC_PREDICATE {
                        Token::null(TableKind::MethodDef)
                    } else if *func == hir::DYNAMIC_REDUCER {
                        Token::null(TableKind::MemberRef)
                    } else { Token::new(TableKind::MethodDef, *func + 1) };
                    self.emit(Instruction::new(Opcode::CallIndirect, Operand::Token(token)));
                }
                let end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(use_null);
                self.patch_to_here(use_undef);
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.patch_to_here(end);
            }
            ExprKind::CallBuiltin { id, args } => {
                for a in args {
                    self.expr(a);
                }
                // A `zun:math` intrinsic that the machine can do in one
                // instruction is emitted as that instruction; everything else
                // keeps calling the native.
                match math_machine_op(*id, args.len()) {
                    Some(op) => self.emit(Instruction::simple(op)),
                    None => self.emit(Instruction::new(
                        Opcode::CallBuiltin,
                        Operand::Imm32(*id as i32),
                    )),
                }
            }
            ExprKind::RegexMatch { matcher, args } => {
                // `self.regex_plan` is a borrow independent of `self`; bind it
                // locally so the arm can also borrow `self` mutably to emit.
                let plan = self.regex_plan;
                match &plan[*matcher as usize] {
                    RegexPlan::Compiled { is_match_rid } => {
                        let rid = *is_match_rid;
                        self.expr(&args[0]); // subject string
                        self.emit(Instruction::new(
                            Opcode::Call,
                            Operand::Token(Token::new(TableKind::MethodDef, rid)),
                        ));
                    }
                    RegexPlan::Fallback { regexp_new_id, is_match_id, pattern, flags } => {
                        // regexpIsMatch(regexpNew(pattern, flags), subject) —
                        // identical to the ordinary (non-static) regex path.
                        let (new_id, match_id) = (*regexp_new_id, *is_match_id);
                        let p = self.module.strings.intern(pattern);
                        self.emit(Instruction::new(Opcode::ConstStr, Operand::StringRef(p.0)));
                        let f = self.module.strings.intern(flags);
                        self.emit(Instruction::new(Opcode::ConstStr, Operand::StringRef(f.0)));
                        self.emit(Instruction::new(
                            Opcode::CallBuiltin,
                            Operand::Imm32(new_id as i32),
                        ));
                        self.expr(&args[0]); // subject string
                        self.emit(Instruction::new(
                            Opcode::CallBuiltin,
                            Operand::Imm32(match_id as i32),
                        ));
                    }
                }
            }
            ExprKind::DynamicNew => {
                self.emit(Instruction::simple(Opcode::NewDynamic));
            }
            ExprKind::RealmGlobal => {
                self.emit(Instruction::simple(Opcode::RealmGlobal));
            }
            ExprKind::DynamicGet { object, name } => {
                self.expr(object);
                let property = self.module.strings.intern(name);
                self.emit(Instruction::new(
                    Opcode::LoadDynamic,
                    Operand::StringRef(property.0),
                ));
            }
            ExprKind::DynamicGetValue { object, key } => {
                self.expr(object);
                self.expr(key);
                self.emit(Instruction::simple(Opcode::LoadDynamicValue));
            }
            ExprKind::OptionalDynamicGet { object, name } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstNull));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_null = self.emit_branch(Opcode::BranchTrue);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_undef = self.emit_branch(Opcode::BranchTrue);
                let property = self.module.strings.intern(name);
                self.emit(Instruction::new(
                    Opcode::LoadDynamic,
                    Operand::StringRef(property.0),
                ));
                let end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(use_null);
                self.patch_to_here(use_undef);
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.patch_to_here(end);
            }
            ExprKind::OptionalDynamicGetValue { object, key } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstNull));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_null = self.emit_branch(Opcode::BranchTrue);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_undef = self.emit_branch(Opcode::BranchTrue);
                self.expr(key);
                self.emit(Instruction::simple(Opcode::LoadDynamicValue));
                let end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(use_null);
                self.patch_to_here(use_undef);
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.patch_to_here(end);
            }
            ExprKind::OptionalCallBuiltin { id, args } => {
                self.expr(&args[0]);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstNull));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_null = self.emit_branch(Opcode::BranchTrue);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_undef = self.emit_branch(Opcode::BranchTrue);
                for arg in &args[1..] {
                    self.expr(arg);
                }
                self.emit(Instruction::new(
                    Opcode::CallBuiltin,
                    Operand::Imm32(*id as i32),
                ));
                let end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(use_null);
                self.patch_to_here(use_undef);
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.patch_to_here(end);
            }
            ExprKind::DynamicSet {
                object,
                name,
                value,
            } => {
                self.expr(object);
                self.expr(value);
                let property = self.module.strings.intern(name);
                self.emit(Instruction::new(
                    Opcode::StoreDynamic,
                    Operand::StringRef(property.0),
                ));
            }
            ExprKind::DynamicSetValue { object, key, value } => {
                self.expr(object);
                self.expr(key);
                self.expr(value);
                self.emit(Instruction::simple(Opcode::StoreDynamicValue));
            }
            ExprKind::DynamicObjectLiteral { entries } => {
                // Allocate once, then set each property on a duplicated handle,
                // discarding the value the store leaves behind. The object stays
                // on the stack as the literal's result.
                self.emit(Instruction::simple(Opcode::NewDynamic));
                for entry in entries {
                    self.emit(Instruction::simple(Opcode::Dup));
                    match entry {
                        hir::ObjectEntry::Data { key, value } => match key {
                            hir::ObjectKey::Named(name) => {
                                self.expr(value);
                                let property = self.module.strings.intern(name);
                                self.emit(Instruction::new(
                                    Opcode::StoreDynamic,
                                    Operand::StringRef(property.0),
                                ));
                            }
                            hir::ObjectKey::Computed(key) => {
                                self.expr(key);
                                self.expr(value);
                                self.emit(Instruction::simple(Opcode::StoreDynamicValue));
                            }
                        },
                        hir::ObjectEntry::Install { builtin, args } => {
                            // The object is the dup'd handle (arg0); push the
                            // remaining args, call the native, and discard its
                            // result so the object stays as the literal's value.
                            for arg in args {
                                self.expr(arg);
                            }
                            self.emit(Instruction::new(
                                Opcode::CallBuiltin,
                                Operand::Imm32(*builtin as i32),
                            ));
                        }
                    }
                    self.emit(Instruction::simple(Opcode::Pop));
                }
            }
            ExprKind::BoxAny { value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::BoxAny));
            }
            ExprKind::ExternalCall { module, name, args } => {
                for a in args {
                    self.expr(a);
                }
                let module_name = self.module.strings.intern(module);
                let member_name = self.module.strings.intern(name);
                let module_ref = self
                    .module
                    .tables
                    .add_module_ref(ModuleRef { name: module_name });
                let arity = u8::try_from(args.len()).expect("external call arity exceeds metadata limit");
                // Keep arity first for the verifier, followed by scalar type
                // tags for the typed native-call boundary. Zero denotes a
                // managed/non-blittable type at the explicit stub boundary.
                let mut signature_bytes = Vec::with_capacity(args.len() + 2);
                signature_bytes.push(arity);
                signature_bytes.extend(args.iter().map(|arg| ffi_type_tag(arg.ty)));
                signature_bytes.push(ffi_type_tag(e.ty));
                let signature = self.module.blobs.intern(&signature_bytes);
                let member_ref = self.module.tables.add_member_ref(MemberRef {
                    parent: module_ref,
                    name: member_name,
                    signature,
                });
                self.emit(Instruction::new(Opcode::Call, Operand::Token(member_ref)));
            }
            ExprKind::MethodCall { func, args } => {
                // args[0] is the receiver; push it and the rest, then dispatch
                // virtually on the receiver's runtime class. The token names the
                // statically resolved method (for its name and arity).
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::CallVirt, Operand::Token(token)));
            }
            ExprKind::StaticCall { func, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::Call, Operand::Token(token)));
            }
            ExprKind::MethodCallRest { func, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallVirtRest,
                    Operand::TokenImm32(token, args.len() as i32),
                ));
            }
            ExprKind::MethodCallRestArray { func, receiver, args } => {
                self.expr(receiver);
                self.expr(args);
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::CallVirtRestArray, Operand::Token(token)));
            }
            ExprKind::MethodCallRestArrayFixed { func, receiver, fixed_args, args } => {
                self.expr(receiver);
                for fixed in fixed_args {
                    self.expr(fixed);
                }
                self.expr(args);
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallVirtRestArrayFixed,
                    Operand::TokenImm32(token, fixed_args.len() as i32),
                ));
            }
            ExprKind::New { class, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = self.layout.type_tokens[*class as usize];
                self.emit(Instruction::new(Opcode::NewObj, Operand::Token(token)));
            }
            ExprKind::NewWithConstructor { class, defaults, func, args } => {
                // Allocate with default field values, then run the constructor
                // on a duplicate of the instance handle. Every call leaves one
                // value (void included), so discard it; the instance remains.
                for d in defaults {
                    self.expr(d);
                }
                let token = self.layout.type_tokens[*class as usize];
                self.emit(Instruction::new(Opcode::NewObj, Operand::Token(token)));
                self.emit(Instruction::simple(Opcode::Dup));
                for a in args {
                    self.expr(a);
                }
                let ctor = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::CallVirt, Operand::Token(ctor)));
                self.emit(Instruction::simple(Opcode::Pop));
            }
            ExprKind::ArrayNew { length } => {
                self.expr(length);
                self.emit(Instruction::simple(Opcode::NewArray));
            }
            ExprKind::ArrayLiteral { elements } => {
                // Allocate an array of fixed length, then store each element on
                // a duplicated handle at its positional index, discarding the
                // value `StoreArray` leaves behind. The array stays on the stack
                // as the literal's result. `StoreArray` boxes each value to Any.
                let length = i32::try_from(elements.len())
                    .expect("array literal length exceeds i32 range");
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(length)));
                self.emit(Instruction::simple(Opcode::NewArray));
                for (index, element) in elements.iter().enumerate() {
                    // A hole leaves the pre-allocated slot alone except that we
                    // overwrite the default fill with `undefined`, so a hole
                    // reads as `undefined` (not `0`) while still counting toward
                    // `length`.
                    let index = i32::try_from(index)
                        .expect("array literal index exceeds i32 range");
                    self.emit(Instruction::simple(Opcode::Dup));
                    self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(index)));
                    match element {
                        Some(element) => self.expr(element),
                        None => self.emit(Instruction::simple(Opcode::ConstUndef)),
                    }
                    self.emit(Instruction::simple(Opcode::StoreArray));
                    self.emit(Instruction::simple(Opcode::Pop));
                }
            }
            ExprKind::ArraySpreadLiteral { parts } => {
                // Build a growable array (length 0) and append each part in
                // source order: a plain element and a hole each push one value
                // (`undefined` for a hole); a spread splices the iterable
                // source's elements in via `arraySpreadAppend`.
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(0)));
                self.emit(Instruction::simple(Opcode::NewArray));
                for part in parts {
                    self.emit(Instruction::simple(Opcode::Dup));
                    match part {
                        hir::ArrayPart::Element(element) => {
                            self.expr(element);
                            self.emit(Instruction::simple(Opcode::ArrayPush));
                        }
                        hir::ArrayPart::Hole => {
                            self.emit(Instruction::simple(Opcode::ConstUndef));
                            self.emit(Instruction::simple(Opcode::ArrayPush));
                        }
                        hir::ArrayPart::Spread { builtin, source } => {
                            self.expr(source);
                            self.emit(Instruction::new(
                                Opcode::CallBuiltin,
                                Operand::Imm32(*builtin as i32),
                            ));
                        }
                    }
                    // Discard the per-part result (push length / native return),
                    // leaving the array as the literal's value.
                    self.emit(Instruction::simple(Opcode::Pop));
                }
            }
            ExprKind::FieldGet {
                object,
                class,
                field_slot,
            } => {
                self.expr(object);
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::LoadField, Operand::Token(token)));
            }
            ExprKind::FieldSet {
                object,
                class,
                field_slot,
                value,
            } => {
                // [object, value] -> stfld -> [value]. stfld leaves the stored
                // value so the assignment yields it; in statement position the
                // always-pop rule discards it.
                self.expr(object);
                self.expr(value);
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::StoreField, Operand::Token(token)));
            }
            ExprKind::StaticFieldGet { class, field_slot } => {
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::LoadStatic, Operand::Token(token)));
            }
            ExprKind::StaticFieldSet { class, field_slot, value } => {
                self.expr(value);
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::StoreStatic, Operand::Token(token)));
            }
            ExprKind::ArrayGet { object, index } => {
                self.expr(object);
                self.expr(index);
                self.emit(Instruction::simple(Opcode::LoadArray));
            }
            ExprKind::ArraySet {
                object,
                index,
                value,
            } => {
                self.expr(object);
                self.expr(index);
                self.expr(value);
                self.emit(Instruction::simple(Opcode::StoreArray));
            }
            ExprKind::ArrayLength { object } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::ArrayLength));
            }
            ExprKind::ArrayPop { object } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::ArrayPop));
            }
            ExprKind::ArrayPush { object, value } => {
                self.expr(object);
                self.expr(value);
                self.emit(Instruction::simple(Opcode::ArrayPush));
            }
            ExprKind::Convert { value } => {
                if value.ty == Ty::Any {
                    self.expr(value);
                    if let Ty::Instance(index) = e.ty {
                        self.emit(Instruction::new(
                            Opcode::CastClass,
                            Operand::Token(Token::new(TableKind::TypeDef, index + 1)),
                        ));
                    } else {
                        self.emit(Instruction::simple(Opcode::UnboxAny));
                    }
                } else {
                    self.expr(value);
                }
                self.emit_conv_to(e.ty);
            }
            ExprKind::Error => unreachable!("ilgen runs only on error-free programs"),
        }
    }
}

fn capture_name(slot: u16) -> String {
    format!("$capture{slot}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use zun_il::disassemble;
    use zun_module::read_body;

    fn lower_src(src: &str) -> Module {
        let mut diags = Vec::new();
        let ast = zun_syntax::parse(src, &mut diags);
        assert!(diags.is_empty(), "parse: {diags:?}");
        let hir = zun_sema::check(src, &ast, &mut diags);
        assert!(diags.is_empty(), "sema: {diags:?}");
        lower(&hir)
    }

    #[test]
    fn lowers_a_non_capturing_arrow_to_a_synthetic_method() {
        let module = lower_src("function main() { let f = x => x * 2; }");
        // main, the synthetic module initializer (PLAN §3.1), then the closure
        // method (closures are appended after all functions).
        assert_eq!(module.tables.method_defs.len(), 3);
        let closure = read_body(&module.code, module.tables.method_defs[2].body_offset)
            .expect("synthetic closure body");
        assert_eq!(closure.param_count, 2);
        assert_eq!(closure.local_count, 2);
        let text = disassemble(closure.code).unwrap();
        assert!(text.contains("ldloc 1"), "{text}");
        assert!(text.contains("mul.f64"), "{text}");
    }

    #[test]
    fn lowers_expression_spans_into_sequence_points() {
        let module = lower_src("function main(): number { return 1 + 2; }");
        assert!(!module.sequence_points.is_empty());
        assert!(module.sequence_points.iter().all(|point| point.method_rid == 1));
        assert!(module.sequence_points.iter().all(|point| point.source_end > point.source_start));
    }

    #[test]
    fn encodes_string_external_call_types_in_member_metadata() {
        let src = "extern function echo(value: string): string from \"host\"; function main(): string { return echo(\"zun\"); }";
        let mut diagnostics = Vec::new();
        let ast = zun_syntax::parse(src, &mut diagnostics);
        assert!(diagnostics.is_empty(), "parse: {diagnostics:?}");
        let hir = zun_sema::check_with_externals(
            src,
            &ast,
            &mut diagnostics,
            &[zun_sema::ExternalFunction {
                module: "host".into(),
                name: "echo".into(),
                params: vec![Ty::String],
                return_ty: Ty::String,
            }],
        );
        assert!(diagnostics.is_empty(), "sema: {diagnostics:?}");
        let module = lower(&hir);
        let member = module
            .tables
            .member_refs
            .first()
            .expect("external member reference");
        assert_eq!(module.blobs.get(member.signature), &[1, 4, 4]);
    }

    #[test]
    fn encodes_two_string_external_call_types_in_member_metadata() {
        let src = "extern function join(left: string, right: string): string from \"host\"; function main(): string { return join(\"a\", \"b\"); }";
        let mut diagnostics = Vec::new();
        let ast = zun_syntax::parse(src, &mut diagnostics);
        assert!(diagnostics.is_empty(), "parse: {diagnostics:?}");
        let hir = zun_sema::check_with_externals(
            src,
            &ast,
            &mut diagnostics,
            &[zun_sema::ExternalFunction {
                module: "host".into(),
                name: "join".into(),
                params: vec![Ty::String, Ty::String],
                return_ty: Ty::String,
            }],
        );
        assert!(diagnostics.is_empty(), "sema: {diagnostics:?}");
        let module = lower(&hir);
        let member = module
            .tables
            .member_refs
            .first()
            .expect("external member reference");
        assert_eq!(module.blobs.get(member.signature), &[2, 4, 4, 4]);
    }

    #[test]
    fn encodes_i32_and_u32_external_call_types_in_member_metadata() {
        let src = "extern function signed(value: i32): i32 from \"host\"; extern function unsigned(value: u32): u32 from \"host\"; function main(): i32 { return signed(1); }";
        let mut diagnostics = Vec::new();
        let ast = zun_syntax::parse(src, &mut diagnostics);
        assert!(diagnostics.is_empty(), "parse: {diagnostics:?}");
        let hir = zun_sema::check_with_externals(
            src,
            &ast,
            &mut diagnostics,
            &[
                zun_sema::ExternalFunction { module: "host".into(), name: "signed".into(), params: vec![Ty::Int(IntKind { bits: 32, signed: true })], return_ty: Ty::Int(IntKind { bits: 32, signed: true }) },
                zun_sema::ExternalFunction { module: "host".into(), name: "unsigned".into(), params: vec![Ty::Int(IntKind { bits: 32, signed: false })], return_ty: Ty::Int(IntKind { bits: 32, signed: false }) },
            ],
        );
        assert!(diagnostics.is_empty(), "sema: {diagnostics:?}");
        let module = lower(&hir);
        let signatures: Vec<_> = module.tables.member_refs.iter().map(|member| module.blobs.get(member.signature).to_vec()).collect();
        assert!(signatures.contains(&vec![1, 5, 5]));
        assert!(signatures.contains(&vec![1, 6, 6]));
    }

    #[test]
    fn simple_function_disassembles_as_expected() {
        let module = lower_src("function main() { let x = 1; print(x); }");
        let body = read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
        let text = disassemble(body.code).unwrap();
        assert_eq!(
            text,
            "0000: const.f64 1\n\
             0009: stloc 0\n\
             000C: ldloc 0\n\
             000F: call.builtin 0\n\
             0014: pop\n\
             0015: ret\n"
        );
        assert_eq!(body.local_count, 1);
        assert!(module.entry_point.is_some());
    }

    #[test]
    fn while_loop_branches_point_at_the_right_places() {
        let module = lower_src("function main() { let n = 3; while (n > 0) { n = n - 1; } }");
        let body = read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
        let text = disassemble(body.code).unwrap();
        assert_eq!(
            text,
            "0000: const.f64 3\n\
             0009: stloc 0\n\
             000C: ldloc 0\n\
             000F: const.f64 0\n\
             0018: cmp.gt\n\
             0019: br.false +23\n\
             001E: ldloc 0\n\
             0021: const.f64 1\n\
             002A: sub.f64\n\
             002B: dup\n\
             002C: stloc 0\n\
             002F: pop\n\
             0030: br -41\n\
             0035: ret\n"
        );
    }

    #[test]
    fn if_else_falls_through_correctly() {
        let module =
            lower_src("function main() { if (true) { print(\"a\"); } else { print(\"b\"); } }");
        let body = read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
        let text = disassemble(body.code).unwrap();
        assert_eq!(
            text,
            "0000: const.true\n\
             0001: br.false +16\n\
             0006: const.str str:20\n\
             000B: call.builtin 0\n\
             0010: pop\n\
             0011: br +11\n\
             0016: const.str str:22\n\
             001B: call.builtin 0\n\
             0020: pop\n\
             0021: ret\n"
        );
    }

    #[test]
    fn string_concat_and_user_calls_lower() {
        let module = lower_src(
            "function greet(who: string): string { return \"hi \" + who; }\n\
             function main() { print(greet(\"zun\")); }",
        );
        // main is function index 1 -> MethodDef RID 2.
        let body = read_body(&module.code, module.tables.method_defs[1].body_offset).unwrap();
        let text = disassemble(body.code).unwrap();
        assert!(text.contains("call MethodDef[1]"), "{text}");

        let greet = read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
        assert!(disassemble(greet.code).unwrap().contains("concat"));
        assert_eq!(
            module.entry_point,
            Some(Token::new(TableKind::MethodDef, 2))
        );
    }

    #[test]
    fn try_catch_finally_lowers_handler_regions() {
        let module = lower_src(
            "class Error {} function main() { try { throw new Error(); } catch (error: Error) { print(1); } finally { print(2); } }",
        );
        let method = &module.tables.method_defs[0];
        let body = read_body(&module.code, method.body_offset).unwrap();
        assert_eq!(body.handlers.len(), 2);
        assert!(body.handlers.iter().any(|h| h.kind == HandlerKind::Catch));
        assert!(body.handlers.iter().any(|h| h.kind == HandlerKind::Finally));
    }
}
