// Cross-runtime: how JSON.stringify is steered by its replacer (function or property
// list) and by toJSON, including the order in which those hooks run.

// --- replacer function: call sequence, `this`, key types, root key ---
const data: any = { a: 1, nested: { b: 2 }, list: [10, 20] };
const visits: string[] = [];
const out = JSON.stringify(data, function (this: any, k: any, v: any) {
  visits.push("[" + JSON.stringify(k) + "|" + typeof k + "|" + (this === data ? "root" : Array.isArray(this) ? "arr" : "obj") + "]");
  return v;
});
console.log("visits=" + visits.join(""));
console.log("visit_out=" + out);
console.log("root_key_is_empty=" + (visits[0] === '[""|string|root]'));
let rootHolderKeys = "";
let rootValue = "";
JSON.stringify(42, function (this: any, k: any, v: any) {
  rootHolderKeys = Object.keys(this).join(",");
  rootValue = String(v) + ":" + typeof v;
  return v;
});
console.log("root_holder_keys=" + rootHolderKeys);
console.log("root_value=" + rootValue);
const idx: string[] = [];
JSON.stringify(["x", "y", "z"], function (k: any, v: any) {
  if (k !== "") idx.push(k + "/" + typeof k);
  return v;
});
console.log("array_keys_are_strings=" + idx.join(","));

// --- replacer return values: undefined drops or nulls, replacements are walked ---
console.log("drop_obj=" + JSON.stringify({ a: 1, b: 2 }, function (k: any, v: any) { return k === "b" ? undefined : v; }));
console.log("drop_arr_null=" + JSON.stringify([1, 2, 3], function (k: any, v: any) { return k === "1" ? undefined : v; }));
console.log("drop_root=" + String(JSON.stringify({ a: 1 }, function () { return undefined; })));
console.log("replace_with_obj=" + JSON.stringify({ a: 1 }, function (k: any, v: any) { return k === "a" ? { deep: [1, 2] } : v; }));
console.log("prim_to_obj=" + JSON.stringify(1, function (k: any, v: any) { return k === "" ? { n: v } : v; }));
console.log("obj_to_prim=" + JSON.stringify({ a: { b: 1 } }, function (k: any, v: any) { return k === "a" ? 0 : v; }));
console.log("multiply=" + JSON.stringify({ a: 1, nested: { b: 2 } }, function (k: any, v: any) { return typeof v === "number" ? v * 10 : v; }));
console.log("bool_to_text=" + JSON.stringify({ keep: 1, active: true }, function (k: any, v: any) { return k === "active" ? (v ? "yes" : "no") : v; }));
const kinds: string[] = [];
JSON.stringify({ u: undefined, f: function () { /* fn */ }, s: Symbol("x"), n: null }, function (k: any, v: any) {
  if (k !== "") kinds.push(k + ":" + typeof v);
  return v;
});
console.log("unserialisable_seen=" + kinds.join(","));
console.log("rescued=" + JSON.stringify({ u: undefined, f: function () { /* fn */ } }, function (k: any, v: any) {
  return k === "" ? v : typeof v === "undefined" ? "was-undefined" : typeof v === "function" ? "was-function" : v;
}));
console.log("replacer_with_space=" + JSON.stringify(JSON.stringify({ a: 1, b: 2 }, function (k: any, v: any) { return k === "b" ? 9 : v; }, 2)));
try {
  JSON.stringify({ a: 1 }, function (k: any, v: any) { if (k === "a") throw new RangeError("nope"); return v; });
  console.log("throwing_replacer=no_throw");
} catch (e: any) {
  console.log("throwing_replacer=" + e.name);
}

// --- replacer values that are not callable and not arrays are ignored ---
console.log("ignored_number=" + JSON.stringify({ a: 1 }, 5 as any));
console.log("ignored_string=" + JSON.stringify({ a: 1 }, "a" as any));
console.log("ignored_null=" + JSON.stringify({ a: 1 }, null));
console.log("ignored_object=" + JSON.stringify({ a: 1 }, { a: true } as any));
console.log("ignored_true=" + JSON.stringify({ a: 1 }, true as any));
console.log("ignored_symbol=" + JSON.stringify({ a: 1 }, Symbol("r") as any));

// --- what counts as a callable replacer and what counts as a property list ---
const shape: any = { a: 1, b: { a: 2, c: 3 }, d: [4, 5] };
function shapeOf(replacer?: any, space?: any): string {
  try { return String((JSON.stringify as any)(shape, replacer, space)); }
  catch (e: any) { return e.name; }
}
console.log("callable_fn=" + shapeOf(function (k: string, v: any) { return typeof v === "number" ? v * 10 : v; }));
console.log("callable_arrow=" + shapeOf((k: string, v: any) => (k === "c" ? undefined : v)));
console.log("callable_bound=" + shapeOf((function (k: string, v: any) { return v; }).bind(null)));
console.log("callable_class=" + shapeOf(class Nope { }));
const callableProxy: any = new Proxy(function (k: string, v: any) { return k === "a" ? "PROXIED" : v; }, {});
console.log("callable_proxy=" + shapeOf(callableProxy));
const arrayProxy: any = new Proxy(["a"], {});
console.log("array_proxy_is_list=" + shapeOf(arrayProxy));
console.log("array_proxy_isArray=" + Array.isArray(arrayProxy));
const revoked = Proxy.revocable(["a"], {});
revoked.revoke();
console.log("revoked_list=" + shapeOf(revoked.proxy));
console.log("array_like_ignored=" + shapeOf({ 0: "a", length: 1 }));
console.log("arguments_ignored=" + shapeOf((function () { return arguments; })("a")));
console.log("typed_array_ignored=" + shapeOf(new Uint8Array([1])));
console.log("string_ignored=" + shapeOf("a"));
console.log("set_ignored=" + shapeOf(new Set(["a"])));
console.log("array_proto_object_ignored=" + shapeOf(Object.create(Array.prototype)));
console.log("number_ignored=" + shapeOf(42));
console.log("null_ignored=" + shapeOf(null));
console.log("undefined_ignored=" + shapeOf(undefined));
console.log("boolean_ignored=" + shapeOf(true));
console.log("symbol_ignored=" + shapeOf(Symbol("s")));
class KeyList extends Array<string> { }
const keyList = KeyList.from(["a"]) as any;
console.log("array_subclass_is_list=" + shapeOf(keyList));
console.log("array_subclass_isArray=" + Array.isArray(keyList));

// --- property list: order follows the list, duplicates and missing keys collapse ---
const value: any = { b: 2, a: 1, c: 3, nested: { z: 9, a: 8 } };
const keys: any[] = ["c", "a", "missing", "a", new String("nested")];
console.log("list_order=" + JSON.stringify(value, keys));
console.log("list_applies_to_each=" + JSON.stringify([value, { a: 4, c: 5 }], keys));
console.log("list_numeric_keys=" + JSON.stringify({ "1": "one", "01": "zero-one", a: "aye" }, [1, "01", "a"]));
console.log("list_wrapper_numeric=" + JSON.stringify({ a: "A", b: "B", "1": "one", null: "nil" }, [new String("b"), new Number(1), Symbol("ignored"), null, "a"] as any));
console.log("list_wrapper_numeric_rows=" + JSON.stringify([{ a: 1, b: 2, "1": 3, null: 4 }], [new String("b"), new Number(1), Symbol("ignored"), null, "a"] as any));
console.log("list_filters=" + JSON.stringify({ a: 1, b: 2, c: 3, d: 4 }, ["a", "c"]));
console.log("list_empty=" + JSON.stringify({ a: 1, b: 2, c: 3, d: 4 }, []));
console.log("list_nested_only_top=" + JSON.stringify({ x: { a: 1, b: 2 }, y: 3 }, ["x", "a"]));
const holedList: any = ["a", , "c"];
console.log("list_holes_walked=" + shapeOf(holedList));
const withInherited: any = ["a"];
Object.setPrototypeOf(withInherited, { 1: "b" });
console.log("list_inherited_entry=" + shapeOf(withInherited));
function listOf(entries: any[]): string {
  return String(JSON.stringify({ a: 1, b: 2, 3: "three" }, entries as any));
}
console.log("entry_string=" + listOf(["a"]));
console.log("entry_number=" + listOf([3]));
console.log("entry_number_wrapper=" + listOf([new Number(3)]));
console.log("entry_string_wrapper=" + listOf([new String("a")]));
console.log("entry_boolean_skipped=" + listOf([true, "a"]));
console.log("entry_null_skipped=" + listOf([null, "a"]));
console.log("entry_undefined_skipped=" + listOf([undefined, "a"]));
console.log("entry_symbol_skipped=" + listOf([Symbol("a"), "a"]));
console.log("entry_object_skipped=" + listOf([{ toString() { return "a"; } }, "b"]));
console.log("entry_duplicate=" + listOf(["a", "a", "b"]));
console.log("entry_empty=" + listOf([]));
console.log("entry_bigint_skipped=" + listOf([1n as any, "a"]));

// --- toJSON: looked up on the value, given the key, its result is serialised again ---
const d = new Date(Date.UTC(2024, 0, 2, 3, 4, 5, 678));
console.log("date_top=" + JSON.stringify(d));
console.log("date_in_obj=" + JSON.stringify({ when: d }));
console.log("date_deep=" + JSON.stringify({ a: { b: [{ c: d }] } }));
console.log("date_iso_from_key=" + JSON.stringify({ d: new Date(Date.UTC(2020, 5, 15)) }));
const keySpy: any = { toJSON: function (k: any) { return "key:" + String(k); } };
console.log("key_top=" + JSON.stringify(keySpy));
console.log("key_in_obj=" + JSON.stringify({ myField: keySpy }));
console.log("key_in_arr=" + JSON.stringify([keySpy, keySpy]));
console.log("key_nested=" + JSON.stringify({ outer: { inner: keySpy } }));
const recursive: any = { toJSON: function () { return { inner: d, list: [1, 2] }; } };
console.log("tojson_result_recursive=" + JSON.stringify(recursive));
const chained: any = { toJSON: function () { return keySpy; } };
console.log("tojson_chained=" + JSON.stringify({ f: chained }));
class Money {
  amount: number;
  constructor(a: number) { this.amount = a; }
  toJSON() { return this.amount + " BRL"; }
}
console.log("class_top=" + JSON.stringify(new Money(10)));
console.log("class_in_obj=" + JSON.stringify({ price: new Money(20) }));
console.log("class_in_arr=" + JSON.stringify([new Money(1), new Money(2)]));
console.log("ret_num=" + JSON.stringify({ v: { toJSON: function () { return 42; } } }));
console.log("ret_null=" + JSON.stringify({ v: { toJSON: function () { return null; } } }));
console.log("ret_bool=" + JSON.stringify({ v: { toJSON: function () { return true; } } }));
console.log("ret_arr=" + JSON.stringify({ v: { toJSON: function () { return [1]; } } }));
console.log("non_function_toJSON_ignored=" + JSON.stringify({ toJSON: 5, other: 1 }));
function Base(this: any) { this.v = 3; }
(Base as any).prototype.toJSON = function () { return "from_proto"; };
console.log("inherited_toJSON=" + JSON.stringify({ b: new (Base as any)() }));
console.log("tojson_with_space=" + JSON.stringify(JSON.stringify({ d: d }, null, 1)));

// --- ordering of the hooks: toJSON runs before the replacer sees the value ---
const hookLog: string[] = [];
const hooked: any = {
  a: 1,
  child: {
    toJSON(key: string) {
      hookLog.push("toJSON:" + key);
      return { b: 2, c: 3 };
    },
  },
};
const hookedOut = JSON.stringify(hooked, (key, value) => {
  hookLog.push("rep:" + key + ":" + (value && value.b ? "child" : typeof value));
  if (key === "c") return undefined;
  return value;
});
console.log("hook_out=" + hookedOut);
console.log("hook_log=" + hookLog.join("|"));
const seenAfter: string[] = [];
const item: any = {
  item: {
    toJSON(key: string) { seenAfter.push("toJSON:" + key); return { x: 3 }; },
  },
};
const afterOut = JSON.stringify(item, (key, v) => {
  if (key === "item") seenAfter.push("replacer:" + JSON.stringify(v));
  return v;
});
console.log("tojson_before_replacer_out=" + afterOut);
console.log("tojson_before_replacer_log=" + seenAfter.join("|"));
const wrapped: any = { toJSON: function () { return "FROM_TOJSON"; } };
console.log("replacer_sees_tojson=" + JSON.stringify({ w: wrapped }, function (k: any, v: any) {
  return k === "w" ? "saw:" + String(v) : v;
}));
console.log("replacer_returns_tojson=" + JSON.stringify({ a: 1 }, function (k: any, v: any) {
  return k === "a" ? { toJSON() { return "inner"; } } : v;
}));

// --- toJSON on built-in prototypes can be installed and removed ---
(BigInt.prototype as any).toJSON = function () { return this.toString() + "n"; };
console.log("bigint_with_tojson=" + JSON.stringify({ x: 5n, arr: [1n, 2n] }));
delete (BigInt.prototype as any).toJSON;
try {
  JSON.stringify(1n);
  console.log("bigint_after_cleanup=no_throw");
} catch (e: any) {
  console.log("bigint_after_cleanup=" + e.name);
}
