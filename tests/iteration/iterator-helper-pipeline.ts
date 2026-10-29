// Iterator helper pipelines: laziness (pull logs), take/drop/flatMap closing rules,
// flatMap's acceptance rules for returned values, the terminal helpers, and the
// callback contract (arguments, index numbering, throw-closes-source).

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

const pulls: string[] = [];

function* naturals(tag: string) {
  try {
    let i = 0;
    while (true) {
      pulls.push(tag + i);
      yield i++;
    }
  } finally {
    pulls.push(tag + "-closed");
  }
}

function* nums(tag: string, upto: number) {
  try {
    for (let i = 1; i <= upto; i++) {
      pulls.push(tag + i);
      yield i;
    }
  } finally {
    pulls.push(tag + "-closed");
  }
}

function* letters(tag: string, values: any[]) {
  try {
    for (let i = 0; i < values.length; i++) {
      pulls.push(tag + i);
      yield values[i];
    }
  } finally {
    pulls.push(tag + "-closed");
  }
}

function* none() {}

// ---- A. laziness: building a chain pulls nothing; each next() pulls only what it needs ----
pulls.length = 0;
const chain: any = naturals("a").map((x: number) => x * 10).filter((x: number) => x > 0).take(3);
out("buildPulls", String(pulls.length));
out("chainP1", JSON.stringify(chain.next()));
out("chainP1Pulls", pulls.join(","));
out("chainP2", JSON.stringify(chain.next()));
out("chainP3", JSON.stringify(chain.next()));
out("chainP3Pulls", pulls.join(","));
out("chainP4", JSON.stringify(chain.next()));
out("chainP4Pulls", pulls.join(","));

pulls.length = 0;
out("takeZero", JSON.stringify(naturals("b").take(0).next()) + " pulls=" + pulls.join(","));

pulls.length = 0;
const dropped: any = naturals("c").drop(2);
out("dropBuildPulls", String(pulls.length));
out("dropFirst", String(dropped.next().value));
out("dropPulls", pulls.join(","));
dropped.return();
out("dropAfterReturn", pulls.join(","));

pulls.length = 0;
out("takeToArray", naturals("d").take(4).toArray().join(","));
out("takeToArrayPulls", pulls.join(","));

pulls.length = 0;
const flat: any = naturals("e").take(3).flatMap((x: number) => [x, x]);
out("flatBuildPulls", String(pulls.length));
out("flatA", JSON.stringify(flat.next()) + " pulls=" + pulls.join(","));
out("flatB", JSON.stringify(flat.next()));
out("flatRest", flat.toArray().join(","));
out("flatPulls", pulls.join(","));

pulls.length = 0;
const deep: any = naturals("f").map((x: number) => x).filter(() => true);
deep.next();
out("deepBeforeReturn", pulls.join(","));
out("deepReturned", JSON.stringify(deep.return("bye")));
out("deepAfterReturn", pulls.join(","));
out("deepNextAfterReturn", JSON.stringify(deep.next()));

pulls.length = 0;
function* finite() {
  try {
    yield 1;
    yield 2;
  } finally {
    pulls.push("finite-closed");
  }
}
const fin: any = finite().map((x: number) => x + 100);
out("finA", JSON.stringify(fin.next()));
out("finB", JSON.stringify(fin.next()));
out("finC", JSON.stringify(fin.next()));
out("finPulls", pulls.join(","));

const once: any = finite().map((x: number) => x);
out("helperSelfIterable", String(once[Symbol.iterator]() === once));
out("helperFirstPass", once.toArray().join(","));
out("helperSecondPass", JSON.stringify(once.toArray()));

// ---- B. terminal helpers: short-circuit, closing, and reduce seed rules ----
pulls.length = 0;
out("reduceSeed", String(nums("a", 4).reduce((acc: number, v: number) => acc + v, 100)));
out("reduceSeedPulls", pulls.join(","));
pulls.length = 0;
out("reduceNoSeed", String(nums("b", 4).reduce((acc: number, v: number) => acc + v)));
out("reduceNoSeedPulls", pulls.join(","));
out("reduceEmptyWithSeed", String(none().reduce((a: any, b: any) => a + b, "seed" as any)));
out("reduceEmptyNoSeed", attempt(() => (none() as any).reduce((a: any, b: any) => a + b)));

const reduceIdx: string[] = [];
nums("c", 3).reduce((acc: any, v: number, i: number) => {
  reduceIdx.push(v + "@" + i);
  return acc;
}, 0);
out("reduceSeedIndices", reduceIdx.join(","));

pulls.length = 0;
out("someHit", String(nums("d", 10).some((v: number) => v === 3)));
out("somePulls", pulls.join(","));
pulls.length = 0;
out("someMiss", String(nums("e", 3).some((v: number) => v > 99)));
out("someMissPulls", pulls.join(","));

pulls.length = 0;
out("everyFail", String(nums("f", 10).every((v: number) => v < 3)));
out("everyFailPulls", pulls.join(","));
pulls.length = 0;
out("everyAllTrue", String(nums("g", 3).every(() => true)));
out("everyAllTruePulls", pulls.join(","));

pulls.length = 0;
out("findHit", String(nums("h", 10).find((v: number) => v % 4 === 0)));
out("findHitPulls", pulls.join(","));
pulls.length = 0;
out("findMiss", String(nums("i", 3).find((v: number) => v > 99)));
out("findMissPulls", pulls.join(","));

pulls.length = 0;
const visited: string[] = [];
const feResult = nums("j", 3).forEach((v: number, i: number) => {
  visited.push(v + "@" + i);
});
out("forEachReturns", String(feResult) + " visited=" + visited.join(","));
out("forEachPulls", pulls.join(","));

pulls.length = 0;
out("forEachThrows", attempt(() => nums("k", 10).forEach((v: number) => {
  if (v === 2) throw new RangeError("stop");
})));
out("forEachThrowPulls", pulls.join(","));

pulls.length = 0;
out("mapThrows", attempt(() => nums("m", 10).map((v: number) => {
  if (v === 2) throw new TypeError("stop");
  return v;
}).toArray()));
out("mapThrowPulls", pulls.join(","));

out("toArrayEmpty", JSON.stringify(none().toArray()));
const it14: any = nums("p", 3);
it14.toArray();
out("afterTerminalNext", JSON.stringify(it14.next()));

// ---- C. flatMap: what the callback may return ----
pulls.length = 0;
out("flatArrays", JSON.stringify(letters("fa", [1, 2]).flatMap((v: any) => [v, [v, v]]).toArray()));
out("flatEmptyArrays", JSON.stringify(letters("fb", [1, 2, 3]).flatMap((v: any) => (v === 2 ? [] : [v])).toArray()));

pulls.length = 0;
function* pair(v: any) {
  yield v + "-x";
  yield v + "-y";
}
out("flatGenerators", letters("fc", ["p", "q"]).flatMap((v: any) => pair(v)).toArray().join(","));
out("flatGeneratorPulls", pulls.join(","));

function counter(k: number) {
  let i = 0;
  return {
    next() {
      return i < k ? { done: false, value: "c" + i++ } : { done: true, value: undefined };
    },
  };
}
out("flatBareIterator", letters("fd", [2, 1]).flatMap((v: any) => counter(v)).toArray().join(","));
out("flatSet", JSON.stringify(letters("fe", [1]).flatMap(() => new Set(["s1", "s2"])).toArray()));
out("flatMap", JSON.stringify(letters("ff", [1]).flatMap(() => new Map([["k", "v"]])).toArray()));
out("flatStringObject", JSON.stringify(letters("fh", [1]).flatMap(() => new String("hi") as any).toArray()));
out("flatOneLevelOnly", JSON.stringify(letters("fq", [1]).flatMap(() => [[1, 2], [3]]).toArray()));

pulls.length = 0;
out("flatPrimitiveString", attempt(() => letters("fg", ["ab"]).flatMap((v: any) => v).toArray()));
out("flatPrimitiveStringPulls", pulls.join(","));

const returnsBad: Array<[string, () => any]> = [
  ["flatNumber", () => 5],
  ["flatNull", () => null],
  ["flatUndefined", () => undefined],
  ["flatPlainObject", () => ({})],
];
for (const [label, ret] of returnsBad) {
  pulls.length = 0;
  out(label, attempt(() => letters("fx", [1]).flatMap(ret).toArray()) + " pulls=" + pulls.join(","));
}

pulls.length = 0;
out("flatCallbackThrows", attempt(() => letters("fm", [1, 2, 3]).flatMap((v: any) => {
  if (v === 2) throw new RangeError("stop");
  return [v];
}).toArray()));
out("flatCallbackThrowPulls", pulls.join(","));

const flatArgs: string[] = [];
letters("fn", ["u", "v", "w"]).flatMap((v: any, i: number) => {
  flatArgs.push(v + "@" + i);
  return [v];
}).toArray();
out("flatCallbackArgs", flatArgs.join(","));

pulls.length = 0;
const lazyFlat: any = letters("fo", ["a", "b"]).flatMap((v: any) => [v + "1", v + "2"]);
out("lazyBeforeNext", pulls.join(","));
out("lazyFirst", JSON.stringify(lazyFlat.next()) + " pulls=" + pulls.join(","));
out("lazySecond", JSON.stringify(lazyFlat.next()) + " pulls=" + pulls.join(","));
out("lazyThird", JSON.stringify(lazyFlat.next()) + " pulls=" + pulls.join(","));

const innerMarks: string[] = [];
function* innerTracked(v: any) {
  try {
    yield v + "-1";
    yield v + "-2";
  } finally {
    innerMarks.push("inner-closed:" + v);
  }
}
pulls.length = 0;
const closing: any = letters("p", ["z"]).flatMap((v: any) => innerTracked(v));
out("closingFirst", String(closing.next().value));
out("closingReturn", JSON.stringify(closing.return("bye")));
out("closingCleanup", innerMarks.join(",") + " sourceClosed=" + pulls.indexOf("p-closed"));

// ---- D. the callback contract: arguments, index numbering, throws ----
const mapArgs: string[] = [];
letters("q", ["x", "y", "z"]).map(function (this: any, v: any, i: number) {
  mapArgs.push(v + "@" + i + "#" + arguments.length);
  return v;
}).toArray();
out("mapArgs", mapArgs.join(","));

const filterArgs: string[] = [];
const kept = nums("r", 5).filter((v: number, i: number) => {
  filterArgs.push(v + "@" + i);
  return v % 2 === 1;
}).toArray();
out("filterIndexCountsSource", filterArgs.join(",") + " kept=" + kept.join(","));

const afterFilter: string[] = [];
nums("s", 4).filter((v: number) => v % 2 === 0).map((v: number, i: number) => {
  afterFilter.push(v + "@" + i);
  return v;
}).toArray();
out("indexRestartsDownstream", afterFilter.join(","));

const afterDrop: string[] = [];
letters("t", [10, 20, 30, 40]).drop(2).map((v: any, i: number) => {
  afterDrop.push(v + "@" + i);
  return v;
}).toArray();
out("indexAfterDrop", afterDrop.join(","));

const reduceNoSeedIdx: string[] = [];
const reduceTotal = letters("u", [1, 2, 3]).reduce((acc: any, v: any, i: number) => {
  reduceNoSeedIdx.push(v + "@" + i);
  return acc + v;
});
out("reduceNoSeedIndexFromOne", reduceNoSeedIdx.join(",") + " total=" + reduceTotal);

const forEachArgs: string[] = [];
letters("v", ["p", "q"]).forEach((v: any, i: number) => {
  forEachArgs.push(v + "@" + i);
});
out("forEachArgs", forEachArgs.join(","));

let calleeThisIsHelper = "unset";
let calleeThisIsSource = "unset";
const source7: any = nums("w", 1);
const helper7: any = source7.map(function (this: any) {
  calleeThisIsHelper = String(this === helper7);
  calleeThisIsSource = String(this === source7);
  return 1;
});
helper7.toArray();
out("callbackThisNotHelperOrSource", calleeThisIsHelper + "," + calleeThisIsSource);

pulls.length = 0;
out("filterThrows", attempt(() => nums("x", 3).filter((v: number) => {
  if (v === 2) throw new RangeError("x");
  return true;
}).toArray()));
out("filterThrowPulls", pulls.join(","));
pulls.length = 0;
out("someThrows", attempt(() => nums("y", 3).some((v: number) => {
  if (v === 2) throw new RangeError("x");
  return false;
})));
out("someThrowPulls", pulls.join(","));
pulls.length = 0;
out("reduceThrows", attempt(() => nums("z", 3).reduce((a: any, v: number) => {
  if (v === 2) throw new RangeError("x");
  return a;
}, 0)));
out("reduceThrowPulls", pulls.join(","));

// the helper's own next() validates what the source returned
const badNextSrc: any = { next() { return 42; } };
out("nonObjectSourceResult", attempt(() => Iterator.from(badNextSrc).map((v: any) => v).next()));
const throwingSrc: any = { next() { throw new EvalError("boom"); } };
out("throwingSourceNext", attempt(() => Iterator.from(throwingSrc).map((v: any) => v).next()));

let truthyCalls = 0;
const truthyDone: any = {
  next() {
    truthyCalls++;
    return truthyCalls === 1 ? { done: 0, value: "v1" } : { done: "yes", value: "ignored" };
  },
};
out("truthyDoneEnds", JSON.stringify(Iterator.from(truthyDone).toArray()) + " calls=" + truthyCalls);

let doneValueCount = 0;
const doneValueSrc: any = {
  next() {
    doneValueCount++;
    return doneValueCount <= 2 ? { done: false, value: "d" + doneValueCount } : { done: true, value: "FINAL" };
  },
};
out("doneValueDropped", JSON.stringify(Iterator.from(doneValueSrc).map((v: any) => v).toArray()));
