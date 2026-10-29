// The well-known symbols as protocol hooks: their shared non-configurable
// descriptors; Symbol.iterator and how each consumer FAILS on a broken iterator;
// custom iterables; Symbol.asyncIterator; Symbol.isConcatSpreadable; the
// Symbol.species consultation points; the RegExp-side hooks (match, replace,
// search, matchAll, split) and the IsRegExp brand; Symbol.unscopables; and where
// Symbol.toStringTag is written in the standard library.

function bad(label: string, fn: () => any): void {
  try { fn(); console.log(label + "=no_throw"); }
  catch (e: any) { console.log(label + "=" + e.constructor.name); }
}
function probe(label: string, fn: () => any): void {
  try { console.log(label + "=ok:" + String(fn())); }
  catch (e: any) { console.log(label + "=" + e.constructor.name); }
}

// --- well-known symbols: own, non-writable, non-enumerable, NON-configurable data properties ---
const names = [
  "iterator", "asyncIterator", "hasInstance", "isConcatSpreadable", "match", "matchAll",
  "replace", "search", "species", "split", "toPrimitive", "toStringTag", "unscopables",
];
let allSymbols = true;
let allOwn = true;
let descriptionsOk = true;
let registeredAny = false;
let flags = "";
for (const n of names) {
  const sym = (Symbol as any)[n];
  if (typeof sym !== "symbol") allSymbols = false;
  if (!Object.prototype.hasOwnProperty.call(Symbol, n)) allOwn = false;
  if (sym.description !== "Symbol." + n) descriptionsOk = false;
  if (Symbol.keyFor(sym) !== undefined) registeredAny = true;
  const d: any = Object.getOwnPropertyDescriptor(Symbol, n);
  const f = String(d.writable) + String(d.enumerable) + String(d.configurable);
  if (flags === "") flags = f;
  else if (flags !== f) flags = "MIXED";
}
console.log("wellknown_count=" + names.length);
console.log("wellknown_all_symbols=" + allSymbols + ":all_own=" + allOwn);
console.log("wellknown_descriptions=" + descriptionsOk);
console.log("wellknown_registered_any=" + registeredAny);
console.log("wellknown_flags_uniform=" + flags);
console.log("wellknown_present_in_names=" + names.every((n) => Object.getOwnPropertyNames(Symbol).indexOf(n) >= 0));
console.log("wellknown_enumerable_keys=" + Object.keys(Symbol).length);
bad("wellknown_redefine_value", () => Object.defineProperty(Symbol, "iterator", { value: 1 }));
bad("wellknown_redefine_writable", () => Object.defineProperty(Symbol, "iterator", { writable: true }));
bad("wellknown_redefine_getter", () => Object.defineProperty(Symbol, "iterator", { get() { return 1; } }));
console.log("wellknown_reflect_delete=" + Reflect.deleteProperty(Symbol, "iterator"));
console.log("wellknown_reflect_set=" + Reflect.set(Symbol, "iterator", 1) + ":still_symbol=" + typeof Symbol.iterator);
console.log("wellknown_same_value_redefine=" + (Object.defineProperty(Symbol, "iterator", { value: Symbol.iterator }) === Symbol));
console.log("toPrimitive_tostring=" + Symbol.toPrimitive.toString() + ":" + Symbol.iterator.description);

// --- Symbol.iterator: how each consumer FAILS (or succeeds) on the same broken object ---
function broken(kind: string): any {
  const o: any = { length: 2, 0: "a", 1: "b" };
  if (kind === "missing") return o;
  if (kind === "undefined") { o[Symbol.iterator] = undefined; return o; }
  if (kind === "null") { o[Symbol.iterator] = null; return o; }
  if (kind === "number") { o[Symbol.iterator] = 42; return o; }
  if (kind === "object") { o[Symbol.iterator] = {}; return o; }
  if (kind === "returns_primitive") { o[Symbol.iterator] = function () { return 5; }; return o; }
  if (kind === "returns_null") { o[Symbol.iterator] = function () { return null; }; return o; }
  if (kind === "no_next") { o[Symbol.iterator] = function () { return {}; }; return o; }
  if (kind === "next_not_callable") { o[Symbol.iterator] = function () { return { next: 1 }; }; return o; }
  if (kind === "next_returns_primitive") { o[Symbol.iterator] = function () { return { next() { return 7; } }; }; return o; }
  if (kind === "next_throws") { o[Symbol.iterator] = function () { return { next() { throw new EvalError("next"); } }; }; return o; }
  if (kind === "hook_throws") { o[Symbol.iterator] = function () { throw new URIError("hook"); }; return o; }
  if (kind === "getter_throws") {
    Object.defineProperty(o, Symbol.iterator, { get() { throw new RangeError("getter"); } });
    return o;
  }
  if (kind === "ok") {
    o[Symbol.iterator] = function () {
      let i = 0;
      return { next: () => (i < 2 ? { value: "v" + i++, done: false } : { value: undefined, done: true }) };
    };
    return o;
  }
  return o;
}
const kinds = ["ok", "missing", "undefined", "null", "number", "object", "returns_primitive", "returns_null",
  "no_next", "next_not_callable", "next_returns_primitive", "next_throws", "hook_throws", "getter_throws"];
const consumers: [string, (v: any) => string][] = [
  ["for_of", (v) => { const out: string[] = []; for (const x of v) out.push(String(x)); return out.join(","); }],
  ["spread_array", (v) => [...v].join(",")],
  ["spread_call", (v) => (function (...args: any[]) { return args.join(","); })(...v)],
  ["destructure", (v) => { const [a, b] = v; return String(a) + "," + String(b); }],
  ["array_from", (v) => Array.from(v).join(",")],
  ["new_set", (v) => [...new Set(v)].join(",")],
  ["yield_star", (v) => { function* g(): any { yield* v; } return [...g()].join(","); }],
  ["object_fromEntries", (v) => Object.keys(Object.fromEntries(v)).join(",")],
];
for (const kind of kinds) {
  for (const [cname, run] of consumers) {
    let tag: string;
    try { tag = "ok(" + run(broken(kind)) + ")"; }
    catch (e: any) { tag = e.constructor.name; }
    console.log("iter." + kind + "." + cname + "=" + tag);
  }
}

// --- Array.from falls back to the array-like path only when there is NO usable hook ---
console.log("array_from_arraylike=" + Array.from({ length: 3, 0: "x", 2: "z" } as any).map(String).join(","));
console.log("array_from_no_length=" + Array.from({ 0: "x" } as any).length);
console.log("array_from_string=" + Array.from("ab").join(","));

// --- a `done` that is merely truthy or falsy is enough; a missing `value` reads as undefined ---
function withDone(v: any): any {
  let i = 0;
  return { [Symbol.iterator]() { return { next() { i++; return { value: i, done: i > 2 ? v : false }; } }; } };
}
console.log("done_truthy_string=" + [...withDone("yes")].join(","));
console.log("done_one=" + [...withDone(1)].join(","));
console.log("done_object=" + [...withDone({})].join(","));
console.log("missing_value=" + [...({ [Symbol.iterator]() { let n = 0; return { next: () => ({ done: n++ > 1 }) }; } } as any)].map(String).join(","));

// --- the hook is read ONCE per consumption; `in` and typeof do not consult it ---
let hookReads = 0;
const counted: any = {};
Object.defineProperty(counted, Symbol.iterator, {
  get() { hookReads++; return function () { let i = 0; return { next: () => (i < 1 ? { value: i++, done: false } : { done: true }) }; }; },
});
console.log("one_read=" + [...counted].join(",") + ":reads=" + hookReads);
console.log("two_reads=" + [...counted].join(",") + ":reads=" + hookReads);
const badNumber = broken("number");
console.log("brand_visible=" + (Symbol.iterator in badNumber) + ":" + typeof badNumber[Symbol.iterator]);
console.log("array_isArray_broken=" + Array.isArray(badNumber));

// --- a primitive is never iterable except a string ---
function consume(v: any): string {
  try { return "ok:" + [...v].length; }
  catch (e: any) { return e.constructor.name; }
}
console.log("prim_number=" + consume(1));
console.log("prim_boolean=" + consume(true));
console.log("prim_null=" + consume(null));
console.log("prim_undefined=" + consume(undefined));
console.log("prim_symbol=" + consume(Symbol("s")));
console.log("prim_bigint=" + consume(10n));
console.log("prim_string=" + consume("abc"));
console.log("plain_object=" + consume({ a: 1 }));

// --- a Map with its hook removed stops being iterable but keeps its other methods ---
const mapHook: any = new Map([["a", 1]]);
mapHook[Symbol.iterator] = undefined;
console.log("map_hook_removed=" + consume(mapHook) + ":get=" + mapHook.get("a") + ":size=" + mapHook.size);
console.log("map_entries_still_ok=" + [...mapHook.entries()].map((e: any) => e.join(":")).join(","));
delete mapHook[Symbol.iterator];
console.log("map_hook_restored=" + consume(mapHook));

// --- custom iterables consumed by for-of, spread, destructuring, Array.from ---
class Range {
  start: number;
  end: number;
  constructor(start: number, end: number) {
    this.start = start;
    this.end = end;
  }
  [Symbol.iterator]() {
    let cur = this.start;
    const end = this.end;
    return {
      next() {
        if (cur <= end) return { value: cur++, done: false };
        return { value: undefined, done: true };
      },
    };
  }
}
const forOfOut: number[] = [];
for (const v of new Range(1, 3)) forOfOut.push(v);
console.log("range_for_of=" + forOfOut.join(","));
console.log("range_spread=" + [...new Range(10, 13)].join(","));
const [ra, rb, rc] = new Range(20, 25) as any;
console.log("range_destructure=" + ra + "," + rb + "," + rc);
console.log("range_array_from=" + Array.from(new Range(3, 6)).join(","));

class Bag {
  data: number[] = [3, 4, 5];
  [Symbol.iterator]() {
    let i = 0;
    const d = this.data;
    return { next() { return i < d.length ? { value: d[i++], done: false } : { value: undefined, done: true }; } };
  }
}
const bagOut: number[] = [];
for (const n of new Bag() as any) bagOut.push(n);
console.log("bag_for_of=" + bagOut.join(","));

// --- Symbol.isConcatSpreadable: an array-like can be spread and an array can refuse ---
const spreadable = { 0: 4, 1: 5, length: 2, [Symbol.isConcatSpreadable]: true };
const refused: any = [6, 7];
refused[Symbol.isConcatSpreadable] = false;
console.log("concat_plain=" + [0].concat([1, 2, 3] as any).join(","));
console.log("concat_array_like=" + [0].concat(spreadable as any).join(","));
console.log("concat_refused_array=" + [0].concat(refused).length + ":" + Array.isArray([0].concat(refused)[1]));
const refusedAgain: any = [1, 2];
refusedAgain[Symbol.isConcatSpreadable] = false;
const arrayLikeOut = [0].concat(refusedAgain, { 0: "a", 1: "b", length: 2, [Symbol.isConcatSpreadable]: true } as any, 3);
console.log("concat_toggle_length=" + arrayLikeOut.length);
console.log("concat_toggle_items=" + arrayLikeOut[0] + ":" + Array.isArray(arrayLikeOut[1]) + ":" + arrayLikeOut[2] + ":" + arrayLikeOut[3] + ":" + arrayLikeOut[4]);

// --- Symbol.species: which constructor the derived-copy operations use ---
function speciesShape(label: string, C: any): void {
  const d: any = Object.getOwnPropertyDescriptor(C, Symbol.species);
  if (d === undefined) { console.log(label + "=absent"); return; }
  console.log(label + "=" + typeof d.get + ":" + d.get.name + ":len=" + d.get.length +
    ":setter=" + typeof d.set + ":" + d.enumerable + ":" + d.configurable + ":returns_this=" + (d.get.call(C) === C));
}
speciesShape("array_species", Array);
speciesShape("map_species", Map);
speciesShape("set_species", Set);
speciesShape("promise_species", Promise);
speciesShape("regexp_species", RegExp);
speciesShape("arraybuffer_species", ArrayBuffer);
speciesShape("object_species", Object);
speciesShape("string_species", String);
speciesShape("weakmap_species", WeakMap);

class Sub extends Array<number> {}
const sub = Sub.from([3, 1, 2]) as any;
console.log("from_is_subclass=" + (sub instanceof Sub));
function kindOf(label: string, v: any): void {
  console.log("species_" + label + "=" + (v instanceof Sub ? "Sub" : Array.isArray(v) ? "Array" : typeof v));
}
kindOf("map", sub.map((x: number) => x));
kindOf("filter", sub.filter(() => true));
kindOf("slice", sub.slice(0));
kindOf("splice", sub.slice().splice(0, 1));
kindOf("concat", sub.concat([9]));
kindOf("flat", sub.flat());
kindOf("flatMap", sub.flatMap((x: number) => [x]));
kindOf("toReversed", sub.toReversed());
kindOf("toSorted", sub.toSorted());
kindOf("toSpliced", sub.toSpliced(0, 1));
kindOf("with", sub.with(0, 5));
kindOf("reverse_in_place", sub.reverse());
kindOf("sort_in_place", sub.sort());
kindOf("static_of", Sub.of(1, 2));

class Plain extends Array<number> {
  static get [Symbol.species]() { return Array; }
}
const plain = Plain.from([1, 2]) as any;
console.log("plain_self=" + (plain instanceof Plain));
console.log("plain_map=" + (plain.map((x: number) => x) instanceof Plain) + ":" + Array.isArray(plain.map((x: number) => x)));
console.log("plain_filter_proto_is_array=" + (Object.getPrototypeOf(plain.filter(() => true)) === Array.prototype));

// --- species undefined or null means "use the default"; any non-constructor throws ---
function speciesResult(value: any): string {
  class Custom extends Array<number> {
    static get [Symbol.species]() { return value; }
  }
  try {
    const r = (Custom.from([1, 2]) as any).map((x: number) => x);
    return "ok:" + (r instanceof Custom ? "Custom" : Array.isArray(r) ? "Array" : typeof r);
  } catch (e: any) {
    return e.constructor.name;
  }
}
console.log("species_undefined=" + speciesResult(undefined));
console.log("species_null=" + speciesResult(null));
console.log("species_number=" + speciesResult(42));
console.log("species_plain_object=" + speciesResult({}));
console.log("species_arrow=" + speciesResult(() => 1));
console.log("species_map_constructor=" + speciesResult(Map));

// --- a species constructor is called with the requested length ---
const seenArgs: string[] = [];
class Recording extends Array<number> {
  static get [Symbol.species]() {
    return function (this: any, ...args: any[]) {
      seenArgs.push(args.length + ":" + args.map(String).join("/"));
      return new Array(...args);
    } as any;
  }
}
const rec = Recording.from([1, 2, 3]) as any;
rec.filter((x: number) => x > 1);
rec.slice(1);
console.log("species_call_args=" + seenArgs.join("|"));

// --- Map and Set carry the accessor, but their derived operations do not consult it ---
class MapSub extends Map<string, number> {
  static get [Symbol.species]() { return Map; }
}
const ms = new MapSub([["a", 1]]);
console.log("mapsub_self=" + (ms instanceof MapSub));
console.log("mapsub_entries_first=" + (ms.entries().next().value as any).join(":"));
class SetSub extends Set<number> {
  static get [Symbol.species]() { return Set; }
}
const ss = new SetSub([1, 2]);
console.log("setsub_union_is_subclass=" + (ss.union(new Set([3])) instanceof SetSub));
console.log("setsub_union_is_set=" + (ss.union(new Set([3])) instanceof Set));

// --- Promise#then, catch and finally consult it; a species of Promise yields plain promises ---
class PromiseSub extends Promise<number> {}
const ps = PromiseSub.resolve(1);
console.log("promise_resolve_kind=" + (ps instanceof PromiseSub));
console.log("promise_then_kind=" + (ps.then(() => 1) instanceof PromiseSub));
console.log("promise_catch_kind=" + (ps.catch(() => 1) instanceof PromiseSub));
console.log("promise_finally_kind=" + (ps.finally(() => 1) instanceof PromiseSub));
class PlainPromise extends Promise<number> {
  static get [Symbol.species]() { return Promise; }
}
const pp = PlainPromise.resolve(1);
console.log("promise_species_plain_self=" + (pp instanceof PlainPromise));
console.log("promise_species_plain_then=" + (pp.then(() => 1) instanceof PlainPromise));

// --- ArrayBuffer#slice and typed-array copying methods consult it ---
class BufSub extends ArrayBuffer {}
const buf = new BufSub(8);
console.log("buffer_slice_subclass=" + (buf.slice(0, 4) instanceof BufSub) + ":" + (buf.slice(0, 4) instanceof ArrayBuffer));
console.log("buffer_slice_length=" + buf.slice(0, 4).byteLength);
class PlainBuf extends ArrayBuffer {
  static get [Symbol.species]() { return ArrayBuffer; }
}
console.log("buffer_species_plain=" + (new PlainBuf(8).slice(0, 4) instanceof PlainBuf));
class U8Sub extends Uint8Array {}
const u8 = new U8Sub([1, 2, 3]);
console.log("typedarray_subarray=" + (u8.subarray(1) instanceof U8Sub));
console.log("typedarray_slice=" + (u8.slice(1) instanceof U8Sub));
console.log("typedarray_map=" + (u8.map((x: number) => x) instanceof U8Sub));
console.log("typedarray_filter=" + (u8.filter(() => true) instanceof U8Sub));

// --- RegExp[Symbol.split] consults species, so a subclass drives the split ---
let splitCtorCalls = 0;
class RegSub extends RegExp {
  static get [Symbol.species]() {
    splitCtorCalls++;
    return RegExp;
  }
}
console.log("regexp_split_via_species=" + "a1b2c".split(new RegSub("\\d", "") as any).join(","));
console.log("regexp_species_consulted=" + (splitCtorCalls > 0));

// --- Symbol.match: the IsRegExp brand. startsWith, endsWith and includes REFUSE it ---
const pretender: any = { [Symbol.match]: true, toString() { return "b"; } };
probe("startsWith_regexp", () => "abc".startsWith(/a/ as any));
probe("endsWith_regexp", () => "abc".endsWith(/c/ as any));
probe("includes_regexp", () => "abc".includes(/b/ as any));
probe("startsWith_pretender", () => "abc".startsWith(pretender));
probe("endsWith_pretender", () => "abc".endsWith(pretender));
probe("includes_pretender", () => "abc".includes(pretender));
function brandedWith(v: any): string {
  const o: any = { toString() { return "b"; } };
  o[Symbol.match] = v;
  try { return "abc".includes(o) ? "true" : "false"; }
  catch (e: any) { return e.constructor.name; }
}
const brandCases: [string, any][] = [
  ["true", true], ["one", 1], ["string", "x"], ["object", {}], ["empty_string", ""],
  ["zero", 0], ["false", false], ["null", null], ["nan", NaN],
];
for (const [label, v] of brandCases) console.log("brand_" + label + "=" + brandedWith(v));
const plainNoBrand: any = { toString() { return "b"; } };
console.log("no_brand_includes=" + "abc".includes(plainNoBrand));
console.log("no_brand_startsWith=" + "bcd".startsWith(plainNoBrand));
const denying: any = /b/;
denying[Symbol.match] = false;
console.log("denying_includes=" + "abc".includes(denying));
console.log("denying_still_regexp=" + (denying instanceof RegExp) + ":" + denying.test("abc"));
const brandedProto: any = Object.create({ [Symbol.match]: true });
brandedProto.toString = function () { return "b"; };
probe("inherited_brand", () => "abc".includes(brandedProto));
let brandReads = 0;
const counting: any = { toString() { return "b"; } };
Object.defineProperty(counting, Symbol.match, { get() { brandReads++; return false; } });
console.log("brand_getter_includes=" + "abc".includes(counting) + ":reads=" + brandReads);
const throwingBrand: any = {};
Object.defineProperty(throwingBrand, Symbol.match, { get() { throw new EvalError("brand"); } });
probe("brand_getter_throws", () => "abc".includes(throwingBrand));

// --- split does NOT ask the brand question; it dispatches on Symbol.split instead ---
console.log("split_regexp=" + "a1b2c".split(/\d/).join(","));
console.log("split_pretender=" + "abc".split(pretender).join("|"));
const splitter: any = {
  [Symbol.split](s: string, limit?: number) {
    return [s.length, limit ?? "none", s.slice(0, 2)];
  },
};
console.log("split_custom_hook=" + "abcdef".split(splitter).join(","));
console.log("split_custom_hook_limit=" + "abcdef".split(splitter, 3).join(","));
console.log("indexOf_pretender=" + "abc".indexOf(pretender));
console.log("lastIndexOf_pretender=" + "abcb".lastIndexOf(pretender));
console.log("concat_pretender=" + "a".concat(pretender));

// --- RegExp called as a function, or with a branded argument, may hand the argument back ---
const selfish: any = { [Symbol.match]: true, constructor: RegExp };
console.log("RegExp_call_returns_same=" + (RegExp(selfish) === selfish));
console.log("RegExp_new_is_fresh=" + (new RegExp(selfish) === selfish));
console.log("RegExp_new_is_regexp=" + (new RegExp(selfish) instanceof RegExp));
const otherCtor: any = { [Symbol.match]: true, constructor: function Other() { /* not RegExp */ }, source: "z", flags: "" };
console.log("RegExp_other_ctor_fresh=" + (RegExp(otherCtor) === otherCtor));
console.log("RegExp_other_ctor_source=" + RegExp(otherCtor).source);
const re = /ab/g;
console.log("RegExp_of_regexp_same=" + (RegExp(re) === re));
console.log("RegExp_of_regexp_with_flags=" + (RegExp(re, "i") === re) + ":" + RegExp(re, "i").flags);
console.log("RegExp_new_of_regexp=" + (new RegExp(re) === re) + ":" + new RegExp(re).flags);
const bare: any = { [Symbol.match]: true };
const built = new RegExp(bare);
console.log("branded_bare_source=" + built.source + ":flags=" + JSON.stringify(built.flags));
console.log("pretender_typeof=" + typeof pretender + ":instanceof=" + (pretender instanceof RegExp));
console.log("pretender_tag=" + Object.prototype.toString.call(pretender));
console.log("regexp_proto_own_match=" + Object.prototype.hasOwnProperty.call(RegExp.prototype, Symbol.match));
console.log("regexp_instance_own_match=" + Object.prototype.hasOwnProperty.call(/x/, Symbol.match));

// --- String.prototype.match reads @@match as a method, so a bare `true` is refused ---
console.log("match_hook=" + "abc".match({ [Symbol.match]() { return "HOOK"; } } as any));
console.log("match_string_fallback=" + JSON.stringify("abcb".match("b" as any)));
probe("match_brand_without_hook", () => JSON.stringify("abzc".match({ [Symbol.match]: true, toString() { return "z"; } } as any)));
probe("match_brand_with_flags", () => JSON.stringify("abzc".match({ [Symbol.match]: true, source: "z", flags: "" } as any)));

// --- Symbol.replace takes over String.prototype.replace and replaceAll entirely ---
const seen: string[] = [];
const hook: any = {
  [Symbol.replace](str: any, repl: any) {
    seen.push("this_is_hook=" + (this === hook));
    seen.push("str=" + typeof str + ":" + String(str));
    const shape = typeof repl === "function" ? "fn:" + repl.name
      : (typeof repl === "object" && repl !== null) ? "obj" : String(repl);
    seen.push("repl=" + typeof repl + ":" + shape);
    return "REPLACED";
  },
};
console.log("replace_hook=" + "abcabc".replace(hook, "X"));
console.log("replace_trace=" + seen.join("|"));
seen.length = 0;
console.log("replace_number_receiver=" + String.prototype.replace.call(12345 as any, hook, "X"));
console.log("replace_number_trace=" + seen.join("|"));
seen.length = 0;
const replFn = function () { return "f"; };
"abc".replace(hook, replFn as any);
console.log("replace_fn_repl=" + seen[2]);
seen.length = 0;
"abc".replace(hook, { toString() { return "obj"; } } as any);
console.log("replace_obj_repl=" + seen[2]);
function returning(v: any): any {
  return "abc".replace({ [Symbol.replace]() { return v; } } as any, "X");
}
console.log("replace_ret_number=" + typeof returning(42) + ":" + returning(42));
console.log("replace_ret_null=" + String(returning(null)) + ":" + typeof returning(null));
console.log("replace_ret_undefined=" + String(returning(undefined)));
console.log("replace_ret_object=" + typeof returning({ a: 1 }));
console.log("replace_ret_array=" + Array.isArray(returning([1, 2])));
bad("replace_hook_throws", () => "abc".replace({ [Symbol.replace]() { throw new RangeError("hook"); } } as any, "X"));
function withReplaceHook(v: any): string {
  const o: any = { toString() { return "b"; } };
  o[Symbol.replace] = v;
  try { return "abc".replace(o, "X"); }
  catch (e: any) { return e.constructor.name; }
}
console.log("replace_hook_undefined=" + withReplaceHook(undefined));
console.log("replace_hook_null=" + withReplaceHook(null));
console.log("replace_hook_number=" + withReplaceHook(42));
console.log("replace_hook_string=" + withReplaceHook("s"));
console.log("replace_hook_object=" + withReplaceHook({}));
const replProto: any = { [Symbol.replace]() { return "FROM_PROTO"; } };
console.log("replace_inherited_hook=" + "abc".replace(Object.create(replProto), "X"));
let lazyReads = 0;
const lazy: any = {};
Object.defineProperty(lazy, Symbol.replace, {
  get() { lazyReads++; return function () { return "LAZY" + lazyReads; }; },
});
console.log("replace_lazy_first=" + "abc".replace(lazy, "X"));
console.log("replace_lazy_second=" + "abc".replace(lazy, "X"));
console.log("replace_lazy_reads=" + lazyReads);
const globalish: any = { [Symbol.match]: true, flags: "g", [Symbol.replace]() { return "ALL"; } };
console.log("replaceAll_global_flag=" + "abc".replaceAll(globalish, "X"));
const nonGlobalish: any = { [Symbol.match]: true, flags: "i", [Symbol.replace]() { return "NEVER"; } };
bad("replaceAll_non_global_flag", () => "abc".replaceAll(nonGlobalish, "X"));
const plainHook: any = { [Symbol.replace]() { return "PLAIN"; } };
console.log("replaceAll_no_match_brand=" + "abc".replaceAll(plainHook, "X"));
const falsyMatch: any = { [Symbol.match]: 0, [Symbol.replace]() { return "FALSY"; } };
console.log("replaceAll_falsy_match=" + "abc".replaceAll(falsyMatch, "X"));
const noFlags: any = { [Symbol.match]: true, [Symbol.replace]() { return "NOFLAGS"; } };
bad("replaceAll_missing_flags", () => "abc".replaceAll(noFlags, "X"));
console.log("regexp_replace_first=" + "a-b-c".replace(/-/, "+"));
console.log("regexp_replace_global=" + "a-b-c".replace(/-/g, "+"));
console.log("regexp_replaceAll=" + "a-b-c".replaceAll(/-/g, "+"));
bad("regexp_replaceAll_non_global", () => "a-b-c".replaceAll(/-/ as any, "+"));
const shadowRe: any = /-/g;
shadowRe[Symbol.replace] = function () { return "SHADOWED"; };
console.log("shadowed_regexp=" + "a-b".replace(shadowRe, "+"));
console.log("shadowed_regexp_all=" + "a-b".replaceAll(shadowRe, "+"));
const replDesc: any = Object.getOwnPropertyDescriptor(RegExp.prototype, Symbol.replace);
console.log("regexp_replace_hook=" + typeof replDesc.value + ":" + replDesc.value.name + ":" + replDesc.value.length);
console.log("regexp_replace_hook_flags=" + replDesc.writable + ":" + replDesc.enumerable + ":" + replDesc.configurable);
console.log("string_replace_arity=" + String.prototype.replace.length + ":" + String.prototype.replaceAll.length);
console.log("plain_string_replace=" + "a-b-c".replace("-", "+"));
console.log("plain_string_replaceAll=" + "a-b-c".replaceAll("-", "+"));
console.log("plain_coerced_search=" + "a1b".replace(1 as any, "+"));

// --- Symbol.search: the hook's answer is returned untouched; a missing hook builds a RegExp ---
const searchTrace: string[] = [];
const searcher: any = {
  [Symbol.search](str: any) {
    searchTrace.push("this=" + (this === searcher) + ",arg=" + typeof str + ":" + String(str) + ",count=" + arguments.length);
    return 7;
  },
};
console.log("search_hook_result=" + "abcdef".search(searcher));
console.log("search_trace=" + searchTrace.join("|"));
console.log("search_hook_type=" + typeof "abc".search(searcher));
console.log("search_hook_string_return=" + "abc".search({ [Symbol.search]() { return "not a number"; } } as any));
console.log("search_hook_object_return=" + typeof "abc".search({ [Symbol.search]() { return { a: 1 }; } } as any));
console.log("search_hook_undefined_return=" + String("abc".search({ [Symbol.search]() { return undefined; } } as any)));
searchTrace.length = 0;
String.prototype.search.call(9876 as any, searcher);
console.log("search_receiver_uncoerced=" + searchTrace.join("|"));
console.log("search_string_arg=" + "abcdef".search("cd"));
console.log("search_regexp_arg=" + "abcdef".search(/e/));
console.log("search_missing=" + "abcdef".search(/zz/));
console.log("search_no_argument=" + "abcdef".search());
console.log("search_undefined_arg=" + "abcdef".search(undefined as any));
console.log("search_null_arg=" + "aXnullY".search(null as any));
console.log("search_number_arg=" + "ab3cd".search(3 as any));
const gre = /b/g;
gre.lastIndex = 4;
console.log("search_ignores_lastIndex=" + "abab".search(gre) + ":" + gre.lastIndex);
function withSearchHook(v: any): string {
  const o: any = { toString() { return "c"; } };
  o[Symbol.search] = v;
  try { return String("abc".search(o)); }
  catch (e: any) { return e.constructor.name; }
}
console.log("search_hook_undefined=" + withSearchHook(undefined));
console.log("search_hook_null=" + withSearchHook(null));
console.log("search_hook_number=" + withSearchHook(1));
console.log("search_hook_object=" + withSearchHook({}));

// --- Symbol.matchAll: delegates like search, and a regexp-like argument must be global ---
const allTrace: string[] = [];
const allHook: any = {
  [Symbol.matchAll](str: any) {
    allTrace.push("this=" + (this === allHook) + ",arg=" + String(str));
    return ["one", "two"];
  },
};
const allResult: any = "abc".matchAll(allHook);
console.log("matchAll_trace=" + allTrace.join("|"));
console.log("matchAll_returns_verbatim=" + Array.isArray(allResult) + ":" + allResult.join(","));
console.log("matchAll_non_iterable_return=" + String("abc".matchAll({ [Symbol.matchAll]() { return 5; } } as any)));
const nonGlobalMatchAll: any = { [Symbol.match]: true, flags: "i", [Symbol.matchAll]() { return "NEVER"; } };
bad("matchAll_non_global_flag", () => "abc".matchAll(nonGlobalMatchAll));
const globalMatchAll: any = { [Symbol.match]: true, flags: "gi", [Symbol.matchAll]() { return "OK"; } };
console.log("matchAll_global_flag=" + "abc".matchAll(globalMatchAll));
const noBrandMatchAll: any = { [Symbol.matchAll]() { return "NOBRAND"; } };
console.log("matchAll_no_brand=" + "abc".matchAll(noBrandMatchAll));
const missingFlagsMatchAll: any = { [Symbol.match]: true, [Symbol.matchAll]() { return "NOFLAGS"; } };
bad("matchAll_missing_flags", () => "abc".matchAll(missingFlagsMatchAll));
bad("matchAll_regexp_non_global", () => "aXbX".matchAll(/X/));
const fromString = [..."aXbXc".matchAll("X" as any)];
console.log("matchAll_string_count=" + fromString.length);
console.log("matchAll_string_indices=" + fromString.map((m: any) => m.index).join(","));
console.log("matchAll_string_match=" + fromString.map((m: any) => m[0]).join(","));
console.log("matchAll_undefined_arg_count=" + [..."ab".matchAll(undefined as any)].length);
const it: any = "aXbX".matchAll(/X/g);
console.log("matchAll_iterator_tag=" + Object.prototype.toString.call(it));
console.log("matchAll_self_iterable=" + (it[Symbol.iterator]() === it));
console.log("matchAll_first=" + it.next().value[0]);
console.log("matchAll_rest=" + [...it].length);
console.log("matchAll_exhausted=" + JSON.stringify(it.next()));
const src = /X/g;
src.lastIndex = 0;
const consumed = [..."aXbX".matchAll(src)];
console.log("matchAll_source_lastIndex=" + src.lastIndex + ":" + consumed.length);
function hookShape(key: any, label: string): void {
  const d: any = Object.getOwnPropertyDescriptor(RegExp.prototype, key);
  console.log(label + "=" + typeof d.value + ":" + d.value.name + ":" + d.value.length + ":" + d.writable + d.enumerable + d.configurable);
}
hookShape(Symbol.search, "proto_search");
hookShape(Symbol.matchAll, "proto_matchAll");
hookShape(Symbol.match, "proto_match");
hookShape(Symbol.split, "proto_split");
console.log("string_arities=" + String.prototype.search.length + ":" + String.prototype.matchAll.length + ":" + String.prototype.match.length);
const shadow: any = /X/g;
shadow[Symbol.search] = function () { return -99; };
shadow[Symbol.matchAll] = function () { return "SHADOW"; };
console.log("shadow_search=" + "aXb".search(shadow));
console.log("shadow_matchAll=" + "aXb".matchAll(shadow));

// --- Symbol.unscopables: the shape of Array.prototype[Symbol.unscopables] ---
const unscop: any = (Array.prototype as any)[Symbol.unscopables];
const unscopDesc: any = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.unscopables);
console.log("unscopables_is_data=" + (unscopDesc.get === undefined && unscopDesc.set === undefined));
console.log("unscopables_flags=" + unscopDesc.writable + ":" + unscopDesc.enumerable + ":" + unscopDesc.configurable);
console.log("unscopables_typeof=" + typeof unscop);
console.log("unscopables_same_each_read=" + (unscop === (Array.prototype as any)[Symbol.unscopables]));
console.log("unscopables_proto=" + String(Object.getPrototypeOf(unscop)));
console.log("unscopables_has_toString=" + ("toString" in unscop));
console.log("unscopables_tag=" + Object.prototype.toString.call(unscop));
const unscopKeys = Object.keys(unscop);
console.log("unscopables_has_entries=" + (unscopKeys.length > 0));
console.log("unscopables_all_own=" + unscopKeys.every((k) => Object.prototype.hasOwnProperty.call(unscop, k)));
console.log("unscopables_all_true=" + unscopKeys.every((k) => unscop[k] === true));
let uniformFlags = "";
for (const k of unscopKeys) {
  const kd: any = Object.getOwnPropertyDescriptor(unscop, k);
  const f = String(kd.writable) + String(kd.enumerable) + String(kd.configurable);
  if (uniformFlags === "") uniformFlags = f;
  else if (uniformFlags !== f) uniformFlags = "MIXED";
}
console.log("unscopables_entry_flags=" + uniformFlags);
console.log("unscopables_no_symbol_keys=" + (Object.getOwnPropertySymbols(unscop).length === 0));
const addedAfterEs5 = [
  "at", "copyWithin", "entries", "fill", "find", "findIndex", "findLast",
  "findLastIndex", "flat", "flatMap", "includes", "keys", "toReversed",
  "toSorted", "toSpliced", "values",
];
let listed = "";
for (const n of addedAfterEs5) listed += n + ":" + (n in unscop) + " ";
console.log("unscopables_expected_present=" + listed.trim());
console.log("unscopables_expected_all_true=" + addedAfterEs5.every((n) => unscop[n] === true));
const es5Names = ["length", "push", "pop", "join", "map", "filter", "forEach", "slice", "splice", "concat", "indexOf", "reverse", "sort", "reduce", "constructor", "with"];
let absent = "";
for (const n of es5Names) absent += n + ":" + (n in unscop) + " ";
console.log("unscopables_es5_absent=" + absent.trim());
console.log("unscopables_all_are_methods=" + unscopKeys.every((k) => typeof (Array.prototype as any)[k] === "function"));
function carries(label: string, o: any): void {
  const has = Object.prototype.hasOwnProperty.call(o, Symbol.unscopables);
  const v = (o as any)[Symbol.unscopables];
  console.log("carries_" + label + "=" + has + ":" + (v === undefined ? "undefined" : typeof v));
}
carries("array_prototype", Array.prototype);
carries("object_prototype", Object.prototype);
carries("string_prototype", String.prototype);
carries("map_prototype", Map.prototype);
carries("set_prototype", Set.prototype);
carries("function_prototype", Function.prototype);
carries("number_prototype", Number.prototype);
carries("math", Math);
carries("json", JSON);
carries("array_instance", [1, 2]);
console.log("unscopables_instance_inherits=" + (([] as any)[Symbol.unscopables] === unscop));
class SubArr extends Array<number> {}
console.log("unscopables_subclass_inherits=" + ((SubArr.prototype as any)[Symbol.unscopables] === unscop));
console.log("unscopables_subclass_own=" + Object.prototype.hasOwnProperty.call(SubArr.prototype, Symbol.unscopables));
unscop.claude2Probe = true;
console.log("unscopables_added=" + (unscop.claude2Probe === true) + ":null_proto=" + (Object.getPrototypeOf(unscop) === null));
console.log("unscopables_delete_added=" + Reflect.deleteProperty(unscop, "claude2Probe") + ":" + ("claude2Probe" in unscop));
console.log("unscopables_count_restored=" + (Object.keys(unscop).length === unscopKeys.length));
const replacedUnscop = { fill: false };
console.log("unscopables_reflect_set=" + Reflect.set(Array.prototype, Symbol.unscopables, replacedUnscop));
console.log("unscopables_after_set=" + ((Array.prototype as any)[Symbol.unscopables] === replacedUnscop));
Object.defineProperty(Array.prototype, Symbol.unscopables, { value: unscop, writable: false, enumerable: false, configurable: true });
console.log("unscopables_restored=" + ((Array.prototype as any)[Symbol.unscopables] === unscop));

// --- Symbol.toStringTag: where the standard library writes it, and as what kind of property ---
function tagOf(label: string, o: any): void {
  if (o === undefined || o === null) { console.log(label + "=host_absent"); return; }
  const d: any = Object.getOwnPropertyDescriptor(o, Symbol.toStringTag);
  if (d === undefined) { console.log(label + "=absent:inherited=" + String((o as any)[Symbol.toStringTag])); return; }
  if (d.get !== undefined || d.set !== undefined) {
    console.log(label + "=accessor:get=" + typeof d.get + ",set=" + typeof d.set + "," + d.enumerable + "," + d.configurable);
    return;
  }
  console.log(label + "=data:" + JSON.stringify(d.value) + ":" + d.writable + "," + d.enumerable + "," + d.configurable);
}
tagOf("Math", Math);
tagOf("JSON", JSON);
tagOf("Reflect", Reflect);
tagOf("Atomics", Atomics);
tagOf("Map.prototype", Map.prototype);
tagOf("Set.prototype", Set.prototype);
tagOf("WeakMap.prototype", WeakMap.prototype);
tagOf("WeakSet.prototype", WeakSet.prototype);
tagOf("Symbol.prototype", Symbol.prototype);
tagOf("BigInt.prototype", BigInt.prototype);
tagOf("Promise.prototype", Promise.prototype);
tagOf("ArrayBuffer.prototype", ArrayBuffer.prototype);
tagOf("SharedArrayBuffer.prototype", typeof SharedArrayBuffer === "undefined" ? undefined : SharedArrayBuffer.prototype);
tagOf("DataView.prototype", DataView.prototype);
tagOf("WeakRef.prototype", WeakRef.prototype);
tagOf("FinalizationRegistry.prototype", FinalizationRegistry.prototype);
tagOf("Object.prototype", Object.prototype);
tagOf("Array.prototype", Array.prototype);
tagOf("Function.prototype", Function.prototype);
tagOf("String.prototype", String.prototype);
tagOf("Number.prototype", Number.prototype);
tagOf("Boolean.prototype", Boolean.prototype);
tagOf("Date.prototype", Date.prototype);
tagOf("RegExp.prototype", RegExp.prototype);
tagOf("Error.prototype", Error.prototype);
tagOf("map_iterator", Object.getPrototypeOf(new Map().entries()));
tagOf("set_iterator", Object.getPrototypeOf(new Set().values()));
tagOf("array_iterator", Object.getPrototypeOf([].values()));
tagOf("string_iterator", Object.getPrototypeOf(""[Symbol.iterator]()));
tagOf("regexp_string_iterator", Object.getPrototypeOf("a".matchAll(/a/g)));
tagOf("iterator_prototype", Iterator.prototype);
tagOf("iterator_helper", Object.getPrototypeOf((new Set([1]).values() as any).map((x: any) => x)));
function* gen(): any { yield 1; }
const genObj = gen();
tagOf("generator_prototype", Object.getPrototypeOf(Object.getPrototypeOf(genObj)));
tagOf("generator_function_proto", Object.getPrototypeOf(gen));
async function* asyncGen(): any { yield 1; }
tagOf("async_generator_function_proto", Object.getPrototypeOf(asyncGen));
async function asyncFn(): Promise<void> { /* shape only */ }
tagOf("async_function_proto", Object.getPrototypeOf(asyncFn));
const TypedArrayProto = Object.getPrototypeOf(Uint8Array.prototype);
tagOf("typedarray_abstract", TypedArrayProto);
tagOf("uint8_prototype", Uint8Array.prototype);
tagOf("float64_prototype", Float64Array.prototype);
const typedTagDesc: any = Object.getOwnPropertyDescriptor(TypedArrayProto, Symbol.toStringTag);
console.log("typedarray_getter=" + typedTagDesc.get.name + ":len=" + typedTagDesc.get.length);
console.log("typedarray_getter_on_u8=" + String(typedTagDesc.get.call(new Uint8Array(1))));
console.log("typedarray_getter_on_plain=" + String(typedTagDesc.get.call({})));
console.log("typedarray_getter_on_number=" + String(typedTagDesc.get.call(5)));
function printed(label: string, v: any): void {
  console.log("printed_" + label + "=" + Object.prototype.toString.call(v));
}
printed("math", Math);
printed("json", JSON);
printed("map", new Map());
printed("promise", Promise.resolve(1));
printed("symbol_primitive", Symbol("x"));
printed("bigint_primitive", 1n);
printed("dataview", new DataView(new ArrayBuffer(1)));
printed("uint8", new Uint8Array(1));
printed("generator_object", genObj);
printed("generator_function", gen);
printed("async_function", asyncFn);
printed("weakref", new WeakRef({}));
printed("map_iterator", new Map().entries());
printed("arguments", (function () { return arguments; })());
class MySet extends Set<number> {}
printed("subclass_default", new MySet());
class TaggedSet extends Set<number> {
  get [Symbol.toStringTag]() { return "TaggedSet"; }
}
printed("subclass_tagged", new TaggedSet());
tagOf("TaggedSet.prototype", TaggedSet.prototype);
const mathTag: any = Object.getOwnPropertyDescriptor(Math, Symbol.toStringTag);
console.log("retag_reflect_set=" + Reflect.set(Math, Symbol.toStringTag, "Nope"));
console.log("retag_unchanged=" + Object.prototype.toString.call(Math));
Object.defineProperty(Math, Symbol.toStringTag, { value: "Arithmetic", writable: false, enumerable: false, configurable: true });
console.log("retag_defined=" + Object.prototype.toString.call(Math));
Object.defineProperty(Math, Symbol.toStringTag, mathTag);
console.log("retag_restored=" + Object.prototype.toString.call(Math));

// --- Symbol.asyncIterator: consumed by for-await-of, with the sync fallback and return() on break ---
(async function main(): Promise<void> {
  const asyncIter: any = { async *[Symbol.asyncIterator]() { yield 1; yield 2; yield 3; } };
  const doubled: number[] = [];
  for await (const n of asyncIter) doubled.push(n * 2);
  console.log("async_iter=" + doubled.join(","));
  const syncFallback: any[] = [];
  for await (const v of [Promise.resolve("p"), "q"]) syncFallback.push(v);
  console.log("async_sync_fallback=" + syncFallback.join(","));
  const notCallable: any = { [Symbol.asyncIterator]: 5 };
  try {
    for await (const _x of notCallable) { /* never reached */ }
    console.log("async_not_callable=no_throw");
  } catch (e: any) {
    console.log("async_not_callable=" + e.constructor.name);
  }
  let closed = false;
  const closable: any = {
    [Symbol.asyncIterator]() {
      let i = 0;
      return {
        next: async () => ({ value: i++, done: false }),
        return: async () => { closed = true; return { done: true, value: undefined }; },
      };
    },
  };
  for await (const v of closable) { if (v === 2) break; }
  console.log("async_return_on_break=" + closed);
  console.log("async_generator_tag=" + Object.prototype.toString.call(asyncIter[Symbol.asyncIterator]()));
})();
