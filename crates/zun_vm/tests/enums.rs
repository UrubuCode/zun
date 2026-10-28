//! `enum` declarations. Members fold to compile-time number/string constants at
//! `Enum.Member` accesses; the enum name is usable as a type (number/string).

use zun_interp::CaptureOutput;

fn run_source(user: &str) -> Vec<String> {
    let src = zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(&src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(&src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let bytes = zun_ilgen::lower(&hir).write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn numeric_enum_auto_increments() {
    let out = run_source(
        r#"enum Color { Red, Green, Blue }
        function main() {
            print(Color.Red);
            print(Color.Green);
            print(Color.Blue);
        }"#,
    );
    assert_eq!(out, ["0", "1", "2"]);
}

#[test]
fn explicit_value_then_resumes_increment() {
    let out = run_source(
        r#"enum E { A = 5, B, C, D = 10, F }
        function main() {
            print(E.A);
            print(E.B);
            print(E.C);
            print(E.D);
            print(E.F);
        }"#,
    );
    assert_eq!(out, ["5", "6", "7", "10", "11"]);
}

#[test]
fn string_enum() {
    let out = run_source(
        r#"enum Dir { Up = "UP", Down = "DOWN" }
        function main() {
            print(Dir.Up);
            print(Dir.Down);
        }"#,
    );
    assert_eq!(out, ["UP", "DOWN"]);
}

#[test]
fn enum_name_is_a_type() {
    // A numeric enum resolves to `number` as a type, so a parameter typed by the
    // enum takes part in arithmetic without boxing.
    let out = run_source(
        r#"enum Level { Low, Mid, High }
        function bump(l: Level): number { return l + 1.0; }
        function main() {
            print(bump(Level.Low));
            print(bump(Level.High));
        }"#,
    );
    assert_eq!(out, ["1", "3"]);
}

#[test]
fn const_enum_and_negative_values() {
    let out = run_source(
        r#"const enum Sign { Neg = -1, Zero = 0, Pos = 1 }
        function main() {
            print(Sign.Neg);
            print(Sign.Zero);
            print(Sign.Pos);
        }"#,
    );
    assert_eq!(out, ["-1", "0", "1"]);
}

#[test]
fn enum_used_in_a_hot_loop() {
    let out = run_source(
        r#"enum Op { Add, Sub }
        function apply(op: Op, a: number, b: number): number {
            if (op == Op.Add) { return a + b; }
            return a - b;
        }
        function main() {
            let sum = 0.0;
            for (let i = 0; i < 2000; i = i + 1) {
                sum = apply(Op.Add, sum, 1.0);
            }
            print(sum);
        }"#,
    );
    assert_eq!(out, ["2000"]);
}
