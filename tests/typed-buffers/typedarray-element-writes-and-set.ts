// Cross-runtime: what happens when a value is WRITTEN into a typed array. Each
// element kind converts differently (wrap, sign, clamp with round-half-to-even,
// truncation toward zero, NaN to 0 for integers, ToBigInt for the bigint kinds),
// TypedArray.prototype.set takes several source shapes with coerced offsets and
// RangeError guards, and an integer-indexed key is never an ordinary property
// (out-of-range writes are dropped, canonical numeric strings never stick).

const t = function (f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name;
  }
};

// ---- Integer wrap-around per kind.
const u8 = new Uint8Array(6);
u8[0] = 256;
u8[1] = 257;
u8[2] = -1;
u8[3] = 1.9;
u8[4] = -1.9;
u8[5] = NaN;
console.log("u8=" + Array.from(u8).join(","));

const u16 = new Uint16Array(4);
u16[0] = 65536;
u16[1] = 65537;
u16[2] = -1;
u16[3] = 70000;
console.log("u16=" + Array.from(u16).join(","));

const i8 = new Int8Array(5);
i8[0] = 127;
i8[1] = 128;
i8[2] = 255;
i8[3] = -129;
i8[4] = 1e10;
console.log("i8=" + Array.from(i8).join(","));

const i32 = new Int32Array(4);
i32[0] = 2147483647;
i32[1] = 2147483648;
i32[2] = 4294967296;
i32[3] = -2147483649;
console.log("i32=" + Array.from(i32).join(","));

// ---- Uint8ClampedArray clamps instead of wrapping and rounds half to EVEN.
const c = new Uint8ClampedArray(9);
c[0] = -5;
c[1] = 300;
c[2] = 0.5;
c[3] = 1.5;
c[4] = 2.5;
c[5] = 3.5;
c[6] = -0.6;
c[7] = NaN;
c[8] = 254.5;
console.log("clamped=" + Array.from(c).join(","));
const cInf = new Uint8ClampedArray(2);
cInf[0] = Infinity;
cInf[1] = -Infinity;
const uInf = new Uint8Array(2);
uInf[0] = Infinity;
uInf[1] = -Infinity;
console.log("clamped_inf=" + Array.from(cInf).join(",") + " wrap_inf=" + Array.from(uInf).join(","));

// ---- Floats keep what they can represent; -0 stays -0; overflow becomes Infinity.
const f32 = new Float32Array(5);
f32[0] = 0.1;
f32[1] = -0;
f32[2] = 1e39;
f32[3] = 16777217;
f32[4] = NaN;
console.log("f32=" + f32[0] + "," + Object.is(f32[1], -0) + "," + f32[2] + "," + f32[3] + "," + Number.isNaN(f32[4]));
const f64 = new Float64Array(3);
f64[0] = 0.1;
f64[1] = -0;
f64[2] = 1e39;
console.log("f64=" + f64[0] + "," + Object.is(f64[1], -0) + "," + f64[2]);
const i16 = new Int16Array(1);
i16[0] = -0;
console.log("int_negzero=" + Object.is(i16[0], -0) + "," + Object.is(i16[0], 0));

// ---- Non-numbers go through ToNumber first; a throwing conversion writes nothing.
const mix = new Uint8Array(6);
mix[0] = "12" as any;
mix[1] = true as any;
mix[2] = null as any;
mix[3] = undefined as any;
mix[4] = "" as any;
mix[5] = { valueOf: () => 7 } as any;
console.log("mix=" + Array.from(mix).join(","));
const guarded = new Uint8Array(1);
guarded[0] = 5;
console.log("throwing_valueof=" + t(() => { guarded[0] = { valueOf: () => { throw new RangeError("nope"); } } as any; return "ok"; }) + " kept=" + guarded[0]);
console.log("symbol_write=" + t(() => { (new Uint8Array(1) as any)[0] = Symbol("s"); return "ok"; }));

// ---- BigInt kinds wrap by their own width and refuse a Number.
const b64 = new BigInt64Array(3);
b64[0] = 2n ** 63n;
b64[1] = -1n;
b64[2] = 2n ** 64n + 5n;
console.log("bigint64=" + b64[0] + "," + b64[1] + "," + b64[2]);
const bu64 = new BigUint64Array(2);
bu64[0] = -1n;
bu64[1] = 2n ** 64n;
console.log("biguint64=" + bu64[0] + "," + bu64[1]);
console.log("bigint_takes_number=" + t(() => { (b64 as any)[0] = 1; return "ok"; }));
console.log("number_takes_bigint=" + t(() => { (new Uint8Array(1) as any)[0] = 1n; return "ok"; }));
console.log("bigint_takes_bool_string=" + t(() => { const x = new BigInt64Array(3); x[0] = true as any; x[1] = "5" as any; x[2] = "" as any; return x.join(","); }));
console.log("bigint_bad_string=" + t(() => { (new BigInt64Array(1) as any)[0] = "x"; return "ok"; }));

// ---- set(): sources are plain arrays (with holes and getters), array-likes,
// strings, and other typed arrays. The offset is coerced before the length check.
console.log("holes_int=" + t(() => { const d = new Uint8Array(3).fill(9); d.set([1, , 3] as any); return d.join(","); }));
console.log("holes_float=" + t(() => { const d = new Float64Array(3).fill(9); d.set([1, , 3] as any); return d.join(","); }));
console.log("undefined_null_elements=" + t(() => { const d = new Uint8Array(2); d.set([undefined, null] as any); return d.join(","); }));
console.log("string_elements=" + t(() => { const d = new Uint8Array(3); d.set(["7", "x", true] as any); return d.join(","); }));

const order: string[] = [];
const arrayLike: any = { length: 3 };
for (const k of [0, 1, 2]) {
  Object.defineProperty(arrayLike, k, { get: () => { order.push("g" + k); return k + 1; }, enumerable: true });
}
console.log("arraylike_getters=" + t(() => { const d = new Uint8Array(3); d.set(arrayLike); return d.join(","); }));
console.log("getter_order=" + order.join(">"));
console.log("missing_indices=" + t(() => { const d = new Uint8Array(3).fill(9); d.set({ length: 3, 1: 5 } as any); return d.join(","); }));
console.log("length_coerced=" + t(() => { const d = new Uint8Array(3).fill(9); d.set({ length: "2", 0: 1, 1: 2 } as any); return d.join(","); }));
console.log("no_length=" + t(() => { const d = new Uint8Array(2).fill(9); d.set({ 0: 1 } as any); return d.join(","); }));
console.log("string_source=" + t(() => { const d = new Uint8Array(3).fill(9); d.set("ab" as any); return d.join(","); }));
console.log("iterable_ignored=" + t(() => {
  const d = new Uint8Array(2).fill(9);
  const src: any = { length: 2, 0: 1, 1: 2 };
  src[Symbol.iterator] = function* () { yield 7; yield 8; };
  d.set(src);
  return d.join(",");
}));
console.log("null_source=" + t(() => new Uint8Array(2).set(null as any)));
console.log("number_source=" + t(() => { const d = new Uint8Array(2).fill(9); d.set(5 as any); return d.join(","); }));
console.log("returns_undefined=" + String(new Uint8Array(2).set([1])));

console.log("offset_fraction=" + t(() => { const d = new Uint8Array(3); d.set([1], 1.9); return d.join(","); }));
console.log("offset_string=" + t(() => { const d = new Uint8Array(3); d.set([1], "2" as any); return d.join(","); }));
console.log("offset_undefined=" + t(() => { const d = new Uint8Array(3); d.set([1], undefined); return d.join(","); }));
console.log("offset_nan=" + t(() => { const d = new Uint8Array(3); d.set([1], NaN); return d.join(","); }));
console.log("offset_negative=" + t(() => new Uint8Array(3).set([1], -1)));
console.log("offset_infinity=" + t(() => new Uint8Array(3).set([1], Infinity)));
console.log("too_long=" + t(() => new Uint8Array(3).set([1, 2], 2)));
console.log("exact_fit=" + t(() => { const d = new Uint8Array(3); d.set([1, 2], 1); return d.join(","); }));
console.log("empty_at_end=" + t(() => { const d = new Uint8Array(2); d.set([], 2); return d.join(","); }));
console.log("empty_past_end=" + t(() => new Uint8Array(2).set([], 3)));

// The range is checked BEFORE a single element of an array-like source is read.
console.log("range_before_read=" + t(() => {
  let reads = 0;
  const src: any = { length: 5 };
  Object.defineProperty(src, 0, { get: () => { reads++; return 1; } });
  try {
    new Uint8Array(2).set(src);
  } catch (e: any) {
    return e.constructor.name + "/reads:" + reads;
  }
  return "no-throw/reads:" + reads;
}));

// A typed-array source converts element by element through the destination kind.
console.log("narrowing=" + t(() => { const d = new Uint8Array(3); d.set(new Float64Array([1.7, -1, 300])); return d.join(","); }));
console.log("widening=" + t(() => { const d = new Float64Array(2); d.set(new Uint8Array([1, 2])); return d.join(","); }));
console.log("clamped_target=" + t(() => { const d = new Uint8ClampedArray(3); d.set(new Float64Array([-5, 2.5, 300])); return d.join(","); }));
console.log("i8_into_i32=" + t(() => { const d = new Int32Array(3); d.set(new Int8Array([-1, 127, -128])); return d.join(","); }));
console.log("i32_into_i8=" + t(() => { const d = new Int8Array(3); d.set(new Int32Array([255, 256, -1])); return d.join(","); }));
console.log("bigint_into_biguint=" + t(() => { const d = new BigUint64Array(2); d.set(new BigInt64Array([-1n, 2n])); return d.join(","); }));
console.log("number_into_bigint=" + t(() => new BigInt64Array(2).set([1, 2] as any)));
console.log("bigint_from_number_kind=" + t(() => new BigInt64Array(2).set(new Uint8Array(1))));

// Overlapping ranges inside ONE buffer behave as if the source were copied first.
const fwd = new Uint8Array([1, 2, 3, 4, 5, 6]);
fwd.set(fwd.subarray(0, 4), 2);
console.log("overlap_forward=" + Array.from(fwd).join(","));
const bwd = new Uint8Array([1, 2, 3, 4, 5, 6]);
bwd.set(bwd.subarray(2, 6), 0);
console.log("overlap_backward=" + Array.from(bwd).join(","));
const words = new Uint16Array(new ArrayBuffer(8));
words.set([1, 2, 3, 4]);
words.set(words.subarray(1, 4), 0);
console.log("overlap_words=" + Array.from(words).join(","));
const shared2 = new ArrayBuffer(4);
const w2 = new Uint16Array(shared2, 0, 2);
const b2 = new Uint8Array(shared2, 0, 2);
w2.set([258, 260]);
b2.set(w2.subarray(0, 2) as any, 0);
console.log("wide_into_narrow_same_buffer=" + Array.from(b2).join(","));

// ---- Out-of-range writes through an integer-indexed key are DROPPED. Reflect.set
// reports the [[Set]] result without turning it into a mode-dependent throw.
const t3 = new Uint8Array([10, 20, 30]);
console.log("oob_set_result=" + Reflect.set(t3, "5", 42) + "," + Reflect.set(t3, "-1", 42));
console.log("oob_read=" + String(t3[5]) + "," + String(t3[-1]) + " oob_in=" + ("5" in t3) + "," + ("-1" in t3));
console.log("oob_keys=" + Object.keys(t3).join(",") + " oob_hasOwn=" + Object.prototype.hasOwnProperty.call(t3, "5") + " oob_desc=" + String(Object.getOwnPropertyDescriptor(t3, "5")));
const strictWrite = function (): string {
  "use strict";
  try {
    (t3 as any)[9] = 1;
    return "no-throw";
  } catch (e: any) {
    return e.constructor.name;
  }
};
console.log("oob_strict_assign=" + strictWrite() + " len=" + t3.length);
const d0: any = Object.getOwnPropertyDescriptor(t3, "0");
console.log("in_desc=" + d0.value + " w:" + d0.writable + " e:" + d0.enumerable + " c:" + d0.configurable);

// Canonical numeric strings that are out of range are dropped; strings that are
// NOT canonical ("+1", "01", "1e1", " 1", "0x1") are ordinary properties and stick.
const n = new Uint8Array(3);
const keys: string[] = ["1.5", "-0", "+1", "01", "1e1", " 1", "0x1"];
console.log("canon_set_results=" + keys.map((k) => k + ":" + Reflect.set(n, k, 9)).join(" "));
console.log("canon_elems=" + Array.from(n).join(",") + " canon_keys=" + Object.keys(n).join("|"));
console.log("canon_reads=" + String((n as any)["1.5"]) + "," + String((n as any)["-0"]) + "," + String((n as any)["01"]) + "," + String((n as any)["+1"]));
console.log("canon_desc_1.5=" + String(Object.getOwnPropertyDescriptor(n, "1.5")) + " canon_desc_01=" + typeof Object.getOwnPropertyDescriptor(n, "01"));
const inf = new Uint8Array(2);
console.log("inf_set_results=" + ["Infinity", "NaN", "-Infinity"].map((k) => k + ":" + Reflect.set(inf, k, 9)).join(" ") + " inf_keys=" + Object.keys(inf).join("|"));
console.log("inf_reads=" + String((inf as any)["Infinity"]) + "," + String((inf as any)["NaN"]));

// ---- defineProperty: an index only accepts a full data descriptor, and cannot leave the range.
console.log("def_oob=" + t(() => Object.defineProperty(n, "9", { value: 1 })));
console.log("def_accessor=" + t(() => Object.defineProperty(n, "0", { get: () => 1 })));
console.log("def_nonconfigurable=" + t(() => Object.defineProperty(n, "0", { value: 5, configurable: false, writable: true, enumerable: true })));
console.log("def_value_only=" + t(() => { Object.defineProperty(n, "1", { value: 7 }); return n[1]; }));
console.log("def_writable_false=" + t(() => Object.defineProperty(n, "0", { value: 1, writable: false })));
console.log("def_plain_key=" + t(() => {
  Object.defineProperty(n, "tag", { value: "x", configurable: true });
  const d: any = Object.getOwnPropertyDescriptor(n, "tag");
  return (n as any).tag + " w:" + d.writable + " e:" + d.enumerable + " c:" + d.configurable;
}));

// ---- [[Delete]]: an in-range index refuses, anything else succeeds.
console.log("delete_oob=" + Reflect.deleteProperty(n, "99") + " delete_in_range=" + Reflect.deleteProperty(n, "0") + " value=" + n[0]);
console.log("delete_string_key=" + Reflect.deleteProperty(n, "01") + " keys=" + Object.keys(n).join("|"));

// ---- Integrity: a non-empty typed array cannot be frozen; an empty one can.
console.log("freeze_nonempty=" + t(() => Object.freeze(new Uint8Array(1))));
console.log("freeze_empty=" + Object.isFrozen(Object.freeze(new Uint8Array(0))) + " frozen_before=" + Object.isFrozen(new Uint8Array(1)));
const sealed = new Uint8Array([1, 2]);
Object.preventExtensions(sealed);
console.log("prevent_then_set=" + Reflect.set(sealed, "0", 9) + " oob=" + Reflect.set(sealed, "9", 1) + " after=" + Array.from(sealed).join(",") + " ext=" + Object.isExtensible(sealed));
const withProp: any = new Uint8Array([1]);
withProp.tag = "x";
console.log("own_keys=" + Reflect.ownKeys(withProp).join("|"));
const forin: string[] = [];
for (const k in withProp) forin.push(k);
console.log("for_in=" + forin.join("|"));
