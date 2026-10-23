//! End-to-end tests for the Promise method + static surface (NUCLEUS #9):
//! `then`/`catch`/`finally` chaining and the `Promise.*` statics, driven
//! through the scheduler via `await`.

use zun_interp::CaptureOutput;

/// Compile `src` to a module, serialize it, run it, and return output lines.
fn run_source(src: &str) -> Vec<String> {
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

#[test]
fn then_chains_and_returns_a_new_promise() {
    let out = run_source(
        "function main() { \
            let p = Promise.resolve(1.0); \
            let q = p.then((x: any) => (x as number) + 1.0); \
            let v: any = await q; \
            print(v as number); }",
    );
    assert_eq!(out, ["2"]);
}

#[test]
fn then_adopts_a_returned_promise() {
    // A callback returning a Promise is chained (adopted), not wrapped.
    let out = run_source(
        "function main() { \
            let p = Promise.resolve(1.0); \
            let q = p.then((x: any) => Promise.resolve((x as number) + 40.0)); \
            let v: any = await q; \
            print(v as number); }",
    );
    assert_eq!(out, ["41"]);
}

#[test]
fn rejection_skips_on_fulfilled_and_propagates() {
    // `then` with no onRejected forwards the rejection; `catch` then handles it.
    let out = run_source(
        "function main() { \
            let p = Promise.reject(\"boom\"); \
            let q = p.then((x: any) => (x as number) + 1.0); \
            let v: any = await q.catch((e: any) => e); \
            print(v as string); }",
    );
    assert_eq!(out, ["boom"]);
}

#[test]
fn catch_handles_a_rejected_promise() {
    let out = run_source(
        "function main() { \
            let v: any = await Promise.reject(\"bad\").catch((e: any) => \"caught\"); \
            print(v as string); }",
    );
    assert_eq!(out, ["caught"]);
}

#[test]
fn finally_runs_and_passes_the_value_through() {
    let out = run_source(
        "function main() { \
            let log = new Array(0); \
            let p = Promise.resolve(5.0); \
            let q = p.finally(() => log.push(\"cleanup\")); \
            let v: any = await q; \
            print(v as number); \
            print(log.length); }",
    );
    assert_eq!(out, ["5", "1"]);
}

#[test]
fn finally_passes_rejection_through() {
    let out = run_source(
        "function main() { \
            let p = Promise.reject(\"nope\"); \
            let q = p.finally(() => 0.0); \
            let v: any = await q.catch((e: any) => e); \
            print(v as string); }",
    );
    assert_eq!(out, ["nope"]);
}

#[test]
fn promise_all_resolves_to_an_array_of_results() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); \
            a[0] = Promise.resolve(1.0); \
            a[1] = Promise.resolve(2.0); \
            let v: any = await Promise.all(a); \
            let arr = v as Array; \
            print(arr[0] as number); \
            print(arr[1] as number); }",
    );
    assert_eq!(out, ["1", "2"]);
}

#[test]
fn promise_all_rejects_on_first_rejection() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); \
            a[0] = Promise.resolve(1.0); \
            a[1] = Promise.reject(\"second\"); \
            let v: any = await Promise.all(a).catch((e: any) => e); \
            print(v as string); }",
    );
    assert_eq!(out, ["second"]);
}

#[test]
fn promise_race_settles_as_the_first_to_settle() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); \
            a[0] = Promise.resolve(10.0); \
            a[1] = Promise.resolve(20.0); \
            let v: any = await Promise.race(a); \
            print(v as number); }",
    );
    assert_eq!(out, ["10"]);
}

#[test]
fn promise_all_settled_describes_each_outcome() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); \
            a[0] = Promise.resolve(7.0); \
            a[1] = Promise.reject(\"err\"); \
            let v: any = await Promise.allSettled(a); \
            let arr = v as Array; \
            let first: any = arr[0]; \
            let second: any = arr[1]; \
            print(first.status as string); \
            print(first.value as number); \
            print(second.status as string); \
            print(second.reason as string); }",
    );
    assert_eq!(out, ["fulfilled", "7", "rejected", "err"]);
}

#[test]
fn promise_any_takes_the_first_fulfilment() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); \
            a[0] = Promise.reject(\"x\"); \
            a[1] = Promise.resolve(99.0); \
            let v: any = await Promise.any(a); \
            print(v as number); }",
    );
    assert_eq!(out, ["99"]);
}

#[test]
fn promise_resolve_returns_an_existing_promise_unchanged() {
    let out = run_source(
        "function main() { \
            let p = Promise.resolve(3.0); \
            let q = Promise.resolve(p); \
            let v: any = await q; \
            print(v as number); }",
    );
    assert_eq!(out, ["3"]);
}
