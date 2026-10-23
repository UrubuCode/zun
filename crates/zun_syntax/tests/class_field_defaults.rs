//! Class fields with default initializers, and the class body not hanging.
//!
//! `ClassField::initializer` has existed in the AST, and the checker has
//! filled missing trailing constructor arguments from it, but the
//! hand-written parser never read one — it always wrote `None`. Worse, on
//! meeting the `=` it did not expect, it neither consumed the token nor gave
//! up, so the member loop spun forever and the process died on an 8 GiB
//! allocation. `class P { y: i64 = 5; }` was enough.

use zun_common::Diagnostic;
use zun_syntax::ast;

fn parse(src: &str) -> (ast::Program, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let program = zun_syntax::parse(src, &mut diagnostics);
    (program, diagnostics)
}

fn class<'a>(program: &'a ast::Program, name_index: usize) -> &'a ast::Class {
    &program.classes[name_index]
}

#[test]
fn a_field_initializer_is_recorded() {
    let (program, diagnostics) = parse("class P { x: i64; y: i64 = 5; }");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let fields = &class(&program, 0).fields;
    assert_eq!(fields.len(), 2);
    assert!(
        fields[0].initializer.is_none(),
        "a field without `=` has no default"
    );
    assert!(
        fields[1].initializer.is_some(),
        "`y: i64 = 5` should carry its initializer"
    );
}

#[test]
fn fields_without_initializers_still_parse() {
    let (program, diagnostics) = parse("class P { x: i64; y: string; }");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(class(&program, 0)
        .fields
        .iter()
        .all(|field| field.initializer.is_none()));
}

#[test]
fn an_initializer_can_be_an_expression() {
    let (program, diagnostics) = parse("class P { x: i64 = 1 + 2 * 3; }");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(class(&program, 0).fields[0].initializer.is_some());
}

#[test]
fn initializers_mix_with_methods_and_modifiers() {
    let (program, diagnostics) = parse(
        "class P { #secret: i64 = 1; static count: i64 = 0; open: string = \"x\";
         value(): i64 { return this.open.length(); } }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let declared = class(&program, 0);
    assert_eq!(declared.fields.len(), 3);
    assert_eq!(declared.methods.len(), 1);
    assert!(declared
        .fields
        .iter()
        .all(|field| field.initializer.is_some()));
}

#[test]
fn a_class_body_the_grammar_cannot_parse_terminates() {
    // The regression that matters: this used to spin until the allocator
    // gave up. Reaching an assertion at all is the result; the diagnostics
    // just confirm it reported rather than silently accepting.
    let (_, diagnostics) = parse("class Bad { ??? x: i64; }");
    assert!(
        !diagnostics.is_empty(),
        "a class body with no valid member must be reported"
    );
}

#[test]
fn an_unterminated_class_body_terminates() {
    let (_, diagnostics) = parse("class Bad { x: i64 ");
    assert!(!diagnostics.is_empty());
}

#[test]
fn a_stray_token_does_not_swallow_the_members_after_it() {
    // Recovery skips one token and resumes, so the well-formed members on
    // either side of the junk still arrive.
    let (program, diagnostics) = parse("class P { a: i64; @ b: i64; }");
    assert!(!diagnostics.is_empty(), "the stray token is reported");
    assert_eq!(
        class(&program, 0).fields.len(),
        2,
        "both well-formed fields should survive recovery"
    );
}
