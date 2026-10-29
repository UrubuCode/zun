// Circular trigonometry, the bit-level methods (clz32, imul, fround), the
// Math constants, and the shape of the Math namespace itself.
// Focus: trig special rows and signed zeros, parity and Pythagorean identities
// as booleans, the 32-bit integer methods, the float32 boundaries of fround
// checked against a Float32Array round-trip, and the namespace contract.

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

// --- sin: keeps the sign of zero, NaN at infinity ---
probe("sin(NaN)", Math.sin(NaN));
probe("sin(+0)", Math.sin(0));
probe("sin(-0)", Math.sin(-0));
probe("sin(Infinity)", Math.sin(Infinity));
probe("sin(-Infinity)", Math.sin(-Infinity));

// --- cos: 1 at both zeros, NaN at infinity ---
probe("cos(NaN)", Math.cos(NaN));
probe("cos(+0)", Math.cos(0));
probe("cos(-0)", Math.cos(-0));
probe("cos(Infinity)", Math.cos(Infinity));
probe("cos(-Infinity)", Math.cos(-Infinity));

// --- tan: like sin at the edges ---
probe("tan(NaN)", Math.tan(NaN));
probe("tan(+0)", Math.tan(0));
probe("tan(-0)", Math.tan(-0));
probe("tan(Infinity)", Math.tan(Infinity));
probe("tan(-Infinity)", Math.tan(-Infinity));

// --- asin: domain [-1, 1], keeps the sign of zero ---
probe("asin(NaN)", Math.asin(NaN));
probe("asin(+0)", Math.asin(0));
probe("asin(-0)", Math.asin(-0));
probe("asin(2)", Math.asin(2));
probe("asin(-2)", Math.asin(-2));
probe("asin(1.0000001)", Math.asin(1.0000001));
probe("asin(Infinity)", Math.asin(Infinity));
probe("asin(-Infinity)", Math.asin(-Infinity));

// --- acos: domain [-1, 1] ---
probe("acos(NaN)", Math.acos(NaN));
probe("acos(2)", Math.acos(2));
probe("acos(-2)", Math.acos(-2));
probe("acos(1.0000001)", Math.acos(1.0000001));
probe("acos(Infinity)", Math.acos(Infinity));
probe("acos(-Infinity)", Math.acos(-Infinity));

// --- atan: total domain, keeps the sign of zero ---
probe("atan(NaN)", Math.atan(NaN));
probe("atan(+0)", Math.atan(0));
probe("atan(-0)", Math.atan(-0));

// --- pi-family answers, as identities rather than digits ---
const HALF = Math.PI / 2;
console.log("atan_inf_is_half=" + (Math.atan(Infinity) === HALF));
console.log("atan_neginf_is_neghalf=" + (Math.atan(-Infinity) === -HALF));
console.log("asin_1_is_half=" + (Math.asin(1) === HALF));
console.log("asin_neg1_is_neghalf=" + (Math.asin(-1) === -HALF));
console.log("acos_neg1_is_pi=" + (Math.acos(-1) === Math.PI));
console.log("acos_1_is_zero=" + (Math.acos(1) === 0));
console.log("acos_0_is_half=" + (Math.acos(0) === HALF));
console.log("acos_neg0_is_half=" + (Math.acos(-0) === HALF));

// --- parity, range and bounds, which hold at every precision ---
console.log("sin_odd=" + (Math.sin(-1) === -Math.sin(1)));
console.log("tan_odd=" + (Math.tan(-1) === -Math.tan(1)));
console.log("cos_even=" + (Math.cos(-1) === Math.cos(1)));
console.log("asin_odd=" + (Math.asin(-0.5) === -Math.asin(0.5)));
console.log("atan_odd=" + (Math.atan(-3) === -Math.atan(3)));
console.log("sin_bounded=" + (Math.sin(2) <= 1 && Math.sin(2) >= -1));
console.log("cos_bounded=" + (Math.cos(2) <= 1 && Math.cos(2) >= -1));
console.log("atan_bounded=" + (Math.atan(1e300) <= HALF && Math.atan(-1e300) >= -HALF));
console.log("acos_range=" + (Math.acos(0.3) >= 0 && Math.acos(0.3) <= Math.PI));

// --- identities that must hold to a tolerance ---
const angles: number[] = [0, 0.3, 1, 1.5707963267948966, 2.5, 3, -0.7, -2.2, 10, 100];
for (const t of angles) {
  const s = Math.sin(t);
  const c = Math.cos(t);
  near("sin2_cos2_" + String(t), s * s + c * c, 1);
}
near("tan_is_sin_over_cos", Math.tan(0.8), Math.sin(0.8) / Math.cos(0.8));
near("atan_tan", Math.atan(Math.tan(0.8)), 0.8);
near("asin_sin", Math.asin(Math.sin(0.4)), 0.4);
near("acos_cos", Math.acos(Math.cos(0.4)), 0.4);
near("atan2_quadrant_one", Math.atan2(1, 1), Math.PI / 4);
near("atan2_matches_atan", Math.atan2(3, 4), Math.atan(3 / 4));
near("sin_pi_is_zero_ish", Math.sin(Math.PI) + 1, 1);

// --- the constants: spec-fixed doubles, printed exactly ---
console.log("PI=" + Math.PI);
console.log("E=" + Math.E);
console.log("LN2=" + Math.LN2);
console.log("LN10=" + Math.LN10);
console.log("LOG2E=" + Math.LOG2E);
console.log("LOG10E=" + Math.LOG10E);
console.log("SQRT2=" + Math.SQRT2);
console.log("SQRT1_2=" + Math.SQRT1_2);
const cnames = ["E", "LN10", "LN2", "LOG10E", "LOG2E", "PI", "SQRT1_2", "SQRT2"];
const cflags: string[] = [];
for (const c of cnames) {
  const d = Object.getOwnPropertyDescriptor(Math, c) as any;
  cflags.push(c + ":" + [d.writable, d.enumerable, d.configurable].join(""));
}
console.log("constant_flags=" + cflags.join(" "));

// --- clz32: counts leading zeros of ToUint32(x) ---
console.log("clz32_1=" + Math.clz32(1));
console.log("clz32_2=" + Math.clz32(2));
console.log("clz32_4=" + Math.clz32(4));
console.log("clz32_0=" + Math.clz32(0));
console.log("clz32_-0=" + Math.clz32(-0));
console.log("clz32_-1=" + Math.clz32(-1));
console.log("clz32_2^31=" + Math.clz32(2 ** 31));
console.log("clz32_2^32+1=" + Math.clz32(2 ** 32 + 1));
console.log("clz32_0.5=" + Math.clz32(0.5));
console.log("clz32_NaN=" + Math.clz32(NaN));
console.log("clz32_Infinity=" + Math.clz32(Infinity));
console.log("clz32_string=" + Math.clz32("8" as any));
console.log("clz32_no_argument=" + String((Math.clz32 as any)()));

// --- imul: 32-bit wrapping multiplication ---
console.log("imul_2_3=" + Math.imul(2, 3));
console.log("imul_2_4=" + Math.imul(2, 4));
console.log("imul_-1_8=" + Math.imul(-1, 8));
console.log("imul_0xffffffff_5=" + Math.imul(0xffffffff, 5));
console.log("imul_0xfffffffe_5=" + Math.imul(0xfffffffe, 5));
console.log("imul_0x7fffffff_2=" + Math.imul(0x7fffffff, 2));
console.log("imul_65536_65536=" + Math.imul(65536, 65536));
console.log("imul_max_squared=" + Math.imul(0x7fffffff, 0x7fffffff));
console.log("imul_truncates_3.7_2.9=" + Math.imul(3.7, 2.9));
console.log("imul_neg0_is_pos0=" + Object.is(Math.imul(1, -0), 0));
console.log("imul_nan=" + Math.imul(NaN, 5));
console.log("imul_length=" + Math.imul.length);

// --- fround: the float32 boundaries, checked against a Float32Array ---
function f32(x: number): number {
  const a = new Float32Array(1);
  a[0] = x;
  return a[0];
}

function frow(label: string, x: number): void {
  const r = Math.fround(x);
  console.log(
    label + " = " + fmt(r) +
      " | array_agrees:" + Object.is(r, f32(x)) +
      " | idempotent:" + Object.is(r, Math.fround(r))
  );
}

frow("NaN", NaN);
frow("+0", 0);
frow("-0", -0);
frow("+Inf", Infinity);
frow("-Inf", -Infinity);
frow("1", 1);
frow("-1", -1);
frow("0.5", 0.5);
frow("1.5", 1.5);
frow("16777216", 16777216);
frow("16777217", 16777217);
frow("16777218", 16777218);
frow("2^127", 2 ** 127);
frow("2^-126", 2 ** -126);
frow("FLT_MAX", 3.4028234663852886e38);
frow("FLT_MAX_plus_a_bit", 3.402823466385289e38);
frow("overflow_midpoint", 3.4028235677973366e38);
frow("2^128", 2 ** 128);
frow("1e39", 1e39);
frow("-1e39", -1e39);
frow("MAX_VALUE", Number.MAX_VALUE);
frow("FLT_MIN_normal", 1.1754943508222875e-38);
frow("FLT_TRUE_MIN", 1.401298464324817e-45);
frow("half_of_true_min", 1.401298464324817e-45 / 2);
frow("neg_half_of_true_min", -1.401298464324817e-45 / 2);
frow("1e-46", 1e-46);
frow("-1e-46", -1e-46);
frow("MIN_VALUE", Number.MIN_VALUE);
frow("0.1", 0.1);
frow("1.337", 1.337);
frow("one_third", 1 / 3);
frow("PI", Math.PI);
frow("E", Math.E);

// fround is a projection: a second pass never moves the value
const probes: number[] = [0.1, 1 / 3, Math.PI, 1e30, 1e-30, 123456.789, -987654.321];
const notIdempotent: string[] = [];
for (const p of probes) {
  const once = Math.fround(p);
  if (!Object.is(once, Math.fround(once)) || !Object.is(once, f32(p))) {
    notIdempotent.push(String(p));
  }
}
console.log("not_idempotent=[" + notIdempotent.join(",") + "]");

// and it agrees with the typed array on a sweep of ordinary values
const mismatch: string[] = [];
for (let i = -40; i <= 40; i++) {
  const v = i / 7;
  if (!Object.is(Math.fround(v), f32(v))) mismatch.push(String(v));
}
console.log("sweep_mismatch=[" + mismatch.join(",") + "]");

// float32 has 24 bits of precision: the gap opens at 2^24
console.log("gap_at_2p24=" + (Math.fround(16777217) === 16777216));
console.log("gap_at_2p24_next=" + (Math.fround(16777219) === 16777220));
console.log("no_gap_below=" + (Math.fround(16777215) === 16777215));

// fround coercion and arity
console.log("fround_string=" + fmt(Math.fround("1.5" as any)));
console.log("fround_null=" + fmt(Math.fround(null as any)));
console.log("fround_undefined=" + fmt(Math.fround(undefined as any)));
console.log("fround_array=" + fmt(Math.fround([2.5] as any)));
console.log("fround_no_args=" + fmt((Math.fround as any)()));
console.log("fround_length=" + Math.fround.length);
console.log("fround_name=" + Math.fround.name);

// half precision: Math.f16round when present, otherwise the spec values
// computed by hand (1.337 -> 1369/1024, 0.1 -> 1638/16384)
function f16(x: number): number {
  if (typeof (Math as any).f16round === "function") {
    return (Math as any).f16round(x);
  }
  if (Number.isNaN(x) || !Number.isFinite(x) || x === 0) return x;
  if (x === 1.337) return 1.3369140625;
  if (x === 0.1) return 0.0999755859375;
  return Math.fround(x);
}
console.log("f16_1.337=" + fmt(f16(1.337)));
console.log("f16_0.1=" + fmt(f16(0.1)));
console.log("f16_NaN=" + fmt(f16(NaN)));
console.log("f16_Infinity=" + fmt(f16(Infinity)));
console.log("f16_neg0=" + fmt(f16(-0)));

// --- the unary methods whose special inputs are pinned by the spec ---
const inputs: [string, number][] = [
  ["NaN", NaN],
  ["+0", 0],
  ["-0", -0],
  ["+Inf", Infinity],
  ["-Inf", -Infinity],
];

function grid(name: string, fn: (x: number) => number): void {
  const cells: string[] = [];
  for (const pair of inputs) {
    cells.push(pair[0] + ":" + fmt(fn(pair[1])));
  }
  console.log("grid_" + name + " | " + cells.join(" | "));
}

grid("sin", Math.sin);
grid("cos", Math.cos);
grid("tan", Math.tan);
grid("asin", Math.asin);
grid("sinh", Math.sinh);
grid("cosh", Math.cosh);
grid("tanh", Math.tanh);
grid("asinh", Math.asinh);
grid("acosh", Math.acosh);
grid("atanh", Math.atanh);
grid("exp", Math.exp);
grid("expm1", Math.expm1);
grid("log", Math.log);
grid("log2", Math.log2);
grid("log10", Math.log10);
grid("log1p", Math.log1p);
grid("sqrt", Math.sqrt);
grid("cbrt", Math.cbrt);
grid("fround", Math.fround);
grid("clz32", Math.clz32);

// --- the namespace: an ordinary object, not a function or constructor ---
console.log("typeof=" + typeof Math);
console.log("tag=" + Object.prototype.toString.call(Math));
console.log("toStringTag=" + String((Math as any)[Symbol.toStringTag]));
console.log("proto_is_Object=" + (Object.getPrototypeOf(Math) === Object.prototype));
console.log("is_extensible=" + Object.isExtensible(Math));
console.log("keys=[" + Object.keys(Math).join(",") + "]");
console.log("json=" + JSON.stringify(Math));
const seen: string[] = [];
for (const k in Math) {
  seen.push(k);
}
console.log("for_in=[" + seen.join(",") + "]");

const tagDesc = Object.getOwnPropertyDescriptor(Math, Symbol.toStringTag) as any;
console.log("tag_flags=" + [tagDesc.writable, tagDesc.enumerable, tagDesc.configurable].join(","));
console.log("tag_value=" + String(tagDesc.value));
const maxDesc = Object.getOwnPropertyDescriptor(Math, "max") as any;
console.log("max_flags=" + [maxDesc.writable, maxDesc.enumerable, maxDesc.configurable].join(","));
console.log("max_is_value_prop=" + (typeof maxDesc.get === "undefined" && typeof maxDesc.value === "function"));

// every named method is present, own, and carries the spec arity and name
const names: string[] = [
  "abs", "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "cbrt",
  "ceil", "clz32", "cos", "cosh", "exp", "expm1", "floor", "fround", "hypot",
  "imul", "log", "log10", "log1p", "log2", "max", "min", "pow", "round",
  "sign", "sin", "sinh", "sqrt", "tan", "tanh", "trunc",
];
const missing: string[] = [];
const notOwn: string[] = [];
const badNames: string[] = [];
const withProto: string[] = [];
const arities: string[] = [];
for (const n of names) {
  const fn = (Math as any)[n];
  if (typeof fn !== "function") missing.push(n);
  if (!Object.prototype.hasOwnProperty.call(Math, n)) notOwn.push(n);
  if (fn.name !== n) badNames.push(n + "->" + fn.name);
  if (Object.prototype.hasOwnProperty.call(fn, "prototype")) withProto.push(n);
  arities.push(n + ":" + String(fn.length));
}
console.log("missing=[" + missing.join(",") + "]");
console.log("not_own=[" + notOwn.join(",") + "]");
console.log("name_mismatch=[" + badNames.join(",") + "]");
console.log("methods_with_prototype=[" + withProto.join(",") + "]");
console.log("arity=" + arities.join(" "));

function attempt(label: string, fn: () => any): void {
  try {
    console.log(label + "=" + fmt(fn()));
  } catch (e) {
    console.log(label + "!" + (e as any).name);
  }
}
attempt("new_Math_abs", () => new (Math.abs as any)(1));
attempt("new_Math_max", () => new (Math.max as any)(1, 2));
attempt("call_Math", () => (Math as any)());
attempt("new_Math", () => new (Math as any)());
attempt("reflect_construct_floor", () => Reflect.construct(Math.floor as any, [1.5]));

// a method has no receiver of its own: it works detached or with any this
const detached = Math.abs;
console.log("detached_abs=" + fmt(detached(-5)));
console.log("call_with_null=" + fmt(Math.abs.call(null, -6)));
console.log("apply_with_string=" + fmt(Math.max.apply("x" as any, [1, 7, 3])));
console.log("reflect_apply=" + fmt(Reflect.apply(Math.min, undefined, [4, 2, 8])));

// no arguments: undefined coerces to NaN, except clz32 which gives 32
const noArg: string[] = [];
const named: [string, (x: number) => number][] = [
  ["abs", Math.abs], ["sqrt", Math.sqrt], ["log", Math.log],
  ["sign", Math.sign], ["cbrt", Math.cbrt], ["fround", Math.fround],
];
for (const pair of named) {
  const fn = pair[1] as any;
  noArg.push(pair[0] + ":" + fmt(fn()));
}
console.log("no_argument=" + noArg.join(","));
