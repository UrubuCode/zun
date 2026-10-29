// Cross-runtime: byte-level access. DataView defaults to BIG endian (the opposite of
// a typed array, which follows the machine), takes unaligned offsets, bounds-checks
// every access against its own window, and coerces each offset with ToIndex BEFORE
// the value is converted. Atomics works on an ordinary ArrayBuffer as well as a
// SharedArrayBuffer, answers the value read BEFORE the write, and refuses element
// kinds that are not integers.

const t = function (f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name;
  }
};
const hexOf = (b: ArrayBuffer | ArrayBufferView): string =>
  Array.from(new Uint8Array(b instanceof ArrayBuffer ? b : b.buffer, b instanceof ArrayBuffer ? 0 : b.byteOffset, b instanceof ArrayBuffer ? b.byteLength : b.byteLength))
    .map((x) => x.toString(16).padStart(2, "0")).join(" ");

// ---- Endianness: big endian by default, little endian on request.
const dv = new DataView(new ArrayBuffer(8));
dv.setUint16(0, 0x1234);
console.log("be_bytes=" + hexOf(dv) + " be_read=" + dv.getUint16(0).toString(16) + " le_read=" + dv.getUint16(0, true).toString(16));
console.log("default_is_big=" + (dv.getUint16(0) === dv.getUint16(0, false)) + " undefined_is_big=" + (dv.getUint16(0, undefined) === dv.getUint16(0, false)) + " truthy_is_little=" + (dv.getUint16(0, 1 as any) === dv.getUint16(0, true)));
dv.setUint32(0, 0x01020304, true);
console.log("le_bytes=" + hexOf(dv.buffer as ArrayBuffer).slice(0, 11) + " le_back=" + dv.getUint32(0, true).toString(16) + " be_view=" + dv.getUint32(0).toString(16));

// Unaligned offsets are legal for a DataView.
dv.setFloat64(0, 0);
dv.setInt32(1, -2, true);
console.log("unaligned_le=" + dv.getInt32(1, true) + " unaligned_be=" + dv.getInt32(1));
dv.setUint16(3, 0xabcd);
console.log("unaligned_u16=" + dv.getUint16(3).toString(16) + " swapped=" + dv.getUint16(3, true).toString(16));

// Every width, written and read back both ways.
const one = new DataView(new ArrayBuffer(8));
one.setInt8(0, -1);
console.log("i8=" + one.getInt8(0) + " u8=" + one.getUint8(0));
one.setInt16(0, -2, true);
console.log("i16=" + one.getInt16(0, true) + " u16=" + one.getUint16(0, true));
one.setFloat32(0, 1.5, true);
console.log("f32=" + one.getFloat32(0, true) + " as_be=" + one.getFloat32(0));
one.setFloat64(0, -0);
console.log("f64_negzero=" + Object.is(one.getFloat64(0), -0));
one.setFloat64(0, Math.PI, true);
console.log("f64_pi=" + (one.getFloat64(0, true) === Math.PI) + " f64_pi_be=" + one.getFloat64(0));
one.setFloat64(0, NaN);
console.log("f64_nan=" + Number.isNaN(one.getFloat64(0)));
one.setBigUint64(0, 2n ** 64n - 1n);
console.log("bigu=" + one.getBigUint64(0) + " bigi=" + one.getBigInt64(0));
one.setBigInt64(0, -1n, true);
console.log("bigi_le=" + one.getBigInt64(0, true) + " bigu_be=" + one.getBigUint64(0));
one.setBigUint64(0, 0x0102030405060708n);
console.log("bigu_bytes=" + hexOf(one.buffer as ArrayBuffer));

// ---- Value coercion per accessor, and the coercion order of offset vs value.
const cv = new DataView(new ArrayBuffer(4));
cv.setUint8(0, 300);
cv.setInt8(1, "129" as any);
cv.setUint8(2, NaN);
cv.setUint8(3, { valueOf: () => 7 } as any);
console.log("value_coercion=" + cv.getUint8(0) + "," + cv.getInt8(1) + "," + cv.getUint8(2) + "," + cv.getUint8(3));
console.log("setter_returns=" + String(cv.setUint8(0, 9)) + " read_back=" + cv.getUint8(0));
console.log("bigint_setter_number=" + t(() => new DataView(new ArrayBuffer(8)).setBigUint64(0, 1 as any)));
const seen: string[] = [];
const order = new DataView(new ArrayBuffer(8));
const offsetArg = { valueOf: () => { seen.push("offset"); return 1; } };
const valueArg = { valueOf: () => { seen.push("value"); return 0x1234; } };
order.setUint16(offsetArg as any, valueArg as any, true);
console.log("coercion_order=" + seen.join(",") + " bytes=" + order.getUint8(1) + "," + order.getUint8(2) + " read=" + order.getUint16(1, true));

// ---- Offset ToIndex on every accessor: a fraction truncates, -0.5 is index 0, a
// negative or Infinity or BigInt refuses, and bounds are checked against the window.
const buf8 = new ArrayBuffer(8);
new Uint8Array(buf8).set([0, 1, 2, 3, 4, 5, 6, 7]);
const acc = new DataView(buf8);
console.log("plain=" + acc.getUint8(1) + " fraction=" + acc.getUint8(1.9 as any) + " string=" + t(() => acc.getUint8("2" as any)));
console.log("bool=" + t(() => acc.getUint8(true as any)) + " null=" + t(() => acc.getUint8(null as any)) + " undefined=" + t(() => acc.getUint8(undefined as any)));
console.log("nan=" + t(() => acc.getUint8(NaN as any)) + " neg_fraction=" + t(() => acc.getUint8(-0.5 as any)));
console.log("negative=" + t(() => acc.getUint8(-1 as any)) + " infinity=" + t(() => acc.getUint8(Infinity as any)));
console.log("bigint_offset=" + t(() => acc.getUint8(1n as any)) + " valueof=" + t(() => acc.getUint8({ valueOf: () => 3 } as any)) + " no_argument=" + t(() => (acc as any).getUint8()));
console.log("last_byte=" + acc.getUint8(7) + " past=" + t(() => acc.getUint8(8)));
console.log("u32_last=" + acc.getUint32(4).toString(16) + " u32_past=" + t(() => acc.getUint32(5)));
console.log("f64_past=" + t(() => acc.getFloat64(1)) + " set_past=" + t(() => acc.setUint32(5, 1)) + " set_negative=" + t(() => acc.setUint8(-1, 1)));

// ---- Constructor arguments: ToIndex coercion, RangeError for an out-of-range window.
console.log("ctor_default_len=" + new DataView(new ArrayBuffer(4)).byteLength + " ctor_offset_only=" + new DataView(new ArrayBuffer(4), 1).byteLength);
console.log("ctor_offset_at_end=" + new DataView(new ArrayBuffer(4), 4).byteLength + " ctor_offset_past=" + t(() => new DataView(new ArrayBuffer(4), 5)));
console.log("ctor_offset_negative=" + t(() => new DataView(new ArrayBuffer(4), -1)) + " ctor_offset_fraction=" + new DataView(new ArrayBuffer(4), 1.9).byteOffset);
console.log("ctor_len_past=" + t(() => new DataView(new ArrayBuffer(4), 2, 3)) + " ctor_len_negative=" + t(() => new DataView(new ArrayBuffer(4), 0, -1)));
console.log("ctor_len_zero=" + new DataView(new ArrayBuffer(4), 2, 0).byteLength + " ctor_len_undefined=" + new DataView(new ArrayBuffer(4), 1, undefined).byteLength);
console.log("ctor_not_buffer=" + t(() => new DataView([1, 2, 3] as any)) + " ctor_typed_array=" + t(() => new DataView(new Uint8Array(4) as any)));
console.log("ctor_no_args=" + t(() => (DataView as any)()) + " ctor_no_new=" + t(() => (DataView as any)(new ArrayBuffer(4))));

// ---- A windowed view: byteOffset shifts index 0, byteLength ends the window.
const buf10 = new ArrayBuffer(10);
new Uint8Array(buf10).set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
const win = new DataView(buf10, 3, 4);
console.log("win=" + win.byteOffset + "," + win.byteLength + " buflen=" + win.buffer.byteLength);
console.log("win_read=" + win.getUint8(0) + " win_last=" + win.getUint8(3));
win.setUint8(0, 99);
console.log("win_write_lands_at=" + new Uint8Array(buf10)[3]);
console.log("win_past=" + t(() => win.getUint8(4)) + " win_straddle=" + t(() => win.getUint32(1)) + " win_fits=" + win.getUint32(0).toString(16));
const tail = new DataView(buf10, 8);
console.log("tail=" + tail.byteOffset + "," + tail.byteLength + " tag=" + Object.prototype.toString.call(tail) + " proto=" + (Object.getPrototypeOf(tail) === DataView.prototype));

// ---- A DataView's own geometry accessors are not typed-array accessors.
console.log("dv_geometry_on_typed=" + t(() => (Object.getOwnPropertyDescriptor(DataView.prototype, "byteLength") as any).get.call(new Uint8Array(4))));

// ---- Shared memory and Atomics.
console.log("sab_type=" + typeof SharedArrayBuffer + " sab_tag=" + Object.prototype.toString.call(new SharedArrayBuffer(1)));
const sab = new SharedArrayBuffer(8);
const shared = new Int32Array(sab);
shared[0] = 1;
console.log("add_prev=" + Atomics.add(shared, 0, 4) + " after_add=" + shared[0]);
console.log("cmp_prev=" + Atomics.compareExchange(shared, 0, 5, 9) + " after_cmp=" + shared[0]);
console.log("cas_miss_prev=" + Atomics.compareExchange(shared, 0, 4, 1) + " after_miss=" + shared[0]);
console.log("exchange_prev=" + Atomics.exchange(shared, 0, 2) + " load=" + Atomics.load(shared, 0));
console.log("wait_not_equal=" + Atomics.wait(shared, 0, 999, 0));
console.log("notify_shared=" + Atomics.notify(shared, 0, 1));

// ---- Atomics on an ORDINARY ArrayBuffer: every op answers the old value, the
// stored value goes through the element kind's conversion.
const i32 = new Int32Array(new ArrayBuffer(16));
console.log("store_returns=" + Atomics.store(i32, 0, 5) + " load=" + Atomics.load(i32, 0));
console.log("add_returns_old=" + Atomics.add(i32, 0, 3) + " now=" + i32[0]);
console.log("sub_returns_old=" + Atomics.sub(i32, 0, 1) + " now=" + i32[0]);
console.log("and_returns_old=" + Atomics.and(i32, 0, 6) + " now=" + i32[0]);
console.log("or_returns_old=" + Atomics.or(i32, 0, 1) + " now=" + i32[0]);
console.log("xor_returns_old=" + Atomics.xor(i32, 0, 3) + " now=" + i32[0]);
console.log("exchange_returns_old=" + Atomics.exchange(i32, 0, 9) + " now=" + i32[0]);
console.log("cas_match=" + Atomics.compareExchange(i32, 0, 9, 4) + " now=" + i32[0]);
console.log("cas_mismatch=" + Atomics.compareExchange(i32, 0, 99, 7) + " unchanged=" + i32[0]);
console.log("store_fraction=" + Atomics.store(i32, 1, 2.9) + " stored=" + i32[1]);
console.log("store_string=" + Atomics.store(i32, 1, "7" as any) + " stored=" + i32[1]);
console.log("store_nan=" + Atomics.store(i32, 1, NaN) + " stored=" + i32[1]);
console.log("store_undefined=" + Atomics.store(i32, 1, undefined as any) + " stored=" + i32[1]);
console.log("wrap_int8=" + (function (): string {
  const a = new Int8Array(2);
  return Atomics.store(a, 0, 200) + "/" + a[0] + "/" + Atomics.add(a, 0, 100) + "/" + a[0];
})());
console.log("wrap_uint32=" + (function (): string {
  const a = new Uint32Array(1);
  Atomics.store(a, 0, -1);
  return a[0] + "/" + Atomics.load(a, 0);
})());

// The index runs ToIndex against the array's length.
console.log("index_string=" + t(() => { Atomics.store(i32, "2" as any, 6); return String(i32[2]); }));
console.log("index_fraction=" + t(() => Atomics.load(i32, 1.5)));
console.log("index_negative=" + t(() => Atomics.load(i32, -1)) + " index_past_end=" + t(() => Atomics.load(i32, 4)));
console.log("index_undefined=" + t(() => Atomics.load(i32, undefined as any)) + " index_nan=" + t(() => Atomics.load(i32, NaN)));

// Which element kinds Atomics accepts: the integer kinds, bigint included.
const kinds = ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array", "Uint32Array", "Float32Array", "Float64Array", "BigInt64Array", "BigUint64Array"];
for (const name of kinds) {
  const ctor: any = (globalThis as any)[name];
  console.log("kind_" + name + "=" + t(() => Atomics.load(new ctor(2), 0)));
}
console.log("plain_array=" + t(() => Atomics.load([1, 2] as any, 0)) + " dataview=" + t(() => Atomics.load(new DataView(new ArrayBuffer(8)) as any, 0)));
console.log("detached=" + t(() => {
  const b = new ArrayBuffer(8);
  const a = new Int32Array(b);
  b.transfer();
  return Atomics.load(a, 0);
}));

// BigInt kinds take bigint operands only.
console.log("bigint_ops=" + t(() => {
  const b = new BigInt64Array(2);
  Atomics.store(b, 0, 5n);
  return String(Atomics.add(b, 0, 2n)) + "/" + b[0] + "/" + String(Atomics.compareExchange(b, 0, 7n, 1n)) + "/" + b[0];
}));
console.log("bigint_number_operand=" + t(() => Atomics.store(new BigInt64Array(1), 0, 5 as any)));
console.log("number_bigint_operand=" + t(() => Atomics.store(new Int32Array(1), 0, 5n as any)));

// wait needs a SHARED buffer; notify does not and reports zero wakes when nothing waits.
console.log("wait_non_shared=" + t(() => (Atomics as any).wait(i32, 0, 0, 0)));
console.log("notify_non_shared=" + Atomics.notify(i32, 0, 0) + " notify_default=" + Atomics.notify(i32, 0));
console.log("is_lock_free=" + [1, 2, 4, 8].map((n) => typeof Atomics.isLockFree(n)).join(",") + " four=" + Atomics.isLockFree(4));
console.log("namespace=" + Object.prototype.toString.call(Atomics) + " callable=" + t(() => (Atomics as any)()));
console.log("members=" + ["add", "and", "compareExchange", "exchange", "isLockFree", "load", "notify", "or", "store", "sub", "wait", "xor"].map((k) => typeof (Atomics as any)[k]).join(","));
