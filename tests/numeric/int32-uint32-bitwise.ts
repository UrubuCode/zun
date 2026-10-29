// The 32-bit integer world: the bitwise operators on positive and negative
// operands, ToInt32 (|0, ~~, >>0) and ToUint32 (>>>0) across their wrap points
// and across 2^63, the shift counts masked to five bits, sign-fill versus
// zero-fill, Math.imul and Math.clz32, the six ways of "making an integer"
// side by side, and a local binding whose stores are all bitwise.

// --- the operators on small signed operands ---
console.log("and=" + (6 & 3) + " or=" + (6 | 3) + " xor=" + (6 ^ 3) + " not0=" + (~0));
console.log("lsh=" + (5 << 2) + " rsh=" + (-8 >> 1) + " trunc=" + (~~3.7) + " hi_bit=" + (1 << 31));
console.log("mix=" + ((-5 & 3) + ":" + (-5 | 3)) + " not5=" + (~5) + " not_neg1=" + (~-1));
console.log("ushr_basic=" + (16 >>> 2) + "," + (-1 >>> 0) + "," + (-8 >>> 1) + "," + (1 >>> 0));

// --- ToInt32 at the wrap points and the special inputs ---
console.log("toi32_basic=" + (-1 | 0) + "," + (~0) + "," + (~5) + "," + (2147483647 + 1 | 0));
console.log("toi32_wrap=" + (4294967296 | 0) + "," + (4294967297 | 0) + "," + (2147483648 | 0) + "," + (0xFFFFFFFF | 0));
console.log("toi32_neg_wrap=" + (-2147483649 | 0) + "," + (1e10 & 0xFF) + "," + (3.9 | 0) + "," + (-3.9 | 0));
console.log("toi32_special=" + (NaN | 0) + "," + (Infinity | 0) + "," + (-Infinity | 0));

// --- ToInt32 across 2^63, where a 64-bit lowering has to change strategy ---
const xs: number[] = [
  0, -0, 1, -1, 0.9, -0.9, 1.5, -1.5,
  2147483647, 2147483648, -2147483648, -2147483649,
  4294967295, 4294967296, 4294967297,
  1e21, 1.7976931348623157e308,
  9007199254740991, -9007199254740991,
  9223372036854774784,
  9223372036854775808,
  -9223372036854775808,
  18446744073709551616,
  1.9342813113834067e25,
  Infinity, -Infinity, NaN,
];
for (const x of xs) {
  console.log(`${x} -> ${x | 0} ${x & 255} ${x >> 1} ${x >>> 1} ${~x} ${(x * 1) | 0}`);
}

// --- ToUint32: the unsigned view, only >>> exposes it ---
console.log("u32_neg=" + (-1 >>> 0) + "," + (-2 >>> 0) + "," + (-5 >>> 0) + "," + (-2147483648 >>> 0));
console.log("u32_neg_wide=" + (-2147483647 >>> 0) + "," + (-4294967295 >>> 0) + "," + (-4294967296 >>> 0) + "," + (-4294967297 >>> 0));
console.log("u32_pos=" + (2147483647 >>> 0) + "," + (2147483648 >>> 0) + "," + (4294967295 >>> 0) + "," + (4294967296 >>> 0));
console.log("u32_wrap=" + (4294967297 >>> 0) + "," + (8589934592 >>> 0) + "," + (8589934593 >>> 0));
console.log("u32_fraction=" + (3.9 >>> 0) + "," + (-3.9 >>> 0) + "," + (0.9 >>> 0) + "," + (-0.9 >>> 0) + "," + (-0 >>> 0));
console.log("u32_fraction_edges=" + (2147483647.9 >>> 0) + "," + (4294967295.999 >>> 0));
console.log("u32_specials=" + (NaN >>> 0) + "," + (Infinity >>> 0) + "," + (-Infinity >>> 0));
console.log("u32_nullish=" + ((undefined as any) >>> 0) + "," + ((null as any) >>> 0));
const uvals: number[] = [-1, -2147483648, 2147483647, 2147483648, 4294967295, 4294967296, 3.9, -3.9, NaN, Infinity, 1e10, -1e10];
for (const v of uvals) {
  console.log(String(v) + " -> int32:" + (v | 0) + " uint32:" + (v >>> 0));
}

// --- shift counts are masked with & 31, for every shift form ---
console.log("shl_masks=" + (1 << 0) + "," + (1 << 31) + "," + (1 << 32) + "," + (1 << 33) + "," + ((1 << 33) === 1));
console.log("shr_masks=" + (-8 >> 33) + "," + (-8 >>> 33) + "," + (0x80000000 >> 31) + "," + (0x80000000 >>> 31));
console.log("ushr_masks=" + (-1 >>> 32) + "," + (-1 >>> 33) + "," + (-1 >>> 31) + "," + (4294967295 >>> 16) + "," + (-1 >>> 16));
console.log("ushr_count_zero=" + (5 >>> 32) + "," + (256 >>> 40) + "," + (-16 >>> 31) + "," + (2147483648 >>> 31));
console.log("shl_wide=" + (1 << 32) + " " + (1 << 33) + " " + (-1 << 31) + " " + (5 << 35));

// --- sign-fill (>>) versus zero-fill (>>>) on negatives ---
console.log("sign_vs_zero_fill=" + (-8 >> 1) + "," + (-8 >>> 1) + "," + (-1 >> 1) + "," + (-1 >>> 1));
console.log("sign_vs_zero_fill2=" + (-16 >> 2) + "," + (-16 >>> 2) + "," + (-8 >> 2) + "," + (-8 >>> 2));
console.log("shift_through_sign=" + (-1 >>> 1) + "," + (-2147483648 >>> 0) + "," + (0xFFFFFFFF >>> 0));

// --- bitwise operands are coerced: fractions truncate, strings and nullish go through ToNumber ---
console.log("coerce_fractions=" + (5.9 | 0) + "," + (-5.9 | 0) + " strings=" + (("7" as any) & 3) + " null=" + ((null as any) ^ 5));
console.log("coerce_bitnot=" + (~0) + "," + (~-1) + "," + (~4294967296) + "," + (~"5" as any) + "," + (~[] as any));
console.log("coerce_strings=" + (("-1" as any) >>> 0) + "," + (("0x10" as any) >>> 0) + "," + (("abc" as any) >>> 0) + "," + (("" as any) >>> 0));
console.log("coerce_bool=" + ((true as any) >>> 0) + "," + ((false as any) | 0) + "," + ((true as any) << 3));

// --- the six ways of making an integer, and where they stop agreeing ---
function row(x: number): void {
  console.log(
    String(x) +
      " | or0:" + (x | 0) +
      " | tilde:" + ~~x +
      " | shr0:" + (x >> 0) +
      " | ushr0:" + (x >>> 0) +
      " | trunc:" + Math.trunc(x) +
      " | floor:" + Math.floor(x)
  );
}
row(3.7);
row(-3.7);
row(0.5);
row(-0.5);
row(-1.5);
row(0);
row(-0);
row(2147483647);
row(2147483648);
row(2147483649);
row(-2147483648);
row(-2147483649);
row(4294967295);
row(4294967296);
row(4294967297.9);
row(6442450944);
row(-4294967297);
row(9007199254740992);
row(9007199254740993);
row(-9007199254740992);
row(1e21);
row(-1e21);
row(NaN);
row(Infinity);
row(-Infinity);
row(Number.MAX_VALUE);
row(Number.MIN_VALUE);

// --- negative zero: only trunc, ceil and round-to-sign keep it through the family ---
const producers: number[] = [-0.5, -0.9, -0, -0.0001];
for (const p of producers) {
  console.log(
    "in:" + String(p) +
      " trunc_neg0:" + Object.is(Math.trunc(p), -0) +
      " ceil_neg0:" + Object.is(Math.ceil(p), -0) +
      " or0_neg0:" + Object.is(p | 0, -0) +
      " tilde_neg0:" + Object.is(~~p, -0) +
      " round_neg0:" + Object.is(Math.round(p), -0)
  );
}

// --- the same conversions on non-numbers ---
const alien: [string, any][] = [
  ["str3.7", "3.7"], ["strneg3.7", "-3.7"], ["strhex", "0x10"], ["strempty", ""],
  ["strpadded", "  12  "], ["null", null], ["undefined", undefined], ["true", true],
  ["emptyarr", []], ["arr7", [7]], ["obj", {}],
];
for (const pair of alien) {
  const a: any = pair[1];
  console.log(
    pair[0] +
      " | or0:" + (a | 0) +
      " | tilde:" + ~~a +
      " | ushr0:" + (a >>> 0) +
      " | trunc:" + Math.trunc(a) +
      " | Number:" + Number(a)
  );
}

// --- Math.imul: a 32-bit product that wraps; Math.clz32: leading zeros of the uint32 view ---
console.log("imul=" + Math.imul(0xffffffff, 5) + "," + Math.imul(0x7fffffff, 2) + "," + Math.imul(0x80000000, 2));
console.log("imul_big=" + Math.imul(0x12345678, 0x9abcdef0) + " imul_small=" + Math.imul(3, 4) + " imul_neg=" + Math.imul(-3, 4));
console.log("clz32=" + [0, 1, 2, 0xffffffff, -1].map((v) => Math.clz32(v)).join(","));
console.log("clz32_coerce=" + Math.clz32(1.9) + "," + Math.clz32(0x100000000) + "," + Math.clz32(-0) + "," + Math.clz32(NaN));

// --- a local whose every store is bitwise is carried as an integer, and must look like a double ---
let signed = -0;
console.log(`kept_neg0=${Object.is(signed, -0)} ${1 / signed}`);
let cleared = -0 | 0;
console.log(`cleared_neg0=${Object.is(cleared, 0)} ${1 / cleared}`);
let low = -2147483648;
console.log(`low ${low} ${low | 0} ${low - 1} ${(low - 1) | 0}`);
let high = 2147483647;
console.log(`high ${high} ${high + 1} ${(high + 1) | 0} ${high << 1}`);
let acc32 = -1;
for (let i = 0; i < 10; i++) acc32 = (acc32 << 3) ^ i;
console.log(`loop ${acc32}`);
let mask = 0;
for (let i = 0; i < 40; i++) mask = mask | (1 << i);
console.log(`mask ${mask}`);
let unsigned = 0;
for (let i = 0; i < 3; i++) unsigned = -1 >>> i;
console.log(`ushift ${unsigned} ${(-1 >>> 0) > 2147483647}`);
let bits = 0;
for (let i = 0; i < 8; i++) bits = bits ^ (i << 2);
console.log(`out ${bits + 0.5} ${bits / 2} ${String(bits)} ${typeof bits}`);
console.log(`arr ${[bits, bits | 1].join(",")} ${JSON.stringify({ bits })}`);
let mixed = 5;
for (let i = 0; i < 3; i++) mixed = i === 1 ? mixed + 0.5 : mixed | 0;
console.log(`mixed ${mixed}`);
