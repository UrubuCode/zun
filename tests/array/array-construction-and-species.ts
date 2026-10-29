// Array construction: the constructor, Array.of, Array.from (iterables,
// array-likes, mapper, thisArg, generic constructors), iterator closing, and
// how Symbol.species decides the constructor of derived arrays.

// Array constructor: one numeric argument is a length, anything else is an element.
const c5 = new Array(5);
console.log("ctorLen=" + c5.length + " ctorHole=" + (0 in c5));
console.log("ctorValues=" + new Array(1, 2, 3).join(",") + " ctorEmpty=" + new Array().length);
console.log("ctorCall=" + Array(3).length + " ctorCallValues=" + Array(1, 2).join(","));
console.log("ctorStr=" + JSON.stringify(Array("3")) + " ctorBool=" + JSON.stringify(Array(true)));
console.log("ctorUndef=" + JSON.stringify(Array(undefined)) + " ctorNull=" + JSON.stringify(Array(null)));
console.log("ctorObj=" + Array({ valueOf: () => 3 } as any).length);
console.log("ctorTwoNumbers=" + JSON.stringify(Array(3, 4)) + " ctorZero=" + Array().length);
for (const v of [-1, 1.5, NaN, Infinity, 4294967296]) {
  try { console.log("ctorRange(" + String(v) + ")=" + new Array(v as any).length); }
  catch (e: any) { console.log("ctorRange(" + String(v) + ")=" + e.name); }
}
const holes = new Array(3);
console.log("ctorHoles=" + [0, 1, 2].map((i) => (i in holes ? "y" : "n")).join("") + " ctorForEach=" + (() => { let n = 0; holes.forEach(() => n++); return n; })());
console.log("ctorMapKeepsHoles=" + JSON.stringify(holes.map(() => 1)) + " ctorFill=" + JSON.stringify(new Array(2).fill(0)));

// Array.of never treats a single numeric argument as a length.
console.log("ofLiteral=" + JSON.stringify([3]) + " of=" + JSON.stringify(Array.of(3)) + " ofLen=" + Array.of(3).length);
console.log("ofMixed=" + Array.of(1, "a", true).join(",") + " ofEmpty=" + Array.of().length + " ofUndef=" + Array.of(undefined).length);
console.log("ofUndefIn=" + (0 in Array.of(undefined)) + " ofTwo=" + Array.of(1, 2).join(","));
for (const v of [-1, 1.5, NaN, Infinity]) {
  try { console.log("ofRange(" + String(v) + ")=" + Array.of(v as any).length); }
  catch (e: any) { console.log("ofRange(" + String(v) + ")=" + e.name); }
}
// Array.of and Array.from are generic over `this`.
function Box(this: any, n?: number) { this.made = true; this.length = n; }
const boxed: any = (Array.of as any).call(Box, "x", "y");
console.log("ofGeneric=" + boxed.made + " len=" + boxed.length + " v=" + boxed[0] + boxed[1]);
const notCtor: any = (Array.of as any).call(undefined, 1, 2);
console.log("ofNonCtor=" + Array.isArray(notCtor) + " len=" + notCtor.length);
console.log("protoIsArray=" + Array.isArray(Array.prototype) + " protoLen=" + Array.prototype.length);
console.log("ctorMeta=" + Array.length + "," + Array.of.length + "," + Array.from.length + "," + Array.isArray.length + " name=" + Array.name);

// Array.from: iterable path, array-like path, mapper, thisArg.
console.log("fromString=" + Array.from("hello").join(",") + " upper=" + Array.from("hello", (c) => c.toUpperCase()).join(""));
console.log("fromMap=" + Array.from([1, 2, 3], (n: number) => n * 2).join(",") + " fromIdx=" + Array.from([5, 6, 7], (x, i) => x + i).join(","));
console.log("fromRange=" + Array.from({ length: 5 }, (_, i) => i).join(",") + " squares=" + Array.from({ length: 4 }, (_, i) => i * i).join(","));
console.log("fromLenOnly=" + Array.from({ length: 4 }).length + " fromSet=" + Array.from(new Set([1, 1, 2, 3, 3])).join(","));
console.log("fromSetMapped=" + Array.from(new Set([1, 2, 3]), (x: number) => x * 2).join(","));
console.log("fromMapEntries=" + Array.from(new Map([[1, "a"]])).map((p) => p[0] + ":" + p[1]).join(","));
const factor = { factor: 3 };
console.log("fromThisArg=" + Array.from([1, 2, 3], function (this: any, x: number) { return x * this.factor; }, factor).join(","));
const mapArity: string[] = [];
Array.from([7, 8], function (...args: any[]) { mapArity.push(String(args.length)); return 0; });
console.log("mapperArity=" + mapArity.join(","));
const probeOrder: string[] = [];
const badProbe: any = { [Symbol.iterator]() { probeOrder.push("pulled"); return [][Symbol.iterator](); } };
try { (Array.from as any)(badProbe, 1); } catch (e: any) { console.log("badMapper=" + e.name); }
console.log("pulledAfterBadMapper=" + probeOrder.includes("pulled"));
function Ctor(this: any, n?: number) { this.marker = "C"; this.length = n; }
const made: any = (Array.from as any).call(Ctor, [1, 2]);
console.log("fromGenericCtor=" + made.marker + " len=" + made.length + " v=" + made[0] + made[1]);

// Array.from on array-likes that have no iterator: length drives the walk.
const al = { length: 3, 0: "a", 1: "b", 2: "c" };
console.log("alBasic=" + Array.from(al as any).join(","));
const gappy = Array.from({ length: 4, 0: "x", 2: "z" } as any);
console.log("alGappy=" + gappy.length + ":" + gappy.map(String).join(",") + " hasIdx1=" + (1 in gappy));
console.log("alOverLen=" + Array.from({ length: 2 } as any).map(String).join(","));
console.log("alZero=" + Array.from({ length: 0, 0: "ignored" } as any).length + " alNoLen=" + Array.from({ 0: "a", 1: "b" } as any).length);
console.log("alStrLen=" + Array.from({ length: "3", 0: 1, 1: 2, 2: 3 } as any).join(","));
console.log("alNegLen=" + Array.from({ length: -5, 0: "a" } as any).length + " alFracLen=" + Array.from({ length: 2.9, 0: "a", 1: "b", 2: "c" } as any).join(","));
console.log("alNanLen=" + Array.from({ length: NaN, 0: "a" } as any).length + " alUndefLen=" + Array.from({ length: undefined, 0: "a" } as any).length);
console.log("alMapFn=" + Array.from({ length: 3, 0: 10, 1: 20, 2: 30 } as any, (v: any, i: number) => v + i).join(","));
console.log("alStringKeys=" + Array.from({ length: 2, "0": "s0", "1": "s1" } as any).join(","));
const protoChild: any = Object.create({ 1: "fromProto" });
protoChild.length = 2;
protoChild[0] = "own";
console.log("alInheritedIdx=" + Array.from(protoChild).join(","));
let getterHits = 0;
const withGetter: any = { length: 2, 0: "plain" };
Object.defineProperty(withGetter, "1", { get() { getterHits++; return "got"; }, enumerable: true });
console.log("alGetter=" + Array.from(withGetter).join(",") + " hits=" + getterHits);
console.log("alStringFromSplits=" + Array.from("a\u{1F600}b").length + " alLikeString=" + Array.from({ length: 3, 0: "x" } as any).length);

// Iterables win over length when both are present.
const both: any = {
  length: 3, 0: "L0", 1: "L1", 2: "L2",
  [Symbol.iterator]() {
    let i = 0;
    return { next() { i++; return i <= 2 ? { value: "I" + i, done: false } : { value: undefined, done: true }; } };
  },
};
console.log("iteratorWins=" + Array.from(both).join(","));

// Array.from closes the source iterator when the mapper throws.
const closeLog: string[] = [];
const closing = {
  [Symbol.iterator]() {
    let n = 0;
    return {
      next() { closeLog.push("next"); return { value: ++n, done: false }; },
      return() { closeLog.push("return"); return { done: true }; },
    };
  },
};
let caughtMapper = "";
try { Array.from(closing, (v) => { if (v === 2) throw new Error("stop"); return v; }); }
catch (e: any) { caughtMapper = e.message; }
console.log("mapperThrow=" + caughtMapper + " log=" + closeLog.join(","));

// A custom iterator that reuses its result object: Array.from snapshots each value.
const shared: any = { value: 0, done: false };
const reuse = {
  [Symbol.iterator]() {
    let i = 0;
    return { next() { shared.value = { n: i }; shared.done = i++ >= 3; return shared; } };
  },
};
console.log("reusedResult=" + Array.from(reuse, (x: any) => x.n).join(","));

// Array.from reads a live array-like: getters run in index order, and a write
// made by one getter is seen by the later reads.
const liveSeen: string[] = [];
const live: any = {
  get length() { liveSeen.push("length"); return 3; },
  get 0() { liveSeen.push("0"); this[1] = "changed"; return "a"; },
  1: "b",
  get 2() { liveSeen.push("2"); return "c"; },
};
console.log("liveFrom=" + Array.from(live, (v: any, i: number) => i + ":" + v).join("|"));
console.log("liveOrder=" + liveSeen.join(","));

// Subclassing: Symbol.species decides which constructor derived results use.
class Sub extends Array {}
const s = Sub.from([1, 2, 3]) as any;
console.log("subFrom=" + (s instanceof Sub) + " isArr=" + Array.isArray(s) + " ofSub=" + (Sub.of(1, 2) instanceof Sub));
console.log("speciesMap=" + (s.map((x: number) => x) instanceof Sub) + " speciesFilter=" + (s.filter(() => true) instanceof Sub));
console.log("speciesSlice=" + (s.slice() instanceof Sub) + " speciesConcat=" + (s.concat([4]) instanceof Sub));
console.log("speciesFlat=" + (s.flat() instanceof Sub) + " speciesFlatMap=" + (s.flatMap((x: number) => [x]) instanceof Sub));
console.log("speciesSplice=" + (s.slice().splice(0, 1) instanceof Sub));
console.log("noSpeciesToSorted=" + (s.toSorted() instanceof Sub) + " noSpeciesWith=" + (s.with(0, 9) instanceof Sub));
console.log("noSpeciesToSpliced=" + (s.toSpliced(0, 1) instanceof Sub) + " noSpeciesToReversed=" + (s.toReversed() instanceof Sub));
console.log("joinType=" + typeof s.join(",") + " reduceType=" + typeof s.reduce((a: number, b: number) => a + b));

class Plain extends Array { static get [Symbol.species]() { return Array; } }
const p = Plain.from([1, 2]) as any;
console.log("plainSpecies=" + (p.map((x: number) => x) instanceof Plain) + " isArr=" + Array.isArray(p.map((x: number) => x)));
const nulled: any = [1, 2];
nulled.constructor = { [Symbol.species]: null };
console.log("nullSpecies=" + Array.isArray(nulled.map((x: number) => x)));
const bad: any = [1, 2];
bad.constructor = { [Symbol.species]: 42 };
try { bad.map((x: number) => x); } catch (e: any) { console.log("badSpecies=" + e.name); }

class Watch extends Array {
  static seen: string[] = [];
  constructor(...args: any[]) { Watch.seen.push(args.length + ":" + String(args[0])); super(...args); }
}
const w = new Watch();
w.push(1, 2, 3);
w.map((x: number) => x);
w.filter(() => true);
console.log("speciesCtorArgs=" + Watch.seen.join(" "));

class Removed extends Array {}
class Source extends Array { static get [Symbol.species]() { return Removed; } }
const source = new Source(1, 2, 3, 4);
const removedPart = source.splice(1, 2, 9);
console.log("speciesSpliceResult=" + (removedPart instanceof Removed) + "," + (removedPart instanceof Source) + "," + removedPart.join(","));
console.log("speciesSpliceSource=" + (source instanceof Source) + "," + source.join(","));

class Weird<T> extends Array<T> {
  static get [Symbol.species]() {
    return function (len: number) { const out: any[] = []; (out as any).createdLength = len; return out; } as any;
  }
}
const weird: any = new Weird(1, 2, 3).map((x: number) => x * 10);
console.log("customSpecies=" + Array.isArray(weird) + " " + weird.join(",") + " created=" + weird.createdLength);

class Result extends Array {}
class ConcatSource extends Array { static get [Symbol.species]() { return Result; } }
const cs = new ConcatSource();
cs.length = 3;
cs[1] = "x";
const concatOut: any = cs.concat(["y"]);
console.log("speciesConcat2=" + (concatOut instanceof Result) + " len=" + concatOut.length + " keys=" + Object.keys(concatOut).join(","));

class FlatResult extends Array {}
class FlatSource extends Array { static get [Symbol.species]() { return FlatResult; } }
const fs: any = new FlatSource();
fs.push(1, [2, [3, [4]]]);
fs[4] = 5;
fs.length = 6;
const flatOut: any = fs.flat("2.9" as any);
console.log("speciesFlat2=" + (flatOut instanceof FlatResult) + " " + flatOut.join(",") + " len=" + flatOut.length + " keys=" + Object.keys(flatOut).join(","));

// Array.from over an iterable interleaves pulls and mapper calls: each element is
// pulled, then mapped, before the next pull.
const order2: string[] = [];
function* gen2() { order2.push("y1"); yield 1; order2.push("y2"); yield 2; }
const mapped2 = Array.from(gen2(), (v: number, i: number) => { order2.push("m" + i); return v * 10; });
console.log("iterInterleave=" + mapped2.join(",") + " order=" + order2.join(" "));

// Array.isArray is true only for real arrays, including subclass instances and Array.prototype.
console.log("isArrayLiteral=" + Array.isArray([1, 2, 3]) + " isArrayEmpty=" + Array.isArray([]) + " isArrayObj=" + Array.isArray({ length: 0 }));
console.log("isArrayString=" + Array.isArray("hello") + " isArrayNumber=" + Array.isArray(123) + " isArrayNull=" + Array.isArray(null) + " isArrayUndef=" + Array.isArray(undefined));
console.log("isArrayLike=" + Array.isArray({ 0: "a", 1: "b", length: 2 }) + " isArraySub=" + Array.isArray(new Sub()) + " isArrayProto=" + Array.isArray(Array.prototype));

// The shape of Array's own surface: prototype methods are non-enumerable, writable,
// configurable functions with the specified length and name; statics follow the same rule.
const protoMethods = [
  "at", "concat", "copyWithin", "entries", "every", "fill", "filter", "find",
  "findIndex", "findLast", "findLastIndex", "flat", "flatMap", "forEach",
  "includes", "indexOf", "join", "keys", "lastIndexOf", "map", "pop", "push",
  "reduce", "reduceRight", "reverse", "shift", "slice", "some", "sort",
  "splice", "toLocaleString", "toReversed", "toSorted", "toSpliced", "toString",
  "unshift", "values", "with",
];
const protoMissing: string[] = [];
const protoBadShape: string[] = [];
for (const m of protoMethods) {
  const d: any = Object.getOwnPropertyDescriptor(Array.prototype, m);
  if (!d) { protoMissing.push(m); continue; }
  if (!(typeof d.value === "function" && d.writable === true && d.enumerable === false && d.configurable === true)) protoBadShape.push(m);
}
console.log("protoMissing=" + (protoMissing.length ? protoMissing.join(",") : "none"));
console.log("protoBadShape=" + (protoBadShape.length ? protoBadShape.join(",") : "none"));
console.log("protoLengths=" + protoMethods.map((m) => m + ":" + (Array.prototype as any)[m].length).join(" "));
console.log("protoNames=" + (protoMethods.filter((m) => (Array.prototype as any)[m].name !== m).join(",") || "none"));
console.log("protoIsArray=" + Array.isArray(Array.prototype) + " protoLen=" + Array.prototype.length + " protoParentIsObject=" + (Object.getPrototypeOf(Array.prototype) === Object.prototype));
console.log("protoTag=" + Object.prototype.toString.call(Array.prototype));
for (const s of ["from", "of", "isArray", "fromAsync"]) {
  const d: any = Object.getOwnPropertyDescriptor(Array, s);
  console.log("static." + s + "=" + (d ? "w:" + d.writable + ",e:" + d.enumerable + ",c:" + d.configurable + ",len:" + d.value.length : "absent"));
}
const lenDesc: any = Object.getOwnPropertyDescriptor(Array, "length");
console.log("arrayLength=" + Array.length + " desc=" + lenDesc.writable + "," + lenDesc.enumerable + "," + lenDesc.configurable + " name=" + Array.name);
console.log("ctorOnProto=" + (Array.prototype.constructor === Array));
const ctorDesc: any = Object.getOwnPropertyDescriptor(Array.prototype, "constructor");
console.log("ctorDesc=" + ctorDesc.writable + "," + ctorDesc.enumerable + "," + ctorDesc.configurable);
console.log("speciesIsGetter=" + (typeof Object.getOwnPropertyDescriptor(Array, Symbol.species)!.get));
const protoSyms = Object.getOwnPropertySymbols(Array.prototype).map((sym) => String(sym)).sort();
console.log("protoSymbols=" + protoSyms.join(","));
console.log("iteratorIsValues=" + ((Array.prototype as any)[Symbol.iterator] === Array.prototype.values));
const unsc: any = (Array.prototype as any)[Symbol.unscopables];
console.log("unscopables=" + String(Object.getPrototypeOf(unsc)) + " keys=" + Object.keys(unsc).sort().join(",") + " allTrue=" + Object.keys(unsc).every((k) => unsc[k] === true));
const protoForIn: string[] = [];
for (const k in [1, 2]) protoForIn.push(k);
console.log("forInOwnOnly=" + protoForIn.join(",") + " protoKeys=" + Object.keys(Array.prototype).length);
