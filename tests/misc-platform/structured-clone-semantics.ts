// Cross-runtime: structuredClone end to end. Sections: primitives and
// independence, containers, graph identity and cycles, binary data and the
// transfer list, boxed values, errors, the object shape it rebuilds, and the
// values it refuses. Failures are reported by constructor name, never by stack.

function t(f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name + "/" + e.name;
  }
}

// ---------------------------------------------------------------------------
// 1. Primitives and independence of the copy.
// ---------------------------------------------------------------------------
console.log("prim_number=" + structuredClone(42));
console.log("prim_string=" + structuredClone("hello"));
console.log("prim_bool=" + structuredClone(true));
console.log("prim_null=" + String(structuredClone(null)));
console.log("prim_undefined=" + String(structuredClone(undefined)));
console.log("negzero=" + Object.is(structuredClone(-0), -0));
console.log("nan=" + Number.isNaN(structuredClone(NaN)));
console.log("infinity=" + structuredClone(Infinity) + "," + structuredClone(-Infinity));
console.log("bigint=" + structuredClone(2n ** 70n) + " typeof=" + typeof structuredClone(1n));
console.log("big_string=" + structuredClone("é\u{1F600}").length);
console.log("primitives_joined=" + [structuredClone(0), structuredClone(""), String(structuredClone(null)), structuredClone(false)].join("|"));

const obj: any = { a: 1, b: { c: 2 } };
const cloned: any = structuredClone(obj);
cloned.b.c = 99;
console.log("orig_nested=" + obj.b.c + " clone_nested=" + cloned.b.c);

const arr: any[] = [1, [2, 3]];
const clonedArr: any = structuredClone(arr);
clonedArr[1][0] = 99;
console.log("orig_arr=" + arr[1][0] + " clone_arr=" + clonedArr[1][0]);

const src: any = { list: [1, 2], nested: { x: 1 } };
const dup: any = structuredClone(src);
dup.list.push(3);
dup.nested.x = 5;
console.log("independent_list=" + src.list.join(",") + "|" + dup.list.join(","));
console.log("independent_nested=" + src.nested.x + "|" + dup.nested.x);

// ---------------------------------------------------------------------------
// 2. Containers: Map, Set, Date, RegExp.
// ---------------------------------------------------------------------------
const original: any = {
  id: 7,
  nested: { ok: true, list: [1, 2, 3] },
  map: new Map([["a", 1], ["b", 2]]),
  set: new Set(["x", "y"]),
};
const copy: any = structuredClone(original);
copy.nested.list.push(4);
copy.map.set("c", 3);
copy.set.add("z");
console.log("orig_list=" + original.nested.list.join(","));
console.log("copy_list=" + copy.nested.list.join(","));
console.log("orig_map=" + Array.from(original.map.entries()).map(function (e: any) { return e[0] + ":" + e[1]; }).join(","));
console.log("copy_map=" + Array.from(copy.map.entries()).map(function (e: any) { return e[0] + ":" + e[1]; }).join(","));
console.log("orig_set=" + Array.from(original.set.values()).join(","));
console.log("copy_set=" + Array.from(copy.set.values()).join(","));

const nestedMap: any = structuredClone(new Map([["a", 1], ["b", new Map([["c", 3]])]]));
nestedMap.get("b").set("c", 99);
console.log("nested_map_clone=" + nestedMap.get("b").get("c"));

const innerSet = new Set([4, 5]);
const setOrig = new Set([1, 2, 3, innerSet]);
const setClone: any = structuredClone(setOrig);
const innerClone: any = [...setClone].filter(function (v: any) { return v instanceof Set; })[0];
innerClone.add(99);
console.log("set_inner_orig_has99=" + innerSet.has(99) + " clone_has99=" + innerClone.has(99));
console.log("set_size=" + setClone.size + " inner_is_copy=" + (innerClone !== innerSet));

const d = new Date("2026-01-01T00:00:00Z");
const dc = structuredClone(d);
dc.setUTCFullYear(2099);
console.log("date_orig=" + d.getUTCFullYear() + " date_clone=" + dc.getUTCFullYear() + " instanceof=" + (dc instanceof Date));
console.log("date_iso=" + structuredClone(new Date(Date.UTC(2024, 4, 10))).toISOString());
const invalidDate: any = structuredClone(new Date(NaN));
console.log("invalid_date=" + (invalidDate instanceof Date) + " " + Number.isNaN(invalidDate.getTime()));

const re = /ab+c/giu;
re.lastIndex = 3;
const rc: any = structuredClone(re);
console.log("regexp=" + rc.source + "/" + rc.flags + " lastIndex=" + rc.lastIndex + " same=" + (rc === re));
console.log("regexp_plain=" + structuredClone(/hello/gi).source + ":" + structuredClone(/hello/gi).flags);

// ---------------------------------------------------------------------------
// 3. Graph identity: shared references stay shared, cycles survive, and the
//    clone is never the source.
// ---------------------------------------------------------------------------
const shared = { id: 1 };
const graph: any = { a: shared, b: shared, list: [shared] };
graph.self = graph;
const gc: any = structuredClone(graph);
console.log("shared_preserved=" + (gc.a === gc.b) + " in_list=" + (gc.list[0] === gc.a));
console.log("not_same_as_source=" + (gc.a !== shared) + " root_copy=" + (gc !== graph));
console.log("cycle=" + (gc.self === gc) + " depth=" + gc.self.self.a.id);

const circ: any = { a: 1 };
circ.self = circ;
const cc: any = structuredClone(circ);
console.log("circ=" + cc.a + " " + (cc.self === cc) + " " + (cc === circ));

const key = { id: 1 };
const keyedMap: any = new Map<any, any>([[key, "v"], ["s", key]]);
const kmc: any = structuredClone(keyedMap);
const clonedKey = [...kmc.keys()][0];
console.log("map_key_identity=" + (kmc.get(clonedKey) === "v") + " value_is_same_key=" + (kmc.get("s") === clonedKey));
console.log("map_key_is_copy=" + (clonedKey !== key) + " size=" + kmc.size);

const ordered: any = structuredClone(new Map([["b", 1], ["a", 2], ["c", 3]]));
console.log("map_order=" + Array.from(ordered.keys()).join(","));
const setShared = { v: 1 };
const twoOfShared: any = structuredClone(new Set([setShared, setShared, { v: 1 }]));
console.log("set_identity=" + twoOfShared.size);
const nestedSetWithDate: any = structuredClone(new Map([["inner", new Set([new Date(0)])]]));
console.log("nested_set_date=" + nestedSetWithDate.get("inner").size + " " + [...nestedSetWithDate.get("inner")][0].toISOString());

const mixed: any = structuredClone({ d: new Date(0), r: /ab+c/gi, m: new Map([["k", { v: 1 }]]), s: new Set([1, 2, 2]) });
console.log("mixed_date=" + (mixed.d instanceof Date) + " " + mixed.d.toISOString());
console.log("mixed_regexp=" + (mixed.r instanceof RegExp) + " " + mixed.r.source + " " + mixed.r.flags);
console.log("mixed_map=" + (mixed.m instanceof Map) + " size=" + mixed.m.size + " deep=" + (mixed.m.get("k") !== undefined));
console.log("mixed_set=" + (mixed.s instanceof Set) + " size=" + mixed.s.size);

// ---------------------------------------------------------------------------
// 4. Binary data: buffers, views, aliasing, element kinds, DataView, resizable.
// ---------------------------------------------------------------------------
const bufSrc = new ArrayBuffer(8);
const bytes = new Uint8Array(bufSrc);
bytes.set([1, 2, 3, 4, 5, 6, 7, 8]);
const clonedBuffer: any = structuredClone(bufSrc);
console.log("buffer=" + clonedBuffer.byteLength + " same=" + (clonedBuffer === bufSrc) + " instanceof=" + (clonedBuffer instanceof ArrayBuffer));
console.log("buffer_bytes=" + Array.from(new Uint8Array(clonedBuffer)).join(","));
new Uint8Array(clonedBuffer)[0] = 99;
console.log("buffer_independent=" + bytes[0] + " src_alive=" + (bufSrc.byteLength === 8));

const view = new Int16Array(bufSrc, 2, 2);
const clonedView: any = structuredClone(view);
console.log("view_kind=" + clonedView.constructor.name + " len=" + clonedView.length + " byteOffset=" + clonedView.byteOffset + " bufferLen=" + clonedView.buffer.byteLength);

// Two views over ONE buffer keep their aliasing after the clone.
const aliasA = new Uint8Array(bufSrc, 0, 4);
const aliasB = new Uint8Array(bufSrc, 2, 4);
const pair: any = structuredClone({ a: aliasA, b: aliasB });
console.log("alias_same_buffer=" + (pair.a.buffer === pair.b.buffer) + " offsets=" + pair.a.byteOffset + "," + pair.b.byteOffset);
pair.a[2] = 77;
console.log("alias_visible=" + pair.b[0]);

const kinds: any[] = [
  new Int8Array([-1]),
  new Uint8Array([255]),
  new Uint8ClampedArray([300]),
  new Int16Array([-2]),
  new Uint16Array([65535]),
  new Int32Array([-3]),
  new Uint32Array([4294967295]),
  new Float32Array([1.5]),
  new Float64Array([-0]),
  new BigInt64Array([-1n]),
  new BigUint64Array([1n]),
];
for (const k of kinds) {
  const c: any = structuredClone(k);
  console.log("kind_" + k.constructor.name + "=" + c.constructor.name + ":" + Array.from(c).join(","));
}
console.log("float64_negzero=" + Object.is(structuredClone(new Float64Array([-0]))[0], -0));

const dv = new DataView(new ArrayBuffer(4), 1, 2);
const clonedDv: any = structuredClone(dv);
console.log("dataview=" + clonedDv.constructor.name + " off=" + clonedDv.byteOffset + " len=" + clonedDv.byteLength + " bufferLen=" + clonedDv.buffer.byteLength);

const rb = new ArrayBuffer(2, { maxByteLength: 8 });
const clonedRb: any = structuredClone(rb);
console.log("resizable=" + clonedRb.resizable + " max=" + clonedRb.maxByteLength + " len=" + clonedRb.byteLength);

console.log("clone_without_transfer_copies=" + (function (): string {
  const buf = new ArrayBuffer(2);
  new Uint8Array(buf)[0] = 3;
  const out: any = structuredClone({ buf });
  new Uint8Array(out.buf)[0] = 9;
  return new Uint8Array(buf)[0] + "/" + new Uint8Array(out.buf)[0];
})());

const ta = new Uint16Array([1, 2, 3]);
const ct: any = structuredClone(ta);
console.log("typed=" + (ct instanceof Uint16Array) + " v=" + Array.from(ct).join(",") + " src_len=" + ta.length);
const abSrc = new Uint8Array([9, 8]).buffer;
const cab: any = structuredClone(abSrc);
console.log("buffer_plain=" + (cab instanceof ArrayBuffer) + " len=" + cab.byteLength + " srcAlive=" + (abSrc.byteLength === 2));

// ---------------------------------------------------------------------------
// 5. The transfer list: what moves, what detaches, what is refused.
// ---------------------------------------------------------------------------
console.log("transfer_basic=" + t(function () {
  const buf = new ArrayBuffer(4);
  new Uint8Array(buf).set([1, 2, 3, 4]);
  const out: any = structuredClone({ buf }, { transfer: [buf] });
  return out.buf.byteLength + "/" + Array.from(new Uint8Array(out.buf)).join(",") + "/" + buf.detached + "/" + buf.byteLength;
}));
console.log("transfer_listed_twice=" + t(function () {
  const buf = new ArrayBuffer(4);
  return structuredClone({ buf }, { transfer: [buf, buf] });
}));
console.log("transfer_listed_twice_source=" + t(function () {
  const buf = new ArrayBuffer(4);
  try {
    structuredClone({ buf }, { transfer: [buf, buf] });
  } catch (e: any) {
    return e.name + "/detached:" + buf.detached;
  }
  return "no-throw";
}));
console.log("transfer_not_in_graph=" + t(function () {
  const buf = new ArrayBuffer(4);
  const out: any = structuredClone({ x: 1 }, { transfer: [buf] });
  return out.x + "/" + buf.detached + "/" + ("buf" in out);
}));
console.log("transfer_value_and_transfer=" + t(function () {
  const buf = new ArrayBuffer(4);
  new Uint8Array(buf)[0] = 7;
  const out: any = structuredClone({ a: buf, b: buf }, { transfer: [buf] });
  return (out.a === out.b) + "/" + new Uint8Array(out.a)[0] + "/" + buf.detached;
}));
console.log("transfer_view_itself=" + t(function () {
  const view = new Uint8Array(4);
  return structuredClone({ view }, { transfer: [view as any] });
}));
console.log("transfer_buffer_of_view=" + t(function () {
  const view = new Uint8Array([1, 2, 3, 4]);
  const out: any = structuredClone({ view }, { transfer: [view.buffer] });
  return Array.from(out.view).join(",") + "/" + view.length + "/" + view.buffer.detached;
}));
console.log("transfer_plain_object=" + t(function () { return structuredClone({ o: 1 }, { transfer: [{} as any] }); }));
console.log("transfer_array=" + t(function () { return structuredClone({ o: 1 }, { transfer: [[1, 2] as any] }); }));
console.log("transfer_function=" + t(function () { return structuredClone({ o: 1 }, { transfer: [(function () { return; }) as any] }); }));
console.log("transfer_map=" + t(function () { return structuredClone({ o: 1 }, { transfer: [new Map() as any] }); }));
console.log("transfer_already_detached=" + t(function () {
  const buf = new ArrayBuffer(4);
  buf.transfer();
  return structuredClone({ buf }, { transfer: [buf] });
}));
console.log("transfer_empty_list=" + t(function () {
  const buf = new ArrayBuffer(2);
  const out: any = structuredClone({ buf }, { transfer: [] });
  return out.buf.byteLength + "/" + buf.detached + "/" + (out.buf === buf);
}));
console.log("options_absent=" + t(function () {
  const buf = new ArrayBuffer(2);
  const out: any = structuredClone({ buf });
  return out.buf.byteLength + "/" + buf.detached;
}));
console.log("options_empty=" + t(function () { return structuredClone({ v: 1 }, {} as any).v; }));
console.log("options_undefined_transfer=" + t(function () { return structuredClone({ v: 1 }, { transfer: undefined } as any).v; }));
console.log("two_buffers_one_moved=" + t(function () {
  const a = new ArrayBuffer(2);
  const b = new ArrayBuffer(4);
  const out: any = structuredClone({ a, b }, { transfer: [a] });
  return out.a.byteLength + "," + out.b.byteLength + "/" + a.detached + "," + b.detached;
}));
console.log("transfer_resizable=" + t(function () {
  const buf = new ArrayBuffer(4, { maxByteLength: 8 });
  const out: any = structuredClone({ buf }, { transfer: [buf] });
  return out.buf.resizable + "/" + out.buf.maxByteLength + "/" + buf.detached;
}));
console.log("error_is_domexception=" + t(function () {
  const buf = new ArrayBuffer(2);
  try {
    structuredClone({ buf }, { transfer: [{} as any] });
    return "no-throw";
  } catch (e: any) {
    return (e instanceof DOMException) + "/" + (e instanceof Error) + "/" + e.name + "/" + e.code;
  }
}));

// ---------------------------------------------------------------------------
// 6. Boxed primitives come back boxed.
// ---------------------------------------------------------------------------
const boxedNumber: any = structuredClone(new Number(5) as any);
const boxedString: any = structuredClone(new String("s") as any);
const boxedBoolean: any = structuredClone(new Boolean(true) as any);
console.log("boxed_number=" + typeof boxedNumber + " " + boxedNumber.valueOf() + " " + (boxedNumber instanceof Number));
console.log("boxed_string=" + typeof boxedString + " " + boxedString.valueOf() + " len=" + boxedString.length);
console.log("boxed_boolean=" + typeof boxedBoolean + " " + boxedBoolean.valueOf());
console.log("boxed_tags=" + Object.prototype.toString.call(boxedNumber) + Object.prototype.toString.call(boxedString));
console.log("boxed_flattening=" + (function (): string {
  const b: any = new Number(5);
  b.extra = 1;
  const c: any = structuredClone(b);
  return typeof c + "/" + (c instanceof Number) + "/" + c.valueOf() + "/" + String(c.extra);
})());

// ---------------------------------------------------------------------------
// 7. Errors: the ES error types keep constructor, name and message; a subclass
//    or AggregateError flattens to a plain Error, extra own props are not carried.
// ---------------------------------------------------------------------------
const errors: Error[] = [
  new Error("e"),
  new TypeError("t"),
  new RangeError("r"),
  new SyntaxError("s"),
  new ReferenceError("f"),
  new EvalError("v"),
  new URIError("u"),
];
for (const e of errors) {
  const c: any = structuredClone(e);
  console.log("error_" + e.constructor.name + "=" + c.constructor.name + "/" + c.name + "/" + c.message);
}
const agg: any = structuredClone(new AggregateError([new Error("a"), new RangeError("b")], "many"));
console.log("aggregate=" + agg.constructor.name + " msg=" + agg.message + " name=" + agg.name);
class MyError extends Error {}
const custom: any = structuredClone(new MyError("custom"));
console.log("subclass_flattened=" + custom.constructor.name + " msg=" + custom.message);
const extraErr: any = new Error("x");
extraErr.code = 42;
console.log("error_extra_prop=" + String(structuredClone(extraErr).code));
const boomErr: any = structuredClone(new RangeError("boom"));
console.log("error_single=" + (boomErr instanceof RangeError) + " name=" + boomErr.name + " msg=" + boomErr.message);

// ---------------------------------------------------------------------------
// 8. The shape it rebuilds: own ENUMERABLE string keys only. Getters are called
//    once and flattened to data; non-enumerable props, symbols and the prototype
//    are dropped.
// ---------------------------------------------------------------------------
const shapeSrc: any = { plain: 1 };
Object.defineProperty(shapeSrc, "getter", { get: function () { return 42; }, enumerable: true, configurable: true });
Object.defineProperty(shapeSrc, "hiddenData", { value: 9, enumerable: false, configurable: true });
Object.defineProperty(shapeSrc, "hiddenGetter", { get: function () { return 8; }, enumerable: false, configurable: true });
Object.defineProperty(shapeSrc, "readOnly", { value: 3, writable: false, enumerable: true, configurable: false });
shapeSrc[Symbol("sym")] = "s";
shapeSrc[Symbol.iterator] = function () { return; };

const shapeClone: any = structuredClone(shapeSrc);
console.log("values=" + shapeClone.plain + "," + shapeClone.getter + "," + shapeClone.readOnly);
console.log("getter_flattened=" + JSON.stringify(Object.getOwnPropertyDescriptor(shapeClone, "getter")));
console.log("readonly_flattened=" + JSON.stringify(Object.getOwnPropertyDescriptor(shapeClone, "readOnly")));
console.log("non_enumerable_dropped=" + ("hiddenData" in shapeClone) + "," + ("hiddenGetter" in shapeClone));
console.log("symbols_dropped=" + Object.getOwnPropertySymbols(shapeClone).length + " well_known=" + (Symbol.iterator in shapeClone));
console.log("keys=" + Object.keys(shapeClone).sort().join(","));
console.log("proto_is_object=" + (Object.getPrototypeOf(shapeClone) === Object.prototype));

console.log("getter_call_count=" + (function (): string {
  let calls = 0;
  const o: any = { get v() { calls++; return 1; } };
  const c: any = structuredClone(o);
  const read = c.v + c.v;
  return calls + "/" + read;
})());
console.log("throwing_getter=" + t(function () {
  return structuredClone({ get v() { throw new RangeError("no"); } });
}));
console.log("getter_reads_source=" + (function (): string {
  const o: any = { n: 5, get double() { return this.n * 2; } };
  const c: any = structuredClone(o);
  c.n = 100;
  return c.double + "";
})());
console.log("accessor_only_object=" + (function (): string {
  const o = { get a() { return 1; }, set a(_v: number) { return; } };
  const c: any = structuredClone(o);
  const desc: any = Object.getOwnPropertyDescriptor(c, "a");
  return c.a + "/" + ("value" in desc) + "/" + String(desc.get);
})());
console.log("nested_getter_shape=" + (function (): string {
  const o: any = { inner: { get g() { return 7; } } };
  const c: any = structuredClone(o);
  const desc: any = Object.getOwnPropertyDescriptor(c.inner, "g");
  return c.inner.g + "/" + desc.writable + desc.enumerable + desc.configurable;
})());

class Point {
  x = 1;
  y = 2;
  sum() { return this.x + this.y; }
}
console.log("class_instance=" + t(function () {
  const c: any = structuredClone(new Point());
  return c.constructor.name + "/" + c.x + "," + c.y + "/" + typeof c.sum + "/" + (c instanceof Point);
}));
console.log("null_prototype_source=" + t(function () {
  const o: any = Object.create(null);
  o.k = 1;
  const c: any = structuredClone(o);
  return c.k + "/" + (Object.getPrototypeOf(c) === Object.prototype);
}));
console.log("array_extra_props=" + t(function () {
  const a: any = [1, 2];
  a.tag = "x";
  const c: any = structuredClone(a);
  return Array.isArray(c) + "/" + c.length + "/" + c.tag + "/" + JSON.stringify(c);
}));
console.log("sparse_array=" + t(function () {
  const c: any = structuredClone([1, , 3]);
  return c.length + "/" + (1 in c) + "/" + String(c[1]) + "/" + JSON.stringify(c);
}));
console.log("array_prototype=" + t(function () {
  const c: any = structuredClone([1]);
  return (Object.getPrototypeOf(c) === Array.prototype) + "/" + (c instanceof Array);
}));
console.log("frozen_source=" + t(function () {
  const c: any = structuredClone(Object.freeze({ a: 1 }));
  return Object.isFrozen(c) + "/" + Object.isExtensible(c);
}));

// ---------------------------------------------------------------------------
// 9. Values it refuses: DataCloneError from the platform, catchable by name.
// ---------------------------------------------------------------------------
function refuse(label: string, v: any): void {
  try {
    structuredClone(v);
    console.log(label + "=cloned");
  } catch (e: any) {
    console.log(label + "=" + e.constructor.name + "/" + e.name);
  }
}
refuse("refuse_function", () => 1);
refuse("refuse_symbol", Symbol("x"));
refuse("refuse_nested_function", { f() {} });
refuse("refuse_weakmap", new WeakMap());
refuse("refuse_proxy", new Proxy({ a: 1 }, {}));
console.log("cloneable_domexception=" + t(function () {
  const c: any = structuredClone(new DOMException("m", "AbortError"));
  return c.name + "/" + c.message + "/" + (c instanceof DOMException);
}));

// ---------------------------------------------------------------------------
// 10. Shared identity across Map keys, Map values and Set members at once.
// ---------------------------------------------------------------------------
const hub: any = { value: 3 };
const root: any = {
  first: hub,
  second: hub,
  map: new Map<any, any>([[hub, { ref: hub }]]),
  set: new Set<any>([hub]),
};
root.self = root;
const hubClone: any = structuredClone(root);
const hubKey = [...hubClone.map.keys()][0];
console.log("hub_root_and_self=" + (hubClone !== root) + " " + (hubClone.self === hubClone));
console.log("hub_first_second=" + (hubClone.first === hubClone.second) + " key_is_first=" + (hubKey === hubClone.first));
console.log("hub_map_value_ref=" + (hubClone.map.get(hubKey).ref === hubClone.first) + " set_has=" + hubClone.set.has(hubClone.first));
