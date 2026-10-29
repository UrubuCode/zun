// Number formatting by digits: toFixed, toPrecision and toExponential. Covers the
// rounding of binary doubles (ties resolve to the larger candidate), the 1e21
// fall-back of toFixed, the argument ranges (0..100, 1..100, 0..100), the order
// of checks (receiver and non-finite before range), argument coercion, and the
// zero and negative-zero renderings.

function attempt(label: string, fn: () => string): void {
  try {
    console.log(label + "=" + fn());
  } catch (e) {
    console.log(label + "!" + (e as Error).name);
  }
}

// --- toFixed: digit sweep over one value and over a value that carries ---
for (let d = 0; d <= 10; d++) {
  console.log("sweep_1.23456789_d" + d + "=" + (1.23456789).toFixed(d));
}
for (let d = 0; d <= 8; d++) {
  console.log("sweep_9.99999_d" + d + "=" + (9.99999).toFixed(d));
}

// --- toFixed: binary doubles that look like ties but are not exact ---
console.log("fixed_0.1_1=" + (0.1).toFixed(1));
console.log("fixed_1.005_2=" + (1.005).toFixed(2));
console.log("fixed_2.005_2=" + (2.005).toFixed(2));
console.log("fixed_8.575_2=" + (8.575).toFixed(2));
console.log("fixed_1.255_2=" + (1.255).toFixed(2));
console.log("fixed_123.456_2=" + (123.456).toFixed(2));
console.log("fixed_0.000001_2=" + (0.000001).toFixed(2));
console.log("fixed_0_3=" + (0).toFixed(3));

// --- toFixed: exact ties go to the larger candidate, so 0.5 rounds up and -1.5 down ---
console.log("tie_0.5_0=" + (0.5).toFixed(0) + " tie_1.5_0=" + (1.5).toFixed(0) + " tie_2.5_0=" + (2.5).toFixed(0));
console.log("tie_neg1.5_0=" + (-1.5).toFixed(0) + " tie_neg2.5_0=" + (-2.5).toFixed(0));
console.log("tie_1.25_1=" + (1.25).toFixed(1) + " neg_1.25_1=" + (-1.25).toFixed(1));
console.log("tie_999.5_0=" + (999.5).toFixed(0));

// --- toFixed(0) has no decimal point; signs of small values are dropped when zero ---
console.log("f0_1.5=" + (1.5).toFixed(0) + " f0_zero=" + (0).toFixed(0) + " f0_neg_zero=" + (-0).toFixed(0));
console.log("f0_0.4=" + (0.4).toFixed(0) + " f0_neg0.4=" + (-0.4).toFixed(0) + " f0_neg0.6=" + (-0.6).toFixed(0));
console.log("f0_1e20=" + (1e20).toFixed(0));
console.log("f_no_arg=" + (3.7).toFixed() + " f_undefined=" + (3.7).toFixed(undefined));
console.log("neg_zero_f2=" + (-0).toFixed(2) + " neg_tiny_f2=" + (-0.0001).toFixed(2) + " neg_tiny_f5=" + (-0.0001).toFixed(5));
console.log("neg_1e_minus7_f2=" + (-1e-7).toFixed(2));

// --- toFixed: tiny magnitudes disappear into zeros; subnormals too ---
console.log("tiny_1e-7_f2=" + (1e-7).toFixed(2) + " tiny_1e-7_f10=" + (1e-7).toFixed(10));
console.log("tiny_1e-10_f5=" + (1e-10).toFixed(5) + " subnormal_f2=" + (5e-324).toFixed(2));
console.log("tiny_0.000000499_f6=" + (0.000000499).toFixed(6));
console.log("min_value_f100_tail=" + Number.MIN_VALUE.toFixed(100).slice(-4) + " len=" + Number.MIN_VALUE.toFixed(100).length);

// --- toFixed: the digit range is 0..100 inclusive, and the digits are exact ---
console.log("f20_of_1=" + (1).toFixed(20));
console.log("f20_of_0.1=" + (0.1).toFixed(20));
console.log("f100_len=" + (1).toFixed(100).length + " f100_zero_len=" + (0).toFixed(100).length);
console.log("f50_of_0.5=" + (0.5).toFixed(50));
attempt("f101", () => (1.5).toFixed(101));
attempt("fneg1", () => (1.5).toFixed(-1));
attempt("f1000", () => (1.5).toFixed(1000));
attempt("finf", () => (1.5).toFixed(Infinity));
attempt("fneginf", () => (1.5).toFixed(-Infinity));
attempt("fnan_arg", () => (1.5).toFixed(NaN));

// --- toFixed: the argument goes through ToIntegerOrInfinity (truncation) ---
console.log("arg_2p9=" + (1.23456).toFixed(2.9) + " arg_2p1=" + (1.23456).toFixed(2.1));
console.log("arg_3p1=" + (1.23456).toFixed(3.1) + " arg_str2=" + (1.23456).toFixed("2" as any));
console.log("arg_str2p9=" + (1.23456).toFixed("2.9" as any) + " arg_true=" + (1.23456).toFixed(true as any));
console.log("arg_false=" + (1.23456).toFixed(false as any) + " arg_null=" + (1.23456).toFixed(null as any));
console.log("arg_empty=" + (1.23456).toFixed("" as any) + " arg_valueOf3=" + (1.23456).toFixed({ valueOf: () => 3 } as any));
console.log("arg_array=" + (1.23456).toFixed([2] as any));
console.log("neg_zero_arg=" + (1.5).toFixed(-0));

// --- toFixed: the 1e21 boundary switches to ToString and drops the digits ---
console.log("e20_f2=" + (1e20).toFixed(2));
console.log("e21_f2=" + (1e21).toFixed(2) + " e21_f0=" + (1e21).toFixed(0));
console.log("e21_neg_f3=" + (-1e21).toFixed(3) + " e21_5_f4=" + (1.5e21).toFixed(4));
console.log("e22_f2=" + (1e22).toFixed(2) + " e30_f2=" + (1e30).toFixed(2));
console.log("just_under_f1=" + (999999999999999999999).toFixed(1));
console.log("e1e23_f0=" + (1e23).toFixed(0) + " e9p999e20_f2=" + (9.999e20).toFixed(2));

// --- toFixed: non-finite receivers ignore the digits, after the range check ---
console.log("nan_f2=" + NaN.toFixed(2) + " inf_f2=" + Infinity.toFixed(2) + " neginf_f0=" + (-Infinity).toFixed(0));
attempt("nan_f101", () => NaN.toFixed(101));
attempt("nan_fneg1", () => NaN.toFixed(-1));
attempt("inf_f101", () => Infinity.toFixed(101));

// --- toPrecision: one significant figure, the most aggressive case ---
console.log("p1_0=" + (0).toPrecision(1) + " p1_1=" + (1).toPrecision(1) + " p1_9=" + (9).toPrecision(1));
console.log("p1_9.5=" + (9.5).toPrecision(1) + " p1_9.99=" + (9.99).toPrecision(1) + " p1_99=" + (99).toPrecision(1));
console.log("p1_123.456=" + (123.456).toPrecision(1) + " p1_0.5=" + (0.5).toPrecision(1) + " p1_0.05=" + (0.05).toPrecision(1));
console.log("p1_1234=" + (1234).toPrecision(1) + " p1_neg9.99=" + (-9.99).toPrecision(1));

// --- toPrecision: switches to exponential when e >= p, and when e < -6 ---
console.log("exp_100_p1=" + (100).toPrecision(1) + " p2=" + (100).toPrecision(2) + " p3=" + (100).toPrecision(3) + " p4=" + (100).toPrecision(4));
console.log("exp_999_p2=" + (999).toPrecision(2) + " p3=" + (999).toPrecision(3));
console.log("exp_1000_p3=" + (1000).toPrecision(3) + " p4=" + (1000).toPrecision(4));
console.log("exp_1234.5_p2=" + (1234.5).toPrecision(2) + " p5=" + (1234.5).toPrecision(5) + " p6=" + (1234.5).toPrecision(6));
console.log("small_1e-6_p1=" + (0.000001).toPrecision(1) + " 1e-7_p1=" + (0.0000001).toPrecision(1));
console.log("small_1e-6_p3=" + (0.000001).toPrecision(3) + " 1e-7_p3=" + (0.0000001).toPrecision(3));
console.log("small_1.23e-6_p2=" + (0.00000123).toPrecision(2) + " 1.23e-5_p2=" + (0.0000123).toPrecision(2));
console.log("small_0.0001234_p2=" + (0.0001234).toPrecision(2) + " 0.00001234_p2=" + (0.00001234).toPrecision(2));

// --- toPrecision: rounding that carries into the exponent ---
console.log("carry_9.99_p2=" + (9.99).toPrecision(2) + " 99.9_p2=" + (99.9).toPrecision(2));
console.log("carry_0.0999_p2=" + (0.0999).toPrecision(2) + " 9999_p3=" + (9999).toPrecision(3));
console.log("carry_0.00009999_p2=" + (0.00009999).toPrecision(2));

// --- toPrecision: undefined means ToString; 1..100 is the range, 0 and 101 throw ---
console.log("pr_omitted=" + (123.456).toPrecision() + " pr_undef=" + (123.456).toPrecision(undefined));
console.log("pr_0.1=" + (0.1).toPrecision() + " pr_1e21=" + (1e21).toPrecision() + " pr_1e-7=" + (1e-7).toPrecision());
console.log("pr_1e21_p3=" + (1e21).toPrecision(3));
console.log("pr_1_p21=" + (1).toPrecision(21) + " pr_0.1_p21=" + (0.1).toPrecision(21));
attempt("pr_0", () => (1).toPrecision(0));
attempt("pr_22", () => (1).toPrecision(22));
attempt("pr_neg1", () => (1).toPrecision(-1));
attempt("pr_101", () => (1).toPrecision(101));
console.log("pr_100_len=" + (1).toPrecision(100).length);
console.log("pr_fraction_trunc=" + (123.456).toPrecision(2.9) + " " + (123.456).toPrecision(4.1));
console.log("pr_nan=" + NaN.toPrecision(3) + " pr_inf=" + Infinity.toPrecision(3) + " pr_neginf=" + (-Infinity).toPrecision(1));
attempt("pr_nan_0", () => NaN.toPrecision(0));

// --- toExponential: default digits, explicit digits, and the 0..100 range ---
console.log("exp_omitted=" + (123.456).toExponential() + " exp_undef=" + (123.456).toExponential(undefined));
console.log("exp_0=" + (123.456).toExponential(0) + " exp_12345_0=" + (12345).toExponential(0));
console.log("exp_12345_2=" + (12345).toExponential(2) + " exp_12345_4=" + (12345).toExponential(4));
console.log("exp_small=" + (0.00012).toExponential() + " exp_small2=" + (0.00012).toExponential(2));
console.log("exp_neg=" + (-123.45).toExponential(2) + " neg12.34_1=" + (-12.34).toExponential(1));
console.log("exp_pos=" + (12.34).toExponential(2) + " exp_123456_2=" + (123456).toExponential(2));
console.log("exp_0.00045_1=" + (0.00045).toExponential(1) + " exp_5=" + (5).toExponential());
console.log("exp_255_3=" + (255).toExponential(3) + " exp_0_2=" + (0).toExponential(2) + " exp_0_omitted=" + (0).toExponential());
console.log("exp_neg_zero_2=" + (-0).toExponential(2));
console.log("exp_100_len=" + String((1).toExponential(100).length));
attempt("exp_101", () => (1).toExponential(101));
attempt("exp_neg1", () => (1).toExponential(-1));
attempt("exp_inf_arg", () => (1).toExponential(Infinity));
console.log("exp_2p9=" + (12345).toExponential(2.9) + " exp_str2=" + (12345).toExponential("2" as any));
console.log("exp_null=" + (12345).toExponential(null as any));
attempt("exp_nan_500", () => NaN.toExponential(500));
attempt("exp_neginf_nan_arg", () => (-Infinity).toExponential(NaN));
attempt("exp_inf_neg5", () => Infinity.toExponential(-5));

// --- step order: non-finite receivers bail out before the range check, toFixed does not ---
attempt("fix_nan_500", () => NaN.toFixed(500));
attempt("fix_nan_neg1", () => NaN.toFixed(-1));
attempt("prec_neginf_200", () => (-Infinity).toPrecision(200));
attempt("exp_neginf_1000", () => (-Infinity).toExponential(1000));

// --- an omitted argument for toPrecision is ToString, but 1e21 switches notation ---
console.log("exp_1e21_omitted=" + (1e21).toExponential());
console.log("exp_zero_3=" + (0).toExponential(3) + " prec_zero_p3=" + (0).toPrecision(3));
console.log("prec_negzero_p2=" + (-0).toPrecision(2) + " prec_negzero_omit=" + (-0).toPrecision());
console.log("prec_12345_str3=" + (12345).toPrecision("3" as any) + " prec_3p9=" + (12345).toPrecision(3.9));
console.log("prec_valueOf2=" + (12345).toPrecision({ valueOf: () => 2 } as any));
attempt("prec_null_is_zero_range_error", () => (12345).toPrecision(null as any));
console.log("prec_true=" + (12345).toPrecision(true as any));

// --- toFixed and toPrecision applied to the same value: digits versus significance ---
console.log("same_value_fixed=" + (123.456).toFixed(1) + " significant=" + (123.456).toPrecision(4));
console.log("sum_0.1_0.2_f1=" + (0.1 + 0.2).toFixed(1) + " p17=" + (0.1 + 0.2).toPrecision(17));
