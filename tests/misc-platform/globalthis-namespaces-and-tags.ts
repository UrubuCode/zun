// Cross-runtime: the global object as an OBJECT, the three ECMAScript namespace
// objects (Math, JSON, Reflect, Atomics), Object.prototype.toString across the
// built-in zoo, and the identity graph between intrinsics. Every probe is
// mode-independent: refused writes are reported through Reflect.set, never by
// relying on a throw that only strict mode produces.

const g: any = globalThis;

function t(f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name;
  }
}

function shape(name: string): string {
  const d: any = Object.getOwnPropertyDescriptor(globalThis, name);
  if (!d) return name + "=<absent>";
  if (d.get || d.set) {
    return name + "=accessor e:" + d.enumerable + " c:" + d.configurable;
  }
  return name + "=data w:" + d.writable + " e:" + d.enumerable + " c:" + d.configurable;
}

function same(label: string, a: any, b: any): void {
  console.log(label + "=" + (a === b));
}

// ---------------------------------------------------------------------------
// 1. The global object: self reference, shape of its own properties.
// ---------------------------------------------------------------------------
console.log("self_reference=" + (g.globalThis === globalThis) + "/" + (g.globalThis.globalThis === globalThis));
console.log("type=" + typeof globalThis);
console.log("tag_is_object_like=" + (Object.prototype.toString.call(globalThis).indexOf("[object ") === 0));
console.log("extensible=" + Object.isExtensible(globalThis));
console.log("prototype_of_ctor_matches=" + (Object.getPrototypeOf(new Map()) === g.Map.prototype) + "/" + (Object.getPrototypeOf(g) !== null));

// Value properties: non-writable, non-enumerable, non-configurable.
shape("undefined");
shape("NaN");
shape("Infinity");
shape("globalThis");
// Function properties.
shape("parseInt");
shape("parseFloat");
shape("isNaN");
shape("isFinite");
shape("decodeURI");
shape("encodeURIComponent");
// Constructor and namespace bindings.
shape("Object");
shape("Array");
shape("Function");
shape("Symbol");
shape("Promise");
shape("Proxy");
shape("Reflect");
shape("Math");
shape("JSON");

console.log("ctor_desc=" + ["Map", "Promise", "WeakRef", "Proxy"].map(function (n) {
  const d: any = Object.getOwnPropertyDescriptor(globalThis, n);
  return n + ":w" + d.writable + "e" + d.enumerable + "c" + d.configurable;
}).join(" "));

console.log("value_props=" + ["undefined", "NaN", "Infinity"].map(function (n) {
  const d: any = Object.getOwnPropertyDescriptor(globalThis, n);
  return n + ":w" + d.writable + "e" + d.enumerable + "c" + d.configurable;
}).join(" "));
console.log("set_undefined=" + Reflect.set(globalThis, "undefined", 1) + " still=" + String(g.undefined));
console.log("set_nan=" + Reflect.set(globalThis, "NaN", 1) + " still=" + String(g.NaN));
console.log("set_infinity=" + Reflect.set(globalThis, "Infinity", 1) + " still=" + String(g.Infinity));
console.log("delete_value_prop=" + Reflect.deleteProperty(globalThis, "NaN") + " still=" + String(g.NaN));
console.log("define_over_value_prop=" + t(function () { return Object.defineProperty(globalThis, "undefined", { value: 1 }); }));

// Enumerability: none of the standard globals is enumerable.
const standardNames = ["Object", "Array", "Math", "JSON", "undefined", "NaN", "Infinity", "globalThis", "Promise"];
console.log("enumerable_standard=" + Object.keys(globalThis).filter(function (k) { return standardNames.indexOf(k) >= 0; }).length);
console.log("enumerable_flags=" + ["Object", "Math", "undefined", "NaN", "globalThis", "structuredClone"].map(function (n) {
  const d: any = Object.getOwnPropertyDescriptor(globalThis, n);
  return String(d.enumerable);
}).join(","));

// A property added by assignment is an ordinary one; one added by defineProperty
// keeps exactly the attributes it was given.
console.log("added_by_assignment=" + (function (): string {
  g.__probeA = 7;
  const d: any = Object.getOwnPropertyDescriptor(globalThis, "__probeA");
  const out = d.value + "/w:" + d.writable + " e:" + d.enumerable + " c:" + d.configurable;
  delete g.__probeA;
  return out + "/gone:" + !("__probeA" in globalThis);
})());
console.log("added_by_define=" + (function (): string {
  Object.defineProperty(globalThis, "__probeB", { value: 1, configurable: true });
  const d: any = Object.getOwnPropertyDescriptor(globalThis, "__probeB");
  const out = "w:" + d.writable + " e:" + d.enumerable + " c:" + d.configurable;
  delete g.__probeB;
  return out;
})());
console.log("added_accessor=" + (function (): string {
  let stored = 0;
  Object.defineProperty(globalThis, "__probeC", {
    get: function () { return stored + 1; },
    set: function (v: number) { stored = v * 2; },
    configurable: true,
  });
  g.__probeC = 5;
  const seen = g.__probeC;
  delete g.__probeC;
  return seen + "/" + stored;
})());
console.log("keys_see_user_property=" + (function (): string {
  g.__probeD = 1;
  const inKeys = Object.keys(globalThis).indexOf("__probeD") >= 0;
  let inForIn = false;
  for (const k in globalThis) {
    if (k === "__probeD") inForIn = true;
  }
  delete g.__probeD;
  return inKeys + "/" + inForIn;
})());
(globalThis as any).__probeE = 1;
console.log(shape("__probeE"));
console.log("delete_probe=" + Reflect.deleteProperty(globalThis, "__probeE") + " gone=" + !("__probeE" in globalThis));

// A constructor binding is writable and configurable: it can be replaced and restored.
console.log("replace_and_restore=" + (function (): string {
  const saved = g.WeakRef;
  g.WeakRef = 1;
  const replaced = typeof g.WeakRef;
  g.WeakRef = saved;
  return replaced + "/" + typeof g.WeakRef + "/" + (g.WeakRef === saved);
})());
console.log("delete_and_restore=" + (function (): string {
  const saved = g.WeakRef;
  const deleted = delete g.WeakRef;
  const gone = "WeakRef" in globalThis;
  g.WeakRef = saved;
  return deleted + "/" + gone + "/" + typeof g.WeakRef;
})());
console.log("shadow_does_not_touch_instances=" + (function (): string {
  const savedMap = g.Map;
  const instance = new Map([["k", 1]]);
  g.Map = null;
  const stillWorks = instance.get("k");
  g.Map = savedMap;
  return String(stillWorks) + "/" + (instance instanceof Map);
})());

// Global names and their identity through two routes.
console.log("identity_ctors=" + [
  ["Object", Object], ["Array", Array], ["Function", Function], ["Promise", Promise], ["Map", Map],
  ["Set", Set], ["Symbol", Symbol], ["Proxy", Proxy], ["Reflect", Reflect], ["Math", Math],
  ["JSON", JSON], ["DOMException", DOMException], ["TextEncoder", TextEncoder],
].map(function (pair: any) {
  return String(g[pair[0]] === pair[1]);
}).join(","));
console.log("identity_functions=" + [
  ["structuredClone", structuredClone], ["queueMicrotask", queueMicrotask], ["setTimeout", setTimeout],
  ["btoa", btoa], ["encodeURIComponent", encodeURIComponent], ["parseInt", parseInt], ["isNaN", isNaN],
].map(function (pair: any) {
  return String(g[pair[0]] === pair[1]);
}).join(","));
console.log("in_operator=" + ["undefined", "NaN", "Infinity", "globalThis", "Math", "Reflect", "Atomics", "structuredClone", "DOMException"].map(function (n) {
  return String(n in globalThis);
}).join(","));
console.log("missing_name=" + ("__definitely_absent" in globalThis) + "/" + typeof g.__definitely_absent + "/" + String(g.__definitely_absent));

// ---------------------------------------------------------------------------
// 2. Namespace objects: Math, JSON, Reflect, Atomics are ordinary objects, not
//    constructors; each carries a non-writable configurable toStringTag.
// ---------------------------------------------------------------------------
const namespaces: [string, any][] = [["Math", Math], ["JSON", JSON], ["Reflect", Reflect], ["Atomics", Atomics]];

for (const pair of namespaces) {
  const ns: any = pair[1];
  const d: any = Object.getOwnPropertyDescriptor(ns, Symbol.toStringTag);
  console.log("tag_" + pair[0] + "=" + ns[Symbol.toStringTag] + " w:" + d.writable + " e:" + d.enumerable + " c:" + d.configurable);
}
for (const pair of namespaces) {
  const ns: any = pair[1];
  console.log("shape_" + pair[0] + "=type:" + typeof ns + " proto_is_object:" + (Object.getPrototypeOf(ns) === Object.prototype) + " has_prototype:" + ("prototype" in ns) + " enumerable_keys:" + Object.keys(ns).length + " extensible:" + Object.isExtensible(ns));
}
for (const pair of namespaces) {
  const ns: any = pair[1];
  console.log("not_callable_" + pair[0] + "=" + t(function () { return ns(); }) + " not_constructible:" + t(function () { return new ns(); }));
}
for (const pair of namespaces) {
  const d: any = Object.getOwnPropertyDescriptor(globalThis, pair[0]);
  console.log("global_" + pair[0] + "=w:" + d.writable + " e:" + d.enumerable + " c:" + d.configurable + " same:" + (d.value === pair[1]));
}

console.log("math_constants=" + ["PI", "E", "LN2", "LN10", "LOG2E", "LOG10E", "SQRT2", "SQRT1_2"].map(function (k) {
  const d: any = Object.getOwnPropertyDescriptor(Math, k);
  return k + ":" + typeof d.value + d.writable + d.enumerable + d.configurable;
}).join(" "));
console.log("math_pi=" + Math.PI + " sqrt2=" + Math.SQRT2);
console.log("math_fn_lengths=" + ["abs", "max", "min", "hypot", "atan2", "pow", "round", "clz32", "imul", "fround"].map(function (k) {
  const f: any = (Math as any)[k];
  return k + ":" + String(f.length);
}).join(" "));
console.log("math_no_this=" + t(function () { const abs = Math.abs; return abs(-3); }));
console.log("math_fn_not_constructible=" + t(function () { return new (Math.abs as any)(1); }));

console.log("json_members=" + ["parse", "stringify"].map(function (k) { return k + ":" + typeof (JSON as any)[k]; }).join(" "));
console.log("json_lengths=" + JSON.parse.length + "," + JSON.stringify.length);
console.log("json_detached=" + t(function () { const stringify = JSON.stringify; return stringify({ a: 1 }); }));

const reflectOps = ["apply", "construct", "defineProperty", "deleteProperty", "get", "getOwnPropertyDescriptor", "getPrototypeOf", "has", "isExtensible", "ownKeys", "preventExtensions", "set", "setPrototypeOf"];
console.log("reflect_arity=" + reflectOps.map(function (k) { return String((Reflect as any)[k].length); }).join(","));
console.log("reflect_all_functions=" + reflectOps.every(function (k) { return typeof (Reflect as any)[k] === "function"; }));
console.log("reflect_apply_is_not_fn_apply=" + (Reflect.apply === Function.prototype.apply));
console.log("reflect_get_receiver=" + t(function () {
  const o: any = { a: 1, get b() { return this.a; } };
  return Reflect.get(o, "b", { a: 99 });
}));

console.log("atomics_members=" + ["add", "and", "compareExchange", "exchange", "isLockFree", "load", "notify", "or", "store", "sub", "wait", "xor"].map(function (k) {
  return typeof (Atomics as any)[k];
}).join(","));
console.log("atomics_arity=" + Atomics.load.length + "," + Atomics.store.length + "," + Atomics.compareExchange.length);

// Tag protection: writes are refused, defineProperty can replace it.
// A namespace binding on the global object can be shadowed and restored, while
// the namespace object itself is untouched.
console.log("global_binding_writable=" + (function (): string {
  const saved = (globalThis as any).Reflect;
  (globalThis as any).Reflect = 1;
  const changed = typeof (globalThis as any).Reflect;
  (globalThis as any).Reflect = saved;
  return changed + "/" + typeof (globalThis as any).Reflect + "/" + ((globalThis as any).Reflect === saved);
})());
console.log("tag_is_not_writable=" + t(function () {
  return Reflect.set(Math, Symbol.toStringTag, "Nope") + "/" + (Math as any)[Symbol.toStringTag];
}));
console.log("tag_is_configurable=" + t(function () {
  const saved: any = Object.getOwnPropertyDescriptor(Math, Symbol.toStringTag);
  Object.defineProperty(Math, Symbol.toStringTag, { value: "Changed", writable: false, enumerable: false, configurable: true });
  const seen = Object.prototype.toString.call(Math);
  Object.defineProperty(Math, Symbol.toStringTag, saved);
  return seen + "/" + Object.prototype.toString.call(Math);
}));

// ---------------------------------------------------------------------------
// 3. Object.prototype.toString across the built-in zoo: three mechanisms behind
//    one observable answer (internal slot, string tag, tag getter).
// ---------------------------------------------------------------------------
function tag(label: string, v: any): void {
  console.log(label + "=" + Object.prototype.toString.call(v));
}

// Slot-driven kinds: no toStringTag property at all.
tag("undefined", undefined);
tag("null", null);
tag("number", 1);
tag("string", "s");
tag("boolean", true);
tag("symbol", Symbol("x"));
tag("bigint", 1n);
tag("object", {});
tag("array", []);
tag("function", function () {});
tag("arrow", () => {});
tag("class", class {});
tag("error", new Error("m"));
tag("typeError", new TypeError("m"));
tag("date", new Date(0));
tag("regexp", /x/);
tag("arguments", (function () { return arguments; })());
tag("boxedNumber", new Number(1));
tag("boxedString", new String("s"));
tag("boxedBoolean", new Boolean(true));
tag("nullProto", Object.create(null));

// Tagged kinds: a string-valued Symbol.toStringTag on the prototype.
tag("map", new Map());
tag("set", new Set());
tag("weakMap", new WeakMap());
tag("weakSet", new WeakSet());
tag("promise", Promise.resolve());
tag("symbolWrapper", Object(Symbol("x")));
tag("bigintWrapper", Object(1n));
tag("generatorFn", function* () {});
tag("generatorObj", (function* () {})());
tag("asyncFn", async function () {});
tag("asyncGenFn", async function* () {});
tag("math", Math);
tag("json", JSON);
tag("reflect", Reflect);
tag("arrayIterator", [].values());
tag("stringIterator", ""[Symbol.iterator]());
tag("mapIterator", new Map().values());
tag("setIterator", new Set().values());
tag("regexpStringIterator", "a".matchAll(/a/g));
tag("weakRef", new WeakRef({}));
tag("finalizationRegistry", new FinalizationRegistry(() => {}));

// Getter-driven kinds: %TypedArray% and DataView answer from an accessor.
tag("uint8", new Uint8Array(1));
tag("dataView", new DataView(new ArrayBuffer(1)));
tag("arrayBuffer", new ArrayBuffer(1));
const taTagDesc: any = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), Symbol.toStringTag);
console.log("typedArrayTagIsGetter=" + (taTagDesc ? typeof taTagDesc.get : "absent"));
console.log("typedArrayTagWrongReceiver=" + String(taTagDesc && taTagDesc.get ? taTagDesc.get.call({}) : "n/a"));

// User tags: plain object, class getter, inherited, non-string value, throwing getter.
const plain: any = {};
plain[Symbol.toStringTag] = "Custom";
tag("userTag", plain);
class Tagged { get [Symbol.toStringTag]() { return "Tagged"; } }
tag("classGetterTag", new Tagged());
tag("inheritedTag", Object.create(plain));
const numericTag: any = {};
numericTag[Symbol.toStringTag] = 42;
tag("nonStringTag", numericTag);
const throwing: any = {};
Object.defineProperty(throwing, Symbol.toStringTag, { get() { throw new RangeError("boom"); } });
try {
  Object.prototype.toString.call(throwing);
  console.log("throwingTag=no-throw");
} catch (e: any) {
  console.log("throwingTag=" + e.constructor.name);
}

// An own Symbol.toStringTag wins over the built-in answer, for array, Map, Date
// and Promise alike. Map.prototype's tag is non-writable, so assignment is refused.
function own(o: any, v: string): any {
  Object.defineProperty(o, Symbol.toStringTag, { value: v, configurable: true });
  return o;
}
console.log("mapProtoTagWritable=" + Object.getOwnPropertyDescriptor(Map.prototype, Symbol.toStringTag)!.writable);
console.log("assignRefused=" + Reflect.set(new Map(), Symbol.toStringTag, "NotMap"));
tag("arrayWithTag", own([], "NotArray"));
tag("mapWithTag", own(new Map(), "NotMap"));
tag("dateWithTag", own(new Date(0), "NotDate"));
tag("promiseWithTag", own(Promise.resolve(), "NotPromise"));

// A Proxy reports its target's kind for the slot-driven cases.
tag("proxyArray", new Proxy([], {}));
tag("proxyFn", new Proxy(function () {}, {}));
tag("proxyPlain", new Proxy({}, {}));

// ---------------------------------------------------------------------------
// 4. Identity graph: objects reachable by two routes are the SAME object, and
//    look-alikes are distinct.
// ---------------------------------------------------------------------------
const arrayIterProto = Object.getPrototypeOf(Object.getPrototypeOf([].values()));
const stringIterProto = Object.getPrototypeOf(Object.getPrototypeOf(""[Symbol.iterator]()));
const mapIterProto = Object.getPrototypeOf(Object.getPrototypeOf(new Map().values()));
const setIterProto = Object.getPrototypeOf(Object.getPrototypeOf(new Set().values()));
same("iterProto_array_string", arrayIterProto, stringIterProto);
same("iterProto_array_map", arrayIterProto, mapIterProto);
same("iterProto_array_set", arrayIterProto, setIterProto);
console.log("iterProtoHasSelfIterator=" + (typeof (arrayIterProto as any)[Symbol.iterator] === "function"));
console.log("iterProtoParentIsObject=" + (Object.getPrototypeOf(arrayIterProto) === Object.prototype));
same("mapValuesKeys", Object.getPrototypeOf(new Map().values()), Object.getPrototypeOf(new Map().keys()));
same("setValuesKeys", Object.getPrototypeOf(new Set().values()), Object.getPrototypeOf(new Set().keys()));
same("setKeysIsValues", Set.prototype.keys, Set.prototype.values);
same("setIteratorIsValues", (Set.prototype as any)[Symbol.iterator], Set.prototype.values);
same("mapIteratorIsEntries", (Map.prototype as any)[Symbol.iterator], Map.prototype.entries);
same("arrayIteratorIsValues", (Array.prototype as any)[Symbol.iterator], Array.prototype.values);

const taProto = Object.getPrototypeOf(Uint8Array.prototype);
same("int8SharesTA", Object.getPrototypeOf(Int8Array.prototype), taProto);
same("float64SharesTA", Object.getPrototypeOf(Float64Array.prototype), taProto);
same("bigint64SharesTA", Object.getPrototypeOf(BigInt64Array.prototype), taProto);
const taCtor = Object.getPrototypeOf(Uint8Array);
same("ctorsShareTA", Object.getPrototypeOf(Float32Array), taCtor);
console.log("taCtorNotFunctionProto=" + (taCtor !== Function.prototype));
console.log("taCtorNotConstructable=" + t(function () { return new (taCtor as any)(1); }));

function* genDecl() {}
async function* asyncGenDecl() {}
async function asyncDecl() {}
const genFnProto = Object.getPrototypeOf(genDecl);
same("genFnProtoShared", Object.getPrototypeOf(function* () {}), genFnProto);
console.log("genFnProtoParentIsFunctionProto=" + (Object.getPrototypeOf(genFnProto) === Function.prototype));
same("genObjProtoFromFn", Object.getPrototypeOf(genDecl()), genDecl.prototype);
console.log("genProtoParentShared=" + (Object.getPrototypeOf(Object.getPrototypeOf(genDecl())) === (genFnProto as any).prototype));
same("asyncFnProtoShared", Object.getPrototypeOf(asyncDecl), Object.getPrototypeOf(async function () {}));
console.log("asyncGenDistinct=" + (Object.getPrototypeOf(asyncGenDecl) !== genFnProto));

class C {}
const classProtoDesc: any = Object.getOwnPropertyDescriptor(C, "prototype");
const fnProtoDesc: any = Object.getOwnPropertyDescriptor(function () {}, "prototype");
console.log("classProto=" + classProtoDesc.writable + "," + classProtoDesc.enumerable + "," + classProtoDesc.configurable);
console.log("fnProto=" + fnProtoDesc.writable + "," + fnProtoDesc.enumerable + "," + fnProtoDesc.configurable);
console.log("arrowHasNoProto=" + (Object.getOwnPropertyDescriptor(() => {}, "prototype") === undefined));
console.log("methodHasNoProto=" + (Object.getOwnPropertyDescriptor({ m() {} }.m, "prototype") === undefined));
console.log("genFnHasProto=" + (Object.getOwnPropertyDescriptor(genDecl, "prototype") !== undefined));

for (const E of [TypeError, RangeError, SyntaxError, ReferenceError, EvalError, URIError]) {
  console.log(E.name + "=" + (Object.getPrototypeOf(E.prototype) === Error.prototype) + "," + (Object.getPrototypeOf(E) === Error));
}
console.log("aggregateChain=" + (Object.getPrototypeOf(AggregateError.prototype) === Error.prototype));

same("objProto", Object.getPrototypeOf({}), Object.prototype);
same("fnProtoRoot", Object.getPrototypeOf(Function.prototype), Object.prototype);
same("objCtorIsFunction", Object.getPrototypeOf(Object), Function.prototype);
console.log("fnProtoIsCallable=" + (typeof Function.prototype === "function") + " returns=" + String((Function.prototype as any)()));
