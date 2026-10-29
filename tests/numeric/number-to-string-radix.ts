// Number to String: the default rendering (shortest round-trip, exponent
// thresholds at 1e21 and 1e-7), every route to a string (String, toString,
// template, concatenation, join, JSON), toString(radix) for integers and for
// fractions that terminate in the target base, radix bounds, and non-finite
// values.

function attempt(label: string, fn: () => string): void {
  try {
    console.log(label + "=" + fn());
  } catch (e) {
    console.log(label + "!" + (e as Error).name);
  }
}

// --- default rendering across the magnitude range ---
const values: [string, number][] = [
  ["zero", 0], ["neg_zero", -0], ["one", 1], ["neg_one", -1], ["forty_two", 42],
  ["neg_123", -123], ["pi_ish", 3.14], ["million", 1000000], ["tenth", 0.1],
  ["classic_sum", 0.1 + 0.2], ["third", 1 / 3], ["half", 0.5], ["exact_quarter", 0.25],
  ["one_e20", 1e20], ["one_e21", 1e21], ["neg_one_e21", -1e21],
  ["one_e_minus6", 1e-6], ["one_e_minus7", 1e-7], ["neg_one_e_minus7", -1e-7],
  ["one_point_five_e_minus7", 1.5e-7], ["big_int_like", 123456789012345680000],
  ["long_fraction", 123456789.123456789], ["nine_e20", 99999999999999999999],
  ["max_safe", Number.MAX_SAFE_INTEGER], ["two_pow_53", 2 ** 53],
  ["two_pow_53_plus_2", 2 ** 53 + 2], ["epsilon", Number.EPSILON],
  ["max_value", Number.MAX_VALUE], ["min_value", Number.MIN_VALUE],
  ["two_pow_minus_1022", 2 ** -1022], ["nan", NaN], ["infinity", Infinity],
  ["neg_infinity", -Infinity],
];
for (const [label, x] of values) {
  console.log("default " + label + "=" + String(x));
}

// --- every route to a string runs the same Number::toString ---
const routeMismatches: string[] = [];
for (const [label, x] of values) {
  const a = String(x);
  if (x.toString() !== a || `${x}` !== a || x + "" !== a || [x].join("") !== a || "".concat(x as any) !== a) {
    routeMismatches.push(label);
  }
}
console.log("route_mismatches=[" + routeMismatches.join(",") + "]");
console.log("route_sample_plus=" + (1e21 + "") + " template=" + `${1e-7}`);
console.log("route_wrapper_String=" + String(new Number(0.1 + 0.2)));
console.log("route_wrapper_plus=" + (new Number(-0) + ""));
console.log("route_Object_neg_zero=" + String(Object(-0)));

// --- JSON is the one route that drops information: non-finite becomes null ---
const jsonDiffers: string[] = [];
for (const [label, x] of values) {
  if (JSON.stringify(x) !== String(x)) jsonDiffers.push(label);
}
console.log("json_differs_on=[" + jsonDiffers.join(",") + "]");
console.log("json_neg_zero=" + JSON.stringify(-0) + " array=" + JSON.stringify([-0, 0]));
console.log("json_non_finite=" + JSON.stringify([NaN, Infinity, -Infinity]));
console.log("json_wrapper_nan=" + JSON.stringify(new Number(NaN)));
console.log("json_wrapper_finite=" + JSON.stringify(new Number(2.5)));
console.log("json_object_nan=" + JSON.stringify({ a: NaN, b: [NaN] }));

// --- the string form of a negative zero hides its sign ---
console.log("neg_zero_string=" + String(-0) + " toString=" + (-0).toString());
console.log("neg_zero_template=" + `${-0}` + " concat=" + (-0 + ""));
console.log("zero_div_neg_zero_string=" + String(0 / -5));

// --- toString(radix) for integers ---
console.log("radix_default_eq_10=" + ((42).toString() === (42).toString(10)));
console.log("bin_255=" + (255).toString(2) + " oct_255=" + (255).toString(8) + " hex_255=" + (255).toString(16));
console.log("base36_255=" + (255).toString(36) + " base36_z=" + (35).toString(36));
console.log("small_bin=" + (10).toString(2) + " small_hex=" + (10).toString(16));
console.log("zero_bin=" + (0).toString(2) + " zero_base36=" + (0).toString(36));
console.log("neg_15_bin=" + (-15).toString(2) + " neg_15_hex=" + (-15).toString(16));
console.log("neg_zero_bin=" + (-0).toString(2));
console.log("n123456789=" + (123456789).toString(2) + "|" + (123456789).toString(8) + "|" + (123456789).toString(16) + "|" + (123456789).toString(36));
const roundTripN = 123456789;
console.log("radix36_roundtrip=" + (parseInt(roundTripN.toString(36), 36) === roundTripN));
console.log("two_pow_53_bin_len=" + (2 ** 53).toString(2).length + " hex=" + (2 ** 53).toString(16));
console.log("max_safe_base36=" + Number.MAX_SAFE_INTEGER.toString(36));
console.log("one_e21_hex=" + (1e21).toString(16));

// --- toString(radix) for fractions that terminate in the target base ---
const exactFractions: [number, number][] = [
  [0.5, 2], [0.25, 2], [0.75, 2], [0.125, 2], [0.375, 2], [0.0625, 2],
  [2.5, 2], [10.625, 2], [255.5, 2], [3.75, 2],
  [0.5, 8], [0.125, 8], [255.5, 8], [10.625, 8], [1.75, 8],
  [0.5, 16], [0.0625, 16], [255.5, 16], [10.625, 16], [4095.9375, 16],
  [0.5, 4], [0.75, 4], [0.5, 32], [0.03125, 32],
];
for (const [x, radix] of exactFractions) {
  console.log("frac " + String(x) + " base" + radix + "=" + x.toString(radix));
}
console.log("neg_frac_bin=" + (-0.5).toString(2) + "|" + (-0.75).toString(2) + "|" + (-2.5).toString(2));
console.log("neg_frac_hex=" + (-10.625).toString(16) + " oct=" + (-255.5).toString(8));

// --- radix argument: ToIntegerOrInfinity, then the 2..36 bounds ---
console.log("radix_fraction_truncates=" + (5).toString(2.9) + " string_radix=" + (5).toString("2" as any));
console.log("radix_undefined=" + (2.5).toString(undefined));
attempt("radix_1", () => (5).toString(1));
attempt("radix_0", () => (5).toString(0));
attempt("radix_37", () => (5).toString(37));
attempt("radix_neg", () => (5).toString(-2));
attempt("radix_36_ok", () => (35).toString(36));
attempt("radix_2_ok", () => (5).toString(2));

// --- non-finite values ignore the radix once it is valid ---
console.log("nan_bin=" + (NaN).toString(2) + " inf_base36=" + (Infinity).toString(36) + " neg_inf_bin=" + (-Infinity).toString(2));

// --- exponent-form thresholds in the default rendering ---
console.log("threshold_e21=" + String(1e21) + " below=" + String(999999999999999900000));
console.log("threshold_small=" + String(1e-7) + " above=" + String(0.000001) + " e7_15=" + String(1.5e-7));
console.log("exp_form_large=" + String(123e20) + " exp_form_small=" + String(123e-20));
console.log("shortest_min=" + String(5e-324) + " shortest_max=" + String(1.7976931348623157e308));
console.log("long_literal_shortest=" + String(0.30000000000000004) + " " + String(1.0000000000000002));
