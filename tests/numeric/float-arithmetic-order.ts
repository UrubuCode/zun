// Double arithmetic is commutative but neither associative nor distributive, so
// the ORDER of operations changes results: summation order, Kahan compensation,
// running products at the range edges, absorption and cancellation, and the
// division-versus-reciprocal split. Also the integer frontier: where +1 stops
// counting (2^53), where EPSILON stops separating neighbours, and how a `let`
// that starts integral behaves when it later takes fractional values.

function pair(label: string, left: number, right: number): void {
  console.log(
    label +
      " | left:" + String(left) +
      " | right:" + String(right) +
      " | equal:" + (left === right) +
      " | diff:" + (left === right ? "0" : (left - right).toExponential(3))
  );
}

// --- addition is not associative ---
pair("assoc_add_1", (0.1 + 0.2) + 0.3, 0.1 + (0.2 + 0.3));
pair("assoc_add_2", (1e16 + 1) + 1, 1e16 + (1 + 1));
pair("assoc_add_3", (1 + 1e100) - 1e100, 1 + (1e100 - 1e100));
pair("assoc_add_4", (0.1 + 0.1) + 0.1, 0.1 + (0.1 + 0.1));
pair("assoc_add_5", (1e-320 + 1) - 1, 1e-320 + (1 - 1));

// --- multiplication is not associative either ---
pair("assoc_mul_1", (0.1 * 0.2) * 0.3, 0.1 * (0.2 * 0.3));
pair("assoc_mul_2", (1e308 * 10) * 1e-10, 1e308 * (10 * 1e-10));
pair("assoc_mul_3", (1e-320 * 1e10) * 1e10, 1e-320 * (1e10 * 1e10));
pair("assoc_mul_4", (3 * 0.1) * 10, 3 * (0.1 * 10));

// --- and multiplication does not distribute over addition ---
pair("distrib_1", 0.1 * (0.2 + 0.3), 0.1 * 0.2 + 0.1 * 0.3);
pair("distrib_2", 3 * (1 / 3), 3 / 3);
pair("distrib_3", (0.1 + 0.2) * 3, 0.1 * 3 + 0.2 * 3);

// --- addition and multiplication ARE commutative for every finite pair ---
const operands: number[] = [0.1, 0.2, 1e16, 1e-320, 3, -7.5, 1e308, Number.EPSILON];
const nonCommutative: string[] = [];
for (const a of operands) {
  for (const b of operands) {
    if (!Object.is(a + b, b + a)) nonCommutative.push("add:" + a + "," + b);
    if (!Object.is(a * b, b * a)) nonCommutative.push("mul:" + a + "," + b);
  }
}
console.log("non_commutative=[" + nonCommutative.join(" ") + "]");

// --- summation order changes the total: forwards, backwards, sorted by magnitude ---
const terms: number[] = [1e16, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1e16];
function sum(list: number[]): number {
  let acc = 0;
  for (const t of list) acc += t;
  return acc;
}
const forwards = sum(terms);
const backwards = sum(terms.slice().reverse());
const byMagnitude = sum(terms.slice().sort((a, b) => Math.abs(a) - Math.abs(b)));
console.log("sum_forwards=" + String(forwards) + " sum_backwards=" + String(backwards) + " sum_by_magnitude=" + String(byMagnitude));
console.log("orders_agree=" + (forwards === backwards) + "," + (forwards === byMagnitude));
console.log("reduce_matches_loop=" + (terms.reduce((a, b) => a + b, 0) === forwards));

// --- Kahan compensation recovers what the naive loop dropped ---
function kahan(list: number[]): number {
  let acc = 0;
  let comp = 0;
  for (const t of list) {
    const y = t - comp;
    const next = acc + y;
    comp = next - acc - y;
    acc = next;
  }
  return acc;
}
const tenths: number[] = [];
for (let i = 0; i < 10; i++) tenths.push(0.1);
console.log("naive_ten_tenths=" + String(sum(tenths)) + " kahan_ten_tenths=" + String(kahan(tenths)));
console.log("naive_is_one=" + (sum(tenths) === 1) + " kahan_is_one=" + (kahan(tenths) === 1));
console.log("naive_terms=" + String(kahan(terms)) + " vs " + String(sum(terms)));

// --- a running product is order-dependent at the range edges ---
function product(list: number[]): number {
  let acc = 1;
  for (const f of list) acc = acc * f;
  return acc;
}
console.log("product_forwards=" + String(product([1e300, 1e-300, 1e300, 1e-300])));
console.log("product_reordered=" + String(product([1e300, 1e300, 1e-300, 1e-300])));
console.log("product_overflows_first=" + String(1e300 * 1e300 * 1e-300 * 1e-300));

// --- absorption and cancellation: a small addend vanishes into a large sum ---
console.log("absorb=" + ((1e16 + 1) - 1e16) + " absorb_reverse=" + (1e16 - 1e16 + 1));
console.log("cancellation=" + String((1 + 1e-16) - 1) + " catastrophic=" + String((1.0000001 - 1) * 1e7));
console.log("x_minus_x_pos_zero=" + Object.is(0.1 - 0.1, 0) + " neg_plus_pos=" + Object.is(-0.1 + 0.1, 0));

// --- division and multiplication by the reciprocal do not agree ---
const recipDisagree: string[] = [];
for (const a of [1, 3, 7, 0.1, 49, 1e300]) {
  if (10 / a !== 10 * (1 / a)) recipDisagree.push(String(a));
}
console.log("div_vs_reciprocal_disagree=[" + recipDisagree.join(",") + "]");
console.log("sqrt_squared_is_two=" + (Math.sqrt(2) * Math.sqrt(2) === 2) + " sqrt_four=" + Math.sqrt(4) + " sqrt_two_sq=" + Math.sqrt(2) * Math.sqrt(2));
console.log("x_over_x=" + (0.1 / 0.1) + "," + (1e-320 / 1e-320) + "," + String(0 / 0));

// --- the integer frontier: exact below 2^53, then +1 stops counting ---
const MS = Number.MAX_SAFE_INTEGER;
console.log("MS_plus_1=" + (MS + 1) + " MS_plus_2=" + (MS + 2) + " MS_plus_3=" + (MS + 3) + " MS_plus_4=" + (MS + 4));
console.log("MS_plus1_eq_plus2=" + (MS + 1 === MS + 2) + " MS_plus3_eq_plus4=" + (MS + 3 === MS + 4));
console.log("MS_times_2=" + MS * 2 + " MS_minus_1_minus_2_eq=" + (-MS - 1 === -MS - 2));
console.log("ms_plus1_literal=" + 9007199254740993 + " two_pow_53_eq=" + (9007199254740992 === 9007199254740993));
console.log("two_pow_53=" + 2 ** 53 + " plus_1=" + (2 ** 53 + 1) + " plus_2=" + (2 ** 53 + 2) + " plus_3=" + (2 ** 53 + 3));
console.log("p53_plus1_eq_p53=" + (2 ** 53 + 1 === 2 ** 53) + " p53_plus2_eq_p53=" + (2 ** 53 + 2 === 2 ** 53));
console.log("safe_at_p53=" + Number.isSafeInteger(2 ** 53) + " safe_at_p53_minus_1=" + Number.isSafeInteger(2 ** 53 - 1));
console.log("big_collisions=" + (18014398509481984 + 1) + " " + (9999999999999999) + " " + (123456789012345680 === 123456789012345690));
console.log("neg_frontier=" + (Number.MIN_SAFE_INTEGER - 1) + " " + (Number.MIN_SAFE_INTEGER - 2) + " eq=" + (Number.MIN_SAFE_INTEGER - 1 === Number.MIN_SAFE_INTEGER - 2));
console.log("max_safe_isSafe=" + Number.isSafeInteger(MS) + " plus1_isSafe=" + Number.isSafeInteger(MS + 1));

// --- EPSILON: 1 + EPSILON is the next double after 1, and the gap doubles near 2 ---
console.log("eps_one=" + (1 + Number.EPSILON > 1) + " half_eps_one=" + (1 + Number.EPSILON / 2 === 1));
console.log("eps_minus_one=" + (1 + Number.EPSILON - 1) + " eps_is_2_pow_neg52=" + (Number.EPSILON === Math.pow(2, -52)));
console.log("eps_two=" + (2 + Number.EPSILON > 2) + " two_eps_two=" + (2 + 2 * Number.EPSILON > 2) + " eq=" + (2 + Number.EPSILON === 2));
console.log("eps_half=" + (0.5 + Number.EPSILON > 0.5));
console.log("point_three_diff=" + Math.abs(0.1 + 0.2 - 0.3) + " below_eps=" + (Math.abs(0.1 + 0.2 - 0.3) < Number.EPSILON));
console.log("ms_times_2_minus_ms=" + (MS * 2 - MS === MS) + " ms_div_2_is_fraction=" + (MS / 2));
console.log("min_safe_mirror=" + (Number.MIN_SAFE_INTEGER + 1) + " " + (Number.MIN_SAFE_INTEGER - 1 === Number.MIN_SAFE_INTEGER));

// --- ++ at the top of the safe range stops moving once the double saturates ---
let big: number = MS;
big++;
const afterFirst = big;
big++;
console.log("maxsafe_plus1=" + afterFirst + " maxsafe_plus2=" + big + " isSafe=" + Number.isSafeInteger(big));

// --- a `let` initialised with an integral float literal must take fractions later ---
let py = 20.0;
py = 10 + 2.7;
console.log("py=" + py);
let vy = 0.0;
vy = vy - 0.272;
console.log("vy=" + vy);
let acc = 0.0;
acc += 0.5;
acc -= 0.1;
console.log("acc=" + acc);
let ok: number = 20.0;
ok = 10 + 2.7;
console.log("ok=" + ok);
let yaw = 0.7;
yaw = yaw + 0.05;
console.log("yaw=" + yaw);

// --- an integer-only loop stays integral and matches the closed form ---
let fa = 0.0;
let fb = 1.0;
let fi = 0.0;
while (fi < 10.0) {
  const t = fa + fb;
  fa = fb;
  fb = t;
  fi = fi + 1.0;
}
console.log("fib10=" + fa + " integral=" + Number.isInteger(fa));
let fib = [0, 1];
for (let k = 2; k <= 78; k++) fib.push(fib[k - 1] + fib[k - 2]);
console.log("fib78=" + String(fib[78]) + " exact=" + (fib[78] === 8944394323791464));
