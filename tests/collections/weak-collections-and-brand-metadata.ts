// Cross-runtime: WeakMap/WeakSet deterministic behaviour (set/add, get/has,
// delete, identity keys, which keys are refused), plus the metadata and brand
// rules shared by all four collections: the `size` accessor, Symbol.toStringTag,
// Symbol.species, the iterator aliases, and brand checks on foreign receivers.
// Nothing here inspects iteration over weak collections or GC timing.

function probe(label: string, fn: () => any): void {
  try {
    const r = fn();
    console.log(label + "=ok:" + String(r));
  } catch (e: any) {
    console.log(label + "=" + e.name);
  }
}

// --- WeakMap basics ---
const wm = new WeakMap<object, any>();
const k1 = {};
const k2 = {};
wm.set(k1, "value1").set(k2, 42);
console.log("wm_basic=" + wm.has(k1) + ":" + wm.get(k1) + ":" + wm.has(k2) + ":" + wm.get(k2));
console.log("wm_update_chain=" + (wm.set(k1, "v2") === wm) + ":" + wm.get(k1));
console.log("wm_delete=" + wm.delete(k1) + ":" + wm.has(k1) + ":" + String(wm.get(k1)) + ":again=" + wm.delete(k1));
console.log("wm_miss=" + String(wm.get({})) + ":" + wm.has({}));

// keys are identity, never structure; mutating a key changes nothing
const a = { same: 1 };
const b = { same: 1 };
const idm = new WeakMap<any, string>([[a, "first"]]);
console.log("identity=" + idm.get(a) + ":" + String(idm.get(b)));
a.same = 99;
console.log("identity_after_mutation=" + idm.get(a));

// set on an existing key updates; three keys in a chain
const w3 = new WeakMap<object, number>();
w3.set(a, 1).set(b, 2).set(a, 3);
console.log("update=" + w3.get(a) + ":" + w3.get(b) + ":" + w3.has(a));
console.log("delete_again=" + w3.delete(a) + ":" + w3.has(a) + ":" + String(w3.get(a)));

// constructor from pairs
const pairKey = {};
const wpair = new WeakMap<object, string>([[pairKey, "p"]]);
console.log("ctor_pairs=" + wpair.get(pairKey) + ":" + typeof new WeakMap(null as any));

// --- WeakSet basics ---
const ws = new WeakSet<object>();
const o1 = { id: 1 };
const o2 = { id: 2 };
console.log("ws_add_self=" + (ws.add(o1) === ws));
ws.add(o2);
console.log("ws_has=" + ws.has(o1) + ":" + ws.has(o2) + ":" + ws.has({ id: 1 }));
console.log("ws_delete=" + ws.delete(o1) + ":" + ws.has(o1) + ":again=" + ws.delete(o1));
const wsFromValues = new WeakSet<object>([o1]);
console.log("ws_ctor=" + wsFromValues.has(o1) + ":" + typeof new WeakSet(null as any));
class LoudWeakSet extends WeakSet<object> {
  static hits = 0;
  add(v: object): this { LoudWeakSet.hits++; return super.add(v); }
}
const loud = new LoudWeakSet([o1, { other: 1 }]);
console.log("subclass_adder=" + LoudWeakSet.hits + ":" + loud.has(o1));

// --- refused keys: writing throws, reading answers quietly ---
const held: any = { id: "held" };
const primitives: any[] = [1, "s", true, null, undefined, 10n, NaN, -0];
const weakMapWithHeld = new WeakMap<any, string>();
const weakSetWithHeld = new WeakSet<any>();
weakMapWithHeld.set(held, "v");
weakSetWithHeld.add(held);
let writes = "";
let adds = "";
for (const p of primitives) {
  try { weakMapWithHeld.set(p, "x"); writes += typeof p + ":ok "; }
  catch (e: any) { writes += typeof p + ":" + e.name + " "; }
  try { weakSetWithHeld.add(p); adds += typeof p + ":ok "; }
  catch (e: any) { adds += typeof p + ":" + e.name + " "; }
}
console.log("wm_set_primitives=" + writes.trim());
console.log("ws_add_primitives=" + adds.trim());
let reads = "";
let hases = "";
let dels = "";
let setHas = "";
for (const p of primitives) {
  reads += typeof p + ":" + String(weakMapWithHeld.get(p)) + " ";
  hases += typeof p + ":" + weakMapWithHeld.has(p) + " ";
  dels += typeof p + ":" + weakMapWithHeld.delete(p) + " ";
  setHas += typeof p + ":" + weakSetWithHeld.has(p) + ":" + weakSetWithHeld.delete(p) + " ";
}
console.log("wm_get_primitives=" + reads.trim());
console.log("wm_has_primitives=" + hases.trim());
console.log("wm_delete_primitives=" + dels.trim());
console.log("ws_has_delete_primitives=" + setHas.trim());
console.log("held_survived=" + weakMapWithHeld.get(held) + ":" + weakSetWithHeld.has(held));

// --- symbols: unregistered is a legal key; Symbol.for is refused ---
const uniq = Symbol("uniq");
const wsym = new WeakMap<any, any>();
probe("unique_set", () => wsym.set(uniq, "sym") === wsym);
probe("unique_get", () => wsym.get(uniq));
probe("unique_has", () => wsym.has(uniq));
probe("unique_delete", () => wsym.delete(uniq));
probe("wellknown_set", () => wsym.set(Symbol.iterator, "wk") === wsym);
probe("bare_symbol_add", () => new WeakSet<any>().add(Symbol()) instanceof WeakSet);
const reg = Symbol.for("claude-weak-key");
probe("registered_set", () => wsym.set(reg, "x"));
probe("registered_add", () => new WeakSet<any>().add(reg));
probe("registered_get", () => wsym.get(reg));
probe("registered_has", () => wsym.has(reg));
probe("registered_delete", () => wsym.delete(reg));
console.log("registry=" + (Symbol.keyFor(reg) === "claude-weak-key") + ":" + (Symbol.keyFor(uniq) === undefined) + ":" + (Symbol.keyFor(Symbol.iterator) === undefined));
const dupA = Symbol("dup");
const dupB = Symbol("dup");
const wdup = new WeakMap<any, string>([[dupA, "first"], [dupB, "second"]] as any);
console.log("dup_symbols=" + wdup.get(dupA) + ":" + wdup.get(dupB));
probe("object_key_set", () => new WeakSet<any>().add(function named() { return 1; }) instanceof WeakSet);
probe("array_key_set", () => new WeakSet<any>().add([]) instanceof WeakSet);

// --- brand checks: no collection accepts another's slot ---
const m = new Map([["a", 1]]);
const s = new Set([1]);
probe("get_plain", () => Map.prototype.get.call({} as any, "a"));
probe("set_plain", () => Map.prototype.set.call({} as any, "a", 1));
probe("clear_plain", () => Map.prototype.clear.call({} as any));
probe("forEach_plain", () => Map.prototype.forEach.call({} as any, () => {}));
probe("setadd_plain", () => Set.prototype.add.call({} as any, 1));
probe("setvalues_plain", () => Set.prototype.values.call({} as any));
probe("get_null", () => Map.prototype.get.call(null as any, "a"));
probe("get_string", () => Map.prototype.get.call("xy" as any, "a"));
probe("map_get_on_set", () => Map.prototype.get.call(s as any, 1));
probe("set_has_on_map", () => Set.prototype.has.call(m as any, "a"));
probe("map_foreach_on_set", () => Map.prototype.forEach.call(s as any, () => {}));
probe("weakmap_get_on_map", () => WeakMap.prototype.get.call(m as any, {}));
probe("map_get_on_weakmap", () => Map.prototype.get.call(new WeakMap() as any, {}));
probe("weakset_has_on_weakmap", () => WeakSet.prototype.has.call(wm as any, k2));
probe("weakset_has_on_set", () => WeakSet.prototype.has.call(new Set([held]) as any, held));
probe("map_proto_get", () => Map.prototype.get.call(Map.prototype as any, "a"));
probe("set_proto_has", () => Set.prototype.has.call(Set.prototype as any, 1));
probe("get_real", () => Map.prototype.get.call(m, "a"));
probe("has_real", () => Set.prototype.has.call(s, 1));
const inherited: any = Object.create(m);
probe("inherited_size", () => inherited.size);
probe("inherited_get", () => inherited.get("a"));
probe("weakmap_no_new", () => (WeakMap as any)());
probe("weakset_no_new", () => (WeakSet as any)());
probe("map_no_new", () => (Map as any)("x"));
probe("map_from_number", () => new Map(7 as any));
probe("map_from_object", () => new Map({} as any).size);
probe("map_from_undefined", () => new Map(undefined as any).size);

// --- `size` is a prototype accessor with a brand check ---
const sizeMap = new Map([["a", 1], ["b", 2]]);
const sizeSet = new Set([1, 2, 3]);
console.log("sizes=" + sizeMap.size + ":" + sizeSet.size);
console.log("own_size=" + Object.prototype.hasOwnProperty.call(sizeMap, "size") + ":names=" + Object.getOwnPropertyNames(sizeMap).length + ":symbols=" + Object.getOwnPropertySymbols(sizeMap).length);
console.log("json=" + JSON.stringify(sizeMap) + ":" + JSON.stringify(sizeSet) + ":in=" + ("size" in sizeMap));
const md: any = Object.getOwnPropertyDescriptor(Map.prototype, "size");
const sd: any = Object.getOwnPropertyDescriptor(Set.prototype, "size");
console.log("map_size_desc=" + typeof md.get + ":" + (md.set === undefined) + ":" + md.enumerable + ":" + md.configurable + ":" + md.get.name + ":" + md.get.length);
console.log("set_size_desc=" + typeof sd.get + ":" + sd.get.name + ":" + sd.enumerable + ":" + sd.configurable + ":distinct=" + (md.get !== sd.get));
probe("size_getter_plain", () => md.get.call({}));
probe("size_getter_on_set", () => md.get.call(sizeSet));
probe("set_size_getter_on_map", () => sd.get.call(sizeMap));
probe("size_getter_null", () => md.get.call(null));
probe("map_proto_size", () => (Map.prototype as any).size);
probe("set_proto_size", () => (Set.prototype as any).size);
class SizeSub extends Map<any, any> {}
const sub = new SizeSub([["x", 1]]);
console.log("sub_size=" + sub.size + ":own=" + Object.prototype.hasOwnProperty.call(sub, "size") + ":here=" + (Object.getOwnPropertyDescriptor(SizeSub.prototype, "size") === undefined));
const shadow: any = new Map([["a", 1]]);
Object.defineProperty(shadow, "size", { value: 99, configurable: true });
console.log("shadowed=" + shadow.size + ":real=" + md.get.call(shadow));
delete shadow.size;
console.log("unshadowed=" + shadow.size);
console.log("weak_size=" + ("size" in new WeakMap()) + ":" + ("size" in new WeakSet()) + ":" + ((new WeakMap() as any).size));

// --- Symbol.toStringTag ---
console.log("tags=" + Object.prototype.toString.call(new Map()) + ":" + Object.prototype.toString.call(new Set()) + ":" + Object.prototype.toString.call(new WeakMap()) + ":" + Object.prototype.toString.call(new WeakSet()));
console.log("proto_tag=" + Object.prototype.toString.call(Map.prototype) + ":" + Object.prototype.toString.call(Set.prototype));
const td: any = Object.getOwnPropertyDescriptor(Map.prototype, Symbol.toStringTag);
console.log("tag_desc=" + td.value + ":" + td.writable + ":" + td.enumerable + ":" + td.configurable);
console.log("tag_values=" + (Object.getOwnPropertyDescriptor(Set.prototype, Symbol.toStringTag) as any).value + ":" + (Object.getOwnPropertyDescriptor(WeakSet.prototype, Symbol.toStringTag) as any).value);
const tagged: any = new Map([["k", 1]]);
Object.defineProperty(tagged, Symbol.toStringTag, { value: "Custom", configurable: true });
console.log("overridden_tag=" + Object.prototype.toString.call(tagged) + ":" + (tagged instanceof Map) + ":" + tagged.size);

// --- Symbol.species and constructor metadata ---
console.log("species=" + (Map[Symbol.species] === Map) + ":" + (Set[Symbol.species] === Set));
const spd: any = Object.getOwnPropertyDescriptor(Map, Symbol.species);
console.log("species_desc=" + typeof spd.get + ":" + (spd.set === undefined) + ":" + spd.enumerable + ":" + spd.configurable + ":" + spd.get.name);
console.log("weak_species=" + (Object.getOwnPropertyDescriptor(WeakMap, Symbol.species) === undefined) + ":" + (Object.getOwnPropertyDescriptor(WeakSet, Symbol.species) === undefined));
console.log("lengths=" + Map.length + ":" + Map.name + ":" + Set.length + ":" + Set.name + ":" + WeakMap.length + ":" + WeakMap.name + ":" + WeakSet.length + ":" + WeakSet.name);
console.log("ctor_back=" + (Map.prototype.constructor === Map) + ":" + new Map().constructor.name + ":" + (Object.getPrototypeOf(Map) === Function.prototype));
console.log("ctor_protos=" + (Object.getPrototypeOf(WeakMap) === Function.prototype) + ":" + (Object.getPrototypeOf(WeakSet) === Function.prototype));

// --- Symbol.iterator aliases ---
console.log("iter_alias=" + ((Map.prototype as any)[Symbol.iterator] === Map.prototype.entries) + ":" + ((Set.prototype as any)[Symbol.iterator] === Set.prototype.values));
console.log("keys_values=" + (Map.prototype.keys === Map.prototype.values) + ":" + (Set.prototype.keys === Set.prototype.values));
console.log("iter_names=" + (Map.prototype as any)[Symbol.iterator].name + ":" + (Set.prototype as any)[Symbol.iterator].name);
console.log("weak_not_iterable=" + ((WeakMap.prototype as any)[Symbol.iterator] === undefined) + ":" + ((WeakSet.prototype as any)[Symbol.iterator] === undefined));
console.log("weak_no_clear=" + (typeof (WeakMap.prototype as any).clear) + ":" + (typeof (WeakSet.prototype as any).forEach));
console.log("proto_keys=" + Object.getOwnPropertySymbols(Map.prototype).map(String).sort().join(",") + ":" + Object.getOwnPropertySymbols(WeakMap.prototype).map(String).sort().join(","));

// --- descriptors: the methods are writable and configurable, not enumerable ---
let enumerated = "";
for (const k in new Map([["a", 1]])) enumerated += k + ",";
console.log("for_in=" + JSON.stringify(enumerated) + ":proto_keys=" + Object.keys(Map.prototype).length + ":" + Object.keys(Set.prototype).length);
const origHas = Set.prototype.has;
let patchedHits = 0;
(Set.prototype as any).has = function (this: any, v: any) { patchedHits++; return origHas.call(this, v); };
console.log("patched_has=" + new Set([1]).has(1) + ":" + patchedHits);
(Set.prototype as any).has = origHas;
console.log("restored_has=" + (Set.prototype.has === origHas));
