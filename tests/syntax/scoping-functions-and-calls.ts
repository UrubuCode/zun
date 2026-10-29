// Scoping, hoisting, closures, parameters, eval, `this`, IIFEs and `new`.
// Strict mode for the whole file: Bun runs TypeScript modules strict, so the
// semantics pinned here are the ones both runtimes share.
"use strict";

// --- var, let and const: hoisting, dead zone and typeof ---
function hoistedVar(): string {
  const before = typeof hoistedValue + ":" + String(hoistedValue);
  var hoistedValue = 3;
  return before + "|" + hoistedValue;
}
console.log("var_hoist=" + hoistedVar());
function tdzTypeof(): string {
  try {
    const probe = typeof pending;
    return "got:" + probe;
  } catch (e) {
    return "threw:" + (e as any).constructor.name;
  }
  let pending = 1;
}
console.log("typeof_tdz=" + tdzTypeof());
{
  try {
    console.log("typeof_tdz_block=" + typeof blockLet);
  } catch (e) {
    console.log("typeof_tdz_block=threw:" + (e as any).constructor.name);
  }
  let blockLet = 1;
  console.log("block_let=" + blockLet);
}
console.log("typeof_undeclared=" + typeof (globalThis as any).neverDeclaredHere);
const shadowed = "outer";
{
  const shadowed = "inner";
  console.log("block_const=" + shadowed);
}
console.log("outer_const=" + shadowed);

// --- block-level function declarations (hoisted within their block) ---
{
  const beforeText = shape();
  function shape(): string { return "inner-block"; }
  console.log("block_hoisted=" + beforeText + "|" + shape());
}
{
  function pick(): number { return 1; }
  console.log("sibling_a=" + pick());
}
{
  function pick(): number { return 2; }
  console.log("sibling_b=" + pick());
}
if (true) {
  function conditional(): string { return "yes"; }
  console.log("in_if=" + conditional());
}
{
  function depth(): string { return "d1"; }
  {
    function depth(): string { return "d2"; }
    {
      function depth(): string { return "d3"; }
      console.log("depth3=" + depth());
    }
    console.log("depth2=" + depth());
  }
  console.log("depth1=" + depth());
}
{
  let counter = 0;
  function bump(): number { counter += 1; return counter; }
  bump();
  bump();
  console.log("block_closure=" + bump());
}
const perIteration: Array<() => number> = [];
for (let i = 0; i < 3; i++) {
  function makeValue(): number { return i * 10; }
  perIteration.push(makeValue);
}
console.log("per_iteration=" + perIteration.map((f) => f()).join(",") + "|" + (perIteration[0] === perIteration[1]));
{
  function named(): void { /* nothing */ }
  console.log("fn_name=" + named.name + "|length=" + named.length + "|prototype=" + Object.prototype.hasOwnProperty.call(named, "prototype"));
}
function host(): string {
  const parts: string[] = [];
  {
    function local(): string { return "block"; }
    parts.push(local());
  }
  parts.push(typeof (globalThis as any).local);
  return parts.join(",");
}
console.log("host_block=" + host());
function inSwitch(n: number): string {
  switch (n) {
    case 0:
      return helper();
    case 1:
      function helper(): string { return "from-helper"; }
      return "case1:" + helper();
    default:
      return "default";
  }
}
console.log("switch_fn=" + inSwitch(1) + "|" + inSwitch(9));
try {
  function inTry(): string { return "try"; }
  console.log("try_fn=" + inTry());
} catch {
  console.log("unreachable");
}

// --- closures and shadowing: a closure binds the innermost visible name at creation ---
let v = "outer";
const readOuter = () => v;
let readInner: () => string;
{
  let v = "inner";
  readInner = () => v;
  v = "inner2";
}
console.log("closure_shadow=" + readOuter() + "|" + readInner());
let depthN = 1;
const l0 = () => depthN;
let l1: () => number;
let l2: () => number;
{
  let depthN = 2;
  l1 = () => depthN;
  {
    let depthN = 3;
    l2 = () => depthN;
  }
}
console.log("three_levels=" + l0() + "," + l1() + "," + l2());
let picked: () => string;
const flag = true;
if (flag) {
  let side = "then";
  picked = () => side;
} else {
  let side = "else";
  picked = () => side;
}
console.log("branch_binding=" + picked());
function varBlock(): string {
  var w = "fn";
  const read = () => w;
  {
    var w2 = "block";
    w = "reassigned";
  }
  return read() + ":" + w2;
}
console.log("var_no_block_scope=" + varBlock());
function letShadowsVar(): string {
  var s = "var";
  const outerRead = () => s;
  let innerRead: () => string;
  {
    let s = "let";
    innerRead = () => s;
  }
  return outerRead() + ":" + innerRead();
}
console.log("let_shadows_var=" + letShadowsVar());
function shadowParam(p: string): string {
  const readParam = () => p;
  let readLocal: () => string;
  {
    let p = "shadowed";
    readLocal = () => p;
  }
  return readParam() + ":" + readLocal();
}
console.log("shadow_param=" + shadowParam("arg"));
let early = "before";
const earlyRead = () => early;
{
  let early = "shadow";
  void early;
}
early = "mutated";
console.log("created_before_shadow=" + earlyRead());
const loopFns: Array<() => string> = [];
let loopOuter = "O";
for (let i = 0; i < 2; i++) {
  {
    let loopOuter = "I" + i;
    loopFns.push(() => loopOuter);
  }
}
console.log("loop_block_shadow=" + loopFns[0]() + "," + loopFns[1]() + "|outer=" + loopOuter);

// a local shadows a top-level function of the same name, read and written by closures
var e = function (): string { return "TOP"; };
function factory(req: (n: string) => { id: string }): () => string {
  var e: { id: string } | undefined, s: string | null = null;
  function get(): string {
    if (s != null) return s;
    const id = (e || (e = req("Env"))).id;
    s = "sess-" + id;
    return s;
  }
  return get;
}
const memo = factory((n: string) => ({ id: "ID:" + n }));
console.log("memo_first=" + memo() + "|memo_again=" + memo() + "|top_intact=" + e());
function paramShadowsTop(e: string): string {
  const inner = function (): string { return "param:" + e; };
  return inner();
}
console.log("param_shadows_top=" + paramShadowsTop("X"));
function twinClosures(): string {
  var n = 0;
  const inc = function (): void { n = n + 1; };
  const read = function (): number { return n; };
  inc();
  inc();
  return "count=" + read();
}
console.log("twin_closures=" + twinClosures());

// a parameter that shadows a module-level name, read inside a loop inside try
const etiqueta = "MODULE";
const c = { v: 1 };
function marca(v: unknown): string { return v === undefined ? "UNDEF" : "ok"; }
function loopInTry(c: object): string {
  try {
    let seen = "never";
    for (let w = 0; w < 1; w++) seen = marca(c);
    return seen;
  } catch (err) {
    return "THREW " + String(err);
  }
}
function whileInTry(c: object): string {
  try {
    let seen = "never";
    let n = 0;
    while (n < 1) { n = n + 1; seen = marca(c); }
    return seen;
  } catch (err) {
    return "THREW " + String(err);
  }
}
function catchBody(c: object): string {
  try {
    throw new Error("x");
  } catch (err) {
    let seen = "never";
    for (let w = 0; w < 1; w++) seen = marca(c);
    return seen;
  }
}
function paramNamed(etiqueta: string): string {
  try {
    let seen = "never";
    for (let w = 0; w < 1; w++) seen = etiqueta;
    return seen;
  } catch (err) {
    return "THREW " + String(err);
  }
}
function closureInLoop(c: object): string {
  const read = (): string => marca(c);
  try {
    let seen = "never";
    for (let w = 0; w < 1; w++) seen = read();
    return seen;
  } catch (err) {
    return "THREW " + String(err);
  }
}
console.log("param_in_try=" + loopInTry(c) + "|" + whileInTry(c) + "|" + catchBody(c) + "|" + closureInLoop(c));
console.log("param_named_module=" + paramNamed("PARAM") + "|module=" + etiqueta);
const e2 = "MODULE";
function catchShadowInLoop(): string {
  let seen = "never";
  try {
    throw "CAUGHT";
  } catch (e2) {
    for (let w = 0; w < 1; w++) seen = e2 as string;
  }
  return seen + "/" + e2;
}
console.log("catch_shadow_loop=" + catchShadowInLoop());

// --- globals: the last link of the scope chain is the global object ---
declare var __xrtFn: (a: number, b: number) => string;
declare var __xrtObj: { mode: string };
(globalThis as Record<string, unknown>)["__xrtFn"] = function (a: number, b: number) {
  return "w:" + a + b;
};
(globalThis as Record<string, unknown>)["__xrtObj"] = { mode: "live" };
function callGlobal(): string { return __xrtFn(1, 2); }
console.log("global_call=" + callGlobal() + "|read=" + __xrtObj.mode);
function paramShadowsGlobal(__xrtFn: string): string { return __xrtFn + "!"; }
console.log("param_shadows_global=" + paramShadowsGlobal("param"));
let missingCall = "NONE";
try {
  (globalThis as Record<string, unknown>)["unusedKey"];
  (missingNameCall as () => void)();
} catch (e) {
  missingCall = (e as any).constructor.name;
}
console.log("missing_call=" + missingCall);
let missingRead = "NONE";
try {
  missingRead = "value:" + ((missingNameRead as number) + 1);
} catch (e) {
  missingRead = (e as any).constructor.name;
}
console.log("missing_read=" + missingRead);
function andLiteral(): string { if (false && (missingA as boolean)) return "y"; return "n"; }
function orLiteral(): string { if (true || (missingB as boolean)) return "y"; return "n"; }
function ternaryFalse(c2: boolean): string { return c2 ? "yes" : (missingC as string); }
console.log("short_circuit=" + andLiteral() + "|" + orLiteral() + "|" + ternaryFalse(true));
function sniff(): string {
  if (typeof missingEnvA !== "undefined" && (missingEnvA as { x: number }).x) return "a";
  if (typeof missingEnvB === "function") return "b";
  return "none";
}
console.log("env_sniff=" + sniff() + "|typeof_missing=" + typeof neverDefinedAtAll);

// --- default parameters, the arguments object and rest parameters ---
function fDefaults(a: number, b = a + 1, c = b + arguments.length): string {
  return [a, b, c].join(",");
}
console.log("defaults_a=" + fDefaults(2) + "|" + fDefaults(2, 10) + "|" + fDefaults(2, undefined, 20));
function gDefaults(a = 1, b = () => a, c = (a = 5)): string {
  return b() + ":" + c + ":" + a;
}
console.log("defaults_scope=" + gDefaults() + "|" + gDefaults(9));
function chained(a = 2, b = a * 3, c = b + a): string { return [a, b, c].join(","); }
console.log("defaults_prior=" + chained() + "|" + chained(4) + "|" + chained(4, undefined, 20));
function argsLen(a: number, b?: number): string { return arguments.length + ":" + String(b); }
console.log("arguments_len=" + argsLen(1) + "|" + argsLen(1, 2));
const sumAll = (...nums: number[]): number => nums.reduce((acc, n) => acc + n, 0);
console.log("rest_arrow=" + sumAll() + "|" + sumAll(1, 2, 3));

// --- direct versus indirect eval ---
const evalGlobal = "global";
function probeEval(): string {
  const evalGlobal = "local";
  const direct = eval("evalGlobal");
  const indirect = (0, eval)("typeof evalGlobal === 'undefined' ? 'missing' : evalGlobal");
  return direct + ":" + indirect;
}
console.log("eval_probe=" + probeEval());
console.log("eval_arithmetic=" + eval("1 + 2 * 3"));
const indirectThis = (0, eval)("this === globalThis");
console.log("indirect_this_global=" + indirectThis);

// --- `this` probes and IIFE patterns used by bundlers ---
console.log("plain_call_this=" + String((function () { return this === undefined; })()));
const r1 = (function () { return 42; })();
const r2 = (() => 99)();
const r3 = ((a: number, b: number) => a + b)(3, 4);
console.log("iife=" + r1 + "," + r2 + "," + r3);
const bundled = (function (global: { count: number }) {
  global.count += 1;
  return global.count;
})({ count: 10 });
console.log("iife_module=" + bundled);
void (function init() {
  console.log("void_iife=called");
})();
const nestedIife = (() => {
  const inner = (() => 7)();
  return inner * 6;
})();
console.log("nested_iife=" + nestedIife);
const scopeResult = (function () {
  var _a = 3, _b = 4, _c = 10;
  var _d = _a + _b;
  var _e = _d * _c;
  var _f = function (g: number) { return g > 0 ? g : -g; };
  return _f(_e - 100);
})();
console.log("minified_iife=" + scopeResult);
var moduleRef = (function () {
  var priv = 0;
  function inc() { priv++; }
  function dec() { priv--; }
  function get() { return priv; }
  return { inc: inc, dec: dec, get: get };
})();
moduleRef.inc(); moduleRef.inc(); moduleRef.inc(); moduleRef.dec();
console.log("module_pattern=" + moduleRef.get());
;(function (root: any, factory: () => any) {
  root._umdLib = factory();
})(globalThis as any, function () {
  const cache = new Map<string, number>();
  return {
    compute(key: string, val: number) {
      if (!cache.has(key)) cache.set(key, val * val);
      return cache.get(key)!;
    },
  };
});
const umd = (globalThis as any)._umdLib;
console.log("umd_compute=" + umd.compute("x", 7) + "," + umd.compute("x", 99) + "," + umd.compute("y", 3));

// --- new: member paths, argument lists and constructors that return objects ---
function Point(this: any, x: number, y: number) {
  this.x = x === undefined ? -1 : x;
  this.y = y === undefined ? -1 : y;
}
(Point as any).prototype.sum = function (this: any) { return this.x + this.y; };
const ns: any = { deep: { Point: Point } };
const p1 = new ns.deep.Point(1, 2);
console.log("new_path=" + p1.sum() + "|" + (p1 instanceof (Point as any)));
const p2 = new (Point as any)();
console.log("new_no_args=" + p2.x + "," + p2.y);
console.log("new_then_call=" + new (Point as any)(3, 4).sum());
function Maker(this: any) {
  this.marker = "ignored";
  return Point;
}
const made = new (Maker as any)();
console.log("ctor_returns_object=" + (made === Point) + "|marker=" + (made.marker === undefined));
const doubleNew = new (new (Maker as any)())(5, 6);
console.log("double_new=" + doubleNew.sum());
function Primitive(this: any) {
  this.kept = 7;
  return 42 as any;
}
console.log("ctor_returns_primitive=" + (new (Primitive as any)()).kept);
function factoryFn(): any { return Point; }
console.log("new_factory_call=" + new (factoryFn())(8, 9).sum());
const newedFactory: any = new (factoryFn as any)();
console.log("new_without_parens=" + (newedFactory === Point));
const order: string[] = [];
function note(tagName: string, value: number): number { order.push(tagName); return value; }
function callee(): any { order.push("callee"); return Point; }
new (callee())(note("a", 1), note("b", 2));
console.log("new_eval_order=" + order.join(","));
class Boxed {
  v: number;
  constructor(v: number) { this.v = v; }
  double(): number { return this.v * 2; }
  static make(v: number): Boxed { return new Boxed(v); }
}
const holder: any = { Boxed: Boxed };
console.log("class_path=" + new holder.Boxed(21).double() + "|static=" + Boxed.make(5).double());
console.log("typeof_new=" + typeof new (Point as any)(0, 0));
console.log("new_optional=" + (new (Point as any)(4, 4) as any)?.sum() + "|" + (new (Point as any)(4, 4) as any)?.nothere?.deep);

// --- automatic semicolon insertion: where a newline does or does not end a statement ---
function returnNewline(): any {
  return
  "never-delivered";
}
console.log("asi_return_newline=" + String(returnNewline()));
function returnObjectNewline(): any {
  return
  { ok: true };
}
console.log("asi_return_object=" + String(returnObjectNewline()));
function continuedByOperator(): number {
  let total = 1
    + 2
    + 3;
  return total;
}
console.log("asi_operator_continues=" + continuedByOperator());
function continuedByDot(): string {
  return "abc"
    .toUpperCase()
    .slice(0, 2);
}
console.log("asi_dot_continues=" + continuedByDot());
function continuedByParen(): string {
  const f = function (x: string): string { return "called:" + x; };
  const value = f
  ("arg");
  return value;
}
console.log("asi_paren_call=" + continuedByParen());
function continuedByBracket(): string {
  const arr = ["zero", "one"]
  const picked = arr
  [1];
  return picked;
}
console.log("asi_bracket_member=" + continuedByBracket());
function taggedByNewline(): string {
  const tag2 = function (parts: any): string { return "tagged:" + parts[0]; };
  const value = tag2
  `body`;
  return value;
}
console.log("asi_tagged_newline=" + taggedByNewline());
function minusContinues(): string {
  let a = 10
  let b = 4
  const c = a
  - b;
  return "c=" + c;
}
console.log("asi_minus_continues=" + minusContinues());
function postfixNewline(): string {
  let a = 1;
  let b = 10;
  a
  ++
  b;
  return "a=" + a + " b=" + b;
}
console.log("asi_prefix_on_next_line=" + postfixNewline());
function breakNewline(): string {
  const seen: string[] = [];
  const outer = "value-not-label";
  outer: for (let i = 0; i < 2; i++) {
    for (let j = 0; j < 3; j++) {
      if (j === 1) break
      outer;
      seen.push(i + "" + j);
    }
    seen.push("outer-continues" + i);
  }
  return seen.join(",");
}
console.log("asi_break_newline=" + breakNewline());
function doWhileAsi(): string {
  let n = 0;
  do n += 1; while (n < 3)
  const after = "next-statement";
  return n + "/" + after;
}
console.log("asi_do_while=" + doWhileAsi());
function forHeadSemis(): string {
  let i = 0;
  for (;;) {
    i += 1;
    if (i > 3) break;
  }
  return "i=" + i;
}
console.log("asi_for_head=" + forHeadSemis());
function singleStatementIf(): string {
  const seen: string[] = [];
  if (false)
    seen.push("inside");
  seen.push("outside");
  return seen.join(",");
}
console.log("asi_single_if=" + singleStatementIf());
const arrowNewlineBody = (x: number): string =>
  "got:" + x;
console.log("asi_arrow_body=" + arrowNewlineBody(3));
