// Cross-runtime: the JSON.parse reviver (call order, `this`, key types, array holes,
// deletion, root replacement, mutation during the walk, the source-text context argument),
// plus the shape of the JSON namespace object and its two functions.

// --- reviver call order: children before parents, indices as strings, holder is an Array ---
const order: string[] = [];
JSON.parse('[10,[20,21],{"k":30}]', function (this: any, key: string, value: any) {
  order.push(key + "(" + typeof key + ")" + ":" + (Array.isArray(this) ? "arr" : typeof this));
  return value;
});
console.log("order=" + order.join("|"));
const nestedTrace: string[] = [];
JSON.parse('{"outer":[1,{"inner":2}]}', function (this: any, key: string, value: any) {
  nestedTrace.push(key + "=" + (typeof value === "object" && value !== null ? (Array.isArray(value) ? "[" + value.length + "]" : "{}") : String(value)));
  return value;
});
console.log("nested_trace=" + nestedTrace.join("|"));
const shapes: string[] = [];
JSON.parse("[1,2]", function (this: any, key: string, value: any) {
  shapes.push(key + ":" + (Array.isArray(this) ? "isArray" : "notArray") + ":len=" + this.length);
  return value;
});
console.log("holder_shapes=" + shapes.join("|"));
let rootInfo = "";
JSON.parse("[1]", function (this: any, key: string, value: any) {
  if (key === "") {
    rootInfo = "key=" + JSON.stringify(key) +
      ",holder_is_array=" + Array.isArray(this) +
      ",holder_keys=" + Object.keys(this).map((k) => JSON.stringify(k)).join("/") +
      ",value_is_array=" + Array.isArray(value);
  }
  return value;
});
console.log("root_info=" + rootInfo);
const parentRows: string[] = [];
JSON.parse('{"outer":{"inner":5}}', function (this: any, key: string, value: any, context: any) {
  parentRows.push(key + "|holder_keys=" + Object.keys(this).join("/") + "|source=" + ("source" in context ? String(context.source) : "-"));
  return value;
} as any);
console.log("parent_rows=" + parentRows.join(" ;; "));

// --- returning undefined deletes: a hole in arrays, a missing key in objects ---
const holed: any = JSON.parse("[1,2,3]", function (key: string, value: any) {
  return key === "1" ? undefined : value;
});
console.log("holed_length=" + holed.length);
console.log("holed_json=" + JSON.stringify(holed));
console.log("holed_has_index=" + (1 in holed) + ":" + Object.prototype.hasOwnProperty.call(holed, "1"));
console.log("holed_read=" + String(holed[1]));
console.log("holed_keys=" + Object.keys(holed).join(","));
let visitedHole = "";
holed.forEach((v: any, i: number) => { visitedHole += i + ":" + v + ","; });
console.log("holed_forEach=" + visitedHole);
console.log("holed_join=" + holed.join("-"));
console.log("holed_spread=" + [...holed].map(String).join(","));
const objHoled: any = JSON.parse('{"a":1,"b":2}', function (key: string, value: any) {
  return key === "b" ? undefined : value;
});
console.log("object_keys_after_delete=" + Object.keys(objHoled).join(","));
console.log("object_has_b=" + ("b" in objHoled));
const allGoneObj: any = JSON.parse('{"a":1,"b":2}', function (k: any, v: any) { return k === "" ? v : undefined; });
console.log("all_keys_deleted=" + JSON.stringify(allGoneObj));
const allGoneArr: any = JSON.parse("[1,2]", function (k: any, v: any) { return k === "" ? v : undefined; });
console.log("all_elements_deleted=" + JSON.stringify(allGoneArr) + ":len=" + allGoneArr.length);
const falsy: any = JSON.parse('{"a":1,"b":2,"c":3,"d":null}', function (k: any, v: any) {
  if (k === "a") return 0;
  if (k === "b") return "";
  if (k === "c") return false;
  if (k === "d") return null;
  return v;
});
console.log("falsy_and_null_kept=" + JSON.stringify(falsy) + ":has_d=" + ("d" in falsy));
const implicit: any = JSON.parse('{"a":1,"b":2}', function (k: any, v: any) {
  if (k === "a") return v;
  if (k === "") return v;
});
console.log("implicit_undefined_deletes=" + JSON.stringify(implicit));
const nestedDrop: any = JSON.parse('{"outer":{"keep":1,"drop":2}}', function (k: any, v: any) {
  return k === "drop" ? undefined : v;
});
console.log("nested_drop=" + JSON.stringify(nestedDrop));
const rootUndef = JSON.parse('{"a":1}', function (k: any, v: any) { return k === "" ? undefined : v; });
console.log("root_is_undefined=" + (rootUndef === undefined));
console.log("root_removed_string=" + String(rootUndef));
console.log("root_replaced=" + JSON.parse("[1,2]", (k, v) => (k === "" ? "ROOT" : v)));
console.log("root_deleted=" + String(JSON.parse("[1,2]", (k, v) => (k === "" ? undefined : v))));
const root = JSON.parse('{"x":2}', (key, value) => (key === "" ? value.x * 5 : value));
console.log("root_from_child=" + root);
const rootWrapped = JSON.parse('{"x":1}', (key, value) => (key === "" ? { wrapped: value } : value));
console.log("root_wrapped=" + JSON.stringify(rootWrapped));

// --- transforms: leaves rewritten in place, element types changed ---
const transformed: any = JSON.parse('{"a":1,"b":2,"c":3}', function (k: any, v: any) {
  return typeof v === "number" ? v * 2 : v;
});
console.log("transform_obj=" + JSON.stringify(transformed));
const plusTen: any = JSON.parse("[1,2,3]", function (key: string, value: any) {
  return typeof value === "number" ? value + 10 : value;
});
console.log("transform_arr=" + plusTen.join(","));
const doubled: any = JSON.parse("[1,2,3]", function (key: string, value: any) {
  return key === "" ? value : (typeof value === "number" ? value * 2 : value);
});
console.log("doubled=" + JSON.stringify(doubled));
const upper: any = JSON.parse('{"count":4,"items":[1,2,3],"label":"go"}', (key: string, value: unknown) => {
  if (key === "count") return (value as number) * 2;
  if (key === "label") return String(value).toUpperCase();
  return value;
});
console.log("upper_count=" + upper.count + ":items=" + upper.items.join(",") + ":label=" + upper.label);
const thisSeen: string[] = [];
const thisObj: any = JSON.parse('{"a":1,"b":2}', function (this: any, key: string, value: any) {
  if (key === "b") thisSeen.push(String(this.a));
  return typeof value === "number" ? value * 2 : value;
});
console.log("this_obj=" + JSON.stringify(thisObj));
console.log("this_seen=" + thisSeen.join("|"));

// --- mutation during the walk: later siblings see changes; growth and deletion behave as specified ---
const seenValues: string[] = [];
const mutated: any = JSON.parse("[1,2,3]", function (this: any, key: string, value: any) {
  if (key !== "") seenValues.push(key + "=" + value);
  if (key === "0") (this as any)[2] = 99;
  return value;
});
console.log("mutation_seen=" + seenValues.join(","));
console.log("mutation_result=" + JSON.stringify(mutated));
const grownKeys: string[] = [];
const grown: any = JSON.parse("[1,2]", function (this: any, key: string, value: any) {
  if (key !== "") grownKeys.push(key);
  if (key === "0" && Array.isArray(this)) (this as any).push("added");
  return value;
});
console.log("grown_keys=" + grownKeys.join(","));
console.log("grown_result=" + JSON.stringify(grown));
const deletedKeys: string[] = [];
const deleted: any = JSON.parse("[1,2,3]", function (this: any, key: string, value: any) {
  if (key !== "") deletedKeys.push(key + "=" + String(value));
  if (key === "0") delete (this as any)[2];
  return value;
});
console.log("deleted_keys=" + deletedKeys.join(","));
console.log("deleted_result=" + JSON.stringify(deleted));

// --- the revived arrays are ordinary and mutable; non-callable revivers are ignored ---
const plainArr: any = JSON.parse("[1,2]", (k, v) => v);
console.log("revived_array_proto=" + (Object.getPrototypeOf(plainArr) === Array.prototype));
console.log("revived_array_extensible=" + Object.isExtensible(plainArr));
plainArr.push(3);
console.log("after_push=" + JSON.stringify(plainArr));
console.log("reviver_number_ignored=" + JSON.stringify(JSON.parse("[1,2]", 5 as any)));
console.log("reviver_null_ignored=" + JSON.stringify(JSON.parse("[1,2]", null as any)));
console.log("reviver_object_ignored=" + JSON.stringify(JSON.parse("[1,2]", {} as any)));
console.log("reviver_undefined_ignored=" + JSON.stringify(JSON.parse("[1,2]", undefined)));
try {
  JSON.parse("[1,2,3]", function (key: string) { if (key === "1") throw new RangeError("stop"); return 0; });
  console.log("throwing_reviver=no_throw");
} catch (e: any) {
  console.log("throwing_reviver=" + e.name);
}

// --- source text context (third argument): primitives carry their exact source text ---
function sourceOf(text: string): string {
  let out = "MISSING";
  JSON.parse(text, function (key: string, value: any, context: any) {
    if (key === "") out = "value=" + String(value) + ",source=" + JSON.stringify(context.source);
    return value;
  } as any);
  return out;
}
console.log("src_integer=" + sourceOf("1"));
console.log("src_trailing_zero=" + sourceOf("1.0"));
console.log("src_trailing_zeros=" + sourceOf("1.500"));
console.log("src_exponent=" + sourceOf("1e2"));
console.log("src_exponent_plus=" + sourceOf("1E+2"));
console.log("src_negative_zero=" + sourceOf("-0"));
console.log("src_precision_lost=" + sourceOf("12345678901234567890"));
console.log("src_tiny=" + sourceOf("1e-400"));
console.log("src_huge=" + sourceOf("1e400"));
console.log("src_string=" + sourceOf('"ab"'));
console.log("src_escaped_string=" + sourceOf('"a\\u0041"'));
console.log("src_true=" + sourceOf("true"));
console.log("src_null=" + sourceOf("null"));
console.log("src_padded=" + sourceOf("  1  "));
function contextOf(text: string): string {
  let out = "MISSING";
  JSON.parse(text, function (key: string, value: any, context: any) {
    if (key === "") {
      out = "type=" + typeof context +
        ",has_source=" + ("source" in context) +
        ",source=" + String(context.source) +
        ",keys=" + Reflect.ownKeys(context).map(String).join("/");
    }
    return value;
  } as any);
  return out;
}
console.log("ctx_object=" + contextOf('{"a":1}'));
console.log("ctx_array=" + contextOf("[1]"));
console.log("ctx_empty_object=" + contextOf("{}"));
console.log("ctx_primitive=" + contextOf("7"));
let ctxShape = "";
JSON.parse("7", function (key: string, value: any, context: any) {
  if (key === "") {
    const desc: any = Object.getOwnPropertyDescriptor(context, "source");
    ctxShape = "proto_is_object=" + (Object.getPrototypeOf(context) === Object.prototype) +
      ",flags=" + desc.writable + desc.enumerable + desc.configurable +
      ",extensible=" + Object.isExtensible(context) +
      ",json=" + JSON.stringify(context);
  }
  return value;
} as any);
console.log("ctx_shape=" + ctxShape);
const seenContexts: any[] = [];
JSON.parse("[1,2]", function (key: string, value: any, context: any) {
  seenContexts.push(context);
  return value;
} as any);
console.log("ctx_count=" + seenContexts.length);
console.log("ctx_distinct=" + (seenContexts[0] === seenContexts[1]));
console.log("ctx_sources=" + seenContexts.map((c: any) => String(c.source)).join(","));
const rows: string[] = [];
JSON.parse('{"a":1.0,"b":[2e0,"s"],"c":{"d":3}}', function (key: string, value: any, context: any) {
  rows.push(key + "=" + ("source" in context ? JSON.stringify(context.source) : "-"));
  return value;
} as any);
console.log("ctx_structure=" + rows.join("|"));
let argCount = -1;
JSON.parse('{"a":1}', function () { argCount = arguments.length; return arguments[1]; } as any);
console.log("reviver_arity=" + argCount);
const bigText = '{"id":9007199254740993,"n":1}';
const kept: any = JSON.parse(bigText, function (key: string, value: any, context: any) {
  if (key === "id") return "RAW:" + context.source;
  return value;
} as any);
console.log("source_preserved=" + kept.id);
console.log("source_lossy_default=" + JSON.parse(bigText).id);
const replaced: string[] = [];
JSON.parse("[1,2]", function (key: string, value: any, context: any) {
  if (key !== "") replaced.push(key + ":" + String(context.source));
  return key === "" ? value : 999;
} as any);
console.log("original_sources_after_replace=" + replaced.join(","));

// --- the JSON namespace: an ordinary object with two methods, not callable or constructible ---
console.log("typeof_json=" + typeof JSON);
console.log("json_tag=" + Object.prototype.toString.call(JSON));
console.log("json_proto_is_object=" + (Object.getPrototypeOf(JSON) === Object.prototype));
console.log("json_extensible=" + Object.isExtensible(JSON));
console.log("json_frozen=" + Object.isFrozen(JSON));
console.log("json_own_names=" + Object.getOwnPropertyNames(JSON).filter((n) => n === "parse" || n === "stringify").sort().join(","));
console.log("json_enumerable_keys=" + JSON.stringify(Object.keys(JSON)));
let forIn = "";
for (const k in JSON) forIn += k + ",";
console.log("json_for_in=" + JSON.stringify(forIn));
function fnShape(label: string, f: any): void {
  console.log(label + "=" + typeof f + ":name=" + f.name + ":length=" + f.length +
    ":has_prototype=" + Object.prototype.hasOwnProperty.call(f, "prototype"));
}
fnShape("stringify", JSON.stringify);
fnShape("parse", JSON.parse);
function flags(label: string, o: any, k: any): void {
  const desc: any = Object.getOwnPropertyDescriptor(o, k);
  if (desc === undefined) { console.log(label + "=absent"); return; }
  console.log(label + "=" + desc.writable + ":" + desc.enumerable + ":" + desc.configurable);
}
flags("stringify_flags", JSON, "stringify");
flags("parse_flags", JSON, "parse");
flags("tag_flags", JSON, Symbol.toStringTag);
console.log("tag_value=" + JSON.stringify((JSON as any)[Symbol.toStringTag]));
function probe(label: string, fn: () => any): void {
  try { console.log(label + "=ok:" + String(fn())); }
  catch (e: any) { console.log(label + "=" + e.name); }
}
probe("new_stringify", () => new (JSON.stringify as any)({}));
probe("new_parse", () => new (JSON.parse as any)("1"));
probe("call_json", () => (JSON as any)());
probe("new_json", () => new (JSON as any)());
probe("parse_no_args", () => (JSON.parse as any)());
const detachedStringify = JSON.stringify;
const detachedParse = JSON.parse;
console.log("detached_stringify=" + detachedStringify({ a: 1 }));
console.log("detached_parse=" + detachedParse('{"a":1}').a);
console.log("stringify_call_null=" + JSON.stringify.call(null, { a: 1 }));
console.log("parse_call_number=" + JSON.parse.call(42 as any, "[1]").length);
console.log("stringify_apply_other=" + JSON.stringify.apply({ unrelated: true }, [{ b: 2 }] as any));
const host: any = { stringify: JSON.stringify, parse: JSON.parse };
console.log("hosted=" + host.stringify(host.parse('{"n":[1,2]}')));
console.log("stringify_extra_args=" + (JSON.stringify as any)({ a: 1 }, null, 0, "ignored", 99));
console.log("parse_extra_args=" + (JSON.parse as any)("[1]", undefined, "ignored").length);
console.log("stringify_no_args=" + String((JSON.stringify as any)()));
console.log("parse_length_name=" + JSON.parse.length + ":" + JSON.parse.name);
console.log("stringify_length_name=" + JSON.stringify.length + ":" + JSON.stringify.name);
const original = JSON.stringify;
let wraps = 0;
(JSON as any).stringify = function (...args: any[]) { wraps++; return original.apply(null, args as any); };
console.log("wrapped=" + JSON.stringify({ w: 1 }) + ":" + wraps);
(JSON as any).stringify = original;
console.log("restored=" + (JSON.stringify === original));
console.log("define_extra=" + Reflect.defineProperty(JSON, "probeExtra", { value: 1, configurable: true }));
console.log("extra_visible=" + ("probeExtra" in JSON) + ":enumerable=" + Object.keys(JSON).length);
console.log("delete_extra=" + Reflect.deleteProperty(JSON, "probeExtra"));
console.log("global_same=" + ((globalThis as any).JSON === JSON));
flags("global_flags", globalThis, "JSON");
console.log("global_typeof=" + typeof (globalThis as any).JSON);
