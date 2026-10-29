// Roots, powers, logarithms, exponentials and hyperbolic functions.
// Focus: sqrt/cbrt on exact results and their special rows, Math.pow and the
// ** operator (special-case table, exact power-of-two and power-of-ten ranges,
// negative bases), the log/exp special rows, hyperbolic special rows, and the
// transcendental identities asserted as booleans inside a tolerance.
// Irrational digits are never printed; only exact doubles or booleans are.

function fmt(v: number): string {
  return String(v) + (Object.is(v, -0) ? "(-0)" : "");
}

function probe(label: string, r: number): void {
  console.log(label + " = " + fmt(r));
}

function near(label: string, a: number, b: number): void {
  const diff = Math.abs(a - b);
  const scale = Math.max(1, Math.abs(a), Math.abs(b));
  console.log(label + "=" + (diff / scale < 1e-12));
}

// --- sqrt: perfect squares round-trip exactly ---
const badSquares: string[] = [];
for (let i = 0; i <= 100; i++) {
  if (Math.sqrt(i * i) !== i) badSquares.push(String(i));
}
console.log("square_roundtrip_failures=[" + badSquares.join(",") + "]");

const badPow2Sqrt: string[] = [];
for (let k = 0; k <= 500; k += 2) {
  if (Math.sqrt(Math.pow(2, k)) !== Math.pow(2, k / 2)) badPow2Sqrt.push(String(k));
}
console.log("pow2_sqrt_failures=[" + badPow2Sqrt.join(",") + "]");

const squares: number[] = [0, 1, 4, 9, 16, 25, 100, 144, 1024, 65536, 16777216, 4294967296];
const sq: string[] = [];
for (const v of squares) {
  sq.push(String(v) + "->" + fmt(Math.sqrt(v)));
}
console.log("sqrt_table=" + sq.join(" "));

const perfect: string[] = [];
for (let i = 0; i <= 50; i++) {
  if (Number.isInteger(Math.sqrt(i))) perfect.push(String(i));
}
console.log("perfect_squares_0_50=" + perfect.join(","));

probe("sqrt(-0)", Math.sqrt(-0));
probe("sqrt(+0)", Math.sqrt(0));
probe("sqrt(-1)", Math.sqrt(-1));
probe("sqrt(-1e-320)", Math.sqrt(-1e-320));
probe("sqrt(Infinity)", Math.sqrt(Infinity));
probe("sqrt(-Infinity)", Math.sqrt(-Infinity));
probe("sqrt(NaN)", Math.sqrt(NaN));
probe("sqrt(2^52)", Math.sqrt(4503599627370496));
console.log("sqrt_MIN_VALUE_squared_is_min=" + (Math.sqrt(Number.MIN_VALUE) * Math.sqrt(Number.MIN_VALUE) === Number.MIN_VALUE));
console.log("sqrt_MAX_VALUE_finite=" + Number.isFinite(Math.sqrt(Number.MAX_VALUE)));
console.log("sqrt_string=" + fmt(Math.sqrt("81" as any)));
console.log("sqrt_bool=" + fmt(Math.sqrt(true as any)));
console.log("sqrt_no_args=" + fmt((Math.sqrt as any)()));

// --- cbrt: perfect cubes round-trip exactly, both signs ---
const badCubes: string[] = [];
for (let i = -60; i <= 60; i++) {
  if (Math.cbrt(i * i * i) !== i) badCubes.push(String(i));
}
console.log("cube_roundtrip_failures=[" + badCubes.join(",") + "]");

const cubes: number[] = [0, 1, 8, 27, 64, 125, 1000, 1000000, 1073741824, -1, -8, -27, -1000];
const cb: string[] = [];
for (const v of cubes) {
  cb.push(String(v) + "->" + fmt(Math.cbrt(v)));
}
console.log("cbrt_table=" + cb.join(" "));

probe("cbrt(+0)", Math.cbrt(0));
probe("cbrt(-0)", Math.cbrt(-0));
probe("cbrt(Infinity)", Math.cbrt(Infinity));
probe("cbrt(-Infinity)", Math.cbrt(-Infinity));
probe("cbrt(NaN)", Math.cbrt(NaN));
console.log("cbrt_is_odd=" + (Math.cbrt(-125) === -Math.cbrt(125)));
probe("pow(-8,1/3)", Math.pow(-8, 1 / 3));
probe("operator(-8)**(1/3)", (-8) ** (1 / 3));
probe("cbrt(-8)", Math.cbrt(-8));
probe("cbrt(-27)", Math.cbrt(-27));
console.log("cbrt_string=" + fmt(Math.cbrt("-27" as any)));

// --- Math.pow special-case table, checked against the ** operator ---
function pow(a: number, b: number): void {
  const viaMethod = Math.pow(a, b);
  const viaOperator = a ** b;
  const agree = Object.is(viaMethod, viaOperator) || (Number.isNaN(viaMethod) && Number.isNaN(viaOperator));
  console.log(String(a) + " ** " + String(b) + " = " + fmt(viaMethod) + " agree:" + agree);
}

// an exponent of zero wins over everything, NaN and infinite bases included
pow(NaN, 0);
pow(NaN, -0);
pow(Infinity, 0);
pow(-Infinity, -0);
pow(0, 0);
pow(-0, 0);
pow(0, -0);

// a NaN exponent poisons, and a base of 1 does not rescue it
pow(1, NaN);
pow(-1, NaN);
pow(2, NaN);
pow(NaN, NaN);
pow(NaN, 1);
pow(NaN, -1);

// |base| == 1 with an infinite exponent is NaN, not 1
pow(1, Infinity);
pow(1, -Infinity);
pow(-1, Infinity);
pow(-1, -Infinity);

// |base| above and below 1 with infinite exponents
pow(2, Infinity);
pow(2, -Infinity);
pow(-2, Infinity);
pow(-2, -Infinity);
pow(0.5, Infinity);
pow(0.5, -Infinity);
pow(-0.5, Infinity);
pow(-0.5, -Infinity);
pow(0, Infinity);
pow(0, -Infinity);
pow(-0, Infinity);
pow(-0, -Infinity);

// an infinite base keeps its sign only for odd integer exponents
pow(Infinity, 1);
pow(Infinity, 2);
pow(Infinity, -1);
pow(Infinity, -2);
pow(-Infinity, 3);
pow(-Infinity, 2);
pow(-Infinity, 2.5);
pow(-Infinity, -3);
pow(-Infinity, -2);
pow(-Infinity, -2.5);

// a zero base, both signs, both directions
pow(0, 3);
pow(0, -3);
pow(0, 0.5);
pow(-0, 3);
pow(-0, 2);
pow(-0, 2.5);
pow(-0, -3);
pow(-0, -2);
pow(-0, -2.5);
pow(0, -0.5);

// a negative base with a non-integer exponent has no real answer
pow(-2, 0.5);
pow(-8, 1 / 3);
pow(-1, 0.5);
pow(-4, 1.5);
pow(-2, -0.5);

// exact integer and simple fractional powers
pow(2, 10);
pow(2, 53);
pow(-2, 3);
pow(-2, 4);
pow(10, 21);
pow(4, 0.5);
pow(9, 0.5);
pow(16, 0.25);
pow(2, -2);
pow(0.5, -1);

console.log("pow_disagree_grid=[" + (function () {
  const disagree: string[] = [];
  for (const b of [2, 3, 10, -2, 0.5, 1.5]) {
    for (let k = -5; k <= 20; k++) {
      if (!Object.is(Math.pow(b, k), b ** k)) disagree.push(b + "^" + k);
    }
  }
  return disagree.join(",");
})() + "]");

// --- powers of two are exact across the whole range, both directions ---
const badPow2: string[] = [];
let acc = 1;
for (let k = 0; k <= 1023; k++) {
  if (Math.pow(2, k) !== acc || 2 ** k !== acc) badPow2.push(String(k));
  acc = acc * 2;
}
console.log("pow2_failures=[" + badPow2.join(",") + "]");
const badNeg: string[] = [];
for (let k = 0; k <= 1023; k++) {
  if (Math.pow(2, -k) !== 1 / Math.pow(2, k)) badNeg.push(String(k));
}
console.log("neg_pow2_failures=[" + badNeg.join(",") + "]");
probe("pow2_1023", Math.pow(2, 1023));
probe("pow2_1024", Math.pow(2, 1024));
probe("pow2_-1074", 2 ** -1074);
probe("pow2_-1075", 2 ** -1075);
console.log("pow2_53_plus_one_lost=" + (2 ** 53 === 2 ** 53 + 1));

// --- powers of ten are exact integers through 10^22 ---
const ten: string[] = [];
for (let k = 0; k <= 22; k++) {
  ten.push(String(k) + ":" + String(10 ** k));
}
console.log("pow10=" + ten.join(" "));
console.log("10e22_is_exact=" + (10 ** 22 === 1e22));
console.log("pow10_309_infinite=" + (10 ** 309 === Infinity));

// --- a negative base: the parity of an integer exponent decides the sign ---
const signs: string[] = [];
for (let k = 0; k <= 8; k++) {
  signs.push(k + ":" + String((-2) ** k));
}
console.log("neg_base=" + signs.join(" "));
probe("neg_base_frac_exponent", (-2) ** 0.5);
probe("neg_base_neg_odd", (-2) ** -3);
probe("neg_zero_odd", (-0) ** 3);
probe("neg_zero_even", (-0) ** 2);
probe("neg_zero_neg_odd", (-0) ** -3);
probe("neg_zero_neg_even", (-0) ** -2);

// --- operator associativity and operand coercion ---
console.log("right_assoc=" + String(2 ** 3 ** 2));
console.log("right_assoc_is_512=" + (2 ** 3 ** 2 === 512));
console.log("left_paren=" + String((2 ** 3) ** 2));
console.log("chain_four=" + String(2 ** 2 ** 2 ** 2));
console.log("string_exponent=" + String(2 ** ("3" as any)));
console.log("bool_exponent=" + String(2 ** (true as any)));
console.log("compound_assign=" + (function () { let x = 3; x **= 2; return String(x); })());
console.log("huge_exponent=" + String(1.0000001 ** 1e12));
console.log("tiny_base_huge_exponent=" + String(0.9999999 ** 1e12));

const order: string[] = [];
const base: any = { valueOf() { order.push("base"); return 2; } };
const exp: any = { valueOf() { order.push("exp"); return 3; } };
console.log("pow_coerced=" + String(Math.pow(base, exp)));
console.log("pow_order=" + order.join(","));
console.log("pow_string_operands=" + String(Math.pow("2" as any, "3" as any)));
console.log("pow_null_base=" + String(Math.pow(null as any, 2)));
console.log("pow_undefined_exp=" + String(Math.pow(2, undefined as any)));
console.log("pow_length=" + Math.pow.length);

// --- log family special rows ---
probe("log(NaN)", Math.log(NaN));
probe("log(+0)", Math.log(0));
probe("log(-0)", Math.log(-0));
probe("log(1)", Math.log(1));
probe("log(-1)", Math.log(-1));
probe("log(-Infinity)", Math.log(-Infinity));
probe("log(Infinity)", Math.log(Infinity));
probe("log2(NaN)", Math.log2(NaN));
probe("log2(+0)", Math.log2(0));
probe("log2(-0)", Math.log2(-0));
probe("log2(1)", Math.log2(1));
probe("log2(2)", Math.log2(2));
probe("log2(8)", Math.log2(8));
probe("log2(1024)", Math.log2(1024));
probe("log2(0.5)", Math.log2(0.5));
probe("log2(MIN_VALUE)", Math.log2(Number.MIN_VALUE));
probe("log2(-1)", Math.log2(-1));
probe("log2(Infinity)", Math.log2(Infinity));
probe("log10(NaN)", Math.log10(NaN));
probe("log10(+0)", Math.log10(0));
probe("log10(-0)", Math.log10(-0));
probe("log10(1)", Math.log10(1));
probe("log10(10)", Math.log10(10));
probe("log10(100)", Math.log10(100));
probe("log10(-1)", Math.log10(-1));
probe("log10(-Infinity)", Math.log10(-Infinity));
probe("log1p(NaN)", Math.log1p(NaN));
probe("log1p(+0)", Math.log1p(0));
probe("log1p(-0)", Math.log1p(-0));
probe("log1p(-1)", Math.log1p(-1));
probe("log1p(-2)", Math.log1p(-2));
probe("log1p(Infinity)", Math.log1p(Infinity));
probe("log1p(-Infinity)", Math.log1p(-Infinity));
console.log("log_zero_is_neginf=" + (Math.log(0) === -Infinity && Math.log2(0) === -Infinity && Math.log10(0) === -Infinity));

// --- exp family special rows ---
probe("exp(NaN)", Math.exp(NaN));
probe("exp(+0)", Math.exp(0));
probe("exp(-0)", Math.exp(-0));
probe("exp(Infinity)", Math.exp(Infinity));
probe("exp(-Infinity)", Math.exp(-Infinity));
probe("exp(1000)", Math.exp(1000));
probe("exp(-1000)", Math.exp(-1000));
probe("expm1(NaN)", Math.expm1(NaN));
probe("expm1(+0)", Math.expm1(0));
probe("expm1(-0)", Math.expm1(-0));
probe("expm1(Infinity)", Math.expm1(Infinity));
probe("expm1(-Infinity)", Math.expm1(-Infinity));
probe("expm1(-1000)", Math.expm1(-1000));
console.log("exp_log_1=" + (Math.exp(Math.log(1)) === 1));
console.log("log_exp_0=" + (Math.log(Math.exp(0)) === 0));
console.log("log2_powers=" + (Math.log2(4) === 2 && Math.log2(16) === 4));

// --- exp, log and friends invert each other, to a tolerance ---
near("exp_log_5", Math.exp(Math.log(5)), 5);
near("log_exp_3", Math.log(Math.exp(3)), 3);
near("exp_1_is_E", Math.exp(1), Math.E);
near("log_E", Math.log(Math.E), 1);
near("log2_of_1024", Math.log2(1024), 10);
near("log10_of_100000", Math.log10(100000), 5);
near("log_10_is_LN10", Math.log(10), Math.LN10);
near("log_2_is_LN2", Math.log(2), Math.LN2);
near("log2_e_is_LOG2E", Math.log2(Math.E), Math.LOG2E);
near("log10_e_is_LOG10E", Math.log10(Math.E), Math.LOG10E);
near("log2_via_log", Math.log2(7), Math.log(7) / Math.LN2);
near("log10_via_log", Math.log10(7), Math.log(7) / Math.LN10);
near("expm1_log1p", Math.expm1(Math.log1p(0.5)), 0.5);
near("log1p_expm1", Math.log1p(Math.expm1(0.25)), 0.25);
near("expm1_small", Math.expm1(1e-10), 1e-10);
near("log1p_small", Math.log1p(1e-10), 1e-10);
near("sqrt_squared", Math.sqrt(7) * Math.sqrt(7), 7);
near("cbrt_cubed", Math.cbrt(7) * Math.cbrt(7) * Math.cbrt(7), 7);
near("cbrt_negative", Math.cbrt(-7), -Math.cbrt(7));
near("pow_half_is_sqrt", Math.pow(7, 0.5), Math.sqrt(7));
near("pow_third_is_cbrt", Math.pow(7, 1 / 3), Math.cbrt(7));
near("hypot_is_sqrt_sum", Math.hypot(3.5, 4.5), Math.sqrt(3.5 * 3.5 + 4.5 * 4.5));
near("sqrt2_constant", Math.sqrt(2), Math.SQRT2);
near("sqrt_half_constant", Math.sqrt(0.5), Math.SQRT1_2);
near("ln2_times_log2e", Math.LN2 * Math.LOG2E, 1);
near("ln10_times_log10e", Math.LN10 * Math.LOG10E, 1);
near("sqrt2_times_sqrt1_2", Math.SQRT2 * Math.SQRT1_2, 1);

// --- hyperbolic special rows ---
probe("sinh(NaN)", Math.sinh(NaN));
probe("sinh(+0)", Math.sinh(0));
probe("sinh(-0)", Math.sinh(-0));
probe("sinh(Infinity)", Math.sinh(Infinity));
probe("sinh(-Infinity)", Math.sinh(-Infinity));
probe("sinh(1000)", Math.sinh(1000));
probe("sinh(-1000)", Math.sinh(-1000));
probe("cosh(NaN)", Math.cosh(NaN));
probe("cosh(+0)", Math.cosh(0));
probe("cosh(-0)", Math.cosh(-0));
probe("cosh(Infinity)", Math.cosh(Infinity));
probe("cosh(-Infinity)", Math.cosh(-Infinity));
probe("cosh(1000)", Math.cosh(1000));
probe("tanh(NaN)", Math.tanh(NaN));
probe("tanh(+0)", Math.tanh(0));
probe("tanh(-0)", Math.tanh(-0));
probe("tanh(Infinity)", Math.tanh(Infinity));
probe("tanh(-Infinity)", Math.tanh(-Infinity));
probe("tanh(1000)", Math.tanh(1000));
probe("tanh(-1000)", Math.tanh(-1000));
probe("asinh(NaN)", Math.asinh(NaN));
probe("asinh(+0)", Math.asinh(0));
probe("asinh(-0)", Math.asinh(-0));
probe("asinh(Infinity)", Math.asinh(Infinity));
probe("asinh(-Infinity)", Math.asinh(-Infinity));
probe("acosh(NaN)", Math.acosh(NaN));
probe("acosh(1)", Math.acosh(1));
probe("acosh(0)", Math.acosh(0));
probe("acosh(-1)", Math.acosh(-1));
probe("acosh(Infinity)", Math.acosh(Infinity));
probe("acosh(-Infinity)", Math.acosh(-Infinity));
probe("atanh(NaN)", Math.atanh(NaN));
probe("atanh(+0)", Math.atanh(0));
probe("atanh(-0)", Math.atanh(-0));
probe("atanh(1)", Math.atanh(1));
probe("atanh(-1)", Math.atanh(-1));
probe("atanh(2)", Math.atanh(2));
probe("atanh(Infinity)", Math.atanh(Infinity));
probe("atanh(-Infinity)", Math.atanh(-Infinity));

console.log("sinh_odd=" + (Math.sinh(-2) === -Math.sinh(2)));
console.log("cosh_even=" + (Math.cosh(-2) === Math.cosh(2)));
console.log("tanh_odd=" + (Math.tanh(-2) === -Math.tanh(2)));
console.log("asinh_odd=" + (Math.asinh(-2) === -Math.asinh(2)));
console.log("atanh_odd=" + (Math.atanh(-0.5) === -Math.atanh(0.5)));
console.log("cosh_ge_1=" + (Math.cosh(0.5) >= 1 && Math.cosh(-3) >= 1));
console.log("tanh_bounded=" + (Math.tanh(5) < 1 && Math.tanh(-5) > -1));
console.log("acosh_nonneg=" + (Math.acosh(2) > 0 && Math.acosh(1) === 0));

near("cosh2_minus_sinh2", Math.cosh(1.3) * Math.cosh(1.3) - Math.sinh(1.3) * Math.sinh(1.3), 1);
near("tanh_is_sinh_over_cosh", Math.tanh(0.9), Math.sinh(0.9) / Math.cosh(0.9));
near("asinh_sinh", Math.asinh(Math.sinh(1.1)), 1.1);
near("acosh_cosh", Math.acosh(Math.cosh(1.1)), 1.1);
near("atanh_tanh", Math.atanh(Math.tanh(0.6)), 0.6);
near("sinh_via_exp", Math.sinh(2), (Math.exp(2) - Math.exp(-2)) / 2);
near("cosh_via_exp", Math.cosh(2), (Math.exp(2) + Math.exp(-2)) / 2);

console.log("lengths=" + [Math.log.length, Math.log2.length, Math.exp.length, Math.sqrt.length, Math.sinh.length, Math.acosh.length].join(","));
console.log("names=" + [Math.log1p.name, Math.expm1.name, Math.cbrt.name, Math.atanh.name].join(","));
