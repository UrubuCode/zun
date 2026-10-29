// Logical operators as selectors: &&, || and ?? return one operand untouched
// (type included) and skip the other one entirely. Also: ?? versus || versus
// parameter defaults versus optional chaining, the ternary evaluating exactly
// one branch, and logical assignment skipping its write. One labelled case
// per line.

function show(v: unknown): string {
  if (v === null) return "null";
  if (typeof v === "function") return "function";
  if (Array.isArray(v)) return "array:" + v.length;
  if (typeof v === "object") return "object";
  if (typeof v === "symbol") return "symbol";
  return typeof v + ":" + String(v);
}

const trace: string[] = [];
function traced(label: string, fn: () => unknown): void {
  trace.length = 0;
  const result = fn();
  console.log(label + "=" + show(result) + " trace=[" + trace.join(",") + "]");
}

// ---------------------------------------------------------------------------
// 1. || and && return the operand itself
// ---------------------------------------------------------------------------
console.log("--- operands returned ---");
console.log("or_zero_left=" + show(0 || "a"));
console.log("or_truthy_left=" + show("a" || 0));
console.log("or_both_falsy=" + show("" || 0));
console.log("or_null_chain=" + show(null || undefined || NaN || 0 || "last"));
console.log("or_false_bigint=" + show(false || 0n));
console.log("or_neg_zero=" + show(-0 || "replaced"));
console.log("or_empty_array=" + show([] || "never"));
console.log("or_nan_to_null=" + show(NaN || null));
console.log("or_boolean_call_is_false=" + show(Boolean(0) || "x"));
console.log("or_chain_first_truthy=" + show(1 || 2 || 3));
console.log("and_truthy_left=" + show(1 && "x"));
console.log("and_zero_left=" + show(0 && "x"));
console.log("and_empty_string_left=" + show("" && "never"));
console.log("and_null_left=" + show(null && 1));
console.log("and_truthy_to_empty_right=" + show(1 && ""));
console.log("and_chain_last=" + show(1 && 2 && 3));
console.log("and_chain_zero_mid=" + show(1 && 0 && 3));
console.log("and_object_right=" + show(1 && {} && "end"));
console.log("and_nan_left=" + show(NaN && "never"));
console.log("and_not_empty_string=" + show(!"" && "yes"));
console.log("and_comparison_booleans=" + show((1 < 2) && (3 < 2)));
console.log("and_or_precedence=" + show(0 && 1 || 2));
console.log("or_and_precedence=" + show(1 || (0 && 3)));

// ---------------------------------------------------------------------------
// 2. ?? only looks at null and undefined
// ---------------------------------------------------------------------------
console.log("--- nullish coalescing ---");
console.log("nullish_null=" + show(null ?? "d"));
console.log("nullish_undefined=" + show(undefined ?? "d"));
console.log("nullish_zero=" + show(0 ?? "d"));
console.log("nullish_nan=" + show(NaN ?? "d"));
console.log("nullish_empty_string=" + show("" ?? "d"));
console.log("nullish_false=" + show(false ?? "d"));
console.log("nullish_zero_bigint=" + show(0n ?? "d"));
console.log("nullish_neg_zero_identity=" + Object.is(-0 ?? 1, -0));
console.log("or_neg_zero_identity=" + Object.is(-0 || 1, -0));
console.log("nullish_chain_first_non_nullish=" + show(null ?? undefined ?? 0 ?? 9));
console.log("nullish_over_or_parens=" + show((0 || null) ?? "d"));

// ---------------------------------------------------------------------------
// 3. Short circuit: the skipped operand is never evaluated
// ---------------------------------------------------------------------------
console.log("--- short circuit ---");
const log: string[] = [];
function note<T>(name: string, value: T): T {
  log.push(name);
  return value;
}
note("a", 1) && note("b", 2);
note("c", 0) && note("d", 3);
note("e", 1) || note("f", 4);
note("g", 0) || note("h", 5);
note("i", null) ?? note("j", 6);
note("k", 0) ?? note("l", 7);
note("m", "") && note("n", 1) || note("o", 2);
console.log("eval_log=" + log.join(","));

// ---------------------------------------------------------------------------
// 4. The conditional operator evaluates exactly one branch
// ---------------------------------------------------------------------------
console.log("--- ternary branches ---");
const tlog: string[] = [];
function pick(name: string, v: number): number {
  tlog.push(name);
  return v;
}
const picked1 = true ? pick("then1", 1) : pick("else1", 2);
const picked2 = 0 ? pick("then2", 3) : pick("else2", 4);
const picked3 = "" ? pick("then3", 5) : [] ? pick("then4", 6) : pick("else3", 7);
console.log("ternary_values=" + picked1 + "," + picked2 + "," + picked3);
console.log("ternary_log=" + tlog.join(","));

// ---------------------------------------------------------------------------
// 5. ||, ??, parameter default, and optional chaining disagree on purpose
// ---------------------------------------------------------------------------
console.log("--- which test replaces the value ---");
function withDefault(a: any = "D"): string {
  return String(a);
}
const probes: [string, unknown][] = [
  ["undefined", undefined],
  ["null", null],
  ["false", false],
  ["zero", 0],
  ["neg_zero", -0],
  ["nan", NaN],
  ["empty_string", ""],
  ["zero_string", "0"],
  ["zero_bigint", 0n],
  ["empty_array", []],
  ["one", 1],
  ["false_wrapper", new Boolean(false)],
];
for (const [label, v] of probes) {
  const orResult = (v as any) || "D";
  const nullishResult = (v as any) ?? "D";
  const paramResult = withDefault(v);
  const length = (v as any)?.length ?? "none";
  console.log(
    label +
      " | or_replaced:" + (orResult === "D") +
      " | nullish_replaced:" + (nullishResult === "D") +
      " | param_replaced:" + (paramResult === "D") +
      " | optional_length:" + length
  );
}

function argInfo(a: any = "D", ...rest: any[]): string {
  return a + ":" + rest.length + ":" + arguments.length;
}
console.log("arg_none=" + argInfo());
console.log("arg_explicit_undefined=" + argInfo(undefined));
console.log("arg_null_and_extra=" + argInfo(null, 1));
console.log("arg_zero_and_extra=" + argInfo(0, 1, 2));

// Defaults are evaluated lazily, only for the missing arguments.
const evaluated: string[] = [];
function mark(tag: string): string {
  evaluated.push(tag);
  return "D" + tag;
}
function lazy(a: any = mark("a"), b: any = mark("b")): string {
  return String(a) + "/" + String(b);
}
console.log("lazy_none="+ lazy() + " evaluated=[" + evaluated.join(",") + "]");
evaluated.length = 0;
console.log("lazy_first_zero=" + lazy(0) + " evaluated=[" + evaluated.join(",") + "]");
evaluated.length = 0;
console.log("lazy_nulls=" + lazy(null, null) + " evaluated=[" + evaluated.join(",") + "]");

// ---------------------------------------------------------------------------
// 6. Optional chaining: a nullish base short-circuits the whole chain,
//    including the computed key expression.
// ---------------------------------------------------------------------------
console.log("--- optional chaining ---");
function key(name: string): string {
  trace.push(name);
  return "x";
}
const present: any = { x: { y: 5 }, fn: () => "called" };
const absent: any = null;
traced("chain_present", () => present?.[key("present")]?.y);
traced("chain_absent", () => absent?.[key("absent")]?.y);
traced("call_present", () => present.fn?.());
traced("call_missing", () => present.missing?.());
traced("chain_on_false", () => (false as any)?.toString());
traced("chain_on_zero", () => (0 as any)?.toFixed(1));
traced("chain_on_empty_string", () => ""?.length);
traced("chain_on_nan", () => (NaN as any)?.toFixed(0));

// ---------------------------------------------------------------------------
// 7. Logical assignment: ??= writes only when nullish, ||= when falsy,
//    &&= when truthy.
// ---------------------------------------------------------------------------
console.log("--- logical assignment values ---");
function assignRow(label: string, initial: unknown): void {
  const a: any = { v: initial };
  const b: any = { v: initial };
  const c: any = { v: initial };
  a.v ??= "N";
  b.v ||= "O";
  c.v &&= "A";
  console.log(label + " | ??=:" + show(a.v) + " | ||=:" + show(b.v) + " | &&=:" + show(c.v));
}
for (const [label, v] of probes) {
  assignRow(label, v);
}

console.log("--- logical assignment accessors ---");
const acc: string[] = [];
const obj: any = {
  _x: "truthy",
  _y: 0,
  _z: null,
  get x() { acc.push("get_x"); return this._x; },
  set x(v: any) { acc.push("set_x"); this._x = v; },
  get y() { acc.push("get_y"); return this._y; },
  set y(v: any) { acc.push("set_y"); this._y = v; },
  get z() { acc.push("get_z"); return this._z; },
  set z(v: any) { acc.push("set_z"); this._z = v; },
};
obj.x ||= "replaced";
obj.y ||= "replaced";
obj.z ??= "filled";
obj.x &&= "kept";
console.log("assign_accessor_log=" + acc.join(","));
console.log("assign_final_x=" + show(obj._x));
console.log("assign_final_y=" + show(obj._y));
console.log("assign_final_z=" + show(obj._z));

const acc2: string[] = [];
const obj2: any = {
  _v: 0,
  get v() { acc2.push("get"); return this._v; },
  set v(n: any) { acc2.push("set"); this._v = n; },
};
obj2.v ??= 9;
obj2.v ||= 7;
obj2.v &&= 0;
obj2.v ??= 99;
console.log("assign_accessor_log2=" + acc2.join(","));
console.log("assign_final_v=" + show(obj2._v));

// An assignment inside a logical expression yields the assigned operand.
let assigned: any = "init";
const viaAssign = (assigned = 0) || "fallback";
console.log("assign_side_effect=" + show(assigned));
console.log("assign_or_result=" + show(viaAssign));
