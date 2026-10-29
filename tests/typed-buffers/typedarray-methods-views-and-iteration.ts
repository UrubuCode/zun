// Cross-runtime: TypedArray methods and their semantics. A typed array is NOT an
// Array: its own map/filter/slice answer a typed array of the same kind (through
// Symbol.species), the length-changing mutators are absent, and a borrowed
// Array.prototype method sees it as a generic array-like. Covers numeric default
// sort and NaN / -0 ordering, the change-by-copy trio, fill / copyWithin / with /
// at, NaN and -0 under indexOf vs includes, subarray vs slice aliasing, and the
// shared %ArrayIteratorPrototype% iteration protocol including detach mid-walk.

const t = function (f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name;
  }
};

// ---- Numeric sort: typed arrays sort numerically by default, Arrays sort by string.
const nums = [10, 9, 2, 100, 1];
console.log("array_default=" + nums.slice().sort().join(","));
console.log("typed_default=" + Array.from(new Int32Array(nums).sort()).join(","));
console.log("typed_negatives=" + Array.from(new Int32Array([3, -10, 0, -1]).sort()).join(","));
console.log("typed_comparator=" + Array.from(new Int32Array(nums).sort((a, b) => b - a)).join(","));
const inPlace = new Uint8Array([3, 1, 2]);
console.log("sort_identity=" + (inPlace.sort() === inPlace) + " " + Array.from(inPlace).join(","));

// Float ordering: -0 before +0, NaN last, infinities at the ends.
const floats = new Float64Array([3, NaN, -0, 0, -1, Infinity, -Infinity, NaN]);
floats.sort();
const shown: string[] = [];
for (let i = 0; i < floats.length; i++) {
  shown.push(Object.is(floats[i], -0) ? "-0" : String(floats[i]));
}
console.log("float_sort=" + shown.join(","));

// Comparator is called with numbers; a NaN result is treated as +0 (stable order kept).
let calls = 0;
const counted = new Uint8Array([2, 1, 3]);
counted.sort((a, b) => {
  calls++;
  return typeof a === "number" && typeof b === "number" ? a - b : 0;
});
console.log("comparator=" + Array.from(counted).join(",") + " called=" + (calls > 0));
console.log("comparator_nan=" + Array.from(new Uint8Array([2, 1, 3]).sort(() => NaN)).join(","));

// Sorting a view sorts only the view's window.
const whole = new Uint8Array([5, 4, 3, 2, 1]);
whole.subarray(1, 4).sort();
console.log("view_sort=" + Array.from(whole).join(","));

// ---- Change-by-copy trio keeps the element kind and leaves the receiver alone.
const src = new Int8Array([3, 1, 2]);
const sorted = src.toSorted();
console.log("toSorted=" + Array.from(sorted).join(",") + " kind=" + sorted.constructor.name + " src=" + Array.from(src).join(","));
console.log("toSorted_cmp=" + Array.from(src.toSorted((a, b) => b - a)).join(","));
const reversed = src.toReversed();
console.log("toReversed=" + Array.from(reversed).join(",") + " kind=" + reversed.constructor.name + " src=" + Array.from(src).join(","));
const withHigh = src.with(1, 300);
console.log("with=" + Array.from(withHigh).join(",") + " kind=" + withHigh.constructor.name + " src=" + Array.from(src).join(","));
console.log("with_negative=" + Array.from(src.with(-1, 7)).join(",") + " with_float_index=" + Array.from(src.with(1.9, 7)).join(","));
console.log("with_oob=" + t(() => src.with(3, 1)) + " with_neg_oob=" + t(() => src.with(-4, 1)));
console.log("with_number_into_bigint=" + t(() => new BigInt64Array(2).with(0, 1 as any)));
console.log("copies_detached=" + (sorted.buffer !== src.buffer) + "," + (reversed.buffer !== src.buffer) + "," + (withHigh.buffer !== src.buffer));
const at = new Uint16Array([7, 8, 9]);
console.log("at=" + at.at(0) + "," + at.at(-1) + "," + String(at.at(3)) + "," + String(at.at(-4)) + " at_float=" + at.at(1.9) + "," + at.at(NaN));
const rev = new Uint8Array([1, 2, 3]);
console.log("reverse_identity=" + (rev.reverse() === rev) + " " + Array.from(rev).join(","));

// ---- Search and element writes over NaN and -0. indexOf is strict equality (NaN never
// matches), includes is SameValueZero (NaN matches), and -0 equals 0 in both.
const f = new Float64Array([NaN, -0, 0, 1, NaN]);
console.log("indexOf_nan=" + f.indexOf(NaN) + " lastIndexOf_nan=" + f.lastIndexOf(NaN) + " includes_nan=" + f.includes(NaN));
console.log("indexOf_zero=" + f.indexOf(0) + " indexOf_negzero=" + f.indexOf(-0) + " lastIndexOf_negzero=" + f.lastIndexOf(-0) + " includes_negzero=" + f.includes(-0));
console.log("negzero_kept=" + Object.is(f[1], -0) + " zero_kept=" + Object.is(f[2], 0));
console.log("find_nan=" + String(f.find((v) => Number.isNaN(v))) + " findIndex=" + f.findIndex((v) => Number.isNaN(v)) + " findLastIndex=" + f.findLastIndex((v) => Number.isNaN(v)));
console.log("indexOf_fromIndex=" + f.indexOf(1, 4) + "," + f.indexOf(1, -2) + "," + f.indexOf(1, -99) + "," + f.indexOf(1, 99));
console.log("includes_fromIndex=" + f.includes(1, 4) + "," + f.includes(1, -2));
console.log("lastIndexOf_fromIndex=" + f.lastIndexOf(NaN, 0) + "," + f.lastIndexOf(0, 1));
console.log("float_search_string=" + f.indexOf("1" as any));
const i32 = new Int32Array([NaN as any, -0, 5]);
console.log("int_stored=" + i32.join(",") + " negzero_kept=" + Object.is(i32[1], -0));
console.log("int_search=" + i32.indexOf(NaN) + "," + i32.includes(NaN) + "," + i32.indexOf(-0) + "," + i32.indexOf(0));
console.log("int_search_string=" + i32.indexOf("5" as any) + " includes_string=" + i32.includes("5" as any));
console.log("at_special=" + String(f.at(0)) + "," + String(f.at(-1)) + "," + String(f.at(-9)) + "," + String(f.at(9)) + "," + String(f.at(1.9)) + "," + String(f.at(NaN)));

// ---- fill: coerces once, clamps the range, answers the same object.
console.log("fill_all=" + new Uint8Array(4).fill(300).join(","));
console.log("fill_range=" + new Uint8Array([1, 2, 3, 4]).fill(9, 1, 3).join(","));
console.log("fill_negative=" + new Uint8Array([1, 2, 3, 4]).fill(9, -2).join(","));
console.log("fill_inverted=" + new Uint8Array([1, 2, 3, 4]).fill(9, 3, 1).join(","));
console.log("fill_past_end=" + new Uint8Array([1, 2]).fill(9, 0, 99).join(","));
console.log("fill_identity=" + (new Uint8Array(2).fill(1) instanceof Uint8Array));
console.log("fill_nan_into_int=" + new Int8Array(2).fill(NaN).join(","));
console.log("fill_coerces_once=" + t(() => {
  let n = 0;
  const a = new Uint8Array(3);
  a.fill({ valueOf: () => { n++; return 4; } } as any);
  return a.join(",") + "/calls:" + n;
}));

// ---- copyWithin moves bytes inside one buffer, overlap included.
console.log("copyWithin=" + new Uint8Array([1, 2, 3, 4, 5]).copyWithin(0, 3).join(","));
console.log("copyWithin_overlap=" + new Uint8Array([1, 2, 3, 4, 5]).copyWithin(1, 0, 4).join(","));
console.log("copyWithin_negative=" + new Uint8Array([1, 2, 3, 4, 5]).copyWithin(-2, 0, 2).join(","));
console.log("copyWithin_noop=" + new Uint8Array([1, 2, 3]).copyWithin(0, 0).join(","));

// ---- with(): a copy with one element replaced; the value goes through element coercion.
console.log("with_nan_float=" + f.with(0, 5).join(","));
console.log("with_oob_high=" + t(() => (new Uint8Array([1, 2, 3]) as any).with(3, 9)) + " with_oob_low=" + t(() => (new Uint8Array([1, 2, 3]) as any).with(-4, 9)));
console.log("with_coerces=" + new Uint8Array([1, 2]).with(0, 300 as any).join(","));
console.log("with_nan_into_int=" + new Int8Array([1]).with(0, NaN as any).join(","));

// ---- The typed array as a receiver of Array-ish methods.
const ta = new Uint8Array([3, 1, 2]);
console.log("own_map=" + ta.map((x) => x * 2).constructor.name + " values=" + ta.map((x) => x * 2).join(","));
console.log("own_map_coerces=" + new Uint8Array([200]).map((x) => x * 2).join(","));
console.log("own_filter=" + ta.filter((x) => x > 1).constructor.name + " values=" + ta.filter((x) => x > 1).join(","));
console.log("own_slice_kind=" + ta.slice(1).constructor.name + " toReversed_kind=" + ta.toReversed().constructor.name);
console.log("absent=" + ["push", "pop", "shift", "unshift", "splice", "concat", "flat", "flatMap", "toSpliced"].map((k) => k + ":" + typeof (ta as any)[k]).join(" "));
console.log("present=" + ["map", "filter", "reduce", "reduceRight", "forEach", "every", "some", "find", "findLast", "reverse", "sort", "join", "at", "with", "toSorted", "toReversed", "entries"].map((k) => typeof (ta as any)[k]).join(","));
console.log("reduce=" + ta.reduce((a, b) => a + b, 0) + " reduceRight=" + ta.reduceRight((a, b) => a + "" + b, "") + " reduce_no_init=" + ta.reduce((a, b) => a + b));
console.log("reduce_empty_no_init=" + t(() => (new Uint8Array(0) as any).reduce((a: number, b: number) => a + b)));
console.log("every_some=" + ta.every((x) => x > 0) + "," + ta.some((x) => x > 2));
const seen: string[] = [];
ta.forEach((v, i, arr) => seen.push(v + ":" + i + ":" + (arr === ta)));
console.log("forEach_args=" + seen.join(" "));
console.log("join=" + ta.join("-") + " join_default=" + ta.join() + " join_undefined=" + new Uint8Array([1, 2]).join(undefined as any));
console.log("string_conversion=" + String(ta) + " template=" + `${ta}` + " concat=" + ("x" + ta));
console.log("is_array=" + Array.isArray(ta) + " instanceof_array=" + (ta instanceof Array) + " instanceof_ta=" + (ta instanceof Uint8Array));

// Borrowed Array.prototype methods treat it as an array-like and answer a plain Array.
console.log("borrow_map=" + (Array.prototype.map.call(ta, (x: number) => x * 2) as any).constructor.name);
console.log("borrow_filter=" + JSON.stringify(Array.prototype.filter.call(ta, (x: number) => x > 1)));
console.log("borrow_slice=" + JSON.stringify(Array.prototype.slice.call(ta, 1)));
console.log("borrow_concat=" + JSON.stringify(Array.prototype.concat.call([], ta as any)));
console.log("borrow_join=" + Array.prototype.join.call(ta, "|") + " borrow_indexOf=" + Array.prototype.indexOf.call(ta, 1));
console.log("borrow_push=" + t(() => {
  const a = new Uint8Array([1, 2]);
  return Array.prototype.push.call(a as any, 9);
}));
console.log("borrow_sort_string_order=" + Array.prototype.sort.call(new Uint8Array([10, 9, 2])).join(","));

// And the reverse borrow: a typed-array method on a plain Array is refused.
console.log("typed_map_on_array=" + t(() => (Uint8Array.prototype.map as any).call([1, 2], (x: number) => x)));
console.log("typed_join_on_array=" + t(() => (Uint8Array.prototype.join as any).call([1, 2], ",")));
console.log("typed_slice_on_array=" + t(() => (Uint8Array.prototype.slice as any).call([1, 2], 0)));
console.log("typed_map_keeps_receiver_kind=" + (Uint8Array.prototype.map as any).call(new Float64Array([1.5]), (x: number) => x).constructor.name);

// JSON and spread see the elements through their own enumeration.
console.log("json=" + JSON.stringify(ta) + " json_empty=" + JSON.stringify(new Uint8Array(0)) + " json_nested=" + JSON.stringify({ v: new Uint8Array([1]) }));
console.log("spread=" + JSON.stringify([...ta]) + " from=" + JSON.stringify(Array.from(ta)));
console.log("object_assign=" + JSON.stringify(Object.assign({}, new Uint8Array([7, 8]))));
console.log("keys=" + Object.keys(ta).join(",") + " values=" + Object.values(ta).join(",") + " entries=" + JSON.stringify(Object.entries(new Uint8Array([8, 9]))));
console.log("tostring_tag=" + Object.prototype.toString.call(ta) + " toString_is_array_toString=" + (Uint8Array.prototype.toString === Array.prototype.toString));

// ---- Species: slice / map / filter build their result through the receiver's species.
class MyU8 extends Uint8Array {
  static get [Symbol.species]() { return Uint16Array; }
}
const mine = new MyU8([1, 2, 3, 4]);
const sp = mine.slice(1, 3);
console.log("species_slice=" + sp.constructor.name + " bpe=" + sp.BYTES_PER_ELEMENT + " values=" + Array.from(sp).join(","));
class BadSpecies extends Uint8Array {
  static get [Symbol.species]() { return BigInt64Array; }
}
const bad = new BadSpecies([1, 2, 3]);
console.log("species_content_mismatch=" + t(() => bad.slice()) + "," + t(() => bad.map((x) => x)));

// ---- Iteration: the same %ArrayIteratorPrototype% an Array uses.
const it = new Uint8Array([1, 2, 3]);
console.log("iterator_is_values=" + (it[Symbol.iterator] === it.values) + " iterator_proto_shared=" + (Object.getPrototypeOf(it.values()) === Object.getPrototypeOf([].values())));
const selfIter = it.values();
console.log("iterator_tag=" + Object.prototype.toString.call(selfIter) + " iterator_self=" + (selfIter[Symbol.iterator]() === selfIter));
console.log("values=" + JSON.stringify(Array.from(it.values())) + " keys=" + JSON.stringify(Array.from(it.keys())) + " entries=" + JSON.stringify(Array.from(it.entries())));
console.log("step_shape=" + JSON.stringify(it.values().next()) + " next_arity=" + it.values().next.length);
console.log("after_done=" + (function (): string {
  const i2 = new Uint8Array([1]).values();
  i2.next();
  const done = i2.next();
  return done.done + "/" + String(done.value) + "/" + JSON.stringify(i2.next());
})());
console.log("has_return_throw=" + typeof (it.values() as any).return + "," + typeof (it.values() as any).throw);
console.log("live_reads=" + (function (): string {
  const a = new Uint8Array([1, 2, 3]);
  const out: number[] = [];
  for (const v of a) {
    out.push(v);
    a[2] = 9;
  }
  return out.join(",");
})());
console.log("for_of_bigint=" + Array.from(new BigInt64Array([1n, 2n])).join(","));
console.log("destructure=" + (function (): string {
  const [a, , c] = new Uint8Array([4, 5, 6]);
  return a + "/" + c;
})());
console.log("rest=" + (function (): string {
  const [head, ...tail] = new Uint8Array([7, 8, 9]);
  return head + "/" + JSON.stringify(tail) + "/" + Array.isArray(tail);
})());
console.log("array_from_kind=" + Array.from(it).constructor.name + " spread_kind=" + [...it].constructor.name + " mapped=" + Array.from(it, (x) => x * 10).join(","));
console.log("from_without_iterator=" + t(() => {
  const a: any = new Uint8Array([1, 2]);
  a[Symbol.iterator] = undefined;
  return Array.from(a).join(",");
}));
console.log("spread_without_iterator=" + t(() => {
  const a: any = new Uint8Array([1, 2]);
  a[Symbol.iterator] = undefined;
  return [...a].join(",");
}));
console.log("shadowed_iterator=" + t(() => {
  const a: any = new Uint8Array([1, 2]);
  a[Symbol.iterator] = function* () { yield 42; };
  return [...a].join(",") + "/" + Array.from(a).join(",");
}));
console.log("detached_next=" + t(() => {
  const buf = new ArrayBuffer(2);
  const a = new Uint8Array(buf);
  const i3 = a[Symbol.iterator]();
  i3.next();
  buf.transfer();
  return JSON.stringify(i3.next());
}));
console.log("detached_from_start=" + t(() => {
  const buf = new ArrayBuffer(2);
  const a = new Uint8Array(buf);
  buf.transfer();
  return JSON.stringify(Array.from(a));
}));
console.log("map_key_identity=" + (function (): string {
  const key = new Uint8Array([1]);
  const m = new Map([[key, "v"]]);
  return String(m.get(key)) + "/" + String(m.get(new Uint8Array([1]))) + "/" + m.size;
})());

// ---- Views versus copies: subarray aliases the buffer, slice does not.
const base = new Uint8Array([1, 2, 3, 4, 5, 6]);
const view = base.subarray(1, 4);
const copy = base.slice(1, 4);
view[0] = 99;
copy[0] = 88;
console.log("base_after=" + Array.from(base).join(",") + " view_after=" + Array.from(view).join(",") + " copy_after=" + Array.from(copy).join(","));
const subSort = new Uint8Array([5, 4, 3, 2, 1]).subarray(1, 4);
const subSortHost = new Uint8Array([5, 4, 3, 2, 1]);
const sharedSub = subSortHost.subarray(1, 4);
sharedSub.sort();
console.log("sub_sort_host=" + Array.from(subSortHost).join(",") + " sub_len=" + subSort.length);
sharedSub[0] = 9;
console.log("sub_write_host=" + Array.from(subSortHost).join(","));
const sliceOfView = view.slice(0);
console.log("slice_of_view=" + Array.from(sliceOfView).join(",") + " buflen=" + sliceOfView.buffer.byteLength + " inner_offset=" + view.subarray(1).byteOffset);
const empty = base.subarray(3, 3);
console.log("empty_view=" + (empty.buffer === base.buffer) + " len=" + empty.length + " off=" + empty.byteOffset);
