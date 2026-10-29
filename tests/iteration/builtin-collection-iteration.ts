// Built-in iterables: Array, String, Map and Set under for-of, spread, Array.from,
// destructuring, entries()/keys()/values(), live mutation during iteration,
// manual drive past done, and the identity and tag facts of built-in iterators.

function out(label: string, value: string): void {
  console.log(label + "=" + value);
}

function attempt(fn: () => unknown): string {
  try {
    return "ok:" + String(fn());
  } catch (e: any) {
    return "threw:" + e.name;
  }
}

const tagOf = (x: unknown): string => Object.prototype.toString.call(x);

// ---- Array: for-of, spread, Array.from, destructuring ----
const arr = ["a", "b", "c"];
const withIndex: string[] = [];
for (const [i, v] of arr.entries()) withIndex.push(i + ":" + v);
out("arrayEntriesForOf", withIndex.join(","));
out("arraySpread", [...arr].join(","));
out("arrayFromMapFn", Array.from(arr, (v, i) => v + i).join(","));
out("arrayFromArrayLike", Array.from({ length: 4 }, (_, i) => i * i).join(","));
out("arrayFromArrayLikeIndexed", Array.from({ length: 2, 0: "x", 1: "y" } as any).join(""));
out("arrayDestructureHole", (() => { const [p, , r] = [1, 2, 3]; return p + "," + r; })());

// for-of binds a fresh let per iteration; nested for-of gives an independent cartesian product
const closures: Array<() => string> = [];
for (const value of ["a", "b", "c"]) closures.push(() => value);
out("forOfLetClosures", closures.map((f) => f()).join(",") + "|" + closures[0]() + closures[2]());
const cart: string[] = [];
for (const x of ["x", "y"]) {
  for (const y of [1, 2, 3]) cart.push(x + y);
}
out("nestedForOfCartesian", cart.join(","));

// ---- Array: sparse arrays report every index through keys/values/entries ----
const sparse: any[] = [];
sparse.length = 5;
sparse[2] = "x";
out("sparseKeys", [...sparse.keys()].join(","));
out("sparseValues", [...sparse.values()].map(String).join(","));
out("sparseEntries", JSON.stringify([...sparse.entries()]));
let sparseForOf = 0;
for (const _v of sparse) sparseForOf++;
out("sparseForOfCount", String(sparseForOf));

// ---- Array: a live iterator reads the current length on every step ----
const live = [1, 2];
const liveIt: any = live[Symbol.iterator]();
liveIt.next();
live.push(3);
out("livePush", liveIt.next().value + "," + liveIt.next().value + "|done=" + liveIt.next().done);

const shrink = [1, 2, 3, 4];
const shrinkIt: any = shrink[Symbol.iterator]();
shrinkIt.next();
shrink.length = 1;
out("shrinkEnds", JSON.stringify(shrinkIt.next()));

const sh = [1, 2, 3];
const i1: any = sh[Symbol.iterator]();
const i2: any = sh[Symbol.iterator]();
i1.next();
i1.next();
out("independentIterators", i1.next().value + "," + i2.next().value);

const half: any = [1, 2, 3, 4][Symbol.iterator]();
half.next();
const rest: number[] = [];
for (const v of half) rest.push(v);
out("forOfResumesHalf", rest.join(","));
const half2: any = [1, 2, 3, 4][Symbol.iterator]();
half2.next();
half2.next();
out("spreadRemainder", [...half2].join(","));

// ---- Array: manual drive past done, with fresh result objects ----
const ai: any = [1, 2][Symbol.iterator]();
out("arrayManualResults", [1, 2, 3, 4, 5].map(() => JSON.stringify(ai.next())).join(" "));
const ai2: any = [9][Symbol.iterator]();
ai2.next();
const doneA = ai2.next();
const doneB = ai2.next();
out("exhaustedStable", (doneA.done === doneB.done) + "," + (doneA.value === undefined) + "," + (doneB.value === undefined));
const fresh: any = [1, 2][Symbol.iterator]();
out("resultsAreFresh", String(fresh.next() !== fresh.next()));
const shapeIt: any = [5][Symbol.iterator]();
out("resultKeys", Object.keys(shapeIt.next()).sort().join(","));
out("emptyArrayFirstNext", JSON.stringify([][Symbol.iterator]().next()));

// ---- String: code points, not UTF-16 units ----
const grin = String.fromCharCode(0xd83d, 0xde00); // U+1F600
const musical = String.fromCharCode(0xd834, 0xdd1e); // U+1D11E
const s1 = "a" + grin + "b";
out("stringLength", String(s1.length));
out("stringIterCount", String([...s1].length));
const unitLens: number[] = [];
for (const c of s1) unitLens.push(c.length);
out("stringForOfUnitLens", unitLens.join(","));
out("stringCodePoints", [...s1].map((c) => c.codePointAt(0)!.toString(16)).join(","));
out("stringSplitUnits", String(s1.split("").length));
out("stringIndexUnit", String(s1[1].length));
out("stringRoundTrip", String([...s1].join("") === s1));
out("stringManyPoints", String([...(grin + musical + grin)].length));
out("stringMapFnPoints", Array.from(s1, (c) => c.codePointAt(0)!).join(","));
out("stringEmptyIter", String([...""].length));
const lone = "x" + String.fromCharCode(0xd83d) + "y";
out("loneHighSurrogate", [...lone].length + "|unit=" + [...lone][1].charCodeAt(0));
const reversed = String.fromCharCode(0xde00, 0xd83d);
out("reversedPairPoints", String([...reversed].length));
out("stringReIterable", String([...s1].length === [...s1].length));
const sIt: any = s1[Symbol.iterator]();
const sSteps: string[] = [];
for (let k = 0; k < 5; k++) {
  const r = sIt.next();
  sSteps.push(r.done ? "done" : String(r.value.length));
}
out("stringManualDrive", sSteps.join(","));
out("stringDestructure", (() => { const [c1, c2] = s1; return c1.length + "," + c2.codePointAt(0)!.toString(16); })());

// ---- Map: pairs, default iterator, insertion order, live mutation ----
const m = new Map<string, number>([["a", 1], ["b", 2], ["c", 3]]);
const pairs: string[] = [];
for (const p of m) pairs.push(String(Array.isArray(p)) + ":" + p.length + ":" + p[0] + "=" + p[1]);
out("mapForOfPairs", pairs.join("|"));
const kv: string[] = [];
for (const [k, v] of m) kv.push(k + "=" + v);
out("mapDestructuredHead", kv.join(","));
out("mapDefaultIsEntries", String(m[Symbol.iterator] === m.entries));
out("mapKeys", [...m.keys()].join(","));
out("mapValues", [...m.values()].join(","));
out("mapEntriesSpread", [...m.entries()].map((e) => e[0] + e[1]).join(","));
out("mapSpreadPairs", JSON.stringify([...m]));
out("mapArrayFromEqualsSpread", String(JSON.stringify(Array.from(m)) === JSON.stringify([...m])));
const [[firstK, firstV]] = m;
out("mapNestedDestructure", firstK + "=" + firstV);

const m2 = new Map<string, number>([["x", 1], ["y", 2]]);
m2.set("x", 99);
out("mapOverwriteKeepsSlot", [...m2.keys()].join(",") + "|" + [...m2.values()].join(","));
const m3 = new Map<string, number>([["p", 1], ["q", 2]]);
m3.delete("p");
m3.set("p", 3);
out("mapReaddMovesToEnd", [...m3.keys()].join(","));
let emptyMapCount = 0;
for (const _p of new Map()) emptyMapCount++;
out("emptyMapIter", String(emptyMapCount));

const liveMap = new Map<string, number>([["a", 1], ["b", 2], ["c", 3]]);
const visitedMap: string[] = [];
for (const [k] of liveMap) {
  visitedMap.push(k);
  if (k === "a") {
    liveMap.delete("b");
    liveMap.set("d", 4);
  }
}
out("mapLiveMutation", visitedMap.join(","));

const srcEntry = new Map<string, number>([["a", 1]]);
const copiedEntry = [...srcEntry][0];
copiedEntry[1] = 5;
out("mapEntryIsCopy", String(srcEntry.get("a")));
const entryPair = [...new Map([["a", 1], ["b", 2]]).entries()];
out("mapEntriesAreArrays", String(Array.isArray(entryPair[0])));
out("mapEntriesDistinct", String(entryPair[0] === entryPair[1]));

const mi: any = m.entries();
const mapSteps: string[] = [];
for (let k = 0; k < 5; k++) {
  const r = mi.next();
  mapSteps.push(r.done ? "done:" + String(r.value) : JSON.stringify(r.value));
}
out("mapIterManualPastDone", mapSteps.join("|"));

// ---- Set: values, default iterator, dedup, live mutation ----
const st = new Set<number>([10, 20, 30]);
const setVals: number[] = [];
for (const v of st) setVals.push(v);
out("setForOf", setVals.join(","));
out("setDefaultIsValues", String(st[Symbol.iterator] === st.values));
out("setKeysIsValues", String(st.keys === st.values));
out("setKeysSeq", [...st.keys()].join(","));
out("setEntries", [...st.entries()].map((e) => e[0] + ":" + e[1]).join(","));
out("setDedupOrder", [...new Set([3, 1, 3, 2, 1])].join(",") + "|size=" + new Set([3, 1, 3, 2, 1]).size);
out("setSpreadDestructure", (() => { const [s0, , s2] = new Set(["p", "q", "r"]); return s0 + s2; })());
const sl = new Set([1, 2]);
const setSeen: number[] = [];
for (const v of sl) {
  setSeen.push(v);
  if (v === 1) {
    sl.add(3);
    sl.delete(2);
  }
}
out("setLiveMutation", setSeen.join(","));
const si: any = st.values();
const setSteps: string[] = [];
for (let k = 0; k < 4; k++) {
  const r = si.next();
  setSteps.push(r.done ? "done" : JSON.stringify(r));
}
out("setIterManualPastDone", setSteps.join("|"));

// ---- Array.from over Map and Set equals spread ----
out("arrayFromSetMapFn", Array.from(new Set(["p", "q"]), (v, i) => v + "@" + i).join(","));
out("arrayFromMapMapFn", Array.from(m, ([k, v]) => k + v).join(","));

// ---- Built-in iterators are self-iterable, and carry their own tags ----
const arrSelf: any = [1, 2].values();
out("arrayIterSelf", String(arrSelf[Symbol.iterator]() === arrSelf));
const mapSelf: any = new Map([["x", 1]]).keys();
out("mapIterSelf", String(mapSelf[Symbol.iterator]() === mapSelf));
const strSelf: any = "ab"[Symbol.iterator]();
out("stringIterSelf", String(strSelf[Symbol.iterator]() === strSelf));
const setSelf: any = new Set([1]).values();
out("setIterSelf", String(setSelf[Symbol.iterator]() === setSelf));

out("tagArrayIter", tagOf([][Symbol.iterator]()));
out("tagStringIter", tagOf(""[Symbol.iterator]()));
out("tagMapIter", tagOf(new Map()[Symbol.iterator]()));
out("tagSetIter", tagOf(new Set()[Symbol.iterator]()));
out("tagRegExpIter", tagOf("aa".matchAll(/a/g)));
