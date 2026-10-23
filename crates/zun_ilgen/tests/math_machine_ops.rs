//! Math intrinsics lower to machine ops instead of native calls.
//!
//! Checks the substitution happens, that it is still anchored to real natives,
//! and that operations without an exact IEEE-754 equivalent were left alone.

use zun_il::{decode_all, Opcode, StackEffect};
use zun_ilgen::MATH_MACHINE_OPS;

/// Compile a source file and return `main`'s opcode sequence.
fn opcodes_of(src: &str, function: &str) -> Vec<Opcode> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:?}");
    let module = zun_ilgen::lower(&hir);

    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == function)
        .expect("function is present");
    let body = zun_module::read_body(&module.code, module.tables.method_defs[rid].body_offset)
        .expect("body decodes");
    decode_all(body.code)
        .expect("code decodes")
        .iter()
        .map(|i| i.opcode)
        .collect()
}

#[test]
fn every_mapped_native_still_exists_in_zun_math() {
    // The mapping is by native name, so a rename would silently stop the
    // substitution and quietly cost performance with nothing failing to
    // build. This is the test that turns that into a failure.
    for &(name, op) in MATH_MACHINE_OPS {
        let (_, native) = zun_natives::resolve("zun:math", name)
            .unwrap_or_else(|| panic!("`zun:math` no longer exports `{name}`"));
        let StackEffect::Fixed { pops, .. } = op.stack_effect() else {
            panic!("{} must have a fixed stack effect", op.mnemonic());
        };
        assert_eq!(
            native.params.len(),
            pops as usize,
            "`{name}` takes {} arguments but {} pops {pops}",
            native.params.len(),
            op.mnemonic()
        );
    }
}

#[test]
fn sqrt_lowers_to_the_machine_op() {
    let ops = opcodes_of(
        r#"import { math } from "zun:core";
        function root(value: number): number { return math.sqrtNumber(value); }
        function main(): number { return root(16.0); }"#,
        "root",
    );
    assert!(ops.contains(&Opcode::SqrtF64), "expected sqrt.f64 in {ops:?}");
    assert!(
        !ops.contains(&Opcode::CallBuiltin),
        "sqrt should not call a native any more: {ops:?}"
    );
}

#[test]
fn min_and_max_lower_to_machine_ops() {
    let ops = opcodes_of(
        r#"import { math } from "zun:core";
        function pick(a: number, b: number): number { return math.minNumber(a, math.maxNumber(a, b)); }
        function main(): number { return pick(1.0, 2.0); }"#,
        "pick",
    );
    assert!(ops.contains(&Opcode::MinF64), "expected min.f64 in {ops:?}");
    assert!(ops.contains(&Opcode::MaxF64), "expected max.f64 in {ops:?}");
    assert!(!ops.contains(&Opcode::CallBuiltin), "{ops:?}");
}

#[test]
fn rounding_ops_lower_to_machine_ops() {
    let ops = opcodes_of(
        r#"import { math } from "zun:core";
        function shape(value: number): number {
            return math.floorNumber(math.ceilNumber(math.truncNumber(value)));
        }
        function main(): number { return shape(1.5); }"#,
        "shape",
    );
    for expected in [Opcode::FloorF64, Opcode::CeilF64, Opcode::TruncF64] {
        assert!(
            ops.contains(&expected),
            "expected {} in {ops:?}",
            expected.mnemonic()
        );
    }
    assert!(!ops.contains(&Opcode::CallBuiltin), "{ops:?}");
}

#[test]
fn round_and_sign_still_call_the_native() {
    // Neither matches an IEEE-754 primitive, so both must keep going through
    // libm rather than being approximated by a machine op.
    for name in ["roundNumber", "signNumber"] {
        let src = format!(
            r#"import {{ math }} from "zun:core";
            function apply(value: number): number {{ return math.{name}(value); }}
            function main(): number {{ return apply(1.5); }}"#
        );
        let ops = opcodes_of(&src, "apply");
        assert!(
            ops.contains(&Opcode::CallBuiltin),
            "{name} must stay on the native path: {ops:?}"
        );
    }
}

#[test]
fn transcendentals_still_call_the_native() {
    let ops = opcodes_of(
        r#"import { math } from "zun:core";
        function curve(value: number): number { return math.sinNumber(value); }
        function main(): number { return curve(1.0); }"#,
        "curve",
    );
    assert!(
        ops.contains(&Opcode::CallBuiltin),
        "sin belongs on libm: {ops:?}"
    );
}

#[test]
fn float_negation_uses_neg_rather_than_subtracting_from_zero() {
    // `0 - x` gives `+0.0` for `x == 0.0`, losing the sign of negative zero.
    let ops = opcodes_of(
        "function flip(value: number): number { return -value; }
         function main(): number { return flip(1.0); }",
        "flip",
    );
    assert!(ops.contains(&Opcode::NegF64), "expected neg.f64 in {ops:?}");
    assert!(
        !ops.contains(&Opcode::SubF64),
        "negation should not subtract: {ops:?}"
    );
}
