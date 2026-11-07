use super::function::{lower_closure_function, lower_function};
use super::*;

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
        // Signatures land with the verifier; until then the signature blob
        // only records that the last parameter is a rest parameter, so a
        // dynamic (`any`-receiver) call can pack the trailing arguments.
        let signature = if f.rest_param.is_some() {
            module.blobs.intern(&[b'R'])
        } else {
            BlobIndex(0)
        };
        module.tables.add_method_def(MethodDef {
            name,
            signature,
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
        let (code, handlers, points) = lower_function(
            f,
            &layout,
            &mut module,
            program.functions.len() as u32,
            &regex_plan,
        );
        module
            .sequence_points
            .extend(
                points
                    .into_iter()
                    .map(|(pc, span)| zun_module::SequencePoint {
                        method_rid: index as u32 + 1,
                        pc,
                        source_start: span.start,
                        source_end: span.end,
                    }),
            );
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
            index as u32,
            &regex_plan,
        );
        module
            .sequence_points
            .extend(
                points
                    .into_iter()
                    .map(|(pc, span)| zun_module::SequencePoint {
                        method_rid: (closure_base + index) as u32 + 1,
                        pc,
                        source_start: span.start,
                        source_end: span.end,
                    }),
            );
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
        let compiled =
            nat.and_then(|nat| zun_regex::zil::compile_is_match(&cr.pattern, &cr.flags, nat));
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
        let name = dst
            .strings
            .intern(&format!("{}${}", src.strings.get(md.name), base));
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
