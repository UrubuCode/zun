//! `T[]` array type annotations (parser + sema): `T[]` of any element type and
//! any depth resolves to the untyped `Array`.

use zun_interp::CaptureOutput;

fn out_std(user: &str) -> Vec<String> {
    let module = zun_compiler::compile_with_default_std(user).expect("compiles");
    let mut o = CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut o).expect("runs");
    o.lines
}

#[test]
fn array_type_annotation_in_param_local_and_return() {
    let lines = out_std(
        r#"
        function sum(xs: number[]): number {
            let t: number = 0.0;
            let i: u64 = 0;
            while (i < xs.length) { t = t + (xs[i] as number); i = i + 1; }
            return t;
        }
        function build(): string[] { return ["a", "b"]; }
        function main(): void {
            let a: number[] = [1.0, 2.0, 3.0];
            print(sum(a));
            print(build().length);
        }
        "#,
    );
    assert_eq!(lines, ["6", "2"]);
}
