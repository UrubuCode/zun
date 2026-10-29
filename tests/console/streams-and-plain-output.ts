// console stream routing and plain (non-object) output that Bun and Node agree on.
// Stdout carries console.log / console.info / console.debug; console.warn,
// console.error and console.assert write to stderr. Only stdout is compared,
// so the stderr lines here must never appear in the stdout stream.
// Object/array pretty-printing is intentionally excluded (it differs by runtime).

// ---- stream routing ----
console.log("stdout_log=1");
console.info("stdout_info=%s|", "i");
console.debug("stdout_debug=%d|", 3);
console.warn("stderr_warn=%s|", "w");
console.error("stderr_error=%s|", "e");
console.assert(true, "assert-true-is-silent");
console.log("after_stderr=reached");

// console.assert with a passing condition is silent and does not throw.
let assertThrew = false;
try {
  console.assert(1 === 1, "not-printed");
} catch {
  assertThrew = true;
}
console.log("assert_threw=" + assertThrew);

// ---- primitives as multiple top-level arguments ----
// Top-level strings print raw; other primitives print via their string form.
console.log("a", 1, true, null, undefined, "s");
console.log(NaN, Infinity, -Infinity);
console.log(0.1 + 0.2, 1e21, 2 ** 53, 1 / 3);
console.log(-0 === 0, Object.is(-0, 0));
console.log("", "", "x");
console.log("x", "", "y");
console.log(1, 2, 3);
console.log(10n, -5n);
console.log(true, false);
console.log(null);
console.log(undefined);
console.log(String(null), String(undefined), `${null}`, `${undefined}`);

// ---- string concatenation (pure JS semantics) ----
console.log("concat=" + 1 + 2);
console.log("sum=" + (1 + 2));
console.log("arith=" + (1 + 2 + "a"));
console.log("mixed=" + true + null + undefined);
console.log("neg=" + -0);
console.log("nan=" + NaN + "|" + Infinity);
console.log(`template ${1 + 1} ${"x"} ${true}`);
console.log("multi\nline\ttabbed");
console.log("unicode=é ñ 日本");
console.log("quote=' \" `");

// ---- console.group indentation (plain strings only) ----
console.group("G");
console.log("inside-group");
console.group("H");
console.log("nested");
console.groupEnd();
console.log("back-one-level");
console.groupEnd();
console.log("outside-group");

// ---- console.count is a deterministic counter ----
console.count("tick");
console.count("tick");
console.count("other");
console.countReset("tick");
console.count("tick");

// ---- API shape of other console methods (typeof only, never invoked) ----
console.log("typeof_table=" + typeof console.table);
console.log("typeof_dir=" + typeof console.dir);
console.log("typeof_time=" + typeof console.time);
console.log("typeof_group=" + typeof console.group);
console.log("typeof_groupEnd=" + typeof console.groupEnd);
console.log("typeof_countReset=" + typeof console.countReset);
console.log("typeof_assert=" + typeof console.assert);

// ---- a console method keeps working when called through a saved reference ----
const savedLog = console.log;
savedLog("saved-reference=%s|", "ok");
console.log("done");
