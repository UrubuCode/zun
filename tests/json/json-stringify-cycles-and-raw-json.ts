// Cross-runtime: cycle detection in JSON.stringify (what throws and what does not), and
// JSON.rawJSON / JSON.isRawJSON, the verbatim-text box that stringify embeds unchanged.

function attempt(label: string, fn: () => any): void {
  try { console.log(label + "=" + String(fn())); }
  catch (e: any) { console.log(label + "=" + e.name); }
}

// --- a repeated NON-cyclic reference is fine however many times it appears ---
const shared = { n: 1 };
attempt("shared_twice", () => JSON.stringify({ a: shared, b: shared }));
attempt("shared_in_array", () => JSON.stringify([shared, shared, shared]));
const diamond = { left: { v: shared }, right: { v: shared } };
attempt("diamond", () => JSON.stringify(diamond));
const sharedArr = [1, 2];
attempt("shared_array", () => JSON.stringify({ a: sharedArr, b: sharedArr }));
const leaf = { x: 0 };
attempt("repeated_leaf", () => JSON.stringify([{ leaf }, { leaf }, [leaf, leaf]]));
const fresh: any = { plain: 1 };
Object.defineProperty(fresh, "made", { get() { return { made: true }; }, enumerable: true });
attempt("fresh_getter_not_cycle", () => JSON.stringify([fresh, fresh]));

// --- a reference back into the stack being serialised is a TypeError ---
const selfObj: any = { a: 1 };
selfObj.me = selfObj;
attempt("object_self", () => JSON.stringify(selfObj));
const selfArr: any = [1, 2];
selfArr.push(selfArr);
attempt("array_self", () => JSON.stringify(selfArr));
const p: any = {};
const q: any = { p };
p.q = q;
attempt("indirect_cycle", () => JSON.stringify(p));
const c1: any = {}; const c2: any = { c1 }; const c3: any = { c2 };
c1.c3 = c3;
attempt("long_cycle", () => JSON.stringify(c3));
const holder: any = { list: [] };
holder.list.push(holder);
attempt("cycle_in_array_element", () => JSON.stringify(holder));
const late: any = { first: "ok", second: 2 };
late.third = late;
attempt("late_cycle", () => JSON.stringify(late));
const getterCycle: any = { plain: 1 };
Object.defineProperty(getterCycle, "up", { get() { return getterCycle; }, enumerable: true });
attempt("getter_cycle", () => JSON.stringify(getterCycle));
const cyc: any = { a: 1 };
const cycProxy: any = new Proxy(cyc, {});
cyc.self = cycProxy;
attempt("proxy_cycle", () => JSON.stringify(cycProxy));
const mapped = new Map<any, any>();
mapped.set("self", mapped);
attempt("map_cycle_is_empty", () => JSON.stringify(mapped));
attempt("map_in_object_cycle_free", () => JSON.stringify({ m: mapped }));
let deep: any = { v: 0 };
for (let i = 1; i <= 12; i++) deep = { v: i, inner: deep };
attempt("deep_no_cycle_length", () => JSON.stringify(deep).length);

// --- a cycle introduced by toJSON, or by the replacer ---
const parent: any = { name: "p" };
const child: any = { toJSON() { return { up: parent }; } };
parent.child = child;
attempt("cycle_via_tojson", () => JSON.stringify(parent));
attempt("tojson_wrapper_alone", () => JSON.stringify(child));
const plain: any = { a: 1 };
attempt("cycle_via_replacer", () => JSON.stringify(plain, function (k: any, v: any) {
  return k === "a" ? plain : v;
}));

// --- cycles removed by a replacer or by toJSON serialise fine ---
attempt("cycle_broken_by_replacer", () => JSON.stringify(selfObj, function (k: any, v: any) {
  return k === "me" ? "<self>" : v;
}));
const cut: any = { a: 1, toJSON() { return { a: this.a }; } };
cut.self = cut;
attempt("cycle_cut_by_tojson", () => JSON.stringify(cut));

// --- the engine recovers after a failed attempt once the cycle is removed ---
attempt("before_fix", () => JSON.stringify(selfObj));
delete selfObj.me;
attempt("after_fix", () => JSON.stringify(selfObj));

// --- JSON.rawJSON: the box, its shape and its flags ---
const J: any = JSON;
const raw: any = J.rawJSON("123");
console.log("raw_typeof=" + typeof raw);
console.log("raw_is_raw=" + J.isRawJSON(raw));
console.log("raw_proto_is_null=" + (Object.getPrototypeOf(raw) === null));
console.log("raw_own_keys=" + Reflect.ownKeys(raw).map(String).join(","));
console.log("raw_value=" + JSON.stringify(raw.rawJSON));
console.log("raw_frozen=" + Object.isFrozen(raw) + ":sealed=" + Object.isSealed(raw));
console.log("raw_extensible=" + Object.isExtensible(raw));
const rawDesc: any = Object.getOwnPropertyDescriptor(raw, "rawJSON");
console.log("raw_prop_flags=" + rawDesc.writable + ":" + rawDesc.enumerable + ":" + rawDesc.configurable);
console.log("raw_write_refused=" + Reflect.set(raw, "rawJSON", "999") + ":still=" + raw.rawJSON);
console.log("raw_add_refused=" + Reflect.defineProperty(raw, "extra", { value: 1 }));
console.log("raw_has_toJSON=" + ("toJSON" in raw));
attempt("raw_string_coercion", () => String(raw));
console.log("raw_rawJSON_is_string=" + (typeof J.rawJSON("1").rawJSON));
console.log("raw_distinct_boxes=" + (J.rawJSON("1") === J.rawJSON("1")));
console.log("raw_set_size=" + new Set([J.rawJSON("1"), J.rawJSON("1")]).size);

// --- isRawJSON recognises the internal slot, not the shape ---
console.log("lookalike_plain=" + J.isRawJSON({ rawJSON: "123" }));
console.log("lookalike_frozen_null_proto=" + J.isRawJSON(Object.freeze(Object.assign(Object.create(null), { rawJSON: "123" }))));
console.log("proxy_of_raw=" + J.isRawJSON(new Proxy(raw, {})));
console.log("is_string=" + J.isRawJSON("123"));
console.log("is_number=" + J.isRawJSON(123));
console.log("is_null=" + J.isRawJSON(null));
console.log("is_undefined=" + J.isRawJSON(undefined));
console.log("is_no_arg=" + J.isRawJSON());
console.log("is_array=" + J.isRawJSON([]));
console.log("is_parsed_number=" + J.isRawJSON(JSON.parse("123")));
console.log("is_parsed_object=" + J.isRawJSON(JSON.parse('{"a":1}')));

// --- stringify embeds the text verbatim: no quotes, no re-parsing, at any depth ---
console.log("raw_top=" + JSON.stringify(raw));
console.log("raw_in_object=" + JSON.stringify({ n: raw }));
console.log("raw_in_array=" + JSON.stringify([raw, raw]));
console.log("raw_nested=" + JSON.stringify({ a: { b: [raw] } }));
console.log("raw_with_indent=" + JSON.stringify(JSON.stringify({ n: raw }, null, 2)));
console.log("raw_lookalike_is_object=" + JSON.stringify({ v: { rawJSON: "123" } }));

// --- a value no double can hold survives: big integers, exponents, escaped strings ---
const big = "12345678901234567890";
console.log("bigint_via_raw=" + JSON.stringify({ id: J.rawJSON(big) }));
console.log("bigint_via_number=" + JSON.stringify({ id: Number(big) }));
console.log("raw_round_trip_text=" + (JSON.stringify({ id: J.rawJSON(big) }) === '{"id":' + big + "}"));
console.log("raw_exp_text=" + JSON.stringify({ v: J.rawJSON("1E1000") }));
function rawText(text: string): string {
  try { return "ok:" + JSON.stringify({ v: J.rawJSON(text) }); }
  catch (e: any) { return e.name; }
}
console.log("raw_number=" + rawText("1"));
console.log("raw_negative_exp=" + rawText("-1.5e10"));
console.log("raw_exponent=" + rawText("1E1000"));
console.log("raw_string=" + rawText('"abc"'));
console.log("raw_escaped_string=" + rawText('"a\\u0041b"'));
console.log("raw_true=" + rawText("true"));
console.log("raw_false=" + rawText("false"));
console.log("raw_null=" + rawText("null"));

// --- structures and malformed text are refused with SyntaxError ---
console.log("raw_object=" + rawText('{"a":1}'));
console.log("raw_array=" + rawText("[1]"));
console.log("raw_empty=" + rawText(""));
console.log("raw_space_before=" + rawText(" 1"));
console.log("raw_space_after=" + rawText("1 "));
console.log("raw_newline=" + rawText("\n1"));
console.log("raw_tab_only=" + rawText("\t"));
console.log("raw_undefined_text=" + rawText("undefined"));
console.log("raw_nan=" + rawText("NaN"));
console.log("raw_infinity=" + rawText("Infinity"));
console.log("raw_leading_plus=" + rawText("+1"));
console.log("raw_leading_zero=" + rawText("01"));
console.log("raw_hex=" + rawText("0x1"));
console.log("raw_single_quotes=" + rawText("'a'"));
console.log("raw_trailing_comma=" + rawText("1,"));
console.log("raw_two_values=" + rawText("1 2"));

// --- the argument is coerced with ToString first; a symbol cannot be coerced ---
attempt("from_number", () => JSON.stringify({ v: J.rawJSON(1) }));
attempt("from_bigint", () => JSON.stringify({ v: J.rawJSON(10n) }));
attempt("from_toString", () => JSON.stringify({ v: J.rawJSON({ toString() { return "7"; } }) }));
attempt("from_undefined", () => JSON.stringify({ v: J.rawJSON(undefined) }));
attempt("from_symbol", () => JSON.stringify({ v: J.rawJSON(Symbol("s")) }));
attempt("no_arg", () => String(J.rawJSON()));

// --- a replacer or toJSON may hand back a box; the metadata of the two functions ---
console.log("from_replacer=" + JSON.stringify({ a: 1 }, (k, v) => (k === "a" ? J.rawJSON("42") : v)));
console.log("from_tojson=" + JSON.stringify({ a: { toJSON() { return J.rawJSON("5"); } } }));
attempt("rawjson_array_refused", () => J.rawJSON("[1,2]"));
console.log("rawJSON_meta=" + typeof J.rawJSON + ":" + J.rawJSON.name + ":" + J.rawJSON.length);
console.log("isRawJSON_meta=" + typeof J.isRawJSON + ":" + J.isRawJSON.name + ":" + J.isRawJSON.length);
