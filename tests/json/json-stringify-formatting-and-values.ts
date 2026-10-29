// Cross-runtime: JSON.stringify output shape, the space argument, string escapes,
// number formatting, and how non-serialisable values and wrapper objects are handled.

// --- basic shapes and top-level primitives ---
console.log("obj=" + JSON.stringify({ a: 1, b: "hi" }));
console.log("arr=" + JSON.stringify([1, 2, 3]));
console.log("bools=" + JSON.stringify({ x: true, y: false }));
console.log("null_member=" + JSON.stringify({ z: null }));
console.log("nested=" + JSON.stringify({ a: { b: { c: [1, { d: [] }] } } }));
console.log("empty_obj=" + JSON.stringify({}));
console.log("empty_arr=" + JSON.stringify([]));
console.log("top_string=" + JSON.stringify("hi"));
console.log("top_number=" + JSON.stringify(42));
console.log("top_bool=" + JSON.stringify(true));
console.log("top_null=" + JSON.stringify(null));
console.log("literal_key_order=" + JSON.stringify({ z: 1, a: 2, m: 3 }));

// --- the space argument: numbers, strings, clamping, and values that mean "compact" ---
const obj = { a: 1, b: { c: 2 } };
const compact = JSON.stringify(obj);
const pretty2 = JSON.stringify(obj, null, 2);
console.log("space_2=" + JSON.stringify(pretty2));
console.log("space_2_lines=" + pretty2.split("\n").length);
console.log("space_1=" + JSON.stringify(JSON.stringify(obj, null, 1)));
console.log("space_0_is_compact=" + (JSON.stringify(obj, null, 0) === compact));
console.log("space_3=" + JSON.stringify(JSON.stringify(obj, null, 3)));
const s10 = JSON.stringify(obj, null, 10);
console.log("clamp_10_line=" + JSON.stringify(s10.split("\n")[1]));
console.log("clamp_11_same=" + (s10 === JSON.stringify(obj, null, 11)));
console.log("clamp_100_same=" + (s10 === JSON.stringify(obj, null, 100)));
console.log("clamp_inf_same=" + (s10 === JSON.stringify(obj, null, Infinity)));
console.log("frac_2_9_same_as_2=" + (JSON.stringify(obj, null, 2.9) === pretty2));
console.log("neg_is_compact=" + (JSON.stringify(obj, null, -1) === compact));
console.log("nan_is_compact=" + (JSON.stringify(obj, null, NaN) === compact));
console.log("null_is_compact=" + (JSON.stringify(obj, null, null) === compact));
console.log("undef_is_compact=" + (JSON.stringify(obj, null, undefined) === compact));
console.log("bool_is_compact=" + (JSON.stringify(obj, null, true) === compact));
console.log("object_is_compact=" + (JSON.stringify(obj, null, {} as any) === compact));
console.log("str_dash=" + JSON.stringify(JSON.stringify(obj, null, "--")));
console.log("str_tab=" + JSON.stringify(JSON.stringify(obj, null, "\t")));
console.log("str_empty_is_compact=" + (JSON.stringify(obj, null, "") === compact));
const longGap = JSON.stringify(obj, null, "0123456789ABCDEF");
console.log("str_clamped_line=" + JSON.stringify(longGap.split("\n")[1]));
console.log("num_wrapper_same=" + (JSON.stringify(obj, null, new Number(2)) === pretty2));
console.log("str_wrapper_same=" + (JSON.stringify(obj, null, new String("--")) === JSON.stringify(obj, null, "--")));
console.log("arr_indent=" + JSON.stringify(JSON.stringify([1, [2]], null, 1)));
console.log("empty_containers_pretty=" + JSON.stringify(JSON.stringify({ a: {}, b: [] }, null, 1)));
console.log("scalar_ignores_space=" + JSON.stringify(JSON.stringify(5, null, 4)));

// --- string escapes: short forms, \u00XX controls, what is left alone, lone surrogates ---
console.log("esc_quote=" + JSON.stringify('a"b'));
console.log("esc_backslash=" + JSON.stringify("a\\b"));
console.log("esc_newline=" + JSON.stringify("a\nb"));
console.log("esc_cr=" + JSON.stringify("a\rb"));
console.log("esc_tab=" + JSON.stringify("a\tb"));
console.log("esc_backspace=" + JSON.stringify("a\bb"));
console.log("esc_formfeed=" + JSON.stringify("a\fb"));
console.log("esc_nul=" + JSON.stringify("a\u0000b"));
console.log("esc_soh=" + JSON.stringify("a\u0001b"));
console.log("esc_vtab=" + JSON.stringify("a\u000bb"));
console.log("esc_escape_char=" + JSON.stringify("a\u001bb"));
console.log("esc_unit_separator=" + JSON.stringify("a\u001fb"));
console.log("esc_del_kept=" + JSON.stringify("a\u007fb"));
console.log("kept_slash=" + JSON.stringify("a/b"));
console.log("kept_apostrophe=" + JSON.stringify("a'b"));
console.log("kept_accent=" + JSON.stringify("café"));
console.log("kept_cjk=" + JSON.stringify("日本"));
console.log("lone_high=" + JSON.stringify("\ud800"));
console.log("lone_low=" + JSON.stringify("\udfff"));
console.log("lone_mid=" + JSON.stringify("a\ud834b"));
console.log("reversed_pair=" + JSON.stringify("\ude00\ud83d"));
const emoji = "😀";
console.log("pair_kept=" + JSON.stringify(emoji));
console.log("pair_out_len=" + JSON.stringify(emoji).length);
console.log("escaped_keys=" + JSON.stringify({ "a\nb": 1, 'a"b': 2, "a\u0001b": 3 }));
console.log("empty_string=" + JSON.stringify(""));

// --- numbers: signed zero, non-finite values, magnitude and exponent notation ---
console.log("neg_zero=" + JSON.stringify(-0));
console.log("neg_zero_in_arr=" + JSON.stringify([-0, 0]));
console.log("inf=" + JSON.stringify(Infinity));
console.log("neg_inf=" + JSON.stringify(-Infinity));
console.log("nan=" + JSON.stringify(NaN));
console.log("nonfinite_obj=" + JSON.stringify({ a: Infinity, b: -Infinity, c: NaN }));
console.log("nonfinite_arr=" + JSON.stringify([Infinity, -Infinity, NaN]));
console.log("overflow_to_inf=" + JSON.stringify(1e308 * 10));
console.log("max_safe=" + JSON.stringify(9007199254740991));
console.log("unsafe_rounds=" + JSON.stringify(9007199254740993));
console.log("max_value=" + JSON.stringify(1.7976931348623157e308));
console.log("min_value=" + JSON.stringify(5e-324));
console.log("e20=" + JSON.stringify(1e20));
console.log("e21=" + JSON.stringify(1e21));
console.log("neg_e21=" + JSON.stringify(-1e21));
console.log("e_minus_6=" + JSON.stringify(0.000001));
console.log("e_minus_7=" + JSON.stringify(1e-7));
console.log("third=" + JSON.stringify(1 / 3));
console.log("point_sum=" + JSON.stringify(0.1 + 0.2));
console.log("big_frac=" + JSON.stringify(123456789.123456789));
console.log("float_int=" + JSON.stringify(100.0));
console.log("neg_big=" + JSON.stringify(-9007199254740991));

// --- non-serialisable values: dropped in objects, null in arrays, undefined at top level ---
const fn = function () { return 1; };
const sym = Symbol("s");
console.log("top_undef_is_undefined=" + (JSON.stringify(undefined) === undefined));
console.log("top_fn_is_undefined=" + (JSON.stringify(fn) === undefined));
console.log("top_sym_is_undefined=" + (JSON.stringify(sym) === undefined));
console.log("obj_drops_undef=" + JSON.stringify({ a: 1, b: undefined, c: 2 }));
console.log("obj_drops_fn=" + JSON.stringify({ a: 1, b: fn, c: 2 }));
console.log("obj_drops_sym=" + JSON.stringify({ a: 1, b: sym, c: 2 }));
console.log("obj_drops_all=" + JSON.stringify({ b: undefined, c: fn, d: sym }));
console.log("arr_undef_null=" + JSON.stringify([1, undefined, 2]));
console.log("arr_fn_null=" + JSON.stringify([1, fn, 2]));
console.log("arr_sym_null=" + JSON.stringify([1, sym, 2]));
console.log("arr_all_null=" + JSON.stringify([undefined, fn, sym]));
const sparse: any[] = [1];
sparse[3] = 4;
console.log("arr_holes_null=" + JSON.stringify(sparse));
const holes: any[] = [];
holes.length = 5;
holes[1] = "x";
holes[4] = undefined;
console.log("arr_length5_holes=" + JSON.stringify(holes) + ":len=" + holes.length);
console.log("nested_drop=" + JSON.stringify({ a: [undefined, { b: undefined, c: 3 }] }));
console.log("mixed_drop=" + JSON.stringify({ list: [fn], gone: fn }));
const undefTojson: any = { toJSON: function () { return undefined; } };
console.log("tojson_undef_top_is_undefined=" + (JSON.stringify(undefTojson) === undefined));
console.log("tojson_undef_in_obj=" + JSON.stringify({ a: undefTojson, b: 1 }));
console.log("tojson_undef_in_arr=" + JSON.stringify([undefTojson, 1]));
try {
  JSON.stringify(1n);
  console.log("bigint_top=no_throw");
} catch (e: any) {
  console.log("bigint_top=" + e.name);
}
try {
  JSON.stringify({ v: 1n });
  console.log("bigint_nested=no_throw");
} catch (e: any) {
  console.log("bigint_nested=" + e.name);
}

// --- wrapper objects are unwrapped to their primitive; expando properties are ignored ---
console.log("wrap_number=" + JSON.stringify(new Number(5)));
console.log("wrap_number_nan=" + JSON.stringify(new Number(NaN)));
console.log("wrap_number_negzero=" + JSON.stringify(new Number(-0)));
console.log("wrap_string=" + JSON.stringify(new String("hi")));
console.log("wrap_string_quoted=" + JSON.stringify(new String('a"b')));
console.log("wrap_bool_true=" + JSON.stringify(new Boolean(true)));
console.log("wrap_bool_false=" + JSON.stringify(new Boolean(false)));
console.log("wrap_nested=" + JSON.stringify({ a: new Number(7) }));
console.log("wrap_in_array=" + JSON.stringify([new Number(1), new String("x"), new Boolean(false)]));
console.log("wrap_deep=" + JSON.stringify({ a: { b: [new Number(3)] } }));
const withExpando: any = new Number(5);
withExpando.extra = "ignored";
console.log("wrap_expando_ignored=" + JSON.stringify(withExpando));
console.log("object_wrapper_num=" + JSON.stringify(Object(5)));
console.log("object_wrapper_str=" + JSON.stringify(Object("h")));
console.log("plain_with_valueof=" + JSON.stringify({ valueOf: () => 9, a: 1 }));
