// Cross-runtime: which properties JSON.stringify reads from an object, in what order,
// when getters run, how built-in and class instances look, and which proxy traps fire.

// --- own enumerable string keys only: no inherited, non-enumerable or symbol keys ---
const base: any = { inherited: "no" };
const derived: any = Object.create(base);
derived.own = "yes";
console.log("inherited_skipped=" + JSON.stringify(derived));
console.log("inherited_visible=" + derived.inherited);
let hiddenReads = 0;
const hidden: any = { shown: 1 };
Object.defineProperty(hidden, "secret", { get() { hiddenReads++; return 9; }, enumerable: false });
console.log("non_enumerable_skipped=" + JSON.stringify(hidden) + ":reads=" + hiddenReads);
const marker = Symbol("s");
console.log("symbol_key_skipped=" + JSON.stringify({ a: 1, [marker]: 2 }));
console.log("defined_hidden_skipped=" + JSON.stringify(Object.defineProperty({ a: 1 }, "hidden", { value: 2 })));
const withAccessor: any = { plain: 1 };
Object.defineProperty(withAccessor, "own_getter", { get() { return 9; }, enumerable: true });
Object.defineProperty(withAccessor, "hidden_getter", { get() { return 9; }, enumerable: false });
console.log("own_getters=" + JSON.stringify(withAccessor));

// --- key order: integer-like keys ascend numerically first, then strings by insertion ---
const order: any = {};
order.zeta = 1;
order.alpha = 2;
order.mid = 3;
console.log("insertion_order=" + JSON.stringify(order));
const intFirst: any = {};
intFirst.foo = "s";
intFirst["2"] = "two";
intFirst.bar = "s2";
intFirst["1"] = "one";
intFirst["0"] = "zero";
console.log("int_first=" + JSON.stringify(intFirst));
console.log("literal_int_order=" + JSON.stringify({ "10": "a", "2": "b", "1": "c" }));
console.log("mixed_literal=" + JSON.stringify({ b: 1, "2": 2, a: 3, "1": 4 }));
const numeric: any = {};
numeric["10"] = "ten";
numeric["9"] = "nine";
numeric["100"] = "hundred";
console.log("numeric_not_lexical=" + JSON.stringify(numeric));
const notIndex: any = {};
notIndex["01"] = "leading_zero";
notIndex["1"] = "one";
notIndex["-1"] = "negative";
notIndex["1.5"] = "float";
notIndex["+1"] = "plus";
notIndex[""] = "empty";
console.log("not_index_keys=" + JSON.stringify(notIndex));
const limit: any = {};
limit["4294967295"] = "not_index";
limit["4294967294"] = "max_index";
limit.tail = "t";
console.log("index_limit=" + JSON.stringify(limit));
const reinsert: any = { x: 1, y: 2, z: 3 };
delete reinsert.x;
reinsert.x = 4;
console.log("reinsert_moves_to_end=" + JSON.stringify(reinsert));
const zInt: any = {};
zInt.z = 0;
zInt[10] = "ten";
zInt[2] = "two";
zInt.a = 1;
console.log("int_keys_mixed=" + JSON.stringify(zInt) + ":keys=" + Object.keys(zInt).join(","));
console.log("nested_numeric_key=" + JSON.stringify({ outer: { b: 1, "0": 2 } }));
console.log("array_position_kept=" + JSON.stringify(["a", "b", "c"]));

// --- getters are read exactly once, in key order, and see live mutations ---
const reads: string[] = [];
const seqValue = { get a() { reads.push("a"); return 1; }, get b() { reads.push("b"); return { c: 2 }; } };
console.log("getters_out=" + JSON.stringify(seqValue));
console.log("getters_read_order=" + reads.join(","));
let counter = 0;
const counted: any = {};
Object.defineProperty(counted, "v", { get() { counter++; return counter; }, enumerable: true });
console.log("counted1=" + JSON.stringify(counted) + ":reads=" + counter);
console.log("counted2=" + JSON.stringify(counted) + ":reads=" + counter);
console.log("counted_repeated=" + JSON.stringify([counted, counted]));

// key list is snapshotted before the walk: keys added by a getter are not visited
const adder: any = { a: 1 };
Object.defineProperty(adder, "b", {
  get() { adder.zzz = "late"; return 2; },
  enumerable: true,
  configurable: true,
});
adder.c = 3;
console.log("adder_out=" + JSON.stringify(adder));
console.log("adder_has_zzz=" + ("zzz" in adder));
console.log("adder_second_pass=" + JSON.stringify(adder));
// a key deleted by a getter before it is reached is elided
const deleter: any = { a: 1 };
Object.defineProperty(deleter, "b", {
  get() { delete deleter.c; return 2; },
  enumerable: true,
  configurable: true,
});
deleter.c = 3;
deleter.d = 4;
console.log("deleter_out=" + JSON.stringify(deleter));
console.log("deleter_keys_after=" + Object.keys(deleter).join(","));
// a later value changed by a getter is written with its new value
const changer: any = { a: 1 };
Object.defineProperty(changer, "b", {
  get() { changer.c = "changed"; return 2; },
  enumerable: true,
  configurable: true,
});
changer.c = "original";
console.log("changer_out=" + JSON.stringify(changer));
// an earlier value changed by a getter was already written
const backwards: any = { a: "first" };
Object.defineProperty(backwards, "b", {
  get() { backwards.a = "mutated"; return 2; },
  enumerable: true,
  configurable: true,
});
console.log("backwards_out=" + JSON.stringify(backwards));
console.log("backwards_a_now=" + backwards.a);
// a toJSON that mutates its holder: later siblings see the change
const holder: any = { keep: 1, item: { toJSON() { holder.keep = 999; return "done"; } }, tail: 2 };
console.log("tojson_mutates=" + JSON.stringify(holder));
console.log("tojson_keep_now=" + holder.keep);
// arrays are walked by length read up front; shrinking mid-walk shows as null
const arr: any = [0, 1, 2, 3];
Object.defineProperty(arr, "1", {
  get() { arr.length = 2; return "one"; },
  configurable: true,
  enumerable: true,
});
console.log("array_shrunk=" + JSON.stringify(arr));
console.log("array_len_after=" + arr.length);
const grow: any = [0, 1];
Object.defineProperty(grow, "0", {
  get() { grow.push(99, 100); return "zero"; },
  configurable: true,
  enumerable: true,
});
console.log("array_grown=" + JSON.stringify(grow));
console.log("array_grown_len=" + grow.length);
// a throwing getter aborts the whole serialisation
const thrower: any = { a: 1 };
Object.defineProperty(thrower, "b", { get() { throw new RangeError("no"); }, enumerable: true });
try {
  JSON.stringify(thrower);
  console.log("thrower=no_throw");
} catch (e: any) {
  console.log("thrower=" + e.name);
}
console.log("thrower_a_still=" + thrower.a);

// --- built-in objects: slot-backed data is invisible, own enumerable indices are visible ---
console.log("map=" + JSON.stringify(new Map([["a", 1]])));
console.log("set=" + JSON.stringify(new Set([1, 2])));
console.log("weakmap=" + JSON.stringify(new WeakMap()));
console.log("weakset=" + JSON.stringify(new WeakSet()));
console.log("map_with_own_prop=" + JSON.stringify(Object.assign(new Map([["a", 1]]), { tagged: true })));
console.log("arraybuffer=" + JSON.stringify(new ArrayBuffer(4)));
console.log("dataview=" + JSON.stringify(new DataView(new ArrayBuffer(4))));
console.log("uint8=" + JSON.stringify(new Uint8Array([1, 2, 3])));
console.log("float64=" + JSON.stringify(new Float64Array([1.5, 2])));
console.log("int8_empty=" + JSON.stringify(new Int8Array(0)));
console.log("typed_in_object=" + JSON.stringify({ data: new Uint8Array([7]) }));
console.log("typed_is_array=" + Array.isArray(new Uint8Array([1])));
console.log("error=" + JSON.stringify(new Error("boom")));
console.log("type_error=" + JSON.stringify(new TypeError("boom")));
const withCode: any = new Error("boom");
withCode.code = "E_CODE";
console.log("error_own_prop=" + JSON.stringify(withCode));
console.log("error_cause=" + JSON.stringify(new Error("boom", { cause: "why" })));
console.log("message_enumerable=" + Object.prototype.propertyIsEnumerable.call(new Error("x"), "message"));
console.log("regexp=" + JSON.stringify(/ab+c/gi));
console.log("regexp_own_prop=" + JSON.stringify(Object.assign(/x/, { note: "kept" })));
console.log("date=" + JSON.stringify(new Date(0)));
console.log("invalid_date=" + JSON.stringify(new Date(NaN)));
console.log("promise=" + JSON.stringify(Promise.resolve(1)));
console.log("map_iterator=" + JSON.stringify(new Map([["a", 1]]).entries()));
console.log("generator=" + JSON.stringify((function* () { yield 1; })()));
console.log("weakref=" + JSON.stringify(new WeakRef({ a: 1 })));
console.log("arguments=" + JSON.stringify((function () { return arguments; })(1, "two")));
console.log("array_like=" + JSON.stringify({ 0: "a", 1: "b", length: 2 }));
console.log("array_extra_prop_dropped=" + JSON.stringify(Object.assign([1, 2], { note: "dropped" })));
console.log("null_proto=" + JSON.stringify(Object.assign(Object.create(null), { a: 1 })));

// --- class instances keep fields, lose methods and prototype accessors ---
class Point {
  x = 1;
  y = 2;
  get computed(): number { return 3; }
  sum(): number { return this.x + this.y; }
  static origin = "0,0";
}
console.log("class_instance=" + JSON.stringify(new Point()));
class Secret {
  #hidden = "no";
  shown = "yes";
  reveal(): string { return this.#hidden; }
}
console.log("private_field_hidden=" + JSON.stringify(new Secret()));

// --- boxed primitives in the same object model ---
console.log("boxed_in_array=" + JSON.stringify([new Number(1), new String("a"), new Boolean(false)]));

// --- Map and Set have no own data, so the usual fix is an explicit conversion ---
const m = new Map<string, number>([["a", 1], ["b", 2]]);
console.log("map_raw=" + JSON.stringify(m));
console.log("map_via_from_entries=" + JSON.stringify(Object.fromEntries(m)));
console.log("map_via_entries_array=" + JSON.stringify([...m]));
console.log("set_via_spread=" + JSON.stringify([...new Set([1, 2])]));
(Map.prototype as any).toJSON = function (this: any) { return [...this]; };
console.log("map_with_prototype_toJSON=" + JSON.stringify(m));
console.log("map_nested_with_prototype_toJSON=" + JSON.stringify({ inner: m }));
delete (Map.prototype as any).toJSON;
console.log("map_after_cleanup=" + JSON.stringify(m));

// --- Proxy: trap sequence for objects and arrays ---
const objLog: string[] = [];
const objProxy = new Proxy({ a: 1, b: 2 }, {
  ownKeys(t: any) { objLog.push("ownKeys"); return Reflect.ownKeys(t); },
  getOwnPropertyDescriptor(t: any, k: any) { objLog.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
  get(t: any, k: any, r: any) { objLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
});
console.log("proxy_obj_out=" + JSON.stringify(objProxy));
console.log("proxy_obj_traps=" + objLog.join("|"));
const arrLog: string[] = [];
const arrProxy = new Proxy([10, 20], {
  get(t: any, k: any, r: any) { arrLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
  ownKeys(t: any) { arrLog.push("ownKeys"); return Reflect.ownKeys(t); },
  getOwnPropertyDescriptor(t: any, k: any) { arrLog.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
});
console.log("proxy_arr_isArray=" + Array.isArray(arrProxy));
console.log("proxy_arr_out=" + JSON.stringify(arrProxy));
console.log("proxy_arr_traps=" + arrLog.join("|"));
console.log("proxy_get_rewrites=" + JSON.stringify(new Proxy({ a: 1, b: 2 }, {
  get(t: any, k: any) { return typeof t[k] === "number" ? t[k] * 100 : t[k]; },
})));
console.log("proxy_ownkeys_filters=" + JSON.stringify(new Proxy({ a: 1, b: 2, c: 3 }, { ownKeys() { return ["a", "c"]; } })));
console.log("proxy_non_enumerable_skipped=" + JSON.stringify(new Proxy({ a: 1, b: 2 }, {
  getOwnPropertyDescriptor(t: any, k: any) {
    const desc: any = Reflect.getOwnPropertyDescriptor(t, k);
    if (k === "b" && desc) desc.enumerable = false;
    return desc;
  },
})));
const tjLog: string[] = [];
const withToJSON = new Proxy({ a: 1 }, {
  get(t: any, k: any, r: any) {
    tjLog.push(String(k));
    if (k === "toJSON") return function () { return "PROXY_JSON"; };
    return Reflect.get(t, k, r);
  },
});
console.log("proxy_tojson_out=" + JSON.stringify(withToJSON));
console.log("proxy_tojson_gets=" + tjLog.join(","));
console.log("proxy_nested=" + JSON.stringify({ p: objProxy, n: 5 }));
console.log("proxy_of_proxy=" + JSON.stringify(new Proxy(new Proxy({ x: 1 }, {}), {})));
const dateProxy = new Proxy(new Date(Date.UTC(2021, 4, 6, 7, 8, 9)), {});
try {
  JSON.stringify(dateProxy);
  console.log("proxy_date=no_throw");
} catch (e: any) {
  console.log("proxy_date=" + e.name);
}
const boom = new Proxy({ a: 1 }, { get() { throw new RangeError("trap"); } });
try {
  JSON.stringify(boom);
  console.log("throwing_trap=no_throw");
} catch (e: any) {
  console.log("throwing_trap=" + e.name);
}
const rev = Proxy.revocable({ a: 1 }, {});
console.log("before_revoke=" + JSON.stringify(rev.proxy));
rev.revoke();
try {
  JSON.stringify(rev.proxy);
  console.log("after_revoke=no_throw");
} catch (e: any) {
  console.log("after_revoke=" + e.name);
}
const fnProxy = new Proxy(function () { /* fn */ }, {});
console.log("fn_proxy_in_obj=" + JSON.stringify({ f: fnProxy }));
console.log("fn_proxy_in_arr=" + JSON.stringify([fnProxy]));
console.log("fn_proxy_top_is_undefined=" + (JSON.stringify(fnProxy) === undefined));
