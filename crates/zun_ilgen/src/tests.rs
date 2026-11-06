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
    // One catch region, plus a finally region guarding the try body and another
    // guarding the catch handler (a throw inside `catch` still runs `finally`).
    assert_eq!(body.handlers.len(), 3, "{:?}", body.handlers);
    assert_eq!(body.handlers.iter().filter(|h| h.kind == HandlerKind::Finally).count(), 2);
    assert!(body.handlers.iter().any(|h| h.kind == HandlerKind::Catch));
    assert!(body.handlers.iter().any(|h| h.kind == HandlerKind::Finally));
}
