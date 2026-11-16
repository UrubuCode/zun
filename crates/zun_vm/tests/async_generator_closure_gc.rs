//! Regression: calling an `async function*` closure from an async function
//! hands its coroutine to the prelude's `__zunMakeAsyncGen`, a managed call
//! that can collect. The call site published no activation, so the caller's
//! locals (an array built just before the call) were neither rooted nor
//! rewritten when the heap moved, and the next `push` hit a stale array.
//!
//! Collection only triggers at a poll (`newobj`, loop back-edge). The padding
//! array allocates past the budget with no poll in between, so the first poll
//! after it is the one inside the prelude call, which makes the test
//! deterministic without `ZUN_GC_STRESS`.

use zun_interp::CaptureOutput;

#[test]
fn async_generator_closure_call_keeps_the_callers_locals_alive() {
    let padding = vec!["{}"; 300].join(", ");
    let src = format!(
        r#"
        async function run(rounds: number): Promise<void> {{
          await Promise.resolve(1);
          let total = 0;
          for (let i = 0; i < rounds; i++) {{
            async function* agen(): AsyncGenerator<string> {{ yield "g1"; yield "g2"; }}
            const got: string[] = [];
            const pad: any[] = [{padding}];
            for await (const v of agen()) got.push(v);
            total += got.length + pad.length - 300;
          }}
          console.log("total=" + total);
        }}
        run(4).then(() => console.log("end"));
    "#
    );
    let module = zun_compiler::compile_with_default_std(&src).expect("compiles");
    let mut output = CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut output).expect("runs");
    assert_eq!(output.lines, ["total=8", "end"]);
}
