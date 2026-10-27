//! `x?: T` optional parameters. Zun has no union types, so an optional parameter
//! with no explicit default is `any` (it accepts the value or `null`) and
//! defaults to `null` when the argument is omitted.

use zun_interp::CaptureOutput;

fn out_std(user: &str) -> Vec<String> {
    let module = zun_compiler::compile_with_default_std(user).expect("compiles");
    let mut o = CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut o).expect("runs");
    o.lines
}

#[test]
fn optional_parameter_in_function_and_method() {
    assert_eq!(
        out_std(
            r#"
            function greet(name: string, suffix?: string): string {
                if (suffix == null) { return name; }
                return name + (suffix as string);
            }
            class Box {
                v: number = 0.0;
                set(x?: number): void {
                    if (x == null) { this.v = 9.0; } else { this.v = x as number; }
                }
            }
            function main(): void {
                print(greet("hi"));
                print(greet("hi", "!"));
                let b = new Box(0.0);
                b.set();
                print(b.v);
                b.set(3.0);
                print(b.v);
            }
            "#,
        ),
        ["hi", "hi!", "9", "3"],
    );
}
