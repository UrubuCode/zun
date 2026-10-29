// Search and predicate methods: indexOf, lastIndexOf, includes (with fromIndex
// rules, NaN, -0, holes, strict equality), find/findIndex/findLast/findLastIndex,
// some and every (short-circuiting and empty arrays), and the thisArg and
// callback-arity contract shared by the callback methods.

// indexOf and lastIndexOf: direction, fromIndex clamping, and strict equality.
const arr = [1, 2, 3, 2, 1];
console.log("indexOf2=" + arr.indexOf(2) + " lastIndexOf2=" + arr.lastIndexOf(2) + " indexOf5=" + arr.indexOf(5) + " lastIndexOf5=" + arr.lastIndexOf(5));
console.log("indexOfFrom2=" + arr.indexOf(2, 2) + " indexOfFrom3=" + arr.indexOf(2, 3) + " lastFrom2=" + arr.lastIndexOf(2, 2) + " lastFrom1=" + arr.lastIndexOf(2, 1));
console.log("indexOf1From1=" + arr.indexOf(1, 1) + " lastIndexOf1=" + arr.lastIndexOf(1) + " lastIndexOf1From3=" + arr.lastIndexOf(1, 3));
console.log("indexOfNeg2=" + arr.indexOf(2, -2) + " indexOfNeg1=" + arr.indexOf(1, -1) + " lastNeg2=" + arr.lastIndexOf(2, -2) + " lastNeg1=" + arr.lastIndexOf(1, -1));
const a6 = [1, 2, 3, 1, 2, 3];
console.log("idxDefault=" + a6.indexOf(1) + " lastDefault=" + a6.lastIndexOf(1));
console.log("idxFrom3=" + a6.indexOf(1, 3) + " lastFrom3=" + a6.lastIndexOf(1, 3));
console.log("idxFrom0=" + a6.indexOf(3, 0) + " lastFrom0=" + a6.lastIndexOf(3, 0));
console.log("idxNeg1=" + a6.indexOf(3, -1) + " lastNeg1b=" + a6.lastIndexOf(3, -1));
console.log("idxNeg99=" + a6.indexOf(1, -99) + " lastNeg99=" + a6.lastIndexOf(1, -99));
console.log("idx99=" + a6.indexOf(1, 99) + " last99=" + a6.lastIndexOf(1, 99));
console.log("idxNaN=" + a6.indexOf(1, NaN) + " lastNaN=" + a6.lastIndexOf(1, NaN) + " idxUndef=" + a6.indexOf(1, undefined) + " lastUndef=" + a6.lastIndexOf(1, undefined));
console.log("idxFrac=" + a6.indexOf(2, 1.9) + " lastFrac=" + a6.lastIndexOf(2, 1.9) + " idxStr=" + a6.indexOf(2, "4" as any) + " lastStr=" + a6.lastIndexOf(2, "4" as any));
console.log("lastArity1=" + a6.lastIndexOf(3) + " lastArityUndef=" + a6.lastIndexOf(3, undefined));
console.log("searchArity=" + [].indexOf.length + "," + [].lastIndexOf.length + " emptyIdx=" + ([] as number[]).indexOf(1) + " emptyLast=" + ([] as number[]).lastIndexOf(1, 5));
console.log("strictNoCoerce=" + [1, 2].indexOf("1" as any) + "," + [1, 2].lastIndexOf("1" as any));
console.log("nanNoMatch=" + [NaN].indexOf(NaN) + "," + [NaN].lastIndexOf(NaN));
console.log("negZeroMatch=" + [-0].indexOf(0) + "," + [0].lastIndexOf(-0));
const ob = {};
console.log("objIdentity=" + [ob].indexOf(ob) + "," + [{}].indexOf({} as any));
const mixed: any[] = [1, "1", 2, "2", true, false, null, undefined];
console.log("strictMixed=" + mixed.indexOf(1) + "," + mixed.indexOf("1") + "," + mixed.indexOf(true) + "," + mixed.indexOf(null) + "," + mixed.indexOf(undefined));
console.log("strictZeroEmpty=" + mixed.includes(0 as any) + "," + mixed.includes("" as any));
const hx: any[] = [undefined, , undefined];
console.log("holeIndexOf=" + hx.indexOf(undefined) + " holeLast=" + hx.lastIndexOf(undefined));
console.log("allHoles=" + [, , ].indexOf(undefined) + "," + [, , ].lastIndexOf(undefined));
const like: any = { length: 3, 0: "a", 1: "b", 2: "a" };
console.log("genericIdx=" + Array.prototype.indexOf.call(like, "a") + " genericLast=" + Array.prototype.lastIndexOf.call(like, "a"));
const shortLen: any = { length: 1, 0: "a", 1: "a" };
console.log("genericShort=" + Array.prototype.lastIndexOf.call(shortLen, "a"));

// includes: SameValueZero, so NaN matches NaN and -0 matches 0; it reads holes as undefined.
const withNaN = [1, 2, 3, NaN, 0, -0];
console.log("incl2=" + withNaN.includes(2) + " incl5=" + withNaN.includes(5) + " inclNaN=" + withNaN.includes(NaN));
console.log("inclFrom2=" + withNaN.includes(2, 2) + " inclFrom0=" + withNaN.includes(2, 0) + " inclNegFrom=" + withNaN.includes(3, -3));
console.log("inclNegZero=" + [0, -0].includes(-0) + " inclPosInNeg=" + [-0].includes(0) + " idxNegZero=" + [0, -0].indexOf(-0) + " idxPosInNeg=" + [-0].indexOf(0));
const dup = [1, 2, 3, 1, 2, 3];
console.log("inclFrom3=" + dup.includes(1, 3) + " inclFrom4=" + dup.includes(1, 4) + " inclNeg2=" + dup.includes(3, -2));
console.log("inclNeg99=" + dup.includes(1, -99) + " incl99=" + dup.includes(1, 99) + " inclNaNFrom=" + dup.includes(1, NaN) + " inclUndefFrom=" + dup.includes(1, undefined));
const own: any[] = [1, undefined, 3];
console.log("inclUndef=" + own.includes(undefined) + " idxUndef=" + own.indexOf(undefined));
console.log("holesIncludesUndef=" + [1, , 3].includes(undefined) + " holesIdxUndef=" + [1, , 3].indexOf(undefined));
console.log("inclNaNIdx=" + [1, NaN, 3].includes(NaN) + " idxNaNIdx=" + [1, NaN, 3].indexOf(NaN) + " findIndexNaN=" + [1, NaN, 3].findIndex((v) => Number.isNaN(v)));
console.log("inclString=" + ["ab", "cd"].includes("b") + " strIncl=" + "hello".includes("e") + " strInclX=" + "hello".includes("x") + " strFrom=" + "hello".includes("l", 3));
console.log("strIndexOf=" + "hello world hello".indexOf("hello") + " strLast=" + "hello world hello".lastIndexOf("hello") + " strFromIdx=" + "hello world hello".indexOf("hello", 5));

// find, findIndex, findLast, findLastIndex: first and last match, undefined or -1 for none.
const fa = [1, 2, 3, 4, 5];
console.log("findNone=" + fa.find((x) => x > 10) + " findHit=" + fa.find((x) => x === 3));
console.log("findIdxNone=" + fa.findIndex((x) => x > 10) + " findIdxHit=" + fa.findIndex((x) => x === 3));
console.log("findEmpty=" + ([] as number[]).find(() => true) + " findIndexArgs=" + [10, 20, 30].findIndex((v, i) => i === 1));
const flast = [1, 4, 7, 4, 2];
console.log("findLast=" + flast.findLast((n) => n % 2 === 0) + " findLastIdx=" + flast.findLastIndex((n) => n % 2 === 0));
console.log("findLastNone=" + flast.findLast((n) => n > 100) + " findLastIdxNone=" + flast.findLastIndex((n) => n > 100));
console.log("findLastFrom45=" + [1, 2, 3, 2, 1].findLast((n) => n % 2 === 0) + " idx=" + [1, 2, 3, 2, 1].findLastIndex((n) => n % 2 === 0));
const order: number[] = [];
[10, 20, 30].findLast((v) => { order.push(v); return v === 20; });
console.log("findLastOrder=" + order.join(","));
const forder: number[] = [];
[10, 20, 30].find((v) => { forder.push(v); return v === 20; });
console.log("findOrder=" + forder.join(","));
const hf: any[] = [undefined, , undefined];
console.log("findLastHole=" + String(hf.findLast((v) => v === undefined)) + " findLastIdxHole=" + hf.findLastIndex((v) => v === undefined));
let probes = 0;
hf.findLastIndex(() => { probes++; return false; });
console.log("findLastProbes=" + probes);
const hp: any[] = [1, , 3];
let findProbes = 0;
hp.find(() => { findProbes++; return false; });
console.log("findVisitsHoles=" + findProbes + " findUndefHole=" + String(hp.find((v) => v === undefined)) + " findIdxUndef=" + hp.findIndex((v) => v === undefined));

// some and every: stop at the first decisive element and are vacuous on empty arrays.
const sv = [2, 4, 6, 8];
console.log("everyEven=" + sv.every((x) => x % 2 === 0) + " someGt5=" + sv.some((x) => x > 5) + " someGt10=" + sv.some((x) => x > 10));
const mixedNums = [1, 2, 3, 4];
console.log("mixedEvery=" + mixedNums.every((x) => x % 2 === 0) + " mixedSome=" + mixedNums.some((x) => x % 2 === 0));
const empty: number[] = [];
console.log("emptyEvery=" + empty.every((x) => x > 0) + " emptySome=" + empty.some((x) => x > 0));
let everyCount = 0;
[1, 2, 3, 4].every((x) => { everyCount++; return x < 3; });
console.log("everyStops=" + everyCount);
let someCount = 0;
[1, 2, 3, 4].some((x) => { someCount++; return x > 2; });
console.log("someStops=" + someCount);
let someAll = 0;
const r1 = [1, 2, 3, 4, 5].some((x) => { someAll++; return x > 2; });
console.log("someShort=" + r1 + " count=" + someAll);
let someNone = 0;
const r2 = [1, 2, 3, 4, 5].some((x) => { someNone++; return x > 10; });
console.log("someNone=" + r2 + " count=" + someNone);
let everyAll = 0;
const r3 = [1, 2, 3, 4, 5].every((x) => { everyAll++; return x < 10; });
console.log("everyAll=" + r3 + " count=" + everyAll);
let everyFirstFail = 0;
[1, 2, 3, 4, 5].every((x) => { everyFirstFail++; return x < 3; });
console.log("everyEarly=" + everyFirstFail);
let someProbes = 0;
[1, 2, 3].some((v) => { someProbes++; return v === 2; });
let everyProbes = 0;
[1, 2, 3].every((v) => { everyProbes++; return v < 2; });
console.log("probes=" + someProbes + "," + everyProbes);
console.log("emptyNoCalls=" + ([] as number[]).every(() => false) + "," + ([] as number[]).some(() => true));
console.log("everyHoles=" + [, , ].every(() => false) + " someHoles=" + [, , ].some(() => true));

// The callback contract: thisArg reaches a plain function but not an arrow or a
// bound function; reduce takes no thisArg; callbacks always receive three arguments.
const marker = { tag: "THIS" };
[1].forEach(function (this: any) { console.log("forEachThis=" + (this === marker)); }, marker);
console.log("mapThis=" + [1].map(function (this: any) { return this === marker; }, marker)[0]);
console.log("filterThis=" + ([1].filter(function (this: any) { return this === marker; }, marker).length === 1));
console.log("someThis=" + [1].some(function (this: any) { return this === marker; }, marker));
console.log("everyThis=" + [1].every(function (this: any) { return this === marker; }, marker));
console.log("findThis=" + ([1].find(function (this: any) { return this === marker; }, marker) === 1));
console.log("findIndexThis=" + [1].findIndex(function (this: any) { return this === marker; }, marker));
console.log("findLastThis=" + ([1].findLast(function (this: any) { return this === marker; }, marker) === 1));
console.log("flatMapThis=" + [1].flatMap(function (this: any) { return [this === marker]; }, marker)[0]);
const outer = { tag: "OUTER" };
console.log("arrowThis=" + [1].map(() => marker !== outer, marker)[0]);
const bound = function (this: any) { return this && this.tag; }.bind(outer);
console.log("boundThis=" + [1].map(bound as any, marker)[0]);
console.log("noThisArg=" + [1].map(function (this: any) { return this === marker; })[0]);
const reduceArgs: string[] = [];
[1, 2].reduce(function (this: any, acc: any, v: any, i: number, arr: any[]) {
  reduceArgs.push("argc=" + arguments.length + " i=" + i + " isArr=" + Array.isArray(arr));
  return acc + v;
}, 0);
console.log("reduceArgs=" + reduceArgs.join(" | "));
const arity: number[] = [];
[1, 2].forEach(function () { arity.push(arguments.length); });
[1, 2].map(function () { arity.push(arguments.length); return 0; });
[1, 2].filter(function () { arity.push(arguments.length); return true; });
console.log("arity=" + arity.join(","));
console.log("extraArgs=" + ([1, 2] as any).join(",", "ignored") + " " + ([1, 2] as any).includes(1, 0, "ignored"));
const counter = { n: 0 };
[1, 2, 3].forEach(function (this: any) { this.n++; }, counter);
console.log("sharedThis=" + counter.n);
const probe = new Proxy([1, 2], { get(t: any, k) { if (k === "0") console.log("probeReadIndex0=true"); return t[k]; } });
for (const m of ["forEach", "map", "filter", "some", "every", "find"]) {
  try { (Array.prototype as any)[m].call(probe, undefined); }
  catch (e: any) { console.log(m + "NonCallable=" + e.name); }
}
