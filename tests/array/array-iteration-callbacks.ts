// Iteration methods forEach, map, filter, reduce and reduceRight: callback
// contracts (arguments, accumulator seeding, return values), the snapshot rules
// for length and the live reads of elements while the walk runs, and the
// TypeErrors for empty reductions and non-callable callbacks.

// The callback receives (value, index, array); the array is the source itself.
const trace: string[] = [];
[10, 20, 30].reduce((acc, v, i, arr) => {
  trace.push("acc=" + acc + " v=" + v + " i=" + i + " len=" + arr.length);
  return acc + v;
});
console.log("reduceTrace=" + trace.join(" | "));
const traceR: string[] = [];
[10, 20, 30].reduceRight((acc, v, i) => { traceR.push(acc + "/" + v + "@" + i); return acc + v; });
console.log("reduceRightTrace=" + traceR.join(" "));

// With a seed the callback runs for every element, including index 0; without one
// it runs for length - 1 elements. A single element without a seed is returned
// without a call.
let withSeed = 0;
[1, 2, 3].reduce((a, b) => { withSeed++; return a + b; }, 0);
let noSeed = 0;
[1, 2, 3].reduce((a, b) => { noSeed++; return a + b; });
console.log("reduceCalls=" + withSeed + "," + noSeed);
let singleCalled = false;
console.log("reduceSingle=" + [7].reduce((a, b) => { singleCalled = true; return a + b; }) + " called=" + singleCalled);
console.log("reduceSum=" + [1, 2, 3, 4].reduce((acc, v) => acc + v, 0) + " product=" + [1, 2, 3, 4].reduce((acc, v) => acc * v, 1));
console.log("reduceMax=" + [1, 2, 3, 4].reduce((acc, v) => Math.max(acc, v)) + " reduceSingleSeed=" + [5].reduce((acc, v) => acc + v, 10));
console.log("reduceStr=" + ["hello", "world"].reduce((acc, v) => acc + " " + v) + " reduceInitUndef=" + [1].reduce((a: any, b: any) => String(a) + "/" + b, undefined));
console.log("reduceObject=" + JSON.stringify([["a", 1], ["b", 2], ["c", 3]].reduce((acc: any, [k, v]: any) => ({ ...acc, [k]: v }), {})));
console.log("reduceRightAll=" + [1, 2, 3, 4].reduceRight((acc, v) => acc + v, 0) + " concat=" + [1, 2, 3, 4].reduceRight((acc, v) => acc + "," + v));
console.log("reduceRightNested=" + JSON.stringify([[1, 2], [3, 4], [5, 6]].reduceRight((acc: any[], v) => acc.concat(v), [])));
console.log("reduceRightDigits=" + [1, 2, 3].reduceRight((acc, cur) => acc * 10 + cur, 0) + " direct=" + [1, 2, 3].reduce((acc, cur) => acc * 10 + cur, 0));
console.log("reduceNoSeedNoCall=" + [42].reduce((acc, val) => acc + val) + " seeded=" + [42].reduce((acc, val) => acc + val, 0));
console.log("reduceEmptyNoSeed=" + ([] as number[]).reduce((a, b) => a + b, 42));
try { ([] as number[]).reduce((a, b) => a + b); } catch (e: any) { console.log("reduceEmptyError=" + e.name); }
try { ([] as number[]).reduceRight((a, b) => a + b); } catch (e: any) { console.log("reduceRightEmptyError=" + e.name); }
try { ([] as any).reduce(1, 0); } catch (e: any) { console.log("reduceBadCallback=" + e.name); }
console.log("reduceGeneric=" + Array.prototype.reduce.call({ length: 3, 0: 1, 1: 2, 2: 3 } as any, (a: any, b: any) => a + b, 0));

// An element appended during reduce is not visited (the length is captured first).
const grow = [1, 2];
console.log("reduceGrow=" + grow.reduce((a, b, i) => { if (i === 0) grow.push(100); return a + b; }, 0) + " len=" + grow.length);

// forEach always returns undefined, and its callback's return value is ignored.
const fe = [1, 2, 3];
console.log("forEachReturn=" + fe.forEach((x) => x * 2));
let feSum = 0;
console.log("forEachSum=" + fe.forEach((x) => { feSum += x; }) + " sum=" + feSum);
console.log("forEachIgnoresReturn=" + fe.forEach((x) => { return x * 10; }));

// map returns a new array of callback results; a callback that returns nothing yields undefined.
const mp = [1, 2, 3];
console.log("mapBasic=" + mp.map((x) => x * 2).join(",") + " mapNoReturn=" + mp.map((x) => { x * 2; }).join(",") + " mapBlock=" + mp.map((x) => { return x * 2; }).join(","));
console.log("mapMixed=" + mp.map((x) => (x > 1 ? x * 2 : x)).join(","));
console.log("mapIndex=" + mp.map((val, idx) => val + idx).join(",") + " mapIdentity=" + (mp.map((x) => x) === mp));

// filter keeps truthy results and returns a new array.
console.log("filterTruthy=" + [0, 1, false, 2, "", 3, null, 4, undefined, 5].filter((x) => x).join(","));
console.log("filterEvens=" + [1, 2, 3, 4, 5].filter((x) => x % 2 === 0).join(",") + " odds=" + [1, 2, 3, 4, 5].filter((x) => x % 2 !== 0).join(","));
console.log("filterNone=" + [1, 2, 3, 4, 5].filter((x) => x > 10).join(",") + "|" + [1, 2, 3].filter((v, idx) => idx > 0).join(","));

// Mutation windows: the walk captures length once and reads each index live.
// An append is never visited, a delete before the visit is skipped, and an
// overwrite ahead of the cursor is seen.
const appended: number[] = [];
const ma = [1, 2, 3];
ma.forEach((v) => { appended.push(v); if (v === 1) ma.push(99); });
console.log("forEachAppend=" + appended.join(",") + " finalLen=" + ma.length);
const deleted: string[] = [];
const mb = [1, 2, 3, 4];
mb.forEach((v, i) => { deleted.push(String(v)); if (i === 0) delete (mb as any)[2]; });
console.log("forEachDelete=" + deleted.join(","));
const shrunk: string[] = [];
const mc = [1, 2, 3, 4, 5];
mc.forEach((v) => { shrunk.push(String(v)); if (v === 2) mc.length = 3; });
console.log("forEachShrink=" + shrunk.join(",") + " len=" + mc.length);
const overwritten: string[] = [];
const md = [1, 2, 3];
md.forEach((v, i) => { overwritten.push(String(v)); if (i === 0) md[2] = 77; });
console.log("forEachOverwrite=" + overwritten.join(","));
const me = [1, 2, 3];
const mapped = me.map((v, i) => { if (i === 0) me.push(9); return v; });
console.log("mapCapturesLen=" + mapped.length + " src=" + me.length);
const mf = [1, 2, 3];
console.log("filterWindow=" + mf.filter((v, i) => { if (i === 0) mf.push(9); return true; }).join(","));

// Map over an array with a getter that deletes and appends ahead of the cursor:
// the deleted index is skipped and the appended one is not visited.
const gm: any[] = [1, 2, 3, 4];
Object.defineProperty(gm, "0", { configurable: true, get() { delete gm[2]; gm.push(5); return 10; } });
const gseen: string[] = [];
const gmapped = gm.map((v, i) => { gseen.push(i + ":" + v); return v * 2; });
console.log("getterMapSeen=" + gseen.join("|"));
console.log("getterMapOut=" + gmapped.length + " keys=" + Object.keys(gmapped).join(",") + " json=" + JSON.stringify(gmapped));
console.log("getterMapLen=" + gm.length);

// reduce observes holes filled before their visit and skips indices deleted before theirs.
const rw = [1, , 3, 4] as any[];
const rwSeen: string[] = [];
const rwSum = rw.reduce((acc, v, i) => {
  rwSeen.push(i + ":" + v);
  if (i === 0) rw[1] = 2;
  if (i === 1) delete rw[3];
  return acc + v;
}, 0);
console.log("reduceWindowSum=" + rwSum + " seen=" + rwSeen.join(",") + " keys=" + Object.keys(rw).join(","));

// Callbacks and thisArg through the callback-based methods that take one.
const ctx = { mul: 3 };
const ta = [1, 2, 3, 4];
const tm = ta.map(function (this: any, v, i, src) {
  if (i === 0) src.push(99);
  if (i === 1) src[3] = 40;
  return v * this.mul;
}, ctx);
console.log("thisArgMap=" + tm.join(",") + " src=" + ta.join(","));
console.log("thisArgFilter=" + ta.filter(function (this: any, v) { return v < this.limit; }, { limit: 10 }).join(","));

// A callback returning a value from reduce's accumulator with a nested reduce.
const matrix = [[1, 2], [3, 4]];
console.log("nestedReduce=" + matrix.reduce((acc, row) => acc + row.reduce((s, x) => s + x, 0), 0));
