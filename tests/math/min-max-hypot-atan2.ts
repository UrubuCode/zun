// Math.min / Math.max, Math.hypot and Math.atan2: the special-case tables and
// the argument-coercion rules that all of them share.
// Focus: empty and single arguments, NaN and Infinity precedence, the -0 vs +0
// ordering of min/max, coercion order (left to right, before any comparison),
// throwing arguments, and the exact rows of hypot and atan2.

function fmt(v: number): string {
  return String(v) + (Object.is(v, -0) ? "(-0)" : "");
}

function attempt(label: string, fn: () => any): void {
  try {
    console.log(label + "=" + fmt(fn()));
  } catch (e) {
    console.log(label + "!" + (e as any).name);
  }
}

// --- identity elements and single arguments ---
console.log("min_empty=" + fmt(Math.min()));
console.log("max_empty=" + fmt(Math.max()));
console.log("min_1=" + fmt(Math.min(1)));
console.log("max_1=" + fmt(Math.max(1)));
console.log("min_neg=" + fmt(Math.min(-5, -10, -1)));
console.log("max_neg=" + fmt(Math.max(-5, -10, -1)));
console.log("min_mixed=" + fmt(Math.min(5, -3, 0, 10)));
console.log("max_mixed=" + fmt(Math.max(5, -3, 0, 10)));
console.log("min_inf=" + fmt(Math.min(1, Infinity)));
console.log("max_inf=" + fmt(Math.max(1, -Infinity)));
console.log("min_4_-2_9=" + fmt(Math.min(4, -2, 9)));
console.log("max_4_-2_9=" + fmt(Math.max(4, -2, 9)));

// --- signed zero: +0 is larger than -0 for both max and min ---
console.log("max(0,-0)=" + fmt(Math.max(0, -0)));
console.log("max(-0,0)=" + fmt(Math.max(-0, 0)));
console.log("min(0,-0)=" + fmt(Math.min(0, -0)));
console.log("min(-0,0)=" + fmt(Math.min(-0, 0)));
console.log("max(-0,-0)=" + fmt(Math.max(-0, -0)));
console.log("min(-0)=" + fmt(Math.min(-0)));

// --- NaN wins in any position; Infinity beats nothing but itself ---
console.log("max_nan_last=" + fmt(Math.max(1, 2, NaN)));
console.log("min_nan_first=" + fmt(Math.min(NaN, 1, 2)));
console.log("max_inf_nan=" + fmt(Math.max(Infinity, NaN)));
console.log("min_neginf_nan=" + fmt(Math.min(-Infinity, NaN)));

// --- comparison is numeric, never lexicographic; every argument is coerced ---
console.log("str_max=" + fmt(Math.max("3" as any, "10" as any)));
console.log("str_min=" + fmt(Math.min("3" as any, "10" as any)));
console.log("mixed_max=" + fmt(Math.max(true as any, null as any, "2" as any)));
console.log("arr_max=" + fmt(Math.max([] as any, [5] as any)));
console.log("obj_max=" + fmt(Math.max({} as any)));
console.log("undef_max=" + fmt(Math.max(undefined as any, 1)));
console.log("emptystr_min=" + fmt(Math.min("" as any, 1)));
console.log("hex_max=" + fmt(Math.max("0x10" as any, 5)));
console.log("wrapper_max=" + fmt(Math.max(new Number(7) as any, 3)));

// --- valueOf is consulted once per argument, in order, even after NaN ---
const log1: string[] = [];
function spy(log: string[], name: string, value: number): any {
  return {
    valueOf() {
      log.push(name);
      return value;
    },
  };
}
console.log("with_nan=" + fmt(Math.max(NaN, spy(log1, "a", 1), spy(log1, "b", 2))));
console.log("coerced_after_nan=" + log1.join(","));

const log2: string[] = [];
console.log("plain=" + fmt(Math.max(spy(log2, "a", 5), spy(log2, "b", 9), spy(log2, "c", 1))));
console.log("order=" + log2.join(","));

// --- a throwing argument in the middle stops the loop: no result at all ---
const log3: string[] = [];
const boom3: any = {
  valueOf() {
    log3.push("boom");
    throw new TypeError("no");
  },
};
try {
  console.log("never=" + fmt(Math.max(spy(log3, "a", 1), boom3, spy(log3, "c", 3))));
} catch (e) {
  console.log("threw=" + (e as any).name);
}
console.log("log_after_throw=" + log3.join(","));

const log4: string[] = [];
const boom4: any = {
  valueOf() {
    log4.push("boom");
    throw new RangeError("no");
  },
};
try {
  Math.min(spy(log4, "a", 1), boom4, spy(log4, "c", 3));
  console.log("min_no_throw");
} catch (e) {
  console.log("min_threw=" + (e as any).name);
}
console.log("min_log=" + log4.join(","));

// --- Symbol and BigInt arguments throw at ToNumber ---
attempt("symbol_max", () => Math.max(1, Symbol("s") as any));
attempt("bigint_max", () => Math.max(1, 2n as any));

// --- @@toPrimitive is asked with hint "number" ---
const hints: string[] = [];
const hinted: any = {
  [Symbol.toPrimitive](hint: string) {
    hints.push(hint);
    return 42;
  },
};
console.log("hinted=" + fmt(Math.max(1, hinted)));
console.log("hint=" + hints.join(","));

// --- spread, apply, and the arity and name of the variadic pair ---
console.log("spread_max=" + fmt(Math.max(...[3, 1, 4, 1, 5])));
console.log("apply_max=" + fmt(Math.max.apply(null, [3, 1, 4, 1, 5])));
console.log("apply_max_empty=" + fmt(Math.max.apply(null, [] as any)));
console.log("spread_min=" + fmt(Math.min(...[3, 1, 4, 1, 5])));
console.log("max_length=" + Math.max.length);
console.log("min_length=" + Math.min.length);
console.log("max_name=" + Math.max.name);

// --- Math.pow, imul, atan2 and hypot coerce LEFT to RIGHT, before computing ---
const order: string[] = [];
function tracked(name: string, value: number): any {
  return {
    valueOf() {
      order.push(name);
      return value;
    },
  };
}
const boom: any = {
  valueOf() {
    throw new RangeError("no");
  },
};

console.log("pow=" + fmt(Math.pow(tracked("base", 2) as any, tracked("exp", 10) as any)));
console.log("pow_order=" + order.join(","));
order.length = 0;
console.log("atan2_sign=" + (Math.atan2(tracked("y", 1), tracked("x", 1)) > 0));
console.log("atan2_order=" + order.join(","));
order.length = 0;
console.log("imul=" + fmt(Math.imul(tracked("a", 3), tracked("b", 4))));
console.log("imul_order=" + order.join(","));
order.length = 0;
console.log("hypot=" + fmt(Math.hypot(tracked("h1", 3), tracked("h2", 4))));
console.log("hypot_order=" + order.join(","));

attempt("pow_throwing_exponent", () => Math.pow(2, boom));
attempt("pow_throwing_base", () => Math.pow(boom, 2));
attempt("atan2_throwing_second", () => Math.atan2(1, boom));
attempt("hypot_throwing_third", () => Math.hypot(3, 4, boom));
attempt("imul_throwing_second", () => Math.imul(1, boom));

order.length = 0;
try {
  Math.pow(tracked("first", 2), boom);
} catch (e) {
  order.push("threw:" + (e as any).name);
}
console.log("coerced_before_throw=" + order.join(","));

// --- hypot: identity, single arguments, and the NaN/Infinity precedence ---
function hyp(label: string, args: number[]): void {
  const r = Math.hypot.apply(null, args as any);
  console.log(label + " hypot(" + args.map(String).join(",") + ") = " + fmt(r));
}

hyp("empty", []);
hyp("one_pos", [3]);
hyp("one_neg", [-3]);
hyp("one_zero", [0]);
hyp("one_negzero", [-0]);
hyp("one_nan", [NaN]);
hyp("one_inf", [Infinity]);
hyp("one_neginf", [-Infinity]);
hyp("inf_then_nan", [Infinity, NaN]);
hyp("nan_then_inf", [NaN, Infinity]);
hyp("neginf_then_nan", [-Infinity, NaN]);
hyp("nan_nan_inf", [NaN, NaN, -Infinity]);
hyp("nan_only", [NaN, 1]);
hyp("nan_nan", [NaN, NaN]);
hyp("inf_inf", [Infinity, -Infinity]);
hyp("inf_finite", [Infinity, 3]);

// --- every all-zero argument list collapses to +0, never -0 ---
hyp("zeros", [0, 0]);
hyp("negzeros", [-0, -0]);
hyp("mixed_zeros", [0, -0]);
hyp("three_negzeros", [-0, -0, -0]);

// --- exact Pythagorean results ---
hyp("3_4", [3, 4]);
hyp("5_12", [5, 12]);
hyp("1_2_2", [1, 2, 2]);
hyp("3_4_12", [3, 4, 12]);
hyp("neg3_neg4", [-3, -4]);
hyp("0_5", [0, 5]);
hyp("neg0_neg5", [-0, -5]);

// --- hypot never overflows or underflows an intermediate square ---
console.log("big_finite=" + isFinite(Math.hypot(1e200, 1e200)));
console.log("big_naive_overflows=" + !isFinite(Math.sqrt(1e200 * 1e200 + 1e200 * 1e200)));
console.log("small_nonzero=" + (Math.hypot(1e-200, 1e-200) > 0));
console.log("small_naive_underflows=" + (Math.sqrt(1e-200 * 1e-200 + 1e-200 * 1e-200) === 0));
console.log("big_scaled_exact=" + (Math.hypot(3e300, 4e300) === 5e300));
console.log("small_scaled_close=" + (Math.abs(Math.hypot(3e-300, 4e-300) / 5e-300 - 1) < 1e-12));
console.log("maxvalue_finite=" + isFinite(Math.hypot(Number.MAX_VALUE, 0)));
console.log("maxvalue_pair_overflows=" + !isFinite(Math.hypot(Number.MAX_VALUE, Number.MAX_VALUE)));

// --- hypot coerces its arguments; string and null inputs are numbers ---
console.log("hypot_strings=" + fmt(Math.hypot("3" as any, "4" as any)));
console.log("hypot_null=" + fmt(Math.hypot(null as any, 5)));
console.log("hypot_undefined=" + fmt(Math.hypot(undefined as any, 5)));
console.log("hypot_bool=" + fmt(Math.hypot(true as any, 0)));
console.log("hypot_arr=" + fmt(Math.hypot([3] as any, [4] as any)));
console.log("hypot_length=" + Math.hypot.length);

// --- atan2: the signed-zero and NaN rows are pinned exactly ---
function atanRow(label: string, y: number, x: number): void {
  const r = Math.atan2(y, x);
  if (Number.isNaN(r) || r === 0) {
    console.log(label + " atan2(" + fmt(y) + "," + fmt(x) + ") = " + fmt(r));
  } else {
    console.log(label + " atan2(" + fmt(y) + "," + fmt(x) + ") = nonzero sign_positive:" + (r > 0) + " within_pi:" + (Math.abs(r) <= Math.PI));
  }
}

// either argument NaN gives NaN
atanRow("nan_y", NaN, 1);
atanRow("nan_x", 1, NaN);
atanRow("nan_both", NaN, NaN);
atanRow("nan_inf", NaN, Infinity);

// y = +0: +0 for non-negative x, pi for negative x (including -0)
atanRow("p0_p0", 0, 0);
atanRow("p0_pos", 0, 1);
atanRow("p0_posinf", 0, Infinity);
atanRow("p0_n0", 0, -0);
atanRow("p0_neg", 0, -1);

// y = -0: the mirror image, and the zero it returns is NEGATIVE
atanRow("n0_p0", -0, 0);
atanRow("n0_pos", -0, 1);
atanRow("n0_posinf", -0, Infinity);
atanRow("n0_n0", -0, -0);
atanRow("n0_neg", -0, -1);

// finite non-zero y against +Infinity x collapses to a signed zero
atanRow("pos_posinf", 1, Infinity);
atanRow("neg_posinf", -1, Infinity);
atanRow("big_posinf", 1e300, Infinity);
atanRow("tiny_posinf", Number.MIN_VALUE, Infinity);

// --- the pi-family rows, as booleans rather than digits ---
const HALF = Math.PI / 2;
console.log("y_pos_x_p0_is_half=" + (Math.atan2(1, 0) === HALF));
console.log("y_pos_x_n0_is_half=" + (Math.atan2(1, -0) === HALF));
console.log("y_neg_x_p0_is_neghalf=" + (Math.atan2(-1, 0) === -HALF));
console.log("y_neg_x_n0_is_neghalf=" + (Math.atan2(-1, -0) === -HALF));
console.log("y_p0_x_n0_is_pi=" + (Math.atan2(0, -0) === Math.PI));
console.log("y_p0_x_neg_is_pi=" + (Math.atan2(0, -1) === Math.PI));
console.log("y_n0_x_n0_is_negpi=" + (Math.atan2(-0, -0) === -Math.PI));
console.log("y_n0_x_neg_is_negpi=" + (Math.atan2(-0, -1) === -Math.PI));
console.log("y_pos_x_neginf_is_pi=" + (Math.atan2(1, -Infinity) === Math.PI));
console.log("y_neg_x_neginf_is_negpi=" + (Math.atan2(-1, -Infinity) === -Math.PI));
console.log("y_posinf_x_finite_is_half=" + (Math.atan2(Infinity, 5) === HALF));
console.log("y_neginf_x_finite_is_neghalf=" + (Math.atan2(-Infinity, 5) === -HALF));
console.log("y_posinf_x_neg_is_half=" + (Math.atan2(Infinity, -5) === HALF));
console.log("y_neginf_x_neg_is_neghalf=" + (Math.atan2(-Infinity, -5) === -HALF));

// --- symmetry and quadrants, which hold whatever the last digit is ---
console.log("neg_y_flips=" + (Math.atan2(-3, 4) === -Math.atan2(3, 4)));
console.log("neg_y_flips2=" + (Math.atan2(-1, -1) === -Math.atan2(1, -1)));
console.log("quadrant1_positive=" + (Math.atan2(1, 1) > 0));
console.log("quadrant2_gt_half=" + (Math.atan2(1, -1) > HALF));
console.log("quadrant3_lt_neghalf=" + (Math.atan2(-1, -1) < -HALF));
console.log("quadrant4_negative=" + (Math.atan2(-1, 1) < 0));
console.log("atan2_in_range=" + (Math.atan2(1, -1) <= Math.PI && Math.atan2(-1, -1) >= -Math.PI));
console.log("atan2_matches_atan=" + (Math.abs(Math.atan2(3, 4) - Math.atan(3 / 4)) < 1e-15));

// --- atan2 arity and coercion ---
console.log("atan2_length=" + Math.atan2.length);
console.log("atan2_no_args_nan=" + Number.isNaN((Math.atan2 as any)()));
console.log("atan2_one_arg_nan=" + Number.isNaN((Math.atan2 as any)(1)));
console.log("atan2_string_args=" + (Math.atan2("1" as any, "0" as any) === HALF));
console.log("atan2_null_y=" + (Math.atan2(null as any, -1) === Math.PI));
console.log("atan2_undefined_nan=" + Number.isNaN(Math.atan2(undefined as any, 1)));
