// Parameter lists are their own scope once they are non-simple (a default, a
// rest, or a destructuring pattern). Covers left-to-right default evaluation and
// the TDZ for later parameters, defaults that see earlier parameters but never the
// body, the body's `var` getting a copy, closures created in a default, the
// simple-list case where a body `var` is the same binding, hoisted declarations
// that collide with a parameter, `length` and `arguments` under each parameter
// shape, and parameters that are captured and reassigned.
export {};

const show = (label: string, value: unknown): void => {
  console.log(label + "=" + JSON.stringify(value));
};

function tryOf(fn: () => unknown): string {
  try {
    return "ok:" + String(fn());
  } catch (e) {
    return "threw:" + (e as any).name;
  }
}

// ---------------------------------------------------------------------------
// 1. Defaults: order, once, only for missing arguments, earlier parameters only
// ---------------------------------------------------------------------------
function earlier(a: number, b: number = a * 2, c: number = a + b): string {
  return a + "/" + b + "/" + c;
}
show("earlier_all", earlier(1));
show("earlier_two", earlier(1, 10));
show("earlier_three", earlier(1, 10, 100));

function later(a: number = (b as any), b: number = 2): string { return a + "/" + b; }
show("later_default_tdz", tryOf(() => later()));
show("later_supplied", later(9));

function selfRef(a: number = (a as any)): string { return String(a); }
show("self_ref_tdz", tryOf(() => selfRef()));

const calls: string[] = [];
function trace(tag: string, v: any): any { calls.push(tag); return v; }
function ordered(
  a: any = trace("a", 1),
  b: any = trace("b", 2),
  c: any = trace("c", 3),
): string {
  return a + "" + b + "" + c;
}
calls.length = 0;
show("all_defaults", [ordered(), calls.join(",")]);
calls.length = 0;
show("middle_given", [ordered(undefined, 9), calls.join(",")]);
calls.length = 0;
show("explicit_undefined", [ordered(undefined, undefined, 9), calls.join(",")]);
calls.length = 0;
show("null_skips_default", [ordered(null, null, null), calls.join(",")]);

// A default may call a function declared later at module level (hoisting).
function usesHoisted(a: number = helperLater()): number { return a; }
function helperLater(): number { return 77; }
show("hoisted_helper", usesHoisted());

// Each call evaluates the default afresh; a shared default object is shared.
function accumulates(acc: any = []): string {
  acc.push("x");
  return String(acc.length);
}
show("fresh_default", [accumulates(), accumulates(), accumulates()]);
const sharedBox: any = { count: 0 };
function sharesBox(box: any = sharedBox): string {
  box.count += 1;
  return String(box.count);
}
show("shared_default", [sharesBox(), sharesBox(), sharesBox({ count: 10 })]);

// A default may call a callback passed earlier.
function compute(fn: (n: number) => number, value = fn(4)): number { return value; }
show("default_calls_callback", [compute((n) => n * 3), compute((n) => n * 3, 99)]);

// ---------------------------------------------------------------------------
// 2. Defaults see earlier parameters, never the body
// ---------------------------------------------------------------------------
function outerHelper(): string { return "outer-helper"; }
function defaultUsesOuter(v: string = outerHelper()): string {
  function outerHelper(): string { return "body-helper"; }
  return v + "|body=" + outerHelper();
}
show("default_uses_outer", defaultUsesOuter());

function seesBodyVar(a: any = typeof hidden): string {
  var hidden = "body-value";
  return String(a) + "/" + hidden;
}
show("default_cannot_see_body_var", seesBodyVar());

function leftOnly(a: number, b: number = a * 2): string {
  var a = 100;
  return "a=" + a + " b=" + b;
}
show("default_sees_left_param", leftOnly(3));

// ---------------------------------------------------------------------------
// 3. The body's `var` against a parameter: one binding or a copy
// ---------------------------------------------------------------------------
function bareVar(x: any): string {
  var x;
  return String(x);
}
show("simple_bare_var_keeps_arg", [bareVar("argument"), bareVar()]);

function initialisedVar(x: any): string {
  var x = "body-value";
  return String(x);
}
show("simple_var_overwrites", initialisedVar("argument"));

function oneBinding(x: any): string {
  const read = (): string => String(x);
  var x = "written-later";
  return read();
}
show("simple_one_binding", oneBinding("argument"));

function separateScopes(x: any, peek: any = (): string => String(x)): string {
  var x = "body-copy";
  return "body=" + String(x) + " param=" + peek();
}
show("default_separate_scopes", separateScopes("argument"));

function withRest(x: any, peek: any = (): string => String(x), ...others: any[]): string {
  var x = "body-copy";
  return "body=" + String(x) + " param=" + peek() + " rest=" + others.length;
}
show("rest_separate_scopes", withRest("argument"));

function bodyClosure(x: any, peek: any = (): string => String(x)): string {
  var x = "body-copy";
  const bodyPeek = (): string => String(x);
  x = "changed-again";
  return "body=" + bodyPeek() + " param=" + peek();
}
show("body_closure_sees_copy", bodyClosure("argument"));

function patternShadow({ p }: any, peek: any = (): string => String(p)): string {
  var p = "body-copy";
  return "body=" + String(p) + " param=" + peek();
}
show("pattern_shadow", patternShadow({ p: "from-pattern" }));

// The copy is taken at entry, so a later write from the list's own closure
// does not reach the body's `var`.
function mutatedInList(a: any, touch: any = (): void => { a = "changed-in-list"; }): string {
  var a = a;
  touch();
  return "body=" + String(a);
}
show("mutated_in_list", mutatedInList("start"));

// A closure created in a default keeps the PARAMETER scope, even when the body
// reassigns the parameter.
function capturesParamScope(a: number = 1, get: () => number = () => a): string {
  a = 50;
  return String(get());
}
show("closure_in_default_sees_param", capturesParamScope());

function paramScopeVsLet(a: number = 1, get: () => number = () => a): string {
  let shadow = a + 100;
  return get() + "/" + shadow;
}
show("param_scope_vs_let", paramScopeVsLet(2));

// ---------------------------------------------------------------------------
// 4. Hoisted declarations and blocks that collide with parameters
// ---------------------------------------------------------------------------
function hoistedOverParam(dup: any): string {
  const seenAtEntry = typeof dup;
  return seenAtEntry + "/" + typeof dup;
  function dup(): void {}
}
show("hoisted_over_param", hoistedOverParam("string-argument"));

function hoistedThenAssigned(dup: any): string {
  const before = typeof dup;
  dup = "now-a-string";
  return before + "->" + typeof dup;
  function dup(): void {}
}
show("hoisted_then_assigned", hoistedThenAssigned(1));

function blockShadow(v: string): string {
  let seen = "";
  {
    let v = "block";
    seen = v;
  }
  return seen + "/" + v;
}
show("block_shadow", blockShadow("param"));

const outerName = "outer";
function shadowsOuter(outerName: string): string {
  const inner = outerName;
  return inner + "/" + outerName;
}
show("param_shadows_outer", [shadowsOuter("parameter"), outerName]);

function nestedShadow(v: string): string {
  function inner(v: string): string { return "inner:" + v; }
  return inner("nested") + " outer:" + v;
}
show("nested_shadow", nestedShadow("outer-arg"));

function selfName(selfName: any): string { return typeof selfName; }
show("param_named_like_fn", [selfName("shadowed"), typeof selfName]);

function levelOne(v: string): string {
  function levelTwo(v: string): string {
    function levelThree(v: string): string { return "3:" + v; }
    return levelThree(v + "-c") + " 2:" + v;
  }
  return levelTwo(v + "-b") + " 1:" + v;
}
show("three_levels", levelOne("a"));

// ---------------------------------------------------------------------------
// 5. Parameters that are captured and reassigned
// ---------------------------------------------------------------------------
// The shape a transpiler emits for `flag = false` as a default.
function withFlagDefault(a: number, flag?: boolean): string {
  flag === undefined && (flag = false);
  const read = (): string => "flag=" + flag;
  return read();
}
show("default_omitted", withFlagDefault(1, undefined));
show("default_given", withFlagDefault(1, true));

function contador(inicio: number): () => number {
  return (): number => {
    inicio = inicio + 1;
    return inicio;
  };
}
const c1 = contador(0);
const c2 = contador(100);
show("per_call_cell", [c1(), c1(), c2()]);

function noLaco(base: number): string {
  const fs: Array<() => number> = [];
  for (let i = 0; i < 3; i++) {
    fs.push((): number => base + i);
  }
  base = base * 10;
  return fs.map((f) => f()).join(",");
}
show("param_reassigned_in_loop", noLaco(1));

// ---------------------------------------------------------------------------
// 6. length, rest identity and arguments under each parameter shape
// ---------------------------------------------------------------------------
function withRestLen(a: number, b: number = 1, ...rest: number[]): number { return a + b + rest.length; }
show("length_all_defaults", ordered.length);
show("length_default_stops", [earlier.length, ordered.length]);
show("length_rest_excluded", withRestLen.length);
function patternLen({ a }: any, [b]: any, c: number): number { return a + b + c; }
show("length_pattern_counts", patternLen.length);

function restIdentity(...rest: number[]): string {
  return Array.isArray(rest) + "/" + (rest === (arguments as any)) + "/" + rest.length;
}
show("rest_is_fresh_array", restIdentity(1, 2, 3));

function unmappedWithDefault(a: number = 0): string {
  const before = String(arguments[0]);
  a = 111;
  const after = String(arguments[0]);
  (arguments as any)[0] = 222;
  return before + "/" + after + "/" + a;
}
show("unmapped_with_default", unmappedWithDefault(7));

function restUnmapped(...rest: number[]): string {
  const before = String(arguments[0]);
  rest[0] = 111;
  return before + "/" + String(arguments[0]) + "/" + rest[0];
}
show("unmapped_with_rest", restUnmapped(7));

function destructuredUnmapped({ v }: any = { v: 0 }): string {
  return JSON.stringify(arguments[0]) + "/" + v;
}
show("unmapped_with_pattern", destructuredUnmapped({ v: 3 }));

// Destructuring defaults at both levels.
function destructured({ x = 1, y = x + 1 }: any = {}, [p = y]: any = []): string {
  return x + "/" + y + "/" + p;
}
show("destructured_all", destructured());
show("destructured_partial", destructured({ x: 5 }));
show("destructured_both", destructured({ x: 5, y: 6 }, [7]));
