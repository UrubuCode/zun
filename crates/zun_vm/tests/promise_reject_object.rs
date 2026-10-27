//! Regression: `Promise.reject(<object>)` must carry the reason first-class, so
//! `await` / `catch (e: Error)` sees the object, not a stringified "[object]".

use zun_interp::CaptureOutput;

#[test]
fn promise_reject_with_error_object_is_caught_as_error() {
    let src = r#"
        async function m2(): Promise {
            try {
                await Promise.reject(new Error("rj"));
                print("no-throw");
            } catch (e: Error) {
                print("caught:" + e.message);
            }
            return Promise.resolve(0.0);
        }
        function main(): void { m2(); }
    "#;
    let module = zun_compiler::compile_with_default_std(src).expect("compiles");
    let mut o = CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut o).expect("runs");
    assert_eq!(o.lines, ["caught:rj"]);
}
