//! End-to-end tests for binding destructuring (NUCLEUS #7 for-of destructuring,
//! #9 resolver destructuring, plus general array/object patterns).
//!
//! The hand parser accepts `let` declarations; the OXC front-end accepts the
//! standard `const [..]`/`const {..}` forms. Both feed the same pattern AST,
//! sema lowering and ZIL, so each case is exercised through one of them.

use zun_interp::CaptureOutput;

/// Compile `src` through the hand parser, round-trip the `.zm`, run it.
fn run(src: &str) -> Vec<String> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

/// Compile `src` through the OXC front-end (standard TS), run it.
fn run_oxc(src: &str) -> Vec<String> {
    let module = zun_compiler::compile_with_oxc(src, true).expect("compiles");
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn array_pattern_binds_elements() {
    let out = run(
        "function main() { let [a, b] = [1, 2]; print((a as number) + (b as number)); }",
    );
    assert_eq!(out, ["3"]);
}

#[test]
fn array_pattern_rest_collects_remainder() {
    let out = run(
        "function main() { \
            let [x, ...rest] = [1, 2, 3]; \
            print(x as number); \
            print(rest.length as number); \
            print(rest[0] as number); \
            print(rest[1] as number); }",
    );
    assert_eq!(out, ["1", "2", "2", "3"]);
}

#[test]
fn array_pattern_hole_skips_element() {
    let out = run(
        "function main() { let [, b] = [10, 20]; print(b as number); }",
    );
    assert_eq!(out, ["20"]);
}

#[test]
fn array_pattern_default_fills_undefined() {
    let out = run(
        "function main() { let [a, b = 9] = [5]; print(a as number); print(b as number); }",
    );
    assert_eq!(out, ["5", "9"]);
}

#[test]
fn nested_array_pattern() {
    let out = run(
        "function main() { let [[a], [b]] = [[1], [2]]; print((a as number) + (b as number)); }",
    );
    assert_eq!(out, ["3"]);
}

#[test]
fn object_pattern_shorthand_and_rename() {
    let out = run_oxc(
        "function main() { \
            const obj = { p: 1, q: 2 }; \
            const { p, q: alias } = obj; \
            const a = p as number; \
            const b = alias as number; \
            print(a); \
            print(b); }",
    );
    assert_eq!(out, ["1", "2"]);
}

#[test]
fn object_pattern_via_oxc_const() {
    let out = run_oxc(
        "function main() { const o = { p: 1, q: 2 }; const { p, q } = o; const s = (p as number) + (q as number); print(s); }",
    );
    assert_eq!(out, ["3"]);
}

#[test]
fn object_pattern_rest_collects_remaining_keys() {
    let out = run_oxc(
        "function main() { \
            const o = { a: 1, b: 2, c: 3 }; \
            const { a, ...rest } = o; \
            const x = a as number; \
            const y = rest.b as number; \
            const z = rest.c as number; \
            print(x); print(y); print(z); }",
    );
    assert_eq!(out, ["1", "2", "3"]);
}

#[test]
fn for_of_entries_destructuring_sums_index_and_value() {
    // NUCLEUS #7 payoff: `for (const [i, v] of arr.entries())`.
    let out = run(
        "function main() { \
            let arr = [10, 20, 30]; \
            let total = 0; \
            for (let [i, v] of arr.entries()) { \
                total = total + (i as number) + (v as number); \
            } \
            print(total); }",
    );
    // indices 0+1+2 = 3, values 10+20+30 = 60 -> 63.
    assert_eq!(out, ["63"]);
}

#[test]
fn with_resolvers_destructuring_resolves() {
    // NUCLEUS #9 payoff: bind the function-slot resolver and call it.
    let out = run(
        "function main() { \
            let { promise, resolve } = Promise.withResolvers(); \
            resolve(7.0); \
            let v: any = await promise; \
            print(v as number); }",
    );
    assert_eq!(out, ["7"]);
}

#[test]
fn with_resolvers_destructuring_rejects() {
    let out = run(
        "function main() { \
            let { promise, reject } = Promise.withResolvers(); \
            reject(\"bad\"); \
            try { let v: any = await promise; } \
            catch (e: string) { print(e); } }",
    );
    assert_eq!(out, ["bad"]);
}
