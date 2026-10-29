// IEEE special values: negative zero (producers, the routes that hide it and
// the ones that reveal it, collections that merge it), NaN (every search and
// identity API and where they disagree), the full special-value tables of *,
// / and %, infinity arithmetic, and the Math functions that preserve or create
// signed zeros and NaN.

// --- producers of negative zero, each one shown through every detector ---
function probe(label: string, x: number): void {
  console.log(
    label +
      " | Object.is=-0:" + Object.is(x, -0) +
      " | ===0:" + (x === 0) +
      " | 1/x:" + String(1 / x) +
      " | sign:" + String(Math.sign(x)) +
      " | String:" + String(x) +
      " | bitOr0:" + String(x | 0)
  );
}
probe("literal_neg_zero", -0);
probe("zero_times_minus1", 0 * -1);
probe("minus1_times_zero", -1 * 0);
probe("zero_div_minus1", 0 / -1);
probe("neg_zero_plus_neg_zero", -0 + -0);
probe("round_minus0.2", Math.round(-0.2));
probe("floor_neg_zero", Math.floor(-0));
probe("ceil_minus0.5", Math.ceil(-0.5));
probe("trunc_minus0.5", Math.trunc(-0.5));
probe("min_0_neg0", Math.min(0, -0));
probe("max_neg0_0", Math.max(-0, 0));
probe("neg_abs_zero", -Math.abs(0));
probe("sqrt_neg_zero", Math.sqrt(-0));
probe("pow_neg_zero_1", Math.pow(-0, 1));
probe("abs_neg_zero", Math.abs(-0));
probe("zero_times_zero", 0 * 0);
probe("zero_plus_zero", 0 + 0);
probe("zero_minus_zero", 0 - 0);
probe("neg_zero_minus_zero", -0 - 0);

// --- what does NOT produce a negative zero ---
console.log("no_neg_zero_sum=" + Object.is(0 + 0, -0) + "," + Object.is(0 + -0, -0) + "," + Object.is(-0 + 0, 0));
console.log("no_neg_zero_diff=" + Object.is(0 - 0, -0) + "," + Object.is(-0 - -0, -0) + "," + Object.is(0 - -0, -0));
console.log("no_neg_zero_products=" + Object.is(0 * 1, -0) + "," + Object.is(2 * 0, -0) + "," + Object.is(-0 * 0, -0));

// --- the routes that hide the sign of zero ---
console.log("hidden_String=" + String(-0) + " toString=" + (-0).toString() + " template=" + `${-0}` + " concat=" + (-0 + ""));
console.log("hidden_toString2=" + (-0).toString(2) + " toFixed=" + (-0).toFixed(2) + " toPrecision=" + (-0).toPrecision(3));
console.log("hidden_toExponential=" + (-0).toExponential(1) + " json=" + JSON.stringify(-0) + " jsonArray=" + JSON.stringify([-0]));
console.log("hidden_join=" + [-0].join("") + " hidden_String_zero_div=" + String(0 / -5));
console.log("hidden_Number_neg_zero_string=" + Number("-0") + " via_parseFloat=" + parseFloat("-0"));

// --- the routes that reveal it ---
console.log("reveal_is=" + Object.is(-0, -0) + "," + Object.is(-0, 0) + "," + Object.is(0, 0));
console.log("reveal_reciprocal=" + (1 / -0 === -Infinity) + "," + (1 / 0 === Infinity) + "," + String(1 / -0));
console.log("reveal_sign=" + String(Math.sign(-0)) + " " + Object.is(Math.sign(-0), -0) + " " + Object.is(Math.sign(0), 0));
console.log("reveal_identity=" + (-0 === 0) + " " + (-0 == 0) + " " + (0 === -0));
console.log("reveal_sign_result=" + Object.is(-0 + 0, 0) + " " + (-0 + 0 === 0));

// --- Math functions that preserve or create the sign of zero ---
console.log("math_ceil=" + Object.is(Math.ceil(-0.5), -0) + " floor_pos=" + Object.is(Math.floor(0), 0));
console.log("math_floor_neg_zero=" + Object.is(Math.floor(-0.0), -0) + " trunc_neg_frac=" + Object.is(Math.trunc(-0.2), -0));
console.log("math_trunc_plain=" + Math.trunc(-4.7) + "," + Math.trunc(4.7));
console.log("math_round_half=" + Math.round(-0.5) + " is_neg0=" + Object.is(Math.round(-0.5), -0));
console.log("math_round_values=" + Math.round(0.5) + "," + Math.round(-1.5) + "," + Math.round(2.5) + "," + Math.round(-2.5));
console.log("math_round_neg_small=" + Math.round(-0.4) + " is_neg0=" + Object.is(Math.round(-0.4), -0) + " " + Object.is(Math.round(-0.1), -0));
console.log("math_round_inexact=" + Math.round(0.49999999999999994) + " " + Math.round(-0.49999999999999994));
console.log("math_sign=" + Math.sign(-3) + "," + Math.sign(0) + "," + String(Math.sign(NaN)) + " neg_zero=" + Object.is(Math.sign(-0), -0));
console.log("math_sign_neg_frac=" + Math.sign(-0.0001) + "," + Math.sign(Infinity) + "," + Math.sign(-Infinity));
console.log("math_abs_coercion=" + Math.abs("-3") + "," + Math.abs(null) + "," + Math.abs(-0) + "," + Math.abs(-Infinity) + "," + Math.abs(NaN));
console.log("math_abs_sign_coerce=" + [Math.sign("-3" as any), Math.sign(null as any), Math.sign(undefined as any)].join(","));
console.log("math_neg_zero_mod_ceil=" + Object.is(Math.ceil(-0), -0) + " " + Object.is(Math.round(-0), -0));
console.log("math_min_max_zero=" + Object.is(Math.min(0, -0), -0) + "," + Object.is(Math.max(0, -0), 0) + "," + Object.is(Math.max(-0, 0), 0));
console.log("math_min_max_nan=" + Number.isNaN(Math.min(1, NaN, 2)) + "," + Number.isNaN(Math.max(NaN, 3)) + "," + String(Math.max(1, NaN, 3)));
console.log("math_min_empty=" + Math.min() + " max_empty=" + Math.max());
console.log("math_pow_neg_frac=" + Math.pow(-8, 1 / 3) + " cbrt_neg=" + Math.cbrt(-8) + " cbrt_27=" + Math.cbrt(27));
console.log("math_pow_neg_zero_odd=" + Object.is(Math.pow(-0, 3), -0) + " even=" + Math.pow(-0, 2));
console.log("math_pow_exp=" + (2 ** -1) + " " + ((-2) ** 2) + " " + ((-2) ** 3) + " " + (4 ** -2) + " " + (9 ** 0.5));
console.log("math_exp_right_assoc=" + (2 ** 3 ** 2) + " " + ((2 ** 3) ** 2));
console.log("math_pow_one_nan=" + Math.pow(1, NaN) + " pow_nan_zero=" + Math.pow(NaN, 0) + " pow_one_inf=" + Math.pow(1, Infinity));

// --- NaN: the one value not equal to itself, and each API's relation on it ---
const nan: number = NaN;
console.log("nan_strict=" + (nan === nan) + " loose=" + (nan == nan) + " not_not_equal=" + (nan !== nan));
console.log("nan_object_is=" + Object.is(nan, nan) + " vs_zero=" + Object.is(nan, 0));
console.log("nan_compare=" + (nan < nan) + "," + (nan > nan) + "," + (nan <= nan) + "," + (nan >= nan));
console.log("nan_arithmetic=" + (NaN + 1) + "," + (NaN * 0) + "," + (NaN ** 0) + "," + (0 / 0) + "," + (Infinity - Infinity));
const arr: number[] = [1, NaN, 3, NaN, 5];
console.log("arr_indexOf=" + arr.indexOf(NaN) + " lastIndexOf=" + arr.lastIndexOf(NaN) + " includes=" + arr.includes(NaN));
console.log("arr_find=" + String(arr.find((x) => Number.isNaN(x))) + " findIndex=" + arr.findIndex((x) => Number.isNaN(x)));
console.log("arr_findLastIndex=" + arr.findLastIndex((x) => Number.isNaN(x)) + " strict_some=" + arr.some((x) => x === NaN));
console.log("arr_isnan_some=" + arr.some((x) => Number.isNaN(x)) + " every_not_nan=" + arr.every((x) => !Number.isNaN(x)));
console.log("arr_filter=" + arr.filter((x) => !Number.isNaN(x)).join(",") + " count=" + arr.filter(Number.isNaN).length);
const f64 = new Float64Array([1, NaN, 3]);
console.log("typed_indexOf=" + f64.indexOf(NaN) + " typed_includes=" + f64.includes(NaN) + " join=" + f64.join(","));
const i32 = new Int32Array(1);
i32[0] = NaN;
console.log("int_array_nan_is=" + String(i32[0]) + " float_keeps=" + Number.isNaN(f64[1]));
const set = new Set<number>([NaN, NaN, 0 / 0, Number("x"), Infinity - Infinity]);
console.log("set_size=" + set.size + " has=" + set.has(NaN) + " values=" + Array.from(set).map((v) => String(v)).join(","));
const map = new Map<number, string>();
map.set(NaN, "first");
map.set(0 / 0, "second");
console.log("map_size=" + map.size + " get=" + String(map.get(NaN)) + " keys=" + Array.from(map.keys()).map((k) => String(k)).join(","));
console.log("map_delete=" + map.delete(NaN) + " size_after=" + map.size);
const dupes: number[] = [1, NaN, 1, NaN, 2];
console.log("dedupe_set=" + Array.from(new Set(dupes)).map((v) => String(v)).join(","));
console.log("dedupe_indexOf=" + dupes.filter((v, i) => dupes.indexOf(v) === i).map((v) => String(v)).join(","));
console.log("dedupe_includes=" + dupes.reduce((acc: number[], v) => (acc.includes(v) ? acc : acc.concat([v])), []).map((v) => String(v)).join(","));
const keyed: any = {};
keyed[NaN] = "a";
keyed[0 / 0] = "b";
console.log("nan_key=" + Object.keys(keyed).join(",") + " value=" + keyed[NaN] + " count=" + Object.keys(keyed).length);
console.log("nan_key_is_string=" + ("NaN" in keyed) + "," + (keyed["NaN"] === keyed[NaN]));
const probes: [string, any][] = [
  ["nan", NaN], ["string_nan", "NaN"], ["undefined", undefined], ["empty_string", ""],
  ["object", {}], ["zero", 0], ["null", null], ["array", []],
];
for (const p of probes) {
  console.log(
    p[0] +
      " | Number.isNaN:" + Number.isNaN(p[1]) +
      " | globalIsNaN:" + isNaN(p[1]) +
      " | Object.is:" + Object.is(p[1], NaN) +
      " | self_ne:" + (p[1] !== p[1])
  );
}
const sortedWithNaN = [3, NaN, 1, NaN, 2].slice().sort((a, b) => a - b);
console.log("sort_length=" + sortedWithNaN.length + " nan_count=" + sortedWithNaN.filter(Number.isNaN).length);
console.log("default_sort=" + [3, NaN, 1, 2].slice().sort().map((v) => String(v)).join(","));
console.log("safe_sort=" + [3, NaN, 1, 2].filter((x) => !Number.isNaN(x)).sort((a, b) => a - b).join(","));
console.log("json_nan=" + JSON.stringify({ a: NaN, b: [NaN] }) + " roundtrip=" + String(JSON.parse(JSON.stringify({ a: NaN })).a));
console.log("nan_strings=" + String(NaN) + "," + `${NaN}` + "," + [NaN].join("") + " boolean=" + Boolean(NaN));
console.log("nan_max_min=" + String(Math.max(NaN)) + "," + String(Math.min(NaN, 1)) + "," + String(Math.abs(NaN)));
console.log("nan_from_parse=" + String(Number("x")) + "," + String(parseInt("x")) + "," + String(parseFloat("x")));

// --- the * table: sign is the XOR of operand signs, and inf*0 is the NaN site ---
function mul(a: number, b: number): void {
  const r = a * b;
  console.log("mul " + String(a) + " * " + String(b) + " = " + String(r) + " neg0:" + Object.is(r, -0));
}
function div(a: number, b: number): void {
  const r = a / b;
  console.log("div " + String(a) + " / " + String(b) + " = " + String(r) + " neg0:" + Object.is(r, -0));
}
function rem(a: number, b: number): void {
  const r = a % b;
  console.log(
    "rem " + String(a) + " % " + String(b) + " = " + String(r) +
      " neg0:" + Object.is(r, -0) + " nan:" + Number.isNaN(r)
  );
}
mul(0, 0);
mul(0, -0);
mul(-0, 0);
mul(-0, -0);
mul(1, 0);
mul(-1, 0);
mul(-1, -0);
mul(-2, 3);
mul(-2, -3);
mul(Infinity, 0);
mul(Infinity, -0);
mul(-Infinity, 0);
mul(-Infinity, -0);
mul(Infinity, -2);
mul(-Infinity, -2);
mul(Infinity, -Infinity);
mul(-Infinity, -Infinity);
mul(NaN, 0);
mul(NaN, Infinity);
mul(Number.MAX_VALUE, 2);
mul(Number.MAX_VALUE, -2);
mul(Number.MIN_VALUE, 0.5);
mul(Number.MIN_VALUE, -0.5);
mul(1e200, 1e200);
mul(1e-200, 1e-200);

// --- the / table: zero denominators are signed infinities, not errors ---
div(1, 0);
div(-1, 0);
div(1, -0);
div(-1, -0);
div(0, 0);
div(-0, 0);
div(0, -0);
div(-0, -0);
div(1, Infinity);
div(-1, Infinity);
div(1, -Infinity);
div(-1, -Infinity);
div(0, Infinity);
div(-0, Infinity);
div(0, -Infinity);
div(Infinity, 2);
div(Infinity, -2);
div(-Infinity, 2);
div(Infinity, 0);
div(Infinity, -0);
div(Infinity, Infinity);
div(-Infinity, Infinity);
div(NaN, 1);
div(1, NaN);
div(6, 3);
div(-6, 3);
div(1, 3);
div(Number.MIN_VALUE, 2);
div(Number.MAX_VALUE, 0.5);

// --- the % table: the sign follows the DIVIDEND, down to negative zero ---
rem(NaN, 1);
rem(1, NaN);
rem(NaN, NaN);
rem(NaN, Infinity);
rem(Infinity, 2);
rem(-Infinity, 2);
rem(Infinity, -2);
rem(Infinity, Infinity);
rem(-Infinity, Infinity);
rem(Infinity, 0);
rem(2, Infinity);
rem(-2, Infinity);
rem(2, -Infinity);
rem(-2, -Infinity);
rem(0, Infinity);
rem(-0, Infinity);
rem(0.5, Infinity);
rem(5, 0);
rem(-5, 0);
rem(5, -0);
rem(0, 0);
rem(-0, -0);
rem(0, 5);
rem(-0, 5);
rem(-0, -5);
rem(0, -5);
rem(7, 3);
rem(-7, 3);
rem(7, -3);
rem(-7, -3);
rem(-6, 3);
rem(-6, -3);
rem(5.5, 2);
rem(-5.5, 2);
rem(5.5, -2);
rem(0.3, 0.1);
rem(1, 0.3);
rem(-1, 0.3);
rem(9007199254740992, 2);
rem(9007199254740991, 2);
rem(1e21, 3);
rem(Number.MAX_VALUE, 2);
rem(Number.MIN_VALUE, Number.MIN_VALUE);
console.log("neg6_mod3_isNeg0=" + Object.is(-6 % 3, -0) + " pos6_mod3_isNeg0=" + Object.is(6 % 3, -0));
console.log("neg6_mod_neg3_isNeg0=" + Object.is(-6 % -3, -0));

// --- % and ** coerce both operands left-then-right through valueOf ---
const seen: string[] = [];
const lhs: any = { valueOf: () => { seen.push("lhs"); return 7; } };
const rhsObj: any = { valueOf: () => { seen.push("rhs"); return 3; } };
console.log("coerced_mod=" + String(lhs % rhsObj) + " order=" + seen.join(","));
console.log("coerced_kinds=" + String(("7" as any) % ("3" as any)) + "," + String((true as any) % (2 as any)) + "," + String((null as any) % (2 as any)) + "," + String((undefined as any) % (2 as any)));
let acc = -7;
acc %= 3;
console.log("compound_mod=" + String(acc));

// --- infinity arithmetic ---
console.log("inf_arith=" + String(Infinity - Infinity) + "," + String(-Infinity + Infinity) + "," + String(Infinity + Infinity));
console.log("inf_times_zero=" + String(Infinity * 0) + " inf_over_inf=" + String(Infinity / Infinity) + " inf_over_finite=" + String(Infinity / 1e308));
console.log("finite_over_inf=" + String(1e308 / Infinity) + " neg0=" + Object.is(1e308 / Infinity, -0));
console.log("neg_finite_over_inf=" + String(-1e308 / Infinity) + " neg0=" + Object.is(-1e308 / Infinity, -0));
console.log("inf_plus_finite=" + String(Infinity + 1e308) + " inf_mod=" + String(Infinity % 2) + " mod_inf=" + String(2 % Infinity));
console.log("inf_comparisons=" + (Infinity > Number.MAX_VALUE) + "," + (-Infinity < -Number.MAX_VALUE) + "," + (Infinity === Infinity));
console.log("inf_is_finite=" + Number.isFinite(Infinity) + "," + Number.isFinite(-Infinity) + "," + isFinite(NaN));
console.log("zero_comparisons=" + (0 === -0) + "," + Object.is(0, -0) + "," + (-0 < 0) + "," + (-0 > 0));
