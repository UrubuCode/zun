//! Compound assignment operators (`+=`, `&=`, `<<=`, ...), desugared in the
//! parser to `a = (a OP b)`.

use zun_interp::CaptureOutput;

fn run_source(src: &str) -> Vec<String> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut out).expect("program should run");
    out.lines
}

fn main_body(body: &str) -> Vec<String> {
    run_source(&format!("function main() {{ {body} }}"))
}

#[test]
fn arithmetic_on_ident() {
    let cases = [
        ("let a = 5.0; a += 3.0; print(a);", "8"),
        ("let a = 5.0; a -= 3.0; print(a);", "2"),
        ("let a = 5.0; a *= 3.0; print(a);", "15"),
        ("let a = 12.0; a /= 4.0; print(a);", "3"),
        ("let a = 17.0; a %= 5.0; print(a);", "2"),
        ("let a = 2.0; a **= 10.0; print(a);", "1024"),
    ];
    for (body, want) in cases {
        assert_eq!(main_body(body), [want], "{body}");
    }
}

#[test]
fn bitwise_and_shift_on_ident() {
    let cases = [
        ("let a: i32 = 12; a &= 10; print(a);", "8"),
        ("let a: i32 = 12; a |= 3; print(a);", "15"),
        ("let a: i32 = 12; a ^= 5; print(a);", "9"),
        ("let a: i32 = 3; a <<= 4; print(a);", "48"),
        ("let a: i32 = -16; a >>= 2; print(a);", "-4"),
        ("let a: i32 = 256; a >>>= 4; print(a);", "16"),
    ];
    for (body, want) in cases {
        assert_eq!(main_body(body), [want], "{body}");
    }
}

#[test]
fn right_associative_chain() {
    assert_eq!(main_body("let a = 1.0; let b = 2.0; a += b += 3.0; print(a); print(b);"), ["6", "5"]);
}

#[test]
fn field_target() {
    let out = run_source(
        "class Counter { value: number; }\n\
         function main() { let c = new Counter(10.0); c.value += 5.0; c.value *= 2.0; print(c.value); }",
    );
    assert_eq!(out, ["30"]);
}

#[test]
fn index_target() {
    let out = main_body("let arr = Buffer.alloc(4); let i: i32 = 1; arr[i] = 2; arr[i] += 40; arr[i] |= 1; print(arr[i]);");
    assert_eq!(out, ["43"]);
}

#[test]
fn bigint_pow() {
    assert_eq!(main_body("let a = 2n; a **= 10n; print(a.toString());"), ["1024"]);
}
