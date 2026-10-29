// Iteration protocols over arrays: keys, values and entries (including for-of
// with destructuring), live array-iterator state, the many ways to copy a
// collection (spread, Array.from, concat, slice.call) and the protocol each one
// uses, Symbol.iterator overrides, iterator-protocol errors, and Array.fromAsync.

// keys, values and entries in for-of, with destructuring in the loop head.
const arr = ["a", "b", "c"];
const e1: string[] = [];
for (const [i, v] of arr.entries()) e1.push(i + ":" + v);
console.log("entries=" + e1.join(","));
const idxOnly: number[] = [];
for (const [i] of arr.entries()) idxOnly.push(i);
console.log("idxOnly=" + idxOnly.join(","));
const valOnly: string[] = [];
for (const [, v] of arr.entries()) valOnly.push(v);
console.log("valOnly=" + valOnly.join(","));
const pairShape: string[] = [];
for (const p of arr.entries()) pairShape.push(String(Array.isArray(p)) + p.length);
console.log("pairShape=" + pairShape.join(","));
const withDefault: string[] = [];
for (const [i, v, extra = "DEF"] of arr.entries() as any) withDefault.push(i + v + extra);
console.log("withDefault=" + withDefault.join(","));
const restPair: string[] = [];
for (const [i, ...rest] of arr.entries()) restPair.push(i + "->" + rest.join("+"));
console.log("restPair=" + restPair.join(","));
const ks: number[] = [];
for (const key of arr.keys()) ks.push(key);
const vs: string[] = [];
for (const val of arr.values()) vs.push(val);
const dflt: string[] = [];
for (const val of arr) dflt.push(val);
console.log("keys=" + ks.join(",") + " values=" + vs.join(",") + " valuesMatchDefault=" + (dflt.join(",") === vs.join(",")));
const letRebind: string[] = [];
for (let [i, v] of arr.entries()) { i = i * 10; v = v.toUpperCase(); letRebind.push(i + v); }
console.log("letRebind=" + letRebind.join(","));
const brk: string[] = [];
for (const [i, v] of arr.entries()) { if (i === 1) break; brk.push(v); }
console.log("break=" + brk.join(",") + " len=" + brk.length);
const cont: string[] = [];
for (const [i, v] of arr.entries()) { if (i === 1) continue; cont.push(v); }
console.log("continue=" + cont.join(","));
const sparseEntries: string[] = [];
for (const [i, v] of ["x", , "z"].entries()) sparseEntries.push(i + ":" + String(v));
console.log("sparseEntries=" + sparseEntries.join(","));
let emptyN = 0;
for (const [_i, _v] of ([] as string[]).entries()) emptyN++;
console.log("emptyEntries=" + emptyN);
const live = ["p"];
const liveOut: string[] = [];
for (const [i, v] of live.entries()) { liveOut.push(i + v); if (i === 0) live.push("q"); if (i > 3) break; }
console.log("liveEntries=" + liveOut.join(","));
const nested: string[] = [];
for (const [i, v] of ["m", "n"].entries()) for (const [j, w] of ["1", "2"].entries()) nested.push(i + v + j + w);
console.log("nestedEntries=" + nested.join(","));
const manual = arr.entries();
const [mi, mv] = manual.next().value as [number, string];
console.log("manualPair=" + mi + mv);
const flatEntries = Array.from(arr.entries()).map((e) => e.join(":")).join(",");
console.log("flatEntries=" + flatEntries + " keysArr=" + Array.from(arr.keys()).join(",") + " valuesArr=" + Array.from(arr.values()).join(","));
console.log("sparseKeysOf=" + Array.from([1, , 3].keys()).join(","));

// Array iterators hold an index into the live array, not a snapshot.
const a = [1, 2, 3];
const it = a[Symbol.iterator]();
console.log("liveFirst=" + JSON.stringify(it.next()));
a.push(4);
console.log("liveAfterPush=" + JSON.stringify(it.next()));
a[2] = 99;
console.log("liveMutated=" + JSON.stringify(it.next()));
console.log("liveLast=" + JSON.stringify(it.next()));
console.log("liveDone=" + JSON.stringify(it.next()));
console.log("livePastDone=" + JSON.stringify(it.next()));
const b = [1, 2, 3, 4];
const ib = b[Symbol.iterator]();
ib.next();
b.length = 1;
console.log("truncatedIter=" + JSON.stringify(ib.next()));
const c = [1, 2];
const i1 = c[Symbol.iterator]();
const i2 = c[Symbol.iterator]();
console.log("indep=" + i1.next().value + "," + i2.next().value + "," + i1.next().value + " distinct=" + (i1 !== i2));
const d = ["x", "y"];
const dk = d.keys(), dv = d.values(), de = d.entries();
console.log("kve=" + dk.next().value + " " + dv.next().value + " " + JSON.stringify(de.next().value));
const f = [1, 2];
const fi = f[Symbol.iterator]();
const r1 = fi.next();
const r2 = fi.next();
console.log("freshResult=" + (r1 !== r2) + " r1=" + r1.value + " r2=" + r2.value);
const g = [1][Symbol.iterator]();
console.log("selfIterable=" + (g[Symbol.iterator]() === g) + " sharedProto=" + (Object.getPrototypeOf([].keys()) === Object.getPrototypeOf([].values())));
console.log("iterTag=" + Object.prototype.toString.call([].values()));
console.log("iterHoles=" + Array.from([1, , 3]).map(String).join(","));
console.log("valuesIsDefault=" + ((Array.prototype as any)[Symbol.iterator] === Array.prototype.values));

// Copying a collection uses different protocols: spread and Array.from(iterable)
// pull the iterator; Array.from(array-like), slice.call and concat read by index.
const pulls: string[] = [];
const custom: any = {
  length: 3,
  0: "i0", 1: "i1", 2: "i2",
  [Symbol.iterator]() {
    let i = 0;
    return { next: () => { pulls.push("pull" + i); return i < 2 ? { value: "it" + i++, done: false } : { value: undefined, done: true }; } };
  },
};
console.log("spreadCustom=" + [...custom].join(",") + " pulls=" + pulls.join(" "));
pulls.length = 0;
console.log("fromCustom=" + Array.from(custom).join(",") + " pulls=" + pulls.join(" "));
pulls.length = 0;
console.log("concatCustom=" + ([] as any).concat(custom).length + " pulls=" + pulls.join(" "));
console.log("sliceCustom=" + Array.prototype.slice.call(custom).join(",") + " pulls=" + pulls.join(" "));
const callCount = (...args: any[]) => args.length + ":" + args.join("|");
console.log("spreadCall=" + callCount(...custom) + " objSpread=" + JSON.stringify({ ...custom }));

// An array with an overridden Symbol.iterator: spread and from follow it, the
// index-based methods ignore it.
const patched: any = [1, 2, 3];
patched[Symbol.iterator] = function* () { yield "A"; yield "B"; };
console.log("patchedSpread=" + [...patched].join(",") + " patchedFrom=" + Array.from(patched).join(","));
console.log("patchedSlice=" + patched.slice().join(",") + " patchedConcat=" + ([] as any).concat(patched).join(","));
console.log("patchedJoin=" + patched.join(",") + " patchedForOf=" + (() => { const o: string[] = []; for (const v of patched) o.push(String(v)); return o.join(","); })());

// Holes: the iterator materialises them as undefined; slice and concat keep them.
const h: any[] = [1, , 3];
const sp = [...h], fr = Array.from(h), sl = h.slice(), cc = ([] as any).concat(h);
const inMap = (x: any[]) => [0, 1, 2].map((i) => (i in x ? "y" : "n")).join("");
console.log("holeCopies spread=" + inMap(sp) + " from=" + inMap(fr) + " slice=" + inMap(sl) + " concat=" + inMap(cc));

// Strings: spread and from split by code point; index-based slice by code unit.
const s = "a\u{1F600}";
console.log("strSpread=" + [...s].length + " strFrom=" + Array.from(s).length + " strSlice=" + Array.prototype.slice.call(s).length);
const set = new Set([1, 2]);
console.log("setSpread=" + [...set].join(",") + " setSliceLen=" + Array.prototype.slice.call(set as any).length);

// Iterator-protocol errors: a non-iterable spread throws, an object literal spread
// ignores it; a non-callable Symbol.iterator throws for both spread and from, and
// an undefined one falls back to the array-like path.
try { console.log([...({ a: 1 } as any)].length); } catch (e: any) { console.log("spreadPlain=" + e.name); }
console.log("fromPlain=" + Array.from({ a: 1 } as any).length);
const bad: any = { length: 1, 0: "z", [Symbol.iterator]: 42 };
try { [...bad]; } catch (e: any) { console.log("badIterSpread=" + e.name); }
try { Array.from(bad); } catch (e: any) { console.log("badIterFrom=" + e.name); }
const undefIter: any = { length: 1, 0: "z", [Symbol.iterator]: undefined };
console.log("undefIterFrom=" + Array.from(undefIter).join(","));

// Array.fromAsync accepts async iterables, sync iterables of promises, and mapping.
async function* gen() { yield 1; yield 2; }
Array.fromAsync(gen(), (x: number) => x * 3).then((r) => console.log("fromAsyncGen=" + r.join(",")));
Array.fromAsync([Promise.resolve("a"), Promise.resolve("b")]).then((r) => console.log("fromAsyncPromises=" + r.join(",")));
