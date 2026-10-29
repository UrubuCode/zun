// Cross-runtime: TypedArray construction and byte geometry across every element
// kind. Covers the four constructor forms (length, array-like or iterable object,
// another typed array, buffer window), the ToIndex coercions and RangeErrors,
// Array.from / Array.of, the length / byteLength / byteOffset / BYTES_PER_ELEMENT
// arithmetic shared by views of different widths over one buffer, subarray and
// slice geometry, and the inherited accessors on %TypedArray%.prototype.

const t = function (f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name;
  }
};

const KINDS: [string, any][] = [
  ["Int8Array", Int8Array],
  ["Uint8Array", Uint8Array],
  ["Uint8ClampedArray", Uint8ClampedArray],
  ["Int16Array", Int16Array],
  ["Uint16Array", Uint16Array],
  ["Int32Array", Int32Array],
  ["Uint32Array", Uint32Array],
  ["Float32Array", Float32Array],
  ["Float64Array", Float64Array],
  ["BigInt64Array", BigInt64Array],
  ["BigUint64Array", BigUint64Array],
];

// Every kind: constructor, prototype and instance agree on the element size, and
// byteLength is always length * BYTES_PER_ELEMENT.
for (const [name, ctor] of KINDS) {
  const a = new ctor(3);
  const bpe: number = ctor.BYTES_PER_ELEMENT;
  console.log("bpe_" + name + "=" + bpe + " proto=" + ctor.prototype.BYTES_PER_ELEMENT + " inst=" + a.BYTES_PER_ELEMENT + " len=" + a.length + " bytes=" + a.byteLength + " ok=" + (a.byteLength === a.length * bpe));
}
const bpeDesc: any = Object.getOwnPropertyDescriptor(Uint16Array, "BYTES_PER_ELEMENT");
console.log("bpe_desc=w:" + bpeDesc.writable + " e:" + bpeDesc.enumerable + " c:" + bpeDesc.configurable);
console.log("bpe_on_intrinsic=" + String((Object.getPrototypeOf(Uint8Array) as any).BYTES_PER_ELEMENT));
console.log("ctor_arity=" + Uint8Array.length + "," + Float64Array.length + " from_arity=" + Uint8Array.from.length + " of_arity=" + Uint8Array.of.length);

// ---- Length form: ToIndex, so fractions truncate and strings/null/booleans convert,
// while negatives, NaN-free overflow and BigInt refuse.
console.log("len_plain=" + new Uint8Array(3).length + " zeroed=" + new Uint8Array(3).join(","));
console.log("len_string=" + t(() => new Uint8Array("3" as any).length));
console.log("len_fraction=" + t(() => new Uint8Array(2.9).length));
console.log("len_negative=" + t(() => new Uint8Array(-1)));
console.log("len_negative_fraction=" + t(() => new Uint8Array(-0.5).length));
console.log("len_nan=" + t(() => new Uint8Array(NaN).length));
console.log("len_null=" + t(() => new Uint8Array(null as any).length));
console.log("len_undefined=" + t(() => new Uint8Array(undefined as any).length));
console.log("len_true=" + t(() => new Uint8Array(true as any).length));
console.log("len_infinity=" + t(() => new Uint8Array(Infinity)));
console.log("len_bigint=" + t(() => new Uint8Array(3n as any).length));
console.log("len_string_primitive_is_index=" + t(() => new Uint8Array("12" as any).length));

// ---- Object form: an iterable is drained through its iterator (and wins over
// length); otherwise an array-like reads .length and the holes become 0.
console.log("arraylike=" + t(() => new Uint8Array({ length: 3, 0: 1, 2: 5 } as any).join(",")));
console.log("arraylike_no_length=" + t(() => new Uint8Array({ 0: 1 } as any).length));
console.log("holes=" + t(() => new Uint8Array([1, , 3] as any).join(",")));
console.log("iterable_set=" + t(() => new Uint8Array(new Set([1, 2, 3]) as any).join(",")));
console.log("iterable_custom_wins_over_length=" + t(() => {
  const src: any = { length: 9 };
  src[Symbol.iterator] = function* () { yield 1; yield 300; };
  return new Uint8Array(src).join(",");
}));
console.log("iterator_not_callable=" + t(() => {
  const src: any = { length: 1, 0: 5 };
  src[Symbol.iterator] = 7;
  return new Uint8Array(src).join(",");
}));
console.log("from_float_kind=" + t(() => new Uint8Array(new Float64Array([1.7, -1, 300])).join(",")));
console.log("widen_kind=" + t(() => new Float64Array(new Uint8Array([1, 2])).join(",")));
console.log("copy_not_share=" + t(() => {
  const src = new Uint8Array([1, 2]);
  const dst = new Uint8Array(src);
  dst[0] = 9;
  return src[0] + "/" + (dst.buffer === src.buffer);
}));

// ---- Buffer form: byteOffset must be a multiple of the element size and in range;
// an inferred length must divide the remaining bytes evenly.
console.log("buffer_whole=" + new Uint32Array(new ArrayBuffer(8)).length);
console.log("buffer_offset=" + new Uint32Array(new ArrayBuffer(8), 4).length);
console.log("buffer_offset_string=" + new Uint32Array(new ArrayBuffer(8), "4" as any).length);
console.log("buffer_offset_fraction=" + new Uint8Array(new ArrayBuffer(4), 1.9).byteOffset);
console.log("buffer_offset_at_end=" + new Uint8Array(new ArrayBuffer(4), 4).length);
console.log("buffer_misaligned=" + t(() => new Uint32Array(new ArrayBuffer(8), 2)));
console.log("buffer_bad_total=" + t(() => new Uint32Array(new ArrayBuffer(6))));
console.log("buffer_offset_past=" + t(() => new Uint8Array(new ArrayBuffer(4), 5)));
console.log("buffer_len_past=" + t(() => new Uint8Array(new ArrayBuffer(4), 2, 3)));
console.log("buffer_offset_negative=" + t(() => new Uint8Array(new ArrayBuffer(4), -1)));
console.log("buffer_explicit_len=" + new Uint16Array(new ArrayBuffer(10), 2, 4).byteLength);
console.log("buffer_explicit_zero=" + new Uint32Array(new ArrayBuffer(16), 8, 0).length + "/" + new Uint32Array(new ArrayBuffer(16), 8, 0).byteOffset);
console.log("buffer_len_undefined=" + new Uint8Array(new ArrayBuffer(4), 1, undefined).length);
console.log("buffer_shares=" + t(() => {
  const buf = new ArrayBuffer(4);
  const a = new Uint8Array(buf);
  const b = new Uint8Array(buf);
  a[0] = 6;
  return b[0] + "/" + (a.buffer === b.buffer);
}));
console.log("empty_buffer_views=" + new Uint32Array(new ArrayBuffer(0)).length + "/" + new DataView(new ArrayBuffer(0)).byteLength);

// ---- Called without new, and the static builders.
console.log("no_new=" + t(() => (Uint8Array as any)(3)));
console.log("from_mapped=" + Uint8Array.from([1, 2, 3], (x) => x * 2).join(","));
console.log("from_string=" + Uint8Array.from("123" as any).join(","));
console.log("from_index_arg=" + Uint8Array.from([9, 9], (_v, i) => i).join(","));
console.log("from_this=" + (Uint8Array.from.call(Int16Array, [1, 2]) as any).constructor.name);
console.log("of=" + Uint8Array.of(1, 300, -1).join(",") + " of_empty=" + Uint8Array.of().length + " from_empty=" + Uint8Array.from([]).length);

// ---- One buffer, three widths: byteOffset is in BYTES, length is in ELEMENTS.
const buf = new ArrayBuffer(16);
const asBytes = new Uint8Array(buf);
const asWords = new Uint16Array(buf, 4);
const asLongs = new Uint32Array(buf, 8, 2);
console.log("bytes_geom=" + asBytes.length + "/" + asBytes.byteOffset + "/" + asBytes.byteLength);
console.log("words_geom=" + asWords.length + "/" + asWords.byteOffset + "/" + asWords.byteLength);
console.log("longs_geom=" + asLongs.length + "/" + asLongs.byteOffset + "/" + asLongs.byteLength);
console.log("same_buffer=" + (asBytes.buffer === buf) + "," + (asWords.buffer === buf) + "," + (asLongs.buffer === buf));

// A write through the widest view is visible through the narrowest; the sorted
// byte set is endian-independent.
asLongs[0] = 0x01020304;
console.log("aliased_bytes=" + Array.from(asBytes.subarray(8, 12)).sort((a, b) => a - b).join(","));
console.log("byte_sum=" + Array.from(asBytes.subarray(8, 12)).reduce((a, b) => a + b, 0));

// subarray recomputes byteOffset from the receiver's own, in elements.
const sub = asWords.subarray(1, 4);
console.log("sub=" + sub.length + "/" + sub.byteOffset + "/" + sub.byteLength + " shares=" + (sub.buffer === buf));
const subOfSub = sub.subarray(1);
console.log("sub_of_sub=" + subOfSub.length + "/" + subOfSub.byteOffset + "/" + subOfSub.byteLength);
console.log("sub_negative=" + asWords.subarray(-2).length + "/" + asWords.subarray(-2).byteOffset);
console.log("sub_inverted=" + asWords.subarray(4, 1).length + "/" + asWords.subarray(4, 1).byteOffset);
console.log("sub_past_end=" + asWords.subarray(3, 99).length + "/" + asWords.subarray(3, 99).byteOffset);
console.log("sub_empty_offset=" + asWords.subarray(6, 6).byteOffset + " sub_kind=" + sub.constructor.name);

// slice copies into a fresh buffer, so byteOffset restarts at zero.
const copy = asWords.slice(1, 4);
console.log("slice_geom=" + copy.length + "/" + copy.byteOffset + "/" + copy.byteLength + " own_buffer=" + (copy.buffer !== buf) + " buffer_len=" + copy.buffer.byteLength);

// Inferred lengths and misalignment with a different width.
console.log("infer_length=" + new Uint32Array(buf).length + " infer_from_offset=" + new Uint32Array(buf, 8).length);
console.log("byte_view_any_offset=" + new Uint8Array(buf, 3).length + " int16_odd_offset=" + t(() => new Int16Array(buf, 3)));
console.log("bpe_times_length=" + KINDS.map(([, ctor]) => {
  const a = new ctor(4);
  return String(a.byteLength === a.length * a.BYTES_PER_ELEMENT);
}).join(","));

// ---- Inherited accessors on %TypedArray%.prototype: geometry is a GETTER, never
// an own property, and a foreign receiver is a TypeError (except toStringTag).
const proto: any = Object.getPrototypeOf(Uint8Array.prototype);
const getterOf = function (key: any): any {
  return (Object.getOwnPropertyDescriptor(proto, key) as any).get;
};
const view = new Int16Array(new ArrayBuffer(16), 4, 3);
for (const key of ["buffer", "byteLength", "byteOffset", "length"]) {
  const own = Object.getOwnPropertyDescriptor(view, key);
  const d: any = Object.getOwnPropertyDescriptor(proto, key);
  console.log("accessor_" + key + "=own:" + (own === undefined) + " get:" + typeof d.get + " set:" + typeof d.set + " e:" + d.enumerable + " c:" + d.configurable);
}
console.log("view_values=" + view.byteLength + "," + view.byteOffset + "," + view.length + " buffer_ok=" + (view.buffer instanceof ArrayBuffer));
for (const key of ["buffer", "byteLength", "byteOffset", "length"]) {
  const g = getterOf(key);
  console.log("foreign_" + key + "=" + t(() => g.call({})) + "/" + t(() => g.call([1, 2])) + "/" + t(() => g.call(new DataView(new ArrayBuffer(4)))) + "/" + t(() => g.call(null)));
}
console.log("cross_kind_receiver=" + getterOf("length").call(new Float64Array(2)));

// Symbol.toStringTag is the one getter that answers undefined for a foreign receiver.
const tagGetter = getterOf(Symbol.toStringTag);
console.log("tag_plain=" + String(tagGetter.call({})) + " tag_array=" + String(tagGetter.call([])) + " tag_typed=" + tagGetter.call(new BigUint64Array(1)));
console.log("tag_null=" + t(() => tagGetter.call(null)));
console.log("tostring_typed=" + Object.prototype.toString.call(new Float32Array(1)) + " tostring_proto=" + Object.prototype.toString.call(Uint8Array.prototype));

// The intrinsic %TypedArray% sits above every concrete kind and is not a global.
const TA: any = Object.getPrototypeOf(Uint8Array);
console.log("intrinsic_name=" + TA.name + " shared=" + (Object.getPrototypeOf(Int16Array) === TA) + "," + (Object.getPrototypeOf(BigInt64Array) === TA));
console.log("intrinsic_not_constructor=" + t(() => new TA(1)) + " intrinsic_global=" + typeof (globalThis as any).TypedArray);
console.log("proto_chain=" + (Object.getPrototypeOf(Uint8Array.prototype) === TA.prototype) + " intrinsic_above=" + (Object.getPrototypeOf(TA) === Function.prototype));
console.log("methods_inherited=" + ["slice", "subarray", "set", "fill", "sort"].map((k) => String(Object.prototype.hasOwnProperty.call(Uint8Array.prototype, k))).join(","));
console.log("own_on_kind_proto=" + ["constructor", "BYTES_PER_ELEMENT"].map((k) => k + ":" + Object.prototype.hasOwnProperty.call(Uint8Array.prototype, k)).join(","));

// ---- subarray is a VIEW (same buffer, bounds clamped like Array#slice), slice is a COPY.
const base = new Uint8Array([1, 2, 3, 4, 5, 6]);
const shallow = base.subarray(1, 4);
const copied = base.slice(1, 4);
console.log("view_offsets=" + shallow.byteOffset + " copy_offset=" + copied.byteOffset + " copy_buflen=" + copied.buffer.byteLength);
console.log("view_shares=" + (shallow.buffer === base.buffer) + " copy_shares=" + (copied.buffer === base.buffer));
shallow[0] = 99;
copied[0] = 88;
console.log("base_after=" + Array.from(base).join(",") + " view_after=" + Array.from(shallow).join(",") + " copy_after=" + Array.from(copied).join(","));
const inner = shallow.subarray(1);
console.log("inner=" + Array.from(inner).join(",") + " inner_offset=" + inner.byteOffset + " inner_shares=" + (inner.buffer === base.buffer));

const src = new Uint8Array([1, 2, 3, 4, 5]);
console.log("sub_neg=" + Array.from(src.subarray(-2)).join(",") + " sub_negneg=" + Array.from(src.subarray(-4, -1)).join(","));
console.log("sub_none=" + Array.from(src.subarray()).join(",") + " sub_past=" + Array.from(src.subarray(2, 99)).join(","));
console.log("sub_reversed=" + src.subarray(4, 2).length + "/" + src.subarray(4, 2).byteOffset + " sub_far=" + src.subarray(99).length + "/" + src.subarray(99).byteOffset);
console.log("slice_neg=" + Array.from(src.slice(-2)).join(",") + " slice_reversed=" + src.slice(4, 2).length + " slice_past=" + Array.from(src.slice(2, 99)).join(","));
console.log("slice_undef_end=" + Array.from(src.slice(1, undefined)).join(","));
console.log("sub_kind=" + src.subarray(0, 1).constructor.name + "/" + src.slice(0, 1).constructor.name + " sub_isarray=" + Array.isArray(src.subarray(0, 1)));

const wide = new Int32Array([10, 20, 30, 40]);
const wsub = wide.subarray(1, 3);
const wcopy = wide.slice(1, 3);
console.log("wide_sub=" + Array.from(wsub).join(",") + " off=" + wsub.byteOffset + " len=" + wsub.byteLength);
console.log("wide_copy=" + Array.from(wcopy).join(",") + " off=" + wcopy.byteOffset + " buflen=" + wcopy.buffer.byteLength);

const empty = base.subarray(3, 3);
console.log("empty_view=" + (empty.buffer === base.buffer) + " len=" + empty.length + " off=" + empty.byteOffset);
