export {};
// Proxy trap surface: which trap fires for which operation, in what order, with
// which arguments, how the handler itself is looked up, and what Reflect
// forwards back to the target when a trap delegates.

function show(label: string, value: unknown): void {
  console.log(label + "=" + String(value));
}

function attempt(label: string, fn: () => unknown): void {
  try {
    show(label, fn());
  } catch (e) {
    console.log(label + "=throw:" + (e as Error).constructor.name);
  }
}

// ---------------------------------------------------------------------------
// 1. get / set / has / deleteProperty with delegation through Reflect
// ---------------------------------------------------------------------------
const basicLog: string[] = [];
const basicTarget: any = { a: 1, b: 2 };
const basic: any = new Proxy(basicTarget, {
  get(t, key, r) { basicLog.push("get:" + String(key)); return Reflect.get(t, key, r); },
  set(t, key, val, r) { basicLog.push("set:" + String(key) + "=" + String(val)); return Reflect.set(t, key, val, r); },
  has(t, key) { basicLog.push("has:" + String(key)); return Reflect.has(t, key); },
  deleteProperty(t, key) { basicLog.push("del:" + String(key)); return Reflect.deleteProperty(t, key); },
});

show("basic_read", basic.a);
basic.b = 99;
show("basic_has", "b" in basic);
show("basic_delete", Reflect.deleteProperty(basic, "b"));
show("basic_target_keys", Object.keys(basicTarget).join(","));
show("basic_trace", basicLog.join("|"));

// a set trap that validates: the throw propagates out of the assignment
const positive: any = new Proxy({} as any, {
  set(t, key, val) {
    if (typeof val !== "number" || val < 0) throw new RangeError("must be positive");
    t[key as string] = val;
    return true;
  },
});
positive.x = 5;
show("validator_x", positive.x);
attempt("validator_negative", () => { positive.y = -1; return "stored"; });
attempt("validator_string", () => Reflect.set(positive, "z", "text"));
show("validator_y_absent", "y" in positive);

// ---------------------------------------------------------------------------
// 2. Trap lookup: absent, undefined, null, non-callable, callable proxy, bound
// ---------------------------------------------------------------------------
const base: any = { a: 1 };
show("lookup_absent", (new Proxy(base, {}) as any).a);
show("lookup_undefined", (new Proxy(base, { get: undefined }) as any).a);
show("lookup_null", (new Proxy(base, { get: null as any }) as any).a);
show("lookup_built_with_number", typeof new Proxy(base, { get: 1 as any }));
attempt("lookup_use_number", () => (new Proxy(base, { get: 1 as any }) as any).a);
attempt("lookup_use_string", () => (new Proxy(base, { get: "x" as any }) as any).a);
attempt("lookup_use_object", () => (new Proxy(base, { get: {} as any }) as any).a);
attempt("lookup_use_symbol", () => (new Proxy(base, { get: Symbol("s") as any }) as any).a);
show("lookup_proxy_trap", (new Proxy(base, { get: new Proxy(function () { return "VIA_PROXY_TRAP"; }, {}) as any }) as any).a);
show("lookup_bound_trap", (new Proxy(base, {
  get: (function (this: any) { return "BOUND:" + this.tag; }).bind({ tag: "B" }) as any,
}) as any).a);
attempt("lookup_class_trap", () => (new Proxy(base, { get: class Ctor {} as any }) as any).a);

const nulled: any = new Proxy({ k: 1 }, {
  get: null as any, set: null as any, has: null as any, deleteProperty: null as any,
  ownKeys: null as any, getOwnPropertyDescriptor: null as any, defineProperty: null as any,
  getPrototypeOf: null as any, setPrototypeOf: null as any,
  isExtensible: null as any, preventExtensions: null as any,
});
show("nulled_get", nulled.k);
show("nulled_has", "k" in nulled);
show("nulled_set", Reflect.set(nulled, "k", 2) + ",v=" + nulled.k);
show("nulled_keys", Reflect.ownKeys(nulled).join("|"));
show("nulled_gopd", (Object.getOwnPropertyDescriptor(nulled, "k") as any).value);
show("nulled_define", Reflect.defineProperty(nulled, "n", { value: 3, configurable: true }));
show("nulled_delete", Reflect.deleteProperty(nulled, "n"));
show("nulled_proto_is_object_proto", Reflect.getPrototypeOf(nulled) === Object.prototype);
show("nulled_extensible", Reflect.isExtensible(nulled));

const mixed: any = new Proxy({ m: 1 }, { get: 5 as any, has() { return true; } });
show("mixed_has", "anything" in mixed);
attempt("mixed_get_not_callable", () => mixed.m);

// the handler is read with [[Get]] on every operation, so an accessor counts
let handlerReads = 0;
const dynamicHandler: any = {};
Object.defineProperty(dynamicHandler, "get", {
  get() { handlerReads++; return () => "call" + handlerReads; },
  configurable: true,
});
const dyn: any = new Proxy(base, dynamicHandler);
show("dyn_1", dyn.a);
show("dyn_2", dyn.a);
show("dyn_3", dyn.b);
show("dyn_handler_reads", handlerReads);

// a handler mutated between operations changes behaviour at once
const live: any = { get() { return "first"; } };
const liveProxy: any = new Proxy(base, live);
show("live_1", liveProxy.a);
live.get = function () { return "second"; };
show("live_2", liveProxy.a);
delete live.get;
show("live_3", liveProxy.a);
live.get = 7;
attempt("live_4_not_callable", () => liveProxy.a);

// ---------------------------------------------------------------------------
// 3. Which trap NAMES are looked up on the handler, per operation
// ---------------------------------------------------------------------------
const looked: string[] = [];

function sniff(target: any): any {
  return new Proxy(target, new Proxy({} as any, {
    get(_h, k) { looked.push(String(k)); return undefined; },
  }));
}

function traceLookups(label: string, fn: (p: any) => void): void {
  looked.length = 0;
  try {
    fn(sniff({ a: 1, b: 2 }));
    show(label, looked.join(","));
  } catch (e) {
    show(label, looked.join(",") + "|throw:" + (e as Error).constructor.name);
  }
}

traceLookups("lk_get", (p) => { void p.a; });
traceLookups("lk_set", (p) => { Reflect.set(p, "a", 9); });
traceLookups("lk_has", (p) => { void ("a" in p); });
traceLookups("lk_delete", (p) => { Reflect.deleteProperty(p, "a"); });
traceLookups("lk_ownKeys", (p) => { Reflect.ownKeys(p); });
traceLookups("lk_gopd", (p) => { Object.getOwnPropertyDescriptor(p, "a"); });
traceLookups("lk_define", (p) => { Reflect.defineProperty(p, "c", { value: 3, configurable: true }); });
traceLookups("lk_getProto", (p) => { Object.getPrototypeOf(p); });
traceLookups("lk_setProto", (p) => { Reflect.setPrototypeOf(p, null); });
traceLookups("lk_isExtensible", (p) => { Object.isExtensible(p); });
traceLookups("lk_preventExtensions", (p) => { Reflect.preventExtensions(p); });

// a callable target adds the two function traps
looked.length = 0;
const fnSniff: any = sniff(function f(x: number) { return x; });
fnSniff(1);
show("lk_apply", looked.join(","));
looked.length = 0;
const ctorSniff: any = sniff(function C(this: any) { (this as any).v = 1; });
new ctorSniff();
show("lk_construct", looked.join(","));

// traps found on the handler's PROTOTYPE work exactly like own ones
const protoHandler: any = {
  get(_t: any, k: any) { return "proto:" + String(k); },
  has() { return true; },
  ownKeys() { return ["fromProto"]; },
};
const inherited: any = new Proxy({ a: 1 }, Object.create(protoHandler));
show("inherited_get", inherited.a);
show("inherited_has", "zz" in inherited);
show("inherited_keys", Reflect.ownKeys(inherited).join("|"));

// an own trap shadows the inherited one, and deleting it uncovers it again
const shadowing: any = Object.create(protoHandler);
shadowing.get = function () { return "own"; };
const shadowed: any = new Proxy({ a: 1 }, shadowing);
show("shadow_own", shadowed.a);
delete shadowing.get;
show("shadow_uncovered", shadowed.a);

class Handler {
  get(_t: any, k: any): string { return "class:" + String(k); }
}
show("class_handler", (new Proxy({ a: 1 }, new Handler() as any) as any).a);
show("bare_null_proto_handler", (new Proxy({ a: 42 }, Object.create(null)) as any).a);

// ---------------------------------------------------------------------------
// 4. Trap sequence of meta-operations (ownKeys, gopd per key, get per key)
// ---------------------------------------------------------------------------
const seqLog: string[] = [];

function makeSeqProxy(): any {
  const target: any = { a: 1, b: 2 };
  Object.defineProperty(target, "hidden", { value: 3, enumerable: false, configurable: true, writable: true });
  return new Proxy(target, {
    ownKeys(t) { seqLog.push("ownKeys"); return Reflect.ownKeys(t); },
    getOwnPropertyDescriptor(t, k) { seqLog.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
    get(t, k, r) { seqLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
    has(t, k) { seqLog.push("has:" + String(k)); return Reflect.has(t, k); },
    set(t, k, v, r) { seqLog.push("set:" + String(k)); return Reflect.set(t, k, v, r); },
    defineProperty(t, k, d) { seqLog.push("define:" + String(k)); return Reflect.defineProperty(t, k, d); },
    deleteProperty(t, k) { seqLog.push("delete:" + String(k)); return Reflect.deleteProperty(t, k); },
    getPrototypeOf(t) { seqLog.push("getProto"); return Reflect.getPrototypeOf(t); },
  });
}

function seq(label: string, fn: (p: any) => void): void {
  seqLog.length = 0;
  fn(makeSeqProxy());
  show(label, seqLog.join(","));
}

seq("seq_ownKeys", (p) => { Reflect.ownKeys(p); });
seq("seq_keys", (p) => { Object.keys(p); });
seq("seq_values", (p) => { Object.values(p); });
seq("seq_entries", (p) => { Object.entries(p); });
seq("seq_assign_from", (p) => { Object.assign({}, p); });
seq("seq_spread", (p) => { const _x = { ...p }; void _x; });
seq("seq_json", (p) => { JSON.stringify(p); });
seq("seq_gopds", (p) => { Object.getOwnPropertyDescriptors(p); });
seq("seq_in", (p) => { void ("a" in p); });
seq("seq_dot_read", (p) => { void p.a; });
seq("seq_dot_write", (p) => { p.c = 3; });
seq("seq_define", (p) => { Object.defineProperty(p, "d", { value: 4, configurable: true }); });
seq("seq_delete", (p) => { delete p.a; });
seq("seq_hasOwn", (p) => { Object.hasOwn(p, "a"); });
seq("seq_instanceof", (p) => { void (p instanceof Object); });

// ownKeys may invent a key: no descriptor means it is skipped, and no get runs
seqLog.length = 0;
const invented: any = new Proxy({} as any, {
  ownKeys() { seqLog.push("ownKeys"); return ["ghost", "real"]; },
  getOwnPropertyDescriptor(_t, k) {
    seqLog.push("gopd:" + String(k));
    return String(k) === "real" ? { value: 1, enumerable: true, configurable: true, writable: true } : undefined;
  },
  get(_t, k) { seqLog.push("get:" + String(k)); return "v"; },
});
const inventedSpread: any = { ...invented };
show("invented_trace", seqLog.join(","));
show("invented_keys", Object.keys(inventedSpread).join("|"));

// a non-enumerable descriptor stops before the get
seqLog.length = 0;
const nonEnum: any = new Proxy({} as any, {
  ownKeys() { seqLog.push("ownKeys"); return ["ne", "e"]; },
  getOwnPropertyDescriptor(_t, k) {
    seqLog.push("gopd:" + String(k));
    return { value: 1, enumerable: String(k) === "e", configurable: true, writable: true };
  },
  get(_t, k) { seqLog.push("get:" + String(k)); return "v"; },
});
show("nonenum_keys", Object.keys(nonEnum).join("|"));
show("nonenum_trace", seqLog.join(","));

// symbols reported by ownKeys are skipped by the string-only operations
seqLog.length = 0;
const sym = Symbol("s");
const withSym: any = new Proxy({ a: 1, [sym]: 2 } as any, {
  ownKeys(t) { seqLog.push("ownKeys"); return Reflect.ownKeys(t); },
  getOwnPropertyDescriptor(t, k) { seqLog.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
  get(t, k, r) { seqLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
});
Object.keys(withSym);
show("sym_keys_trace", seqLog.join(","));
seqLog.length = 0;
const symSpread: any = { ...withSym };
show("sym_spread_trace", seqLog.join(",") + "|copied=" + String(symSpread[sym]));

// ---------------------------------------------------------------------------
// 5. Own-key and descriptor virtualisation (ownKeys + gopd + get)
// ---------------------------------------------------------------------------
const secret: any = { public1: 1, public2: 2, _private: 3, __internal: 4 };
const filtered: any = new Proxy(secret, {
  ownKeys(t) { return Object.keys(t).filter((k) => !k.startsWith("_")); },
  getOwnPropertyDescriptor(t, key) {
    if (String(key).startsWith("_")) return undefined;
    return Object.getOwnPropertyDescriptor(t, key);
  },
});
show("filtered_keys", Object.keys(filtered).join(","));
show("filtered_names", Object.getOwnPropertyNames(filtered).join(","));
show("filtered_has_private", "_private" in filtered);

const virtual: any = new Proxy({} as any, {
  getOwnPropertyDescriptor(_t, k) {
    return { value: String(k).toUpperCase(), writable: true, enumerable: true, configurable: true };
  },
  ownKeys() { return ["foo", "bar"]; },
  get(_t, k) { return k === "foo" ? "FOO" : undefined; },
});
show("virtual_keys", Object.keys(virtual).join(","));
show("virtual_foo", virtual.foo);
show("virtual_json", JSON.stringify(virtual));

// an ownKeys order is the trap's order, not the target's, and filtering by
// enumerability uses the gopd trap
const reordered: any = new Proxy({ a: 1, b: 2, c: 3 }, {
  ownKeys() { return ["c", "hidden", "a", "b"]; },
  getOwnPropertyDescriptor(t, key) {
    if (key === "hidden") return { value: 9, enumerable: false, configurable: true };
    if (key === "b") return { value: 2, enumerable: false, configurable: true };
    return Reflect.getOwnPropertyDescriptor(t, key);
  },
});
show("reorder_ownKeys", Reflect.ownKeys(reordered).join(","));
show("reorder_keys", Object.keys(reordered).join(","));
show("reorder_spread", JSON.stringify({ ...reordered }));

// a has trap answers `in` for keys the target never had
const range: any = new Proxy({} as any, {
  has(_t, key) { const n = Number(key); return Number.isInteger(n) && n >= 0 && n < 10; },
  get(_t, key) { const n = Number(key); return n * n; },
});
show("range_in_5", 5 in range);
show("range_in_15", 15 in range);
show("range_7", range[7]);

// ---------------------------------------------------------------------------
// 6. defineProperty trap: argument shape, coercion, and refusal
// ---------------------------------------------------------------------------
let seenDesc: any = null;
const defSpy: any = new Proxy({} as any, {
  defineProperty(t, k, d) { seenDesc = d; return Reflect.defineProperty(t, k, d); },
});
Reflect.defineProperty(defSpy, "a", { value: 1 });
show("desc_value_only", Reflect.ownKeys(seenDesc).join("|"));
Reflect.defineProperty(defSpy, "b", { value: 2, writable: true, enumerable: true, configurable: true });
show("desc_full", Reflect.ownKeys(seenDesc).sort().join("|"));
Reflect.defineProperty(defSpy, "c", { get() { return 3; }, configurable: true });
show("desc_accessor", Reflect.ownKeys(seenDesc).sort().join("|"));
Reflect.defineProperty(defSpy, "d", { enumerable: false });
show("desc_lonely", Reflect.ownKeys(seenDesc).join("|"));
show("desc_plain_proto", Object.getPrototypeOf(seenDesc) === Object.prototype);
Reflect.defineProperty(defSpy, "e", { value: 5, enumerable: 1 as any, configurable: "" as any });
show("desc_coerced", "e=" + seenDesc.enumerable + ",c=" + seenDesc.configurable);

const assignSeen: string[] = [];
const assignProxy: any = new Proxy({} as any, {
  defineProperty(t, k, d) {
    assignSeen.push(String(k) + ":" + Reflect.ownKeys(d).sort().join("+"));
    return Reflect.defineProperty(t, k, d);
  },
});
Reflect.set(assignProxy, "fresh", 1);
show("assign_routes_to_define", assignSeen.join(","));

const refusedTarget: any = {};
const refuses: any = new Proxy(refusedTarget, { defineProperty() { return false; } });
show("define_refused_reflect", Reflect.defineProperty(refuses, "x", { value: 1 }));
attempt("define_refused_object", () => { Object.defineProperty(refuses, "x", { value: 1 }); return "ok"; });
show("define_refused_target_keys", Object.getOwnPropertyNames(refusedTarget).length);
const truthyReport: any = new Proxy({} as any, { defineProperty(t, k, d) { Reflect.defineProperty(t, k, d); return 1 as any; } });
show("define_truthy_report", Reflect.defineProperty(truthyReport, "y", { value: 2, configurable: true }));
const emptyReport: any = new Proxy({} as any, { defineProperty(t, k, d) { Reflect.defineProperty(t, k, d); return "" as any; } });
show("define_falsy_report", Reflect.defineProperty(emptyReport, "y", { value: 2, configurable: true }));

// ---------------------------------------------------------------------------
// 7. Set / deleteProperty / has trap reports, via Reflect and assignment
// ---------------------------------------------------------------------------
const setRefusedTarget: any = { a: 1 };
const setRefuses: any = new Proxy(setRefusedTarget, { set() { return false; } });
show("set_refused_reflect", Reflect.set(setRefuses, "a", 2));
show("set_refused_target", setRefusedTarget.a);
attempt("set_refused_assign_strict", () => { setRefuses.a = 3; return "ok"; });
attempt("set_refused_assign_object", () => { Object.assign(setRefuses, { a: 4 }); return "ok"; });
const truthySet: any = new Proxy({ a: 1 }, { set() { return "yes" as any; } });
show("set_truthy_report", Reflect.set(truthySet, "a", 2));
const zeroSet: any = new Proxy({ a: 1 }, { set() { return 0 as any; } });
show("set_zero_report", Reflect.set(zeroSet, "a", 2));
const idleTarget: any = { a: 1 };
const idleSet: any = new Proxy(idleTarget, { set() { return true; } });
show("set_idle_reports", Reflect.set(idleSet, "a", 99));
show("set_idle_target", idleTarget.a);

const delRefused: any = new Proxy({ a: 1 }, { deleteProperty() { return false; } });
show("delete_refused_reflect", Reflect.deleteProperty(delRefused, "a"));
attempt("delete_refused_strict", () => { delete delRefused.a; return "deleted"; });
const hasHide: any = new Proxy({ a: 1 }, { has() { return false; } });
show("has_hidden", "a" in hasHide);
show("has_hidden_read", hasHide.a);

// ---------------------------------------------------------------------------
// 8. Apply and construct basics (callable proxies)
// ---------------------------------------------------------------------------
function greet(name: string) { return "Hello, " + name; }
const pGreet: any = new Proxy(greet, {
  apply(target, thisArg, args) { return (target.apply(thisArg, args) as string).toUpperCase(); },
});
show("apply_upper", pGreet("world"));

class Point {
  x: number;
  y: number;
  constructor(x: number, y: number) { this.x = x; this.y = y; }
}
const PPoint: any = new Proxy(Point, {
  construct(Target, args) {
    console.log("construct_args=" + args.join(","));
    return new Target(args[0], args[1]);
  },
});
const pt: any = new PPoint(3, 4);
show("construct_point", pt.x + "," + pt.y);
show("construct_instanceof_class", pt instanceof Point);

function Sum(this: any, a: number, b: number) {
  if (new.target) this.value = a + b;
  return a + b;
}
const seenCalls: string[] = [];
const sumProxy: any = new Proxy(Sum, {
  apply(target, thisArg, args) { seenCalls.push("apply:" + args.join(",")); return Reflect.apply(target, thisArg, args) * 2; },
  construct(target, args, newTarget) {
    seenCalls.push("construct:" + args.join(",") + ":" + (newTarget === sumProxy));
    return Reflect.construct(target, args, newTarget);
  },
});
show("sum_call", sumProxy(2, 3));
const sumInst: any = new sumProxy(4, 5);
show("sum_new_value", sumInst.value);
show("sum_new_instanceof", (sumInst instanceof sumProxy) + ":" + (sumInst instanceof Sum));
show("sum_trace", seenCalls.join("|"));

// a construct trap sees a newTarget of the proxy for `new proxy`, and the
// trap can stamp the instance it built
function F(this: any, x: number) { this.x = x; return x + 1; }
const P: any = new Proxy(F, {
  apply(target) { return Reflect.apply(target, { x: 0 }, [0]) + 10; },
  construct(target, args, newTarget) {
    const obj: any = Reflect.construct(target, args, newTarget);
    obj.tag = newTarget === P ? "proxy" : "other";
    return obj;
  },
});
show("stamp_call", P(2));
const stamped: any = new P(5);
show("stamp_fields", stamped.x + ":" + stamped.tag + ":" + (stamped instanceof F) + ":" + (stamped instanceof P));

// the apply trap receives thisArg verbatim and args as a real Array
const callInfo: string[] = [];
const spy: any = new Proxy(Sum, {
  apply(_t, thisArg, args) {
    callInfo.push("this=" + (thisArg === null ? "null" : typeof thisArg) + ",array=" + Array.isArray(args) + ",n=" + args.length);
    return "A";
  },
});
show("spy_method_call", spy.call({ x: 1 }, 3));
show("spy_apply", spy.apply(null, [4, 5]));
show("spy_reflect_apply", Reflect.apply(spy, undefined, [6]));
show("spy_info", callInfo.join("|"));

// ---------------------------------------------------------------------------
// 9. Reflect namespace: forwarding, shape, and primitive refusal
// ---------------------------------------------------------------------------
show("reflect_typeof", typeof Reflect);
show("reflect_tag", Object.prototype.toString.call(Reflect));
show("reflect_proto_is_object", Object.getPrototypeOf(Reflect) === Object.prototype);
show("reflect_extensible", Object.isExtensible(Reflect));
attempt("reflect_call", () => (Reflect as any)());
attempt("reflect_construct_ns", () => new (Reflect as any)());

const reflectNames: string[] = Object.getOwnPropertyNames(Reflect).sort();
show("reflect_names", reflectNames.join("|"));
show("reflect_symbols", Object.getOwnPropertySymbols(Reflect).map(String).join("|"));
show("reflect_enumerable_count", Object.keys(Reflect).length);
show("reflect_tag_value", (Reflect as any)[Symbol.toStringTag]);
for (const n of reflectNames) {
  const fn: any = (Reflect as any)[n];
  const d = Object.getOwnPropertyDescriptor(Reflect, n) as any;
  show("reflect_m_" + n, typeof fn + ",name=" + fn.name + ",len=" + fn.length +
    ",w=" + d.writable + ",e=" + d.enumerable + ",c=" + d.configurable +
    ",proto=" + (Object.getPrototypeOf(fn) === Function.prototype) +
    ",hasPrototype=" + ("prototype" in fn));
}
for (const n of reflectNames) {
  try {
    new ((Reflect as any)[n])({}, "k");
    show("reflect_new_" + n, "ok");
  } catch (e) {
    show("reflect_new_" + n, "throw:" + (e as Error).constructor.name);
  }
}

attempt("reflect_get_primitive", () => Reflect.get(1 as any, "toFixed"));
attempt("reflect_set_primitive", () => Reflect.set("s" as any, "0", "x"));
attempt("reflect_has_primitive", () => Reflect.has(1 as any, "x"));
attempt("reflect_ownKeys_primitive", () => Reflect.ownKeys("ab" as any).join("|"));
attempt("reflect_getproto_primitive", () => Reflect.getPrototypeOf("ab" as any));
attempt("reflect_gopd_primitive", () => Reflect.getOwnPropertyDescriptor("ab" as any, "0"));
attempt("reflect_delete_primitive", () => Reflect.deleteProperty(1 as any, "x"));
attempt("reflect_isExtensible_primitive", () => Reflect.isExtensible(1 as any));
attempt("reflect_preventExtensions_primitive", () => Reflect.preventExtensions(1 as any));
attempt("reflect_define_primitive", () => Reflect.defineProperty(1 as any, "x", { value: 1 }));
attempt("reflect_setproto_primitive", () => Reflect.setPrototypeOf(1 as any, null));
attempt("reflect_apply_nonfn", () => Reflect.apply({} as any, null, []));
attempt("reflect_construct_nonfn", () => Reflect.construct({} as any, []));
show("object_keys_string_coerces", Object.keys("ab").join("|"));
show("object_getproto_string_coerces", Object.getPrototypeOf("ab") === String.prototype);
show("object_isExtensible_number", Object.isExtensible(1 as any));

const keyHolder: any = { undefined: "U" };
const keySym = Symbol("k");
keyHolder[keySym] = "S";
show("reflect_key_missing", Reflect.get(keyHolder, undefined as any));
show("reflect_key_symbol", Reflect.get(keyHolder, keySym));
show("reflect_key_number", Reflect.get({ 3: "three" } as any, 3 as any));
show("reflect_has_undefined_key", Reflect.has(keyHolder, undefined as any));
attempt("reflect_define_bad_desc", () => Reflect.defineProperty({} as any, "x", 1 as any));
attempt("reflect_apply_bad_args", () => Reflect.apply(function () { return 1; }, null, 5 as any));
attempt("reflect_apply_missing_args", () => (Reflect.apply as any)(function () { return 1; }, null));

// ---------------------------------------------------------------------------
// 10. Coercion goes through the get trap: @@toPrimitive, valueOf, toString
// ---------------------------------------------------------------------------
const coerceLog: string[] = [];

function coercible(target: any): any {
  return new Proxy(target, {
    get(t, k, r) { coerceLog.push(String(k)); return Reflect.get(t, k, r); },
  });
}

function coerce(label: string, fn: (p: any) => string, target?: any): void {
  coerceLog.length = 0;
  const p = coercible(target === undefined ? { valueOf() { return 7; }, toString() { return "SEVEN"; } } : target);
  let out: string;
  try {
    out = fn(p);
  } catch (e) {
    out = "throw:" + (e as Error).constructor.name;
  }
  show(label, out + "|" + coerceLog.join(","));
}

coerce("co_String", (p) => String(p));
coerce("co_template", (p) => `${p}`);
coerce("co_concat_left", (p) => p + "");
coerce("co_plus_number", (p) => String(p + 1));
coerce("co_Number", (p) => String(Number(p)));
coerce("co_unary_plus", (p) => String(+p));
coerce("co_multiply", (p) => String(p * 2));
coerce("co_loose_number", (p) => String(p == 7));
coerce("co_loose_string", (p) => String(p == "SEVEN"));
coerce("co_relational", (p) => String(p < 8));
coerce("co_property_key", (p) => { const o: any = {}; o[p] = 1; return Object.keys(o).join("|"); });
coerce("co_array_join", (p) => [p].join(","));
coerce("co_only_toString", (p) => String(p * 1), { toString() { return "3"; } });
coerce("co_only_valueOf", (p) => String(p * 2), { valueOf() { return 4; } });
coerce("co_valueOf_object", (p) => String(p + ""), { valueOf() { return {}; }, toString() { return "FALLBACK"; } });
coerce("co_neither", (p) => String(p + ""), Object.create(null));

const hints: string[] = [];
const withToPrimitive: any = {
  [Symbol.toPrimitive](hint: string) { hints.push(hint); return hint === "string" ? "S" : 11; },
  valueOf() { return 99; },
  toString() { return "NEVER"; },
};
coerce("co_sym_string", (p) => String(p), withToPrimitive);
coerce("co_sym_number", (p) => String(+p), withToPrimitive);
coerce("co_sym_default", (p) => String(p + ""), withToPrimitive);
show("co_hints", hints.join(","));

coerceLog.length = 0;
const injected: any = new Proxy({} as any, {
  get(t, k, r) {
    coerceLog.push(String(k));
    if (k === Symbol.toPrimitive) return (hint: string) => "INJ_" + hint;
    return Reflect.get(t, k, r);
  },
});
show("co_injected_string", String(injected) + "|" + coerceLog.join(","));
coerceLog.length = 0;
show("co_injected_default", (injected + "") + "|" + coerceLog.join(","));

function coerceWithSymbol(label: string, sym: any): void {
  const t: any = { [Symbol.toPrimitive]: sym, valueOf() { return 1; }, toString() { return "T"; } };
  try {
    show(label, String(new Proxy(t, {}) + ""));
  } catch (e) {
    show(label, "throw:" + (e as Error).constructor.name);
  }
}
coerceWithSymbol("co_sym_null_absent", null);
coerceWithSymbol("co_sym_undefined_absent", undefined);
coerceWithSymbol("co_sym_number_throws", 5);
coerceWithSymbol("co_sym_object_throws", {});
try {
  show("co_sym_returns_object", String(new Proxy({ [Symbol.toPrimitive]() { return {}; } }, {}) + ""));
} catch (e) {
  show("co_sym_returns_object", "throw:" + (e as Error).constructor.name);
}

// ---------------------------------------------------------------------------
// 11. Object.assign: proxy as source (ownKeys, gopd, get) and as target (set)
// ---------------------------------------------------------------------------
const assignLog: string[] = [];

function assignSource(target: any): any {
  return new Proxy(target, {
    ownKeys(t) { assignLog.push("src:ownKeys"); return Reflect.ownKeys(t); },
    getOwnPropertyDescriptor(t, k) { assignLog.push("src:gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
    get(t, k, r) { assignLog.push("src:get:" + String(k)); return Reflect.get(t, k, r); },
  });
}

function assignTarget(target: any): any {
  return new Proxy(target, {
    set(t, k, v, r) { assignLog.push("dst:set:" + String(k) + "=" + String(v)); return Reflect.set(t, k, v, r); },
    getOwnPropertyDescriptor(t, k) { assignLog.push("dst:gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
    defineProperty(t, k, d) { assignLog.push("dst:define:" + String(k)); return Reflect.defineProperty(t, k, d); },
  });
}

function assignRun(label: string, fn: () => string): void {
  assignLog.length = 0;
  let out: string;
  try {
    out = fn();
  } catch (e) {
    out = "throw:" + (e as Error).constructor.name;
  }
  show(label, out + "|" + assignLog.join(","));
}

assignRun("as_source_only", () => JSON.stringify(Object.assign({}, assignSource({ a: 1, b: 2 }))));
assignRun("as_target_only", () => { const t: any = {}; Object.assign(assignTarget(t), { x: 1, y: 2 }); return JSON.stringify(t); });
assignRun("as_both", () => { const t: any = {}; Object.assign(assignTarget(t), assignSource({ a: 1 })); return JSON.stringify(t); });
assignRun("as_two_sources", () => JSON.stringify(Object.assign({}, assignSource({ a: 1 }), assignSource({ b: 2 }))));
assignRun("as_overlap", () => JSON.stringify(Object.assign({ a: 0 }, assignSource({ a: 1 }))));
assignRun("as_nonenumerable_skipped", () => {
  const src: any = { visible: 1 };
  Object.defineProperty(src, "hidden", { value: 2, enumerable: false, configurable: true });
  return JSON.stringify(Object.assign({}, assignSource(src)));
});
assignRun("as_lying_descriptor", () => {
  const p: any = new Proxy({ a: 1, b: 2 }, {
    getOwnPropertyDescriptor(t, k) {
      assignLog.push("src:gopd:" + String(k));
      const d = Reflect.getOwnPropertyDescriptor(t, k) as any;
      if (k === "a" && d) d.enumerable = false;
      return d;
    },
    get(t, k, r) { assignLog.push("src:get:" + String(k)); return Reflect.get(t, k, r); },
  });
  return JSON.stringify(Object.assign({}, p));
});

const assignSymbol = Symbol("s");
const assignSymSource: any = { a: 1 };
assignSymSource[assignSymbol] = "S";
const symCopy: any = Object.assign({}, new Proxy(assignSymSource, {}));
show("assign_symbol_copied", symCopy[assignSymbol] + ",keys=" + Reflect.ownKeys(symCopy).map(String).join("|"));

const partialTarget: any = {};
const partial: any = new Proxy(partialTarget, {
  set(t, k, v, r) { if (k === "b") return false; return Reflect.set(t, k, v, r); },
});
attempt("assign_partial_refused", () => { Object.assign(partial, { a: 1, b: 2, c: 3 }); return "ok"; });
show("assign_partial_written", Object.keys(partialTarget).join("|"));

const throwingSource: any = new Proxy({ a: 1, get boom(): number { throw new RangeError("x"); }, c: 3 }, {});
const throwTarget: any = {};
attempt("assign_getter_throws", () => { Object.assign(throwTarget, throwingSource); return "ok"; });
show("assign_throw_written", Object.keys(throwTarget).join("|"));

const identityTarget: any = {};
const identityProxy: any = new Proxy(identityTarget, {});
show("assign_returns_proxy", Object.assign(identityProxy, { z: 1 }) === identityProxy);
show("assign_returns_not_target", Object.assign(identityProxy, { z: 1 }) === identityTarget);

show("assign_string_source", JSON.stringify(Object.assign({}, new Proxy(new String("ab"), {}))));
show("assign_nullish_skipped", JSON.stringify(Object.assign({ k: 1 }, null, undefined, new Proxy({ m: 2 }, {}))));

const ghost: any = new Proxy({ real: 1 } as any, { ownKeys() { return ["real", "ghost"]; } });
show("assign_ghost_skipped", JSON.stringify(Object.assign({}, ghost)));

const frozenUnder: any = Object.freeze({ a: 1 });
attempt("assign_frozen_under", () => { Object.assign(new Proxy(frozenUnder, {}), { a: 2 }); return "ok"; });
show("assign_frozen_value", frozenUnder.a);

assignLog.length = 0;
const selfTarget: any = { a: 1, b: 2 };
const selfProxy: any = new Proxy(selfTarget, {
  ownKeys(t) { assignLog.push("ownKeys"); return Reflect.ownKeys(t); },
  getOwnPropertyDescriptor(t, k) { assignLog.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
  get(t, k, r) { assignLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
  set(t, k, v, r) { assignLog.push("set:" + String(k)); return Reflect.set(t, k, v, r); },
});
Object.assign(selfProxy, selfProxy);
show("assign_self_value", JSON.stringify(selfTarget));
show("assign_self_trace", assignLog.join(","));

const boxedProxy: any = new Proxy(new Number(3), {});
Object.assign(boxedProxy, { extra: "E" });
show("assign_boxed", boxedProxy.extra + ",keys=" + Object.keys(boxedProxy).join("|"));
