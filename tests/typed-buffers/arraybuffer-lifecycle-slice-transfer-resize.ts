// Cross-runtime: the ArrayBuffer lifecycle. slice() makes a COPY with Array-style
// clamped bounds through Symbol.species; ArrayBuffer.isView answers for views only.
// A resizable buffer ({ maxByteLength }) makes length-tracking views follow it, while
// fixed-length views go out of bounds when it shrinks. transfer() moves the bytes and
// DETACHES the source, every view over a detached buffer keeps its identity but
// reports zero, and which operations answer undefined versus throw is pinned.

const t = function (f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name;
  }
};
const bytesOf = (b: ArrayBuffer): string => Array.from(new Uint8Array(b)).join(",");

// ---- slice(): a copy, clamped like Array#slice.
const buf = new ArrayBuffer(8);
new Uint8Array(buf).set([1, 2, 3, 4, 5, 6, 7, 8]);
console.log("slice_range=" + bytesOf(buf.slice(2, 5)));
console.log("slice_from=" + bytesOf(buf.slice(6)));
console.log("slice_all=" + bytesOf(buf.slice(0)) + " len=" + buf.slice(0).byteLength);
console.log("slice_negative_start=" + bytesOf(buf.slice(-3)));
console.log("slice_negative_end=" + bytesOf(buf.slice(1, -5)));
console.log("slice_both_negative=" + bytesOf(buf.slice(-4, -2)));
console.log("slice_inverted=" + buf.slice(5, 2).byteLength);
console.log("slice_past_end=" + bytesOf(buf.slice(6, 99)));
console.log("slice_before_start=" + bytesOf(buf.slice(-99, 2)));
console.log("slice_empty=" + buf.slice(3, 3).byteLength);
console.log("slice_undefined_end=" + buf.slice(2, undefined).byteLength);
console.log("slice_fraction=" + buf.slice(1.9, 3.9).byteLength + " values=" + bytesOf(buf.slice(1.9, 3.9)));
console.log("slice_string_start=" + buf.slice("2" as any).byteLength);
console.log("slice_nan_start=" + buf.slice(NaN as any, 2).byteLength);
console.log("slice_no_args=" + buf.slice().byteLength);
const copied = buf.slice(0, 2);
new Uint8Array(copied)[0] = 99;
console.log("slice_is_copy=" + new Uint8Array(buf)[0] + "/" + (copied === buf));
console.log("slice_kind=" + (buf.slice(0) instanceof ArrayBuffer) + " tag=" + Object.prototype.toString.call(buf.slice(0)));
console.log("slice_wrong_receiver=" + t(() => (ArrayBuffer.prototype.slice as any).call(new Uint8Array(4), 0)));

// ---- Species decides the constructor of the copy.
console.log("species_default=" + (ArrayBuffer[Symbol.species] === ArrayBuffer));
const speciesDesc: any = Object.getOwnPropertyDescriptor(ArrayBuffer, Symbol.species);
console.log("species_desc=" + typeof speciesDesc.get + "/" + String(speciesDesc.set) + "/" + speciesDesc.configurable);
console.log("subclass_slice=" + t(() => {
  class Sub extends ArrayBuffer {}
  return new Sub(4).slice(0).constructor.name;
}));
console.log("species_override=" + t(() => {
  class Sub extends ArrayBuffer {
    static get [Symbol.species]() { return ArrayBuffer; }
  }
  return new Sub(4).slice(0).constructor.name;
}));
console.log("species_not_buffer=" + t(() => {
  class Sub extends ArrayBuffer {
    static get [Symbol.species]() { return Array as any; }
  }
  return new Sub(4).slice(0);
}));

// ---- isView: every typed-array kind and DataView, nothing else.
const candidates: [string, any][] = [
  ["Uint8Array", new Uint8Array(1)],
  ["Float64Array", new Float64Array(1)],
  ["BigInt64Array", new BigInt64Array(1)],
  ["Uint8ClampedArray", new Uint8ClampedArray(1)],
  ["subarray", new Uint8Array(2).subarray(1)],
  ["DataView", new DataView(new ArrayBuffer(4))],
  ["ArrayBuffer", new ArrayBuffer(4)],
  ["Array", [1, 2]],
  ["null", null],
  ["undefined", undefined],
  ["number", 5],
  ["lookalike", { byteLength: 4 }],
  ["prototype", Uint8Array.prototype],
];
for (const [label, value] of candidates) {
  console.log("isView_" + label + "=" + ArrayBuffer.isView(value));
}
console.log("isView_detached=" + (function (): string {
  const b = new ArrayBuffer(2);
  const v = new Uint8Array(b);
  b.transfer();
  return String(ArrayBuffer.isView(v));
})());
console.log("isView_no_arg=" + t(() => (ArrayBuffer.isView as any)()) + " isView_arity=" + ArrayBuffer.isView.length);

// ---- Resizable buffers: the constructor options and resize() refusals.
console.log("resize_exists=" + typeof (ArrayBuffer.prototype as any).resize + " transfer=" + typeof ArrayBuffer.prototype.transfer + " toFixed=" + typeof (ArrayBuffer.prototype as any).transferToFixedLength);
console.log("resize_non_resizable=" + t(() => (new ArrayBuffer(4) as any).resize(8)));
console.log("resize_over_max=" + t(() => (new ArrayBuffer(4, { maxByteLength: 8 }) as any).resize(9)));
console.log("resize_negative=" + t(() => (new ArrayBuffer(4, { maxByteLength: 8 }) as any).resize(-1)));
console.log("resize_fraction=" + t(() => { const b: any = new ArrayBuffer(4, { maxByteLength: 8 }); b.resize(5.9); return b.byteLength; }));
console.log("resize_string=" + t(() => { const b: any = new ArrayBuffer(4, { maxByteLength: 8 }); b.resize("6"); return b.byteLength; }));
console.log("resize_undefined=" + t(() => { const b: any = new ArrayBuffer(4, { maxByteLength: 8 }); b.resize(undefined); return b.byteLength; }));
console.log("resize_to_max=" + t(() => { const b: any = new ArrayBuffer(4, { maxByteLength: 8 }); b.resize(8); return b.byteLength; }));
console.log("resize_returns=" + t(() => (new ArrayBuffer(2, { maxByteLength: 4 }) as any).resize(3)));
console.log("grow_zero_fills=" + t(() => {
  const b: any = new ArrayBuffer(2, { maxByteLength: 6 });
  new Uint8Array(b).set([7, 7]);
  b.resize(4);
  return bytesOf(b);
}));
console.log("shrink_then_grow=" + t(() => {
  const b: any = new ArrayBuffer(4, { maxByteLength: 8 });
  new Uint8Array(b).set([1, 2, 3, 4]);
  b.resize(2);
  b.resize(4);
  return bytesOf(b);
}));
console.log("ctor_over_max=" + t(() => new ArrayBuffer(8, { maxByteLength: 4 }).byteLength));
console.log("ctor_max_negative=" + t(() => new ArrayBuffer(0, { maxByteLength: -1 })));
console.log("ctor_max_fraction=" + t(() => new ArrayBuffer(0, { maxByteLength: 4.9 }).maxByteLength));
console.log("ctor_max_undefined=" + t(() => { const b = new ArrayBuffer(4, { maxByteLength: undefined } as any); return b.resizable + "/" + b.maxByteLength; }));
console.log("ctor_options_ignored=" + t(() => { const b = new ArrayBuffer(4, { other: 9 } as any); return b.resizable + "/" + b.maxByteLength; }));
console.log("ctor_length_negative=" + t(() => new ArrayBuffer(-1)));
console.log("ctor_length_string=" + t(() => new ArrayBuffer("4" as any).byteLength));
console.log("ctor_no_new=" + t(() => (ArrayBuffer as any)(4)));
console.log("max_equal=" + new ArrayBuffer(4, { maxByteLength: 4 }).resizable + " max_zero=" + new ArrayBuffer(0, { maxByteLength: 0 }).resizable);
console.log("max_below_length=" + t(() => new ArrayBuffer(8, { maxByteLength: 4 })));

// ---- Length-tracking views follow the buffer; fixed-length views go out of bounds.
const rb = new ArrayBuffer(4, { maxByteLength: 16 });
console.log("resizable=" + rb.resizable + " max=" + rb.maxByteLength + " len=" + rb.byteLength + " plain=" + new ArrayBuffer(4).resizable);
const tracking = new Uint8Array(rb);
const fixed = new Uint8Array(rb, 0, 4);
const trackingFromOffset = new Uint8Array(rb, 2);
tracking.set([1, 2, 3, 4]);
console.log("start_lengths=" + tracking.length + "," + fixed.length + "," + trackingFromOffset.length);
rb.resize(8);
console.log("grown=" + rb.byteLength + " tracking=" + tracking.length + " [" + bytesOf(rb) + "]");
console.log("grown_fixed=" + fixed.length + " grown_offset=" + trackingFromOffset.length + " [" + Array.from(trackingFromOffset).join(",") + "]");
tracking[7] = 9;
rb.resize(4);
console.log("shrunk=" + rb.byteLength + " tracking=" + tracking.length + " [" + bytesOf(rb) + "]");
rb.resize(8);
console.log("regrown_zeroed=" + bytesOf(rb));

const far = new Uint8Array(rb, 4, 4);
console.log("far_before=" + far.length + " read=" + String(far[0]));
rb.resize(4);
console.log("far_after=" + far.length + " read=" + String(far[0]) + " byteOffset=" + far.byteOffset + " byteLength=" + far.byteLength);
console.log("far_set=" + t(() => far.set([1])) + " far_slice=" + t(() => far.slice(0).length));
rb.resize(8);
console.log("far_restored=" + far.length + " read=" + String(far[0]));

const dvTrack = new DataView(rb);
const dvFixed = new DataView(rb, 0, 4);
console.log("dv_lengths=" + dvTrack.byteLength + "," + dvFixed.byteLength);
rb.resize(6);
console.log("dv_after=" + dvTrack.byteLength + "," + dvFixed.byteLength);
rb.resize(0);
console.log("zero=" + rb.byteLength + " tracking=" + tracking.length);
rb.resize(2);
console.log("back=" + rb.byteLength + " tracking=" + tracking.length + " [" + bytesOf(rb) + "]");
console.log("over_max=" + t(() => rb.resize(17)) + " negative=" + t(() => rb.resize(-1)) + " after_failed=" + rb.byteLength);
console.log("slice_resizable=" + (function (): string {
  const c = rb.slice(0, 2);
  return c.resizable + " len=" + c.byteLength;
})());
console.log("slice_of_resizable_fixed=" + (function (): string {
  const c = new ArrayBuffer(4, { maxByteLength: 8 }).slice(0, 2);
  return c.byteLength + "/" + c.resizable + "/" + c.maxByteLength;
})());

// ---- transfer(): moves the bytes, detaches the source, and the new length decides
// truncation or zero-extension. Resizability survives transfer() but not
// transferToFixedLength().
const src = new ArrayBuffer(4);
const view = new Uint8Array(src);
const dv = new DataView(src);
view.set([1, 2, 3, 4]);
console.log("before_transfer=" + src.detached + " len=" + src.byteLength);
const moved = src.transfer();
console.log("after_transfer=" + src.detached + " src_len=" + src.byteLength + " moved=" + moved.byteLength + " [" + bytesOf(moved) + "] moved_detached=" + moved.detached);
console.log("view_after=" + view.constructor.name + " len=" + view.length + " bytelen=" + view.byteLength + " byteoffset=" + view.byteOffset + " same_buffer=" + (view.buffer === src));
console.log("view_read=" + String(view[0]) + " view_set=" + Reflect.set(view, "0", 9) + " write_then_read=" + String(view[0]) + " in=" + ("0" in view) + " keys=" + JSON.stringify(Object.keys(view)));
for (const name of ["set", "slice", "subarray", "fill", "sort", "indexOf", "join", "iterate", "at", "copyWithin", "toSorted"]) {
  let outcome = "no-throw";
  try {
    if (name === "set") view.set([1]);
    else if (name === "slice") view.slice(0);
    else if (name === "subarray") outcome = "len:" + view.subarray(0).length;
    else if (name === "fill") view.fill(0);
    else if (name === "sort") view.sort();
    else if (name === "indexOf") outcome = "idx:" + view.indexOf(1);
    else if (name === "join") outcome = "join:" + JSON.stringify(view.join(","));
    else if (name === "iterate") outcome = "iter:" + Array.from(view).length;
    else if (name === "at") outcome = "at:" + String(view.at(0));
    else if (name === "copyWithin") view.copyWithin(0, 1);
    else if (name === "toSorted") view.toSorted();
  } catch (e: any) {
    outcome = e.constructor.name;
  }
  console.log("detached_" + name + "=" + outcome);
}
console.log("dv_read=" + t(() => dv.getUint8(0)) + " dv_byteLength=" + t(() => dv.byteLength));
console.log("new_view=" + t(() => new Uint8Array(src)) + " detached_slice=" + t(() => src.slice(0)) + " transfer_again=" + t(() => src.transfer()));
console.log("resize_after_transfer=" + t(() => { const b: any = new ArrayBuffer(4, { maxByteLength: 8 }); b.transfer(); return b.resize(6); }));
console.log("detached_props=" + (function (): string {
  const b = new ArrayBuffer(4);
  b.transfer();
  return b.byteLength + "/" + b.detached + "/" + b.resizable + "/" + b.maxByteLength;
})());

const grow = new ArrayBuffer(2);
new Uint8Array(grow).set([7, 8]);
const grown = grow.transfer(4);
console.log("transfer_grow=" + grown.byteLength + " [" + bytesOf(grown) + "]");
const shrunk = grown.transfer(1);
console.log("transfer_shrink=" + shrunk.byteLength + " [" + bytesOf(shrunk) + "] zero=" + shrunk.transfer(0).byteLength);
console.log("transfer_negative=" + t(() => new ArrayBuffer(4).transfer(-1)) + " transfer_fraction=" + new ArrayBuffer(4).transfer(2.9).byteLength);
console.log("transfer_same_bytes=" + (function (): string {
  const b = new ArrayBuffer(4);
  new Uint8Array(b).set([1, 2, 3, 4]);
  const c = b.transfer();
  return c.byteLength + "/" + bytesOf(c) + "/" + b.detached;
})());
console.log("transfer_twice=" + t(() => { const b = new ArrayBuffer(4); b.transfer(); return b.transfer(); }));
console.log("transfer_wrong_receiver=" + t(() => (ArrayBuffer.prototype.transfer as any).call({})) + " " + t(() => (ArrayBuffer.prototype.transfer as any).call(new Uint8Array(4))));

const rkeep = new ArrayBuffer(2, { maxByteLength: 8 });
const kept = rkeep.transfer();
console.log("transfer_keeps_resizable=" + kept.resizable + "/" + kept.maxByteLength + "/" + kept.byteLength);
const rfixed = new ArrayBuffer(2, { maxByteLength: 8 });
const fixedOut = rfixed.transferToFixedLength();
console.log("to_fixed=" + fixedOut.resizable + "/" + fixedOut.byteLength + "/" + fixedOut.maxByteLength + "/" + rfixed.detached);
console.log("to_fixed_length_arg=" + (new ArrayBuffer(4) as any).transferToFixedLength(6).byteLength);
console.log("transfer_of_plain=" + new ArrayBuffer(4).transfer().resizable);

// ---- structuredClone with a transfer list detaches the source the same way.
const cloneSrc = new ArrayBuffer(3);
new Uint8Array(cloneSrc).set([4, 5, 6]);
const clone = structuredClone(cloneSrc, { transfer: [cloneSrc] });
console.log("clone_transfer=" + cloneSrc.detached + " clone=" + bytesOf(clone) + " clone_len=" + clone.byteLength);
