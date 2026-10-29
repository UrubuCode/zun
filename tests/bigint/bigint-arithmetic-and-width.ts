// Consolidated BigInt fixture: literals, exact arithmetic at arbitrary size,
// division/remainder/shift/power edge cases, bitwise operators over an infinite
// two's-complement representation, and width wrapping with asIntN/asUintN and
// typed-array stores. Every printed value is deterministic.

function op(label: string, fn: () => any): void {
  try {
    console.log(label + "=" + String(fn()));
  } catch (e) {
    console.log(label + "!" + (e as any).name);
  }
}

// --- literals, radix literals and the basic operators ---
console.log("vals=" + [10n, 0xFFn, 0o17n, 0b101n, 1_000n].map(String).join(","));
console.log("big_literal=" + 123456789012345678901234567890n);
console.log("typeof_literal=" + typeof 1n + " typeof_zero=" + typeof 0n);
console.log("add=" + String(10n + 5n));
console.log("mul=" + String(7n * -3n));
console.log("div=" + String(17n / 3n));
console.log("mod=" + String(17n % 3n));
console.log("pow=" + String(2n ** 8n));
console.log("cmp=" + String(10n > 9n));
console.log("bigint_plus_bigint=" + String(3n + 3n) + " unary_minus=" + String(-(5n)));
op("unary_plus_refused", () => +(1n as any));
op("unary_plus_wrapper_refused", () => +(Object(1n) as any));

// --- exact factorials, past the double's exact range ---
function factorial(n: bigint): bigint {
  let acc = 1n;
  for (let i = 2n; i <= n; i++) acc = acc * i;
  return acc;
}
console.log("20!=" + String(factorial(20n)));
console.log("25!=" + String(factorial(25n)));
console.log("30!=" + String(factorial(30n)));
console.log("50!=" + String(factorial(50n)));
console.log("50!_digits=" + factorial(50n).toString().length);
console.log("100!_digits=" + factorial(100n).toString().length);
const f100 = factorial(100n).toString();
console.log("100!_trailing_zeros=" + (f100.length - f100.replace(/0+$/, "").length));
console.log("big_div=" + String(factorial(30n) / factorial(28n)));
console.log("big_mod=" + String(factorial(30n) % 1000000007n));

// --- the same computation in Number diverges where the double gives out ---
function numberFactorial(n: number): number {
  let acc = 1;
  for (let i = 2; i <= n; i++) acc = acc * i;
  return acc;
}
const firstLoss: string[] = [];
for (let n = 1; n <= 25; n++) {
  if (BigInt(numberFactorial(n)) !== factorial(BigInt(n))) firstLoss.push(String(n));
}
console.log("number_factorial_first_wrong=" + firstLoss[0] + " all_wrong_from=" + firstLoss.join(","));
console.log("18!_number=" + String(numberFactorial(18)) + " exact=" + String(factorial(18n)));
console.log("21!_number=" + String(numberFactorial(21)) + " exact=" + String(factorial(21n)));

// --- Fibonacci with a swap, and an identity that must hold exactly ---
function fib(n: number): bigint {
  let a = 0n;
  let b = 1n;
  for (let i = 0; i < n; i++) {
    const t = a + b;
    a = b;
    b = t;
  }
  return a;
}
console.log("fib_50=" + String(fib(50)));
console.log("fib_100=" + String(fib(100)));
console.log("fib_200=" + String(fib(200)));
console.log("fib_300_digits=" + fib(300).toString().length);
console.log("fib_identity=" + (fib(100) === fib(99) + fib(98)));

// --- powers of two, exactly, well past the double's range ---
console.log("2^64=" + String(2n ** 64n) + " 2^64-1=" + String(2n ** 64n - 1n));
console.log("2^100=" + String(2n ** 100n));
console.log("2^1024_digits=" + (2n ** 1024n).toString().length);
console.log("2^4096_digits=" + (2n ** 4096n).toString().length);
console.log("2^64_is_shift=" + ((1n << 64n) === 2n ** 64n));
console.log("2^1024_finite=" + Number.isFinite(Number(2n ** 1024n)) + " as_number=" + String(Number(2n ** 1024n)));
console.log("BigInt_of_2p53=" + String(BigInt(2 ** 53)) + " vs " + String(2n ** 53n));

// --- the safe-integer frontier, from both sides ---
const safe = BigInt(Number.MAX_SAFE_INTEGER);
console.log("max_safe=" + String(safe));
console.log("max_safe_plus_1=" + String(safe + 1n) + " max_safe_plus_2=" + String(safe + 2n));
console.log("number_plus_1=" + String(Number.MAX_SAFE_INTEGER + 1) + " number_plus_2=" + String(Number.MAX_SAFE_INTEGER + 2));
console.log("number_loses_it=" + (Number.MAX_SAFE_INTEGER + 1 === Number.MAX_SAFE_INTEGER + 2));
console.log("bigint_keeps_it=" + (safe + 1n === safe + 2n));
console.log("roundtrip_through_number=" + (BigInt(Number(safe + 2n)) === safe + 2n));

// --- truncating division, remainder sign, and the reconstruction identity ---
op("div_pp", () => 7n / 2n);
op("div_np", () => -7n / 2n);
op("div_pn", () => 7n / -2n);
op("div_nn", () => -7n / -2n);
op("div_exact", () => 100n / 5n);
op("div_smaller", () => 1n / 2n);
op("div_smaller_neg", () => -1n / 2n);
op("div_huge", () => (10n ** 30n) / 7n);
op("mod_pp", () => 7n % 2n);
op("mod_np", () => -7n % 2n);
op("mod_pn", () => 7n % -2n);
op("mod_nn", () => -7n % -2n);
op("mod_exact", () => 6n % 3n);
op("identity_neg", () => (-7n / 2n) * 2n + (-7n % 2n));
op("identity_pos", () => (7n / 2n) * 2n + (7n % 2n));

// --- no Infinity to fall back on ---
op("div_by_zero", () => 1n / 0n);
op("mod_by_zero", () => 1n % 0n);
op("zero_div_zero", () => 0n / 0n);
op("zero_div_one", () => 0n / 1n);
op("neg_zero_is_zero", () => Object.is(-0n, 0n) + "," + String(-0n));

// --- exponentiation ---
op("pow_zero", () => 0n ** 0n);
op("pow_neg_base_odd", () => (-2n) ** 3n);
op("pow_neg_base_even", () => (-2n) ** 4n);
op("pow_big", () => 2n ** 64n);
op("pow_neg_exp", () => 2n ** -1n);
op("pow_zero_neg_exp", () => 0n ** -1n);
op("pow_one_neg_exp", () => 1n ** -1n);

// --- shifts: negative counts reverse direction; >> floors negatives ---
op("shl", () => 1n << 8n);
op("shl_big", () => 1n << 100n);
op("shl_neg_count", () => 8n << -2n);
op("shl_neg_value", () => -1n << 3n);
op("shr", () => 256n >> 4n);
op("shr_neg_count", () => 1n >> -8n);
op("shr_to_zero", () => 1n >> 1n);
op("shr_neg_value", () => -1n >> 1n);
op("shr_neg_value2", () => -5n >> 1n);
op("shr_neg_far", () => -5n >> 100n);
op("shr_pos_far", () => 5n >> 100n);
op("ushr_refused", () => 8n >>> 1n);

// --- bitwise over an infinite two's complement ---
op("not_5", () => ~5n);
op("not_neg1", () => ~-1n);
op("not_0", () => ~0n);
op("and_neg", () => -1n & 255n);
op("and_neg_neg", () => -1n & -2n);
op("or_neg", () => -2n | 1n);
op("xor_self_neg", () => -1n ^ -1n);
op("xor_neg", () => -1n ^ 5n);
op("and_big", () => (2n ** 100n) & 1n);
op("or_big", () => (2n ** 100n) | 1n);

// --- asIntN / asUintN: modular wrapping at arbitrary width ---
function row(bits: number, v: bigint): void {
  console.log("bits=" + bits + " v=" + v + " int=" + BigInt.asIntN(bits, v) + " uint=" + BigInt.asUintN(bits, v));
}
row(0, 0n); row(0, 1n); row(0, -1n); row(0, 255n);
row(1, 0n); row(1, 1n); row(1, 2n); row(1, 3n); row(1, -1n); row(1, -2n);
row(3, 7n); row(3, 4n);
row(8, 127n); row(8, 128n); row(8, 255n); row(8, 256n); row(8, -1n); row(8, -128n); row(8, -129n);
row(32, 2147483647n); row(32, 2147483648n); row(32, 4294967295n); row(32, 4294967296n); row(32, -2147483648n);
row(64, 9223372036854775807n); row(64, 9223372036854775808n);
row(64, 18446744073709551615n); row(64, 18446744073709551616n);
row(64, -9223372036854775808n); row(64, -9223372036854775809n);
row(128, 2n ** 200n); row(128, -(2n ** 200n));
const w = BigInt.asIntN(16, 70000n);
console.log("idempotent=" + (BigInt.asIntN(16, w) === w) + " w=" + w);
console.log("result_type=" + typeof BigInt.asIntN(8, 1n));
console.log("float_bits=" + BigInt.asUintN(8.0, 300n) + " frac_bits=" + BigInt.asIntN(3.9, 7n));
console.log("string_bits=" + BigInt.asUintN("8" as any, 300n) + " string_value=" + BigInt.asUintN(8, "300" as any));
op("neg_bits", () => BigInt.asUintN(-1, 1n));
op("number_value", () => BigInt.asUintN(8, 1 as any));
op("asIntN_wraps_2p63", () => BigInt.asIntN(64, 2n ** 63n));
op("asUintN_wraps_minus1", () => BigInt.asUintN(64, -1n));

// --- typed arrays: stores wrap to the element width ---
const signed = new BigInt64Array(3);
signed[0] = -1n;
signed[1] = 2n ** 63n;
signed[2] = -(2n ** 63n) - 1n;
console.log("i64_wrap=" + Array.from(signed).map(String).join(","));
const unsigned = new BigUint64Array(3);
unsigned[0] = -1n;
unsigned[1] = 2n ** 64n;
unsigned[2] = 5n;
console.log("u64_wrap=" + Array.from(unsigned).map(String).join(","));
const arr = new BigInt64Array(3);
arr[0] = 1n;
arr[1] = -2n;
arr[2] = 9007199254740991n;
console.log("arr=" + Array.from(arr).join(","));
console.log("i64_sort=" + Array.from(BigInt64Array.from([3n, -1n, 2n]).sort()).map(String).join(","));
