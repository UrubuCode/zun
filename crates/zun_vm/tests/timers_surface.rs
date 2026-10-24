//! End-to-end tests for the `timers` surface on the VM's unified micro+macro
//! event loop (ASYNC slice 4, PLAN §5.1): `setTimeout`/`setInterval`,
//! `clearTimeout`/`clearInterval`, `queueMicrotask`, `setImmediate`, and the
//! micro-before-macro ordering guarantee. Virtual-time fast-forward makes each
//! deterministic without real sleeping.

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
fn set_timeout_zero_runs_after_the_synchronous_body() {
    let out = run_source(
        "function main() { \
            setTimeout(() => { print(\"timer\"); }, 0.0); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "timer"]);
}

#[test]
fn set_timeout_with_a_delay_still_runs_its_side_effect() {
    // The callback's side effect is observed after `run()` has drained the loop,
    // with the delay fast-forwarded (no real wait).
    let out = run_source(
        "function main() { \
            setTimeout(() => { print(\"fired\"); }, 50.0); \
            print(\"before\"); }",
    );
    assert_eq!(out, ["before", "fired"]);
}

#[test]
fn queue_microtask_runs_before_a_zero_timeout() {
    // Micro-before-macro: the microtask drains fully before the first macrotask.
    let out = run_source(
        "function main() { \
            setTimeout(() => { print(\"macro\"); }, 0.0); \
            queueMicrotask(() => { print(\"micro\"); }); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "micro", "macro"]);
}

#[test]
fn a_promise_reaction_runs_before_a_zero_timeout() {
    // The ordering guarantee proved with a real Promise reaction: the `.then`
    // microtask runs before the `setTimeout(_, 0)` macrotask.
    let out = run_source(
        "function main() { \
            setTimeout(() => { print(\"macro\"); }, 0.0); \
            Promise.resolve(1.0).then((x: any) => { print(\"micro\"); }); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "micro", "macro"]);
}

#[test]
fn set_immediate_runs_as_a_macrotask() {
    let out = run_source(
        "function main() { \
            setImmediate(() => { print(\"immediate\"); }); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "immediate"]);
}

#[test]
fn clear_timeout_before_firing_cancels_the_callback() {
    let out = run_source(
        "function main() { \
            let h = setTimeout(() => { print(\"should not run\"); }, 10.0); \
            clearTimeout(h); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync"]);
}

#[test]
fn nested_timeout_drains_fully() {
    // A timer whose callback schedules another timer drains through the loop.
    let out = run_source(
        "function main() { \
            setTimeout(() => { \
                print(\"a\"); \
                setTimeout(() => { print(\"b\"); }, 0.0); \
            }, 0.0); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "a", "b"]);
}

#[test]
fn set_interval_fires_repeatedly_until_cleared() {
    // The interval fires at t=1,2,3,4; a timeout at t=5 clears it. Equal-due
    // ties break by arm order, so the clearing timeout wins at t=5.
    let out = run_source(
        "function main() { \
            let count = new Array(0); \
            let h = setInterval(() => { count.push(1.0); }, 1.0); \
            setTimeout(() => { clearInterval(h); print(count.length); }, 5.0); }",
    );
    assert_eq!(out, ["4"]);
}

#[test]
fn clear_interval_from_within_a_later_timer_stops_further_fires() {
    // After clearing, no further interval callback runs even though the loop
    // continues to drain other work.
    let out = run_source(
        "function main() { \
            let log = new Array(0); \
            let h = setInterval(() => { log.push(1.0); }, 2.0); \
            setTimeout(() => { clearInterval(h); }, 3.0); \
            setTimeout(() => { print(log.length); }, 10.0); }",
    );
    // Interval fires at t=2 only (t=4 interval is cleared at t=3).
    assert_eq!(out, ["1"]);
}
