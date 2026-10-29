// Property descriptors: defineProperty defaults, partial redefinition, data<->accessor
// transitions, non-configurable locks, ToPropertyDescriptor coercion and read order,
// defineProperties, and getOwnPropertyDescriptor(s) shapes.
// `export {}` makes the file a module (strict), so refused writes are observed through
// Reflect.set and thrown errors are caught explicitly. Nothing depends on host mode.
export {};

function shape(target: any, key: string): string {
  const d = Object.getOwnPropertyDescriptor(target, key) as any;
  if (d === undefined) return "undefined";
  if ("get" in d || "set" in d) {
    return "acc{get=" + typeof d.get + ",set=" + typeof d.set + ",e=" + d.enumerable + ",c=" + d.configurable + "}";
  }
  return "data{v=" + String(d.value) + ",w=" + d.writable + ",e=" + d.enumerable + ",c=" + d.configurable + "}";
}

function attempt(label: string, fn: () => unknown): void {
  try {
    fn();
    console.log(label + "=ok");
  } catch (e: any) {
    console.log(label + "=throw:" + e.constructor.name);
  }
}

// ---------- 1. Defaults for omitted flags ----------
const fresh: any = {};
Object.defineProperty(fresh, "x", { value: 1 });
console.log("defaults=" + shape(fresh, "x"));
console.log("defaults_keys=" + Object.keys(fresh).length + ",hasOwn=" + Object.hasOwn(fresh, "x"));
console.log("defaults_write=" + Reflect.set(fresh, "x", 2) + ",read=" + fresh.x);
attempt("defaults_redefine_conf", () => Object.defineProperty(fresh, "x", { configurable: true }));

// an empty descriptor on a MISSING key creates an all-false data slot
const emptyNew: any = {};
Object.defineProperty(emptyNew, "e", {});
console.log("empty_new=" + shape(emptyNew, "e") + ",in=" + ("e" in emptyNew));
// an empty descriptor on an EXISTING key changes nothing
const plain: any = { d: 1 };
Object.defineProperty(plain, "d", {});
console.log("empty_existing=" + shape(plain, "d"));

// ---------- 2. Partial redefinition and data<->accessor transitions ----------
const o: any = {};
Object.defineProperty(o, "p", { value: 1, writable: true, enumerable: true, configurable: true });
console.log("start=" + shape(o, "p"));

Object.defineProperty(o, "p", { enumerable: false });
console.log("partial=" + shape(o, "p"));

let backing = "acc";
Object.defineProperty(o, "p", { get() { return backing; } });
console.log("to_acc=" + shape(o, "p"));
console.log("read_acc=" + o.p + ",write_no_setter=" + Reflect.set(o, "p", "ignored") + ",after=" + o.p);

Object.defineProperty(o, "p", { set(v: string) { backing = "<" + v + ">"; } });
console.log("with_set=" + shape(o, "p"));
console.log("write_setter=" + Reflect.set(o, "p", "hello") + ",read=" + o.p);

Object.defineProperty(o, "p", { value: 42 });
console.log("to_data=" + shape(o, "p"));
console.log("write_readonly=" + Reflect.set(o, "p", 99) + ",after=" + o.p);

Object.defineProperty(o, "p", { get() { return "g2"; }, configurable: true });
console.log("acc_again=" + shape(o, "p"));
Object.defineProperty(o, "p", { value: 7, writable: true });
console.log("data_again=" + shape(o, "p"));

// accessor with no getter reads undefined and swallows writes
const noget: any = {};
let sink = "none";
Object.defineProperty(noget, "r", { set(v: string) { sink = v; }, configurable: true });
console.log("noget_read=" + noget.r + ",write=" + Reflect.set(noget, "r", "written") + ",sink=" + sink);
console.log("noget_shape=" + shape(noget, "r"));

// assignment never converts a data property into an accessor
console.log("assign_keeps_data=" + Reflect.set(plain, "d", 2) + ",shape=" + shape(plain, "d"));

// ---------- 3. Non-configurable locks ----------
const locked: any = {};
Object.defineProperty(locked, "locked", { value: 1, writable: true, enumerable: true, configurable: false });
console.log("lock_start=" + shape(locked, "locked"));
attempt("lock_same", () => Object.defineProperty(locked, "locked", { value: 1, writable: true, enumerable: true, configurable: false }));
attempt("lock_empty", () => Object.defineProperty(locked, "locked", {}));
attempt("lock_newvalue", () => Object.defineProperty(locked, "locked", { value: 2 }));
console.log("lock_after_newvalue=" + shape(locked, "locked"));
attempt("lock_enum_flip", () => Object.defineProperty(locked, "locked", { enumerable: false }));
attempt("lock_conf_on", () => Object.defineProperty(locked, "locked", { configurable: true }));
attempt("lock_to_accessor", () => Object.defineProperty(locked, "locked", { get() { return 0; } }));
attempt("lock_w_down", () => Object.defineProperty(locked, "locked", { writable: false }));
console.log("lock_after_w_down=" + shape(locked, "locked"));
attempt("lock_value_after", () => Object.defineProperty(locked, "locked", { value: 3 }));
attempt("lock_same_value_after", () => Object.defineProperty(locked, "locked", { value: 2 }));
attempt("lock_w_up", () => Object.defineProperty(locked, "locked", { writable: true }));
console.log("lock_final=" + shape(locked, "locked"));
console.log("lock_reflect_conf=" + Reflect.defineProperty(locked, "locked", { configurable: true }));
console.log("lock_reflect_same=" + Reflect.defineProperty(locked, "locked", { value: 2 }));
console.log("lock_delete=" + Reflect.deleteProperty(locked, "locked") + ",still=" + ("locked" in locked));

// non-configurable accessor: identical get/set accepted, anything else refused
const g = function () { return "G"; };
const s = function () { /* ignored */ };
const acc: any = {};
Object.defineProperty(acc, "a", { get: g, set: s, enumerable: false, configurable: false });
console.log("acc_start=" + shape(acc, "a"));
attempt("acc_same", () => Object.defineProperty(acc, "a", { get: g, set: s }));
attempt("acc_other_get", () => Object.defineProperty(acc, "a", { get: function () { return "H"; } }));
attempt("acc_drop_set", () => Object.defineProperty(acc, "a", { get: g, set: undefined }));
attempt("acc_to_data", () => Object.defineProperty(acc, "a", { value: 1 }));
console.log("acc_read=" + acc.a);

// SameValue, not ===: NaN matches NaN, and 0 vs -0 is a difference
const nz: any = {};
Object.defineProperty(nz, "n", { value: NaN, writable: false, configurable: false });
attempt("nan_same", () => Object.defineProperty(nz, "n", { value: NaN }));
Object.defineProperty(nz, "z", { value: 0, writable: false, configurable: false });
attempt("zero_same", () => Object.defineProperty(nz, "z", { value: 0 }));
attempt("zero_vs_negzero", () => Object.defineProperty(nz, "z", { value: -0 }));

// ---------- 4. ToPropertyDescriptor coercion and read order ----------
const order: string[] = [];
const probe: any = {};
Object.defineProperty(probe, "set", { get() { order.push("set"); return undefined; }, enumerable: true });
Object.defineProperty(probe, "get", { get() { order.push("get"); return undefined; }, enumerable: true });
Object.defineProperty(probe, "writable", { get() { order.push("writable"); return true; }, enumerable: true });
Object.defineProperty(probe, "value", { get() { order.push("value"); return 1; }, enumerable: true });
Object.defineProperty(probe, "configurable", { get() { order.push("configurable"); return true; }, enumerable: true });
Object.defineProperty(probe, "enumerable", { get() { order.push("enumerable"); return true; }, enumerable: true });
attempt("probe", () => Object.defineProperty({} as any, "k", probe));
console.log("read_order=" + order.join("|"));
console.log("declared_order=" + Object.keys(probe).join("|"));

// fields are read through the prototype chain
const inheritedDesc: any = Object.create({ value: "FROM_PROTO", enumerable: true });
const t1: any = {};
Object.defineProperty(t1, "k", inheritedDesc);
const d1 = Object.getOwnPropertyDescriptor(t1, "k") as any;
console.log("inherited=" + d1.value + ",e=" + d1.enumerable + ",w=" + d1.writable);

// the three flags go through ToBoolean
function flags(desc: any): string {
  const t: any = {};
  Object.defineProperty(t, "k", desc);
  const d = Object.getOwnPropertyDescriptor(t, "k") as any;
  return "w=" + d.writable + ",e=" + d.enumerable + ",c=" + d.configurable;
}
console.log("flags_strings=" + flags({ value: 1, writable: "false", enumerable: "0", configurable: "" }));
console.log("flags_numbers=" + flags({ value: 1, writable: 1, enumerable: 0, configurable: -1 }));
console.log("flags_objects=" + flags({ value: 1, writable: {}, enumerable: [], configurable: null }));
console.log("flags_nan=" + flags({ value: 1, writable: NaN, enumerable: Infinity, configurable: undefined }));

// a present-but-undefined flag still counts as present and coerces to false
const t2: any = {};
Object.defineProperty(t2, "k", { value: 1, writable: true, enumerable: true, configurable: true });
Object.defineProperty(t2, "k", { enumerable: undefined });
const d2 = Object.getOwnPropertyDescriptor(t2, "k") as any;
console.log("present_undefined=e=" + d2.enumerable + ",w=" + d2.writable);

function tryDesc(label: string, desc: any): void {
  try {
    Object.defineProperty({} as any, "k", desc);
    console.log(label + "=ok");
  } catch (e: any) {
    console.log(label + "=throw:" + e.constructor.name);
  }
}
tryDesc("get_number", { get: 1 });
tryDesc("get_null", { get: null });
tryDesc("get_undefined", { get: undefined });
tryDesc("set_object", { set: {} });
tryDesc("get_arrow", { get: () => 1 });
tryDesc("value_and_get", { value: 1, get() { return 2; } });
tryDesc("value_and_set", { value: 1, set() { /* noop */ } });
tryDesc("writable_and_get", { writable: true, get() { return 2; } });
tryDesc("writable_and_set_undefined", { writable: true, set: undefined });
tryDesc("value_undefined_and_get", { value: undefined, get() { return 2; } });
tryDesc("desc_number", 1);
tryDesc("desc_string", "value");
tryDesc("desc_null", null);
tryDesc("desc_undefined", undefined);

// unknown fields are ignored; key case matters
const t3: any = {};
Object.defineProperty(t3, "k", { value: 5, enumerable: true, bogus: "ignored", VALUE: 9 } as any);
const d3 = Object.getOwnPropertyDescriptor(t3, "k") as any;
console.log("extra_fields=" + Object.keys(d3).join("|") + ",v=" + d3.value);

// a function works as a descriptor: its own properties are read
function fnDesc(): void { /* noop */ }
(fnDesc as any).value = "from-function";
(fnDesc as any).enumerable = true;
const t4: any = {};
Object.defineProperty(t4, "k", fnDesc as any);
console.log("fn_desc=" + t4.k + ",keys=" + Object.keys(t4).join("|"));

// the KEY is coerced before the descriptor is validated
const keyLog: string[] = [];
const weirdKey: any = { toString() { keyLog.push("key"); return "coerced"; } };
tryDesc("key_then_desc", { get: 1 });
try {
  Object.defineProperty({} as any, weirdKey, { get: 1 } as any);
} catch (e: any) {
  console.log("key_desc_throw=" + e.constructor.name + ",key_read=" + keyLog.join("|"));
}

// defineProperties reads the descriptor map's own ENUMERABLE keys only
const map: any = { a: { value: 1, enumerable: true } };
Object.defineProperty(map, "b", { value: { value: 2, enumerable: true }, enumerable: false });
const t5: any = Object.defineProperties({}, map);
console.log("defineProperties=" + Reflect.ownKeys(t5).join("|"));

// defineProperties with mixed data and accessor descriptors
let cell = 3;
const dp: any = {};
Object.defineProperties(dp, {
  fixed: { value: 7, enumerable: true },
  live: { get() { return cell; }, set(v: number) { cell = v * 2; }, enumerable: false },
});
dp.live = 5;
console.log("dp_values=" + dp.fixed + "," + dp.live + ",keys=" + Object.keys(dp).join("|"));
console.log("dp_descs=" + JSON.stringify(Object.getOwnPropertyDescriptors(dp), (_k, v) => (typeof v === "function" ? "fn" : v)));

// defineProperties gathers every descriptor before applying any: a throw leaves the target clean
const partial: any = {};
const badMap: any = {
  get a() { console.log("gather_a"); return { value: 1, enumerable: true }; },
  get b() { console.log("gather_b"); throw new RangeError("bad"); },
};
try {
  Object.defineProperties(partial, badMap);
} catch (e: any) {
  console.log("partial_throw=" + e.name + ":" + e.message);
}
console.log("partial_keys=" + Object.keys(partial).length);

// accessor delete and redefinition through the same property
let live2 = 1;
const ad: any = {};
Object.defineProperty(ad, "x", {
  get() { return live2; },
  set(v: number) { live2 = v * 2; },
  enumerable: true,
  configurable: true,
});
ad.x = 5;
console.log("acc_delete_before=" + ad.x + ":" + live2 + ":" + Object.keys(ad).join(","));
console.log("acc_delete=" + delete ad.x);
ad.x = 9;
console.log("acc_delete_after=" + ad.x + ":" + live2 + ":" + Object.keys(ad).join(","));

// ---------- 5. Own-property introspection ----------
// propertyIsEnumerable inspects only own descriptors
const pe = Object.create({ inherited: 1 });
Object.defineProperty(pe, "hidden", { value: 2 });
pe.visible = 3;
console.log("pie_inherited=" + pe.propertyIsEnumerable("inherited"));
console.log("pie_hidden=" + pe.propertyIsEnumerable("hidden"));
console.log("pie_visible=" + pe.propertyIsEnumerable("visible"));
const peObj = { a: 1, b: 2 };
console.log("pie_missing=" + peObj.propertyIsEnumerable("c") + ",a=" + peObj.propertyIsEnumerable("a"));

// getOwnPropertyNames includes non-enumerable keys; Object.keys does not
const names: any = { a: 1, b: 2 };
Object.defineProperty(names, "hidden", { value: 3, enumerable: false });
console.log("gopn=" + Object.getOwnPropertyNames(names).sort().join(","));
console.log("gopn_arr=" + Object.getOwnPropertyNames([1, 2]).join(","));
console.log("gopn_empty=" + Object.getOwnPropertyNames({}).length);
console.log("gopn_str=" + Object.getOwnPropertyNames(new String("hi")).sort().join(","));

// getOwnPropertyDescriptors: keys are every own key, values are descriptors
const all = Object.getOwnPropertyDescriptors({ x: 1, get y() { return 2; } });
console.log("gopds_keys=" + Object.keys(all).sort().join(","));
console.log("gopds_x=" + all.x.value + ",w=" + all.x.writable + ",e=" + all.x.enumerable + ",c=" + all.x.configurable);
console.log("gopds_y=" + typeof all.y.get + ",set=" + all.y.set);

// a getter defined through defineProperty is non-enumerable when told so
const gobj: any = { a: 1, get b() { return 2; } };
Object.defineProperty(gobj, "c", { value: 3, writable: false, enumerable: false });
const gd = Object.getOwnPropertyDescriptors(gobj);
console.log("gopds_c=" + gd.c.value + ",w=" + gd.c.writable + ",e=" + gd.c.enumerable);
console.log("gopds_b_get=" + typeof gd.b.get + ",e=" + gd.b.enumerable);

// array length descriptor is special: writable, not enumerable, not configurable
const arrLen = [1, 2, 3];
const ld = Object.getOwnPropertyDescriptor(arrLen, "length") as any;
console.log("len_desc=" + ld.value + ":" + ld.writable + ":" + ld.enumerable + ":" + ld.configurable);
Object.defineProperty(arrLen, "length", { writable: false });
console.log("len_writable_after=" + Object.getOwnPropertyDescriptor(arrLen, "length")!.writable);
attempt("len_push_frozen", () => arrLen.push(4));
console.log("len_items=" + arrLen.join(","));

// Reflect.getOwnPropertyDescriptor answers undefined for a missing key
console.log("reflect_missing=" + Reflect.getOwnPropertyDescriptor({}, "b"));
console.log("reflect_found=" + JSON.stringify(Reflect.getOwnPropertyDescriptor({ v: 1 }, "v")));

// Reflect.defineProperty answers a boolean where Object.defineProperty throws
const refl: any = {};
Object.defineProperty(refl, "frozen", { value: 1, configurable: false });
console.log("reflect_define_ok=" + Reflect.defineProperty(refl, "fresh", { value: 2, configurable: true }));
console.log("reflect_define_refused=" + Reflect.defineProperty(refl, "frozen", { value: 9 }));
