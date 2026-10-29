// The double itself: decimal literals rounded to the nearest double at parse
// time (ties to even), toFixed(20) as a microscope on the stored value, the
// shortest round-trip rendering, collisions above 2^53, the IEEE-754 bit
// pattern read through a DataView with explicit endianness, the float32
// truncation, and the two ends of the exponent range (overflow to Infinity,
// underflow through the subnormals to a signed zero).

// --- what the classic literals really store ---
function exact(label: string, x: number): void {
  console.log(label + " | stored:" + String(x) + " | exact20:" + x.toFixed(20));
}
exact("0.1", 0.1);
exact("0.2", 0.2);
exact("0.3", 0.3);
exact("0.1+0.2", 0.1 + 0.2);
exact("0.7", 0.7);
exact("1.005", 1.005);
exact("2.675", 2.675);
exact("8.575", 8.575);
exact("1.5", 1.5);
exact("0.25", 0.25);
exact("EPSILON", Number.EPSILON);
console.log("sum_vs_literal=" + (0.1 + 0.2 === 0.3) + " sum_is_next=" + (0.1 + 0.2 === 0.30000000000000004));
console.log("gap=" + (0.1 + 0.2 - 0.3).toExponential(4) + " within_eps=" + (Math.abs(0.1 + 0.2 - 0.3) < Number.EPSILON));
console.log("classic_prints=" + (0.1 + 0.2) + " " + (0.3 - 0.1) + " " + (0.1 * 3) + " " + (1.1 + 2.2));
console.log("classic_exact_prints=" + (0.1 + 0.2 === 0.3) + " " + (0.1).toString() + " " + (0.5 - 0.1 - 0.4));

// --- distinct literals that land on the SAME double ---
const collisions: [string, number, number][] = [
  ["0.1", 0.1, 0.1000000000000000055511151231257827],
  ["1e-1", 1e-1, 0.1],
  ["one_third", 1 / 3, 0.3333333333333333],
  ["max_safe", 9007199254740993, 9007199254740992],
  ["max_safe_plus2", 9007199254740994, 9007199254740994],
  ["big_odd", 10000000000000001, 10000000000000000],
  ["huge", 1e23, 99999999999999991611392],
  ["huge_neighbour", 1.0000000000000001e23, 1e23],
  ["tiny", 5e-324, 3e-324],
  ["tiny_round_down", 2e-324, 0],
];
for (const c of collisions) {
  console.log("collide_" + c[0] + "=" + (c[1] === c[2]) + " value=" + String(c[1]));
}

// --- ties to even at 2^53: consecutive integers past 2^53 count by twos ---
const boundary: number[] = [
  9007199254740991, 9007199254740992, 9007199254740993, 9007199254740994,
  9007199254740995, 9007199254740996, 9007199254740997,
];
console.log("above_2p53=" + boundary.map((b) => String(b)).join(" "));
console.log("odd_rounds_to_even=" + (9007199254740993 === 9007199254740992) + "," + (9007199254740995 === 9007199254740996));
console.log("two_pow_54_plus_1=" + String(18014398509481985) + " two_pow_54=" + String(2 ** 54));

// --- 1e23 is famously not 10^23; the exact digits say so ---
console.log("1e23_neighbours=" + (1e23 === 99999999999999991611392) + "," + (1e23 === 100000000000000000000000));
console.log("1e23_integer=" + Number.isInteger(1e23) + " 1e22_exact=" + (1e22).toFixed(0));

// --- Number(text) rounds exactly like a literal does ---
const parsed: string[] = [
  "0.1", "0.30000000000000004", "9007199254740993", "1e23",
  "0.1000000000000000055511151231257827", "1.7976931348623159e308",
  "4.9e-324", "2.4e-324", "2.5e-324",
];
for (const p of parsed) {
  console.log("Number(" + p + ")=" + String(Number(p)));
}
console.log("literal_matches_parse=" + (Number("0.1") === 0.1) + "," + (Number("9007199254740993") === 9007199254740993));

// --- the string form is the SHORTEST text that reads back to the same double ---
const roundtrip: number[] = [
  0.1, 0.3, 1 / 3, 1e23, 5e-324, Number.MAX_VALUE, Number.EPSILON,
  0.1 + 0.2, 123456789.123456789, 2 ** 53 + 2, 1.7976931348623157e308,
];
const broken: string[] = [];
for (const r of roundtrip) {
  if (Number(String(r)) !== r) broken.push(String(r));
}
console.log("roundtrip_failures=[" + broken.join(",") + "]");
console.log("shortest_forms=" + roundtrip.map((r) => String(r) + ":" + String(String(r).length)).join(" "));
console.log("extra_digits_ignored=" + (0.1 === 0.10000000000000000000001) + " extra_matter=" + (0.1 === 0.10000000000000002));
console.log("trailing_zeros=" + (1.5 === 1.50000000000000000000) + " exponent_forms=" + (1e2 === 100) + "," + (1.5e-3 === 0.0015));

// --- the IEEE-754 bit pattern, read through a DataView with an explicit byte order ---
const buf = new ArrayBuffer(8);
const view = new DataView(buf);
function bits(x: number): string {
  view.setFloat64(0, x, false);
  let out = "";
  for (let i = 0; i < 8; i++) {
    out += view.getUint8(i).toString(16).padStart(2, "0");
  }
  return out;
}
function parts(x: number): string {
  view.setFloat64(0, x, false);
  const hi = view.getUint32(0, false);
  const lo = view.getUint32(4, false);
  const sign = hi >>> 31;
  const exponent = (hi >>> 20) & 0x7ff;
  const mantissaHi = hi & 0xfffff;
  return "s" + sign + " e" + exponent + " m" + mantissaHi.toString(16) + ":" + lo.toString(16).padStart(8, "0");
}
const values: [string, number][] = [
  ["+0", 0], ["-0", -0], ["1", 1], ["-1", -1], ["2", 2], ["0.5", 0.5], ["0.1", 0.1],
  ["0.2", 0.2], ["0.3", 0.3], ["0.1+0.2", 0.1 + 0.2], ["1/3", 1 / 3], ["PI", Math.PI],
  ["E", Math.E], ["EPSILON", Number.EPSILON], ["MIN_VALUE", Number.MIN_VALUE],
  ["MAX_VALUE", Number.MAX_VALUE], ["MAX_SAFE_INTEGER", Number.MAX_SAFE_INTEGER],
  ["2^53", 2 ** 53], ["2^-1074", 2 ** -1074], ["smallest_normal", 2 ** -1022],
  ["largest_subnormal", 2 ** -1022 - 2 ** -1074], ["+Infinity", Infinity], ["-Infinity", -Infinity],
];
for (const pair of values) {
  console.log(pair[0] + " | " + bits(pair[1]) + " | " + parts(pair[1]));
}
const broke: string[] = [];
for (const pair of values) {
  view.setFloat64(0, pair[1], true);
  const back = view.getFloat64(0, true);
  if (!Object.is(back, pair[1])) broke.push(pair[0]);
}
console.log("roundtrip_failures_le=[" + broke.join(",") + "]");
view.setFloat64(0, 0.1, false);
const big: number[] = [];
for (let i = 0; i < 8; i++) big.push(view.getUint8(i));
view.setFloat64(0, 0.1, true);
const little: number[] = [];
for (let i = 0; i < 8; i++) little.push(view.getUint8(i));
console.log("big_endian=" + big.join(",") + " little_endian=" + little.join(","));
console.log("is_reversal=" + (big.join(",") === little.slice().reverse().join(",")));
view.setFloat64(0, 0.1);
const defaultBytes: number[] = [];
for (let i = 0; i < 8; i++) defaultBytes.push(view.getUint8(i));
console.log("default_is_big_endian=" + (defaultBytes.join(",") === big.join(",")));
console.log("nan_is_nan_after_roundtrip=" + Number.isNaN((view.setFloat64(0, NaN, false), view.getFloat64(0, false))));
console.log("biased_exponents=" + [-4, -3, -2, -1, 0, 1, 2, 3, 4].map((k) => {
  view.setFloat64(0, 2 ** k, false);
  return String(k) + ":" + String((view.getUint32(0, false) >>> 20) & 0x7ff);
}).join(" "));

// --- assembling a double from its halves ---
view.setUint32(0, 0x7ff00000, false);
view.setUint32(4, 0, false);
console.log("assembled_infinity=" + String(view.getFloat64(0, false)));
view.setUint32(0, 0x80000000, false);
view.setUint32(4, 0, false);
console.log("assembled_neg_zero=" + Object.is(view.getFloat64(0, false), -0));
view.setUint32(0, 0, false);
view.setUint32(4, 1, false);
console.log("assembled_min_value=" + (view.getFloat64(0, false) === Number.MIN_VALUE));
view.setFloat64(0, 1, false);
console.log("one_as_uint32_pair=" + view.getUint32(0, false) + "," + view.getUint32(4, false));

// --- float32: the same bits truncated to 24 significant bits ---
const f32 = new Float32Array(1);
const u32 = new Uint32Array(f32.buffer);
const f32bits: string[] = [];
for (const v of [0, -0, 1, 0.1, Infinity, -Infinity, 2 ** -149]) {
  f32[0] = v;
  f32bits.push(String(v) + ":" + u32[0].toString(16).padStart(8, "0"));
}
console.log("float32_bits=" + f32bits.join(" "));
f32[0] = 0.1;
console.log("float32_0.1_widened=" + String(f32[0]) + " exact_to_double=" + (f32[0] === 0.1));
f32[0] = 16777217;
console.log("float32_2p24_plus1=" + String(f32[0]));
f32[0] = 3.4028235e38 * 2;
console.log("float32_overflow=" + String(f32[0]));

// --- the top of the range: where a finite result stops being finite ---
console.log("max_times_2=" + String(Number.MAX_VALUE * 2) + " max_plus_max=" + String(Number.MAX_VALUE + Number.MAX_VALUE));
console.log("neg_max_times_2=" + String(-Number.MAX_VALUE * 2) + " max_squared=" + String(Number.MAX_VALUE * Number.MAX_VALUE));
console.log("max_times_1p1=" + String(Number.MAX_VALUE * 1.1) + " max_plus_one_same=" + (Number.MAX_VALUE + 1 === Number.MAX_VALUE));
console.log("max_plus_1e291_same=" + (Number.MAX_VALUE + 1e291 === Number.MAX_VALUE) + " max_plus_1e292=" + String(Number.MAX_VALUE + 1e292));
console.log("max_next_is_inf=" + (Number.MAX_VALUE * (1 + Number.EPSILON) === Infinity) + " half_max_doubled=" + (Number.MAX_VALUE / 2 * 2 === Number.MAX_VALUE));
console.log("1e308_times_10=" + String(1e308 * 10) + " 2p1023_times_2=" + String(2 ** 1023 * 2));
console.log("max_is_finite=" + Number.isFinite(Number.MAX_VALUE) + " next_is=" + Number.isFinite(Number.MAX_VALUE * 2));
console.log("parse_overflow=" + String(Number("1e400")) + "," + String(Number("-1e400")));
console.log("parse_max_boundary=" + String(Number("1.7976931348623157e308")) + "," + String(Number("1.7976931348623159e308")));

// --- the bottom: MIN_VALUE is the smallest SUBNORMAL, not the smallest normal ---
console.log("min_value=" + String(Number.MIN_VALUE) + " is_2_pow_neg1074=" + (Number.MIN_VALUE === 2 ** -1074));
console.log("smallest_normal=" + String(2 ** -1022) + " largest_subnormal=" + String(2 ** -1022 - 2 ** -1074));
console.log("subnormal_is_smaller=" + (2 ** -1022 - 2 ** -1074 < 2 ** -1022) + " normal_over_2=" + String(2 ** -1022 / 2));
console.log("min_over_2=" + String(Number.MIN_VALUE / 2) + " pos0=" + Object.is(Number.MIN_VALUE / 2, 0));
console.log("neg_min_over_2=" + String(-Number.MIN_VALUE / 2) + " neg0=" + Object.is(-Number.MIN_VALUE / 2, -0));
console.log("min_times_0p6=" + String(Number.MIN_VALUE * 0.6) + " min_times_0p4=" + String(Number.MIN_VALUE * 0.4));
console.log("min_over_3=" + String(Number.MIN_VALUE / 3) + " min_over_2_times_2=" + String(Number.MIN_VALUE / 2 * 2));
console.log("three_halves_of_min=" + String(Number.MIN_VALUE * 1.5));
console.log("tiny_product=" + String(1e-320 * 1e-10) + "," + String(-1e-320 * 1e-10) + " neg0=" + Object.is(-1e-320 * 1e-10, -0));
const sub3 = Number.MIN_VALUE * 3;
console.log("sub_times3=" + String(sub3) + " div3_roundtrip=" + (sub3 / 3 === Number.MIN_VALUE));
console.log("subnormal_gap_const=" + (2 ** -1074 * 2 - 2 ** -1074 === 2 ** -1074));
console.log("subnormals_finite=" + Number.isFinite(Number.MIN_VALUE) + "," + Number.isFinite(Number.MIN_VALUE * 7));
console.log("subnormal_not_zero=" + (Number.MIN_VALUE !== 0) + " truthy=" + (Number.MIN_VALUE ? "yes" : "no"));
console.log("subnormal_toFixed=" + Number.MIN_VALUE.toFixed(3) + " toExponential=" + Number.MIN_VALUE.toExponential(3));
console.log("one_plus_min=" + (1 + Number.MIN_VALUE === 1) + " one_plus_eps=" + (1 + Number.EPSILON === 1));
console.log("one_plus_half_eps=" + (1 + Number.EPSILON / 2 === 1) + " with_min=" + (1 + (Number.EPSILON / 2 + Number.MIN_VALUE) === 1));
console.log("parse_underflow=" + String(Number("1e-400")) + " neg0=" + Object.is(Number("-1e-400"), -0));
console.log("literal_underflow=" + String(1e-400) + " neg0=" + Object.is(-1e-400, -0));
const edges: [string, number][] = [
  ["+overflow", 1e308 * 10], ["-overflow", -1e308 * 10],
  ["+underflow", 1e-320 * 1e-10], ["-underflow", -1e-320 * 1e-10],
];
for (const e of edges) {
  console.log(e[0] + "=" + String(e[1]) + " sign=" + String(Math.sign(e[1])) + " neg0=" + Object.is(e[1], -0));
}
