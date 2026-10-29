// Symbols as property KEYS: where they are visible and where they are hidden,
// how the copying and reflection surfaces treat them, the ownKeys ordering,
// Proxy traps and ownKeys invariants, and symbol-keyed class members.

function bad(label: string, fn: () => any): void {
  try { fn(); console.log(label + "=no_throw"); }
  catch (e: any) { console.log(label + "=" + e.constructor.name); }
}
function keyOf(k: any): string {
  return typeof k === "symbol" ? "sym(" + String(k.description) + ")" : "str(" + String(k) + ")";
}
function keysOf(list: any[]): string {
  return list.map(keyOf).join(",");
}

const s1 = Symbol("alpha");
const s2 = Symbol.for("claude-keys-beta");

// --- a symbol-keyed property is invisible to the string-key surface ---
const src: any = { plain: 1, other: 2 };
src[s1] = "sym1";
src[s2] = "sym2";
console.log("keys=" + Object.keys(src).join(","));
console.log("values=" + Object.values(src).join(","));
console.log("entries=" + Object.entries(src).map((e: any) => e[0]).join(","));
console.log("own_property_names=" + Object.getOwnPropertyNames(src).join(","));
console.log("json=" + JSON.stringify(src));
let forIn = "";
for (const k in src) forIn += k + ";";
console.log("for_in=" + forIn);
console.log("json_symbol_value=" + JSON.stringify({ a: s1 }));
console.log("json_symbol_in_array=" + JSON.stringify([s1]));
console.log("json_symbol_key_only=" + JSON.stringify({ [s1]: 1 }));

// --- and visible to the symbol surface, read through the same key ---
const syms = Object.getOwnPropertySymbols(src);
console.log("symbols_count=" + syms.length + ":" + keysOf(syms));
console.log("symbol_read=" + src[s1] + ":" + src[s2]);
console.log("in_operator=" + (s1 in src) + ":" + (Symbol("alpha") in src));
console.log("has_own=" + Object.prototype.hasOwnProperty.call(src, s1));
console.log("string_twin_absent=" + (String(s1) in src));

// --- Reflect.ownKeys: integer indices, then strings, then symbols, each in insertion order ---
const ordered: any = { b: 1 };
ordered[s1] = 1;
ordered.a = 1;
ordered[2] = 1;
ordered[s2] = 1;
ordered[0] = 1;
const orderedKeys = Reflect.ownKeys(ordered);
console.log("ownKeys_kinds=" + orderedKeys.map((k: any) => typeof k).join(","));
console.log("ownKeys_order=" + keysOf(orderedKeys));

// --- a symbol key never collides with the string that describes it ---
const collide: any = {};
collide[s1] = "symbol_value";
collide[String(s1)] = "string_value";
collide.alpha = "plain";
console.log("no_collision=" + collide[s1] + ":" + collide[String(s1)] + ":" + collide.alpha);
console.log("collide_count=" + Reflect.ownKeys(collide).length);

// --- copying operations carry enumerable symbol keys ---
const assigned: any = Object.assign({}, src);
console.log("assign_string_keys=" + Object.keys(assigned).join(","));
console.log("assign_symbol_count=" + Object.getOwnPropertySymbols(assigned).length);
console.log("assign_values=" + assigned[s1] + ":" + assigned[s2]);
const spread: any = { ...src };
console.log("spread_symbol_count=" + Object.getOwnPropertySymbols(spread).length);
console.log("spread_values=" + spread[s1] + ":" + spread[s2]);
const { [s1]: pulled, ...rest } = src;
console.log("destructured=" + pulled + ":rest_keys=" + Reflect.ownKeys(rest).length + ":rest_symbols=" + Object.getOwnPropertySymbols(rest).length);
console.log("optional_chain=" + String(src?.[s1]) + ":" + String((null as any)?.[s1]));

// --- a NON-enumerable symbol key is skipped by copying, but still readable and reflected ---
const hidden: any = {};
Object.defineProperty(hidden, s1, { value: "nope", enumerable: false });
console.log("hidden_read=" + hidden[s1]);
console.log("hidden_own_symbols=" + Object.getOwnPropertySymbols(hidden).length);
console.log("hidden_assign=" + Object.getOwnPropertySymbols(Object.assign({}, hidden)).length);
console.log("hidden_spread=" + Object.getOwnPropertySymbols({ ...hidden }).length);
const descs: any = Object.getOwnPropertyDescriptors(hidden);
console.log("descriptors_symbols=" + Object.getOwnPropertySymbols(descs).length + ":enumerable=" + descs[s1].enumerable + ":value=" + descs[s1].value);
const restored: any = Object.defineProperties({}, descs);
console.log("restored_value=" + restored[s1] + ":own_symbols=" + Object.getOwnPropertySymbols(restored).length);
console.log("restored_enumerable=" + (Object.getOwnPropertyDescriptor(restored, s1) as any).enumerable);
const created: any = Object.create(null, { [s1]: { value: "created", enumerable: true } } as any);
console.log("create_descriptor_map=" + created[s1]);
const del: any = { [s1]: 1 };
console.log("delete_result=" + (delete del[s1]) + ":left=" + Object.getOwnPropertySymbols(del).length);

// --- symbol-keyed members of a class: an instance field, a method, a static and a tag getter ---
class WithSym {
  static [s2] = "static";
  [s1] = "field";
  [s2](): string { return "method"; }
  get [Symbol.toStringTag]() { return "Keyed"; }
}
const inst: any = new WithSym();
console.log("class_field=" + inst[s1]);
console.log("class_method=" + inst[s2]());
console.log("class_static=" + (WithSym as any)[s2]);
console.log("class_tag=" + Object.prototype.toString.call(inst));
console.log("class_instance_symbols=" + keysOf(Object.getOwnPropertySymbols(inst)));
console.log("class_proto_symbols=" + Object.getOwnPropertySymbols(WithSym.prototype).map(keyOf).sort().join(","));
console.log("class_method_enumerable=" + (Object.getOwnPropertyDescriptor(WithSym.prototype, s2) as any).enumerable);

// --- Proxy: every trap receives the symbol as a symbol, never as its string ---
const traps: string[] = [];
const target: any = { plain: 1 };
target[s1] = "one";
const p: any = new Proxy(target, {
  get(t: any, k: any, r: any) { traps.push("get " + keyOf(k)); return Reflect.get(t, k, r); },
  set(t: any, k: any, v: any, r: any) { traps.push("set " + keyOf(k)); return Reflect.set(t, k, v, r); },
  has(t: any, k: any) { traps.push("has " + keyOf(k)); return Reflect.has(t, k); },
  deleteProperty(t: any, k: any) { traps.push("delete " + keyOf(k)); return Reflect.deleteProperty(t, k); },
  defineProperty(t: any, k: any, d: any) { traps.push("define " + keyOf(k)); return Reflect.defineProperty(t, k, d); },
  getOwnPropertyDescriptor(t: any, k: any) { traps.push("gopd " + keyOf(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
  ownKeys(t: any) { traps.push("ownKeys"); return Reflect.ownKeys(t); },
});
console.log("proxy_read=" + p[s1]);
console.log("proxy_write=" + (p[s2] = "two"));
console.log("proxy_has=" + (s1 in p) + ":missing=" + (Symbol("nope") in p));
console.log("proxy_delete=" + Reflect.deleteProperty(p, s2));
console.log("proxy_define=" + Reflect.defineProperty(p, s2, { value: "redefined", enumerable: true, configurable: true, writable: true }));
console.log("proxy_descriptor=" + (Object.getOwnPropertyDescriptor(p, s2) as any).value);
console.log("proxy_traps=" + traps.join("|"));

traps.length = 0;
console.log("proxy_ownKeys=" + keysOf(Reflect.ownKeys(p)));
console.log("proxy_getOwnPropertySymbols=" + keysOf(Object.getOwnPropertySymbols(p)));
console.log("proxy_getOwnPropertyNames=" + Object.getOwnPropertyNames(p).join(","));
console.log("proxy_Object_keys=" + Object.keys(p).join(","));
console.log("proxy_ownKeys_traps=" + traps.join("|"));

console.log("reflect_get=" + Reflect.get(p, s1));
console.log("reflect_set=" + Reflect.set(p, s1, "one_prime") + ":" + Reflect.get(p, s1));
console.log("reflect_has=" + Reflect.has(p, s1));
console.log("reflect_gopd=" + JSON.stringify(Reflect.getOwnPropertyDescriptor(p, s1)));

// --- a trap may answer differently per key kind ---
const selective: any = new Proxy({}, {
  get(_t: any, k: any) { return typeof k === "symbol" ? "SYMBOL" : "STRING"; },
  has(_t: any, k: any) { return typeof k === "symbol"; },
  ownKeys() { return [Symbol.for("claude-keys-only")]; },
  getOwnPropertyDescriptor() { return { value: 1, enumerable: true, configurable: true }; },
});
console.log("selective_symbol_get=" + selective[s1]);
console.log("selective_string_get=" + selective.anything);
console.log("selective_has=" + (s1 in selective) + ":" + ("x" in selective));
console.log("selective_ownKeys=" + keysOf(Reflect.ownKeys(selective)));
console.log("selective_Object_keys=" + Object.keys(selective).length);
console.log("selective_symbols=" + keysOf(Object.getOwnPropertySymbols(selective)));

// --- ownKeys invariants: keep a non-configurable key, never duplicate, only property keys ---
const fixed: any = {};
Object.defineProperty(fixed, s1, { value: "fixed", configurable: false, enumerable: false, writable: false });
const dropping: any = new Proxy(fixed, { ownKeys() { return []; } });
bad("ownKeys_drops_non_configurable", () => Reflect.ownKeys(dropping));
const dup: any = new Proxy({}, {
  ownKeys() { return [s1, s1]; },
  getOwnPropertyDescriptor() { return { value: 1, enumerable: true, configurable: true }; },
});
bad("ownKeys_duplicate_symbol", () => Reflect.ownKeys(dup));
const badKeys: any = new Proxy({}, { ownKeys() { return [1 as any]; } });
bad("ownKeys_number_entry", () => Reflect.ownKeys(badKeys));

// --- for-in skips symbols even when the trap offers them; spread and assign keep them ---
const enumerable: any = new Proxy({ str: 1 }, {
  ownKeys(t: any) { return [s1, ...Reflect.ownKeys(t)]; },
  getOwnPropertyDescriptor(t: any, k: any) {
    return typeof k === "symbol" ? { value: 1, enumerable: true, configurable: true } : Reflect.getOwnPropertyDescriptor(t, k);
  },
});
let enumeratedKeys = "";
for (const k in enumerable) enumeratedKeys += keyOf(k) + ",";
console.log("for_in_proxy=" + enumeratedKeys);
console.log("spread_proxy_keys=" + keysOf(Reflect.ownKeys({ ...enumerable })));
console.log("assign_proxy_keys=" + keysOf(Reflect.ownKeys(Object.assign({}, enumerable))));
