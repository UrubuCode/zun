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

// --- Slice 3: reactions on PENDING sources (the fix). Each reaction's side
// effect prints during the turn-drain, after the synchronous body's prints, so
// the output order is the observable proof that the reaction ran as a
// microtask rather than inline or not at all. ---

#[test]
fn then_on_a_pending_promise_runs_when_it_later_resolves() {
    // The fix: a `then` on a PENDING promise schedules a reaction that runs
    // when the source settles. Under the old eager-at-settle path this printed
    // only "sync" because the reaction was never driven.
    let out = run_source(
        "function main() { \
            let p = new Promise(); \
            p.then((x: any) => { print(x as number); }); \
            print(\"sync\"); \
            p.resolve(5.0); }",
    );
    assert_eq!(out, ["sync", "5"]);
}

#[test]
fn catch_on_a_pending_promise_runs_on_later_rejection() {
    let out = run_source(
        "function main() { \
            let p = new Promise(); \
            p.catch((e: any) => { print(e as string); }); \
            p.reject(\"boom\"); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "boom"]);
}

#[test]
fn finally_on_a_pending_promise_runs_on_settle() {
    let out = run_source(
        "function main() { \
            let p = new Promise(); \
            p.finally(() => { print(\"cleanup\"); }); \
            p.resolve(1.0); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "cleanup"]);
}

#[test]
fn an_already_settled_then_still_does_not_run_inline() {
    // The async-not-sync guarantee: even on an already-resolved source the
    // callback runs on the drain, after the synchronous "sync".
    let out = run_source(
        "function main() { \
            Promise.resolve(7.0).then((x: any) => { print(x as number); }); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "7"]);
}

#[test]
fn reactions_run_in_attach_order_and_chain_within_one_drain() {
    // Multiple `then`s on one promise run in attach order; a reaction scheduled
    // by a reaction (the chained `d`) runs in the same drain, after c.
    let out = run_source(
        "function main() { \
            let p = new Promise(); \
            p.then((x: any) => { print(\"a\"); }); \
            p.then((x: any) => { print(\"b\"); }); \
            p.then((x: any) => { print(\"c\"); }).then((y: any) => { print(\"d\"); }); \
            p.resolve(1.0); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "a", "b", "c", "d"]);
}

#[test]
fn promise_all_over_pending_inputs_resolves_after_both_settle() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); \
            let p1 = new Promise(); \
            let p2 = new Promise(); \
            a[0] = p1; \
            a[1] = p2; \
            Promise.all(a).then((v: any) => { print(\"all done\"); }); \
            p1.resolve(1.0); \
            print(\"mid\"); \
            p2.resolve(2.0); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["mid", "sync", "all done"]);
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
