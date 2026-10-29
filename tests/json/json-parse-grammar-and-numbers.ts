// Cross-runtime: JSON.parse grammar (what is refused, what is accepted), number forms,
// string escapes, whitespace and argument coercion, own-property semantics, deep nesting,
// and round-trip equality with JSON.stringify. Errors are reported by e.name only.

function p(label: string, text: any): void {
  try {
    const v = JSON.parse(text);
    console.log(label + "=ok:" + typeof v + ":" + JSON.stringify(v));
  } catch (e: any) {
    console.log(label + "=" + e.name);
  }
}

// --- the grammar is stricter than JavaScript: commas, quoting, and named values ---
p("trailing_comma_obj", '{"a":1,}');
p("trailing_comma_arr", "[1,2,]");
p("leading_comma_arr", "[,1]");
p("double_comma", "[1,,2]");
p("single_quoted_string", "'x'");
p("single_quoted_key", "{'a':1}");
p("unquoted_key", "{a:1}");
p("NaN", "NaN");
p("Infinity", "Infinity");
p("neg_Infinity", "-Infinity");
p("undefined", "undefined");
p("null_ok", "null");
p("true_ok", "true");
p("false_ok", "false");
p("comment_line", "1 // x");
p("comment_block", "/* x */ 1");
p("two_values", "1 2");
p("empty_input", "");
p("colon_missing", '{"a" 1}');
p("empty_object", "{}");
p("empty_array", "[]");
p("nested_ok", '{"a":[1,{"b":null}]}');
p("duplicate_key_last_wins", '{"a":1,"a":2}');
p("bom_prefix", String.fromCharCode(0xfeff) + "1");

// --- number grammar: no plus sign, no leading zeros, no bare dot, no hex or separators ---
p("plus_one", "+1");
p("leading_dot", ".5");
p("trailing_dot", "1.");
p("leading_zero", "01");
p("negative_leading_zero", "-01");
p("zero_dot_five", "0.5");
p("hex", "0x10");
p("octal", "0o10");
p("underscore", "1_000");
p("exponent_plus_ok", "1e+2");
p("exponent_bare_ok", "1e2");
p("exponent_no_digits", "1e");
p("minus_alone", "-");
p("double_minus", "--1");
p("negative_dot", "-.5");

// --- number values: signed zero, exponents, precision, rounding ---
const negZero = JSON.parse("-0");
console.log("neg_zero=" + negZero);
console.log("neg_zero_is=" + Object.is(negZero, -0));
console.log("neg_zero_eq_zero=" + (negZero === 0));
console.log("neg_zero_in_arr_is=" + Object.is(JSON.parse("[-0]")[0], -0));
console.log("pos_zero_is=" + Object.is(JSON.parse("0"), 0));
console.log("zero=" + JSON.parse("0"));
console.log("neg_five=" + JSON.parse("-5"));
console.log("max_safe=" + JSON.parse("9007199254740991"));
console.log("exp_lower=" + JSON.parse("1e3"));
console.log("exp_upper=" + JSON.parse("1E3"));
console.log("exp_plus=" + JSON.parse("1e+3"));
console.log("exp_minus=" + JSON.parse("1e-3"));
console.log("exp_frac=" + JSON.parse("1.5e2"));
console.log("exp_zero=" + JSON.parse("5e0"));
console.log("exp_neg_base=" + JSON.parse("-2.5e3"));
console.log("exp_big=" + JSON.parse("1e21"));
console.log("exp_overflow=" + JSON.parse("1e400"));
console.log("exp_underflow=" + JSON.parse("1e-400"));
console.log("frac=" + JSON.parse("0.5"));
console.log("precision_loss=" + JSON.parse("9007199254740993"));
console.log("many_digits=" + JSON.parse("0.1000000000000000055511151231257827"));
console.log("long_int=" + JSON.parse("123456789012345678901234567890"));
console.log("point_one=" + JSON.parse("0.1"));
console.log("whole_point_zero=" + JSON.parse("1.0"));

// --- whitespace: exactly tab, LF, CR and space; everything else is refused ---
p("ws_space", " 1 ");
p("ws_tab", "\t1\t");
p("ws_lf", "\n1\n");
p("ws_cr", "\r1\r");
p("ws_mixed", " \t\r\n [ 1 , 2 ] \n\t ");
p("ws_inside_object", '{ "a" : 1 , "b" : 2 }');
function ws(label: string, code: number): void {
  p(label, String.fromCharCode(code) + "1");
}
ws("ws_vertical_tab", 0x0b);
ws("ws_form_feed", 0x0c);
ws("ws_nbsp", 0x00a0);
ws("ws_bom", 0xfeff);
ws("ws_line_separator", 0x2028);
ws("ws_paragraph_separator", 0x2029);
ws("ws_ideographic_space", 0x3000);
ws("ws_nul", 0x0000);
p("ws_inside_string_kept", '"  a  "');
p("ws_only_space_string", '" "');

// --- the first argument goes through ToString; objects may supply the text ---
p("arg_number", 1);
p("arg_float", 1.5);
p("arg_true", true);
p("arg_false", false);
p("arg_null", null);
p("arg_undefined", undefined);
p("arg_array_one", [1]);
p("arg_array_empty", []);
p("arg_array_two", [1, 2]);
p("arg_nested_array", [[1]]);
p("arg_plain_object", {});
p("arg_string_object", new String("[1,2]"));
p("arg_tostring_json", { toString() { return '{"from":"toString"}'; } });
p("arg_valueof_ignored", { valueOf() { return "1"; }, toString() { return "2"; } });
p("arg_toprimitive_hook", { [Symbol.toPrimitive](h: any) { return h === "string" ? '"str"' : '"other"'; } });
try {
  JSON.parse(Symbol("s") as any);
  console.log("arg_symbol=no_throw");
} catch (e: any) {
  console.log("arg_symbol=" + e.name);
}
const fa = JSON.parse("[1,2]");
const fb = JSON.parse("[1,2]");
console.log("fresh_each_call=" + (fa === fb) + ":" + JSON.stringify(fa));
console.log("array_proto=" + (Object.getPrototypeOf(fa) === Array.prototype));
console.log("object_proto=" + (Object.getPrototypeOf(JSON.parse("{}")) === Object.prototype));

// --- string escapes: decoded forms, lone surrogates, and forms the grammar rejects ---
p("esc_newline", '"a\\nb"');
p("esc_solidus", '"a\\/b"');
p("esc_unicode", '"\\u0041"');
p("esc_surrogate_pair", '"\\ud83d\\ude00"');
p("esc_lone_high_surrogate", '"\\ud800"');
p("esc_lone_low_surrogate", '"\\udfff"');
p("esc_bad_v", '"a\\vb"');
p("esc_bad_x", '"a\\x41b"');
p("esc_bad_zero", '"a\\0b"');
p("esc_short_unicode", '"\\u41"');
p("esc_bad_unicode_digits", '"\\uZZZZ"');
p("raw_newline_in_string", '"a' + String.fromCharCode(10) + 'b"');
p("raw_bell_in_string", '"a' + String.fromCharCode(7) + 'b"');
p("raw_del_in_string_ok", '"a' + String.fromCharCode(127) + 'b"');
p("unterminated_string", '"abc');
const decoded = JSON.parse('"line\\nquote\\"slash\\\\tab\\tA\\u0041"');
console.log("escapes_decoded=" + JSON.stringify(decoded));
console.log("escapes_length=" + decoded.length);
console.log("lone_surrogate_length=" + JSON.parse('"\\ud800"').length);

// --- own properties: "__proto__", "constructor" and duplicates are ordinary data ---
const parsed: any = JSON.parse('{"__proto__":{"polluted":true},"ok":1}');
console.log("proto_key_own=" + Object.prototype.hasOwnProperty.call(parsed, "__proto__"));
console.log("proto_keys=" + Object.keys(parsed).join(","));
console.log("proto_own_names=" + Object.getOwnPropertyNames(parsed).join(","));
const protoDesc: any = Object.getOwnPropertyDescriptor(parsed, "__proto__");
console.log("proto_desc_is_data=" + (protoDesc.get === undefined && protoDesc.set === undefined));
console.log("proto_desc_flags=" + protoDesc.writable + ":" + protoDesc.enumerable + ":" + protoDesc.configurable);
console.log("proto_value=" + JSON.stringify(protoDesc.value));
console.log("proto_unchanged=" + (Object.getPrototypeOf(parsed) === Object.prototype));
console.log("not_polluted=" + (parsed.polluted === undefined));
console.log("object_prototype_clean=" + (({} as any).polluted === undefined));
console.log("sibling_key=" + parsed.ok);
const literal: any = { __proto__: { fromLiteral: true }, ok: 1 };
console.log("literal_proto_inherits=" + literal.fromLiteral);
const assigned: any = {};
assigned["__proto__"] = { fromAssign: true };
console.log("assign_proto_inherits=" + assigned.fromAssign);
const primProto: any = JSON.parse('{"__proto__":42}');
console.log("prim_proto_own=" + Object.prototype.hasOwnProperty.call(primProto, "__proto__"));
console.log("prim_proto_value=" + primProto["__proto__"]);
const nestedProto: any = JSON.parse('{"a":{"__proto__":{"x":1}}}');
console.log("nested_proto_own=" + Object.prototype.hasOwnProperty.call(nestedProto.a, "__proto__"));
const inArrayProto: any = JSON.parse('[{"__proto__":{"y":2}}]');
console.log("in_array_proto_own=" + Object.prototype.hasOwnProperty.call(inArrayProto[0], "__proto__"));
const ctor: any = JSON.parse('{"constructor":{"prototype":{"hacked":true}}}');
console.log("ctor_own=" + Object.prototype.hasOwnProperty.call(ctor, "constructor"));
console.log("ctor_value=" + JSON.stringify(ctor.constructor));
console.log("real_ctor_intact=" + (({} as any).constructor === Object));
console.log("global_object_clean=" + (({} as any).polluted === undefined) + ":" + (([] as any).polluted === undefined));
const dupes: any = JSON.parse('{"a":1,"b":2,"a":3}');
console.log("dup_value=" + JSON.stringify(dupes));
console.log("dup_keys=" + Object.keys(dupes).join(","));

// --- deep nesting: arrays, objects, and alternating shapes ---
function nestArray(depth: number): string {
  let s = "1";
  for (let i = 0; i < depth; i++) s = "[" + s + "]";
  return s;
}
function nestObject(depth: number): string {
  let s = "1";
  for (let i = 0; i < depth; i++) s = '{"a":' + s + "}";
  return s;
}
function arrayDepth(v: any): number {
  let d = 0;
  while (Array.isArray(v)) { v = v[0]; d++; }
  return d;
}
function objectDepth(v: any): number {
  let d = 0;
  while (v !== null && typeof v === "object") { v = v.a; d++; }
  return d;
}
const a10: any = JSON.parse(nestArray(10));
console.log("arr10_depth=" + arrayDepth(a10));
console.log("arr10_leaf=" + a10[0][0][0][0][0][0][0][0][0][0]);
console.log("arr200_depth=" + arrayDepth(JSON.parse(nestArray(200))));
const o10: any = JSON.parse(nestObject(10));
console.log("obj10_depth=" + objectDepth(o10));
console.log("obj10_leaf=" + o10.a.a.a.a.a.a.a.a.a.a);
console.log("obj100_depth=" + objectDepth(JSON.parse(nestObject(100))));
let alt = "7";
for (let i = 0; i < 20; i++) alt = i % 2 === 0 ? "[" + alt + "]" : '{"k":' + alt + "}";
const altParsed: any = JSON.parse(alt);
console.log("alt_leaf=" + altParsed.k[0].k[0].k[0].k[0].k[0].k[0].k[0].k[0].k[0].k[0]);
const wide: any = JSON.parse('{"a":[{"b":[1,2,{"c":{"d":[3,{"e":4}]}}]}]}');
console.log("wide_e=" + wide.a[0].b[2].c.d[1].e);
console.log("wide_d0=" + wide.a[0].b[2].c.d[0]);
console.log("wide_json=" + JSON.stringify(wide));

// --- round-trip: stringify then parse returns an equal structure ---
console.log("rt_nest_array=" + (JSON.stringify(JSON.parse(nestArray(30))) === nestArray(30)));
console.log("rt_nest_object=" + (JSON.stringify(JSON.parse(nestObject(30))) === nestObject(30)));
const mixed = { text: "café 😀", values: [0, false, null, { x: 2 }], empty: {}, n: -0.5, big: 1e21 };
const encoded = JSON.stringify(mixed);
const decodedMixed: any = JSON.parse(encoded);
console.log("rt_encoded=" + encoded);
console.log("rt_text_equal=" + (decodedMixed.text === mixed.text));
console.log("rt_nested_x=" + decodedMixed.values[3].x + ":empty_keys=" + Object.keys(decodedMixed.empty).length);
console.log("rt_stable=" + (JSON.stringify(decodedMixed) === encoded));
const sum = 0.1 + 0.2;
console.log("rt_float_exact=" + (JSON.parse(JSON.stringify(sum)) === sum));
const tricky = "a\u0000b \"q\" \\ \n end";
console.log("rt_string_equal=" + (JSON.parse(JSON.stringify(tricky)) === tricky));
console.log("rt_empty_string=" + JSON.parse(JSON.stringify("")).length);
console.log("rt_negzero_becomes_zero=" + Object.is(JSON.parse(JSON.stringify(-0)), 0));
