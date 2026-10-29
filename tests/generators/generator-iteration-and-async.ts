// Generators as iterables: every consumer that abandons or drains a generator
// (for-of break/return/throw, destructuring, Array.from, spread), generator
// methods and the prototype chain, native array/map/set iterators, generators
// that cross dynamic boundaries (arrays, objects, arguments, Map, class fields,
// forwarding functions), lazy vs eager shapes, and a short async-generator
// section (queued requests, awaited yields, finally on return, for-await closing).

const trace: string[] = [];

// Runs fn and reports how it ended: "no-throw" or the thrown constructor name.
function attempt(fn: () => unknown): string {
  try {
    fn();
    return "no-throw";
  } catch (e: any) {
    return e.constructor.name;
  }
}

// An infinite generator whose finally records that it was closed.
function* counted(tag: string) {
  try {
    let i = 0;
    while (true) {
      trace.push(tag + ":yield" + i);
      yield i++;
    }
  } finally {
    trace.push(tag + ":closed");
  }
}

function* three() {
  try {
    yield 1;
    yield 2;
    yield 3;
  } finally {
    trace.push("three:closed");
  }
}

// ── consumers that abandon a generator: each must close it exactly once ─────
trace.length = 0;
const g1 = counted("break");
for (const v of g1) {
  if (v === 2) break;
}
console.log("break=" + trace.join("|"));
console.log("breakDead=" + JSON.stringify(g1.next()));

trace.length = 0;
const g2 = counted("return");
function loopReturn() {
  for (const v of g2) {
    if (v === 1) return "early";
  }
  return "full";
}
console.log("loopReturn=" + loopReturn());
console.log("loopReturnTrace=" + trace.join("|"));

trace.length = 0;
const g3 = counted("throw");
let caught3 = "no";
try {
  for (const v of g3) {
    if (v === 1) throw new RangeError("body");
  }
} catch (e: any) {
  caught3 = e.constructor.name;
}
console.log("bodyThrow=" + caught3 + " trace=" + trace.join("|"));

trace.length = 0;
const g4 = counted("destructure");
const [a4, b4] = g4;
console.log("destructure=" + a4 + "," + b4 + " trace=" + trace.join("|"));
console.log("destructureDead=" + JSON.stringify(g4.next()));

trace.length = 0;
const [head, ...rest] = three();
console.log("restDrain head=" + head + " rest=" + rest.join(",") + " trace=" + trace.join("|"));

trace.length = 0;
const g6 = counted("hole");
const [, , third] = g6;
console.log("hole third=" + third + " trace=" + trace.join("|"));

trace.length = 0;
function* two() {
  try {
    yield "x";
  } finally {
    trace.push("two:closed");
  }
}
function thrower(): never {
  throw new TypeError("default");
}
let caught7 = "no";
try {
  const [p7, q7 = thrower()] = two();
  console.log("unreachable " + p7 + q7);
} catch (e: any) {
  caught7 = e.constructor.name;
}
console.log("defaultThrow=" + caught7 + " trace=" + trace.join("|"));

trace.length = 0;
const g8 = counted("arrayFrom");
let caught8 = "no";
try {
  Array.from(g8, function (v: number) {
    if (v === 1) throw new EvalError("map");
    return v;
  });
} catch (e: any) {
  caught8 = e.constructor.name;
}
console.log("mapperThrow=" + caught8 + " trace=" + trace.join("|"));

trace.length = 0;
const g9 = three();
const drained: number[] = [];
for (const v of g9) drained.push(v);
console.log("drained=" + drained.join(",") + " trace=" + trace.join("|"));
console.log("extraReturn=" + JSON.stringify(g9.return(0)));

// break out of an inner loop closes the inner generator; break outer closes both
trace.length = 0;
const outerG = counted("outer");
const innerG = counted("inner");
outerLoop: for (const o of outerG) {
  for (const i2 of innerG) {
    if (i2 === 1) break outerLoop;
  }
}
console.log("nestedBreak trace=" + trace.join("|"));

trace.length = 0;
const spread = [...three()];
console.log("spread=" + spread.join(",") + " trace=" + trace.join("|"));

// ── generator methods, the iterable protocol and the prototype chain ────────
class Bag {
  items: number[];
  constructor(items: number[]) {
    this.items = items;
  }
  *[Symbol.iterator]() {
    for (let i = 0; i < this.items.length; i++) yield this.items[i];
  }
  *doubled() {
    for (const v of this) yield v * 2;
  }
  static *range(k: number) {
    for (let i = 0; i < k; i++) yield i;
  }
}
const bag = new Bag([1, 2, 3]);
console.log("bagForOf=" + Array.from(bag).join(","));
console.log("bagSpread=" + [...bag].join(","));
console.log("bagDoubled=" + [...bag.doubled()].join(","));
console.log("bagStatic=" + [...Bag.range(4)].join(","));

// `this` inside the generator method is resolved at call time
const borrowed = (Bag.prototype as any)[Symbol.iterator].call({ items: [7, 8] });
console.log("borrowed=" + Array.from(borrowed).join(","));

const it1 = bag[Symbol.iterator]();
const it2 = bag[Symbol.iterator]();
console.log("freshEachCall=" + (it1 !== it2));
console.log("freshFirst=" + it1.next().value + " " + it2.next().value);
console.log("freshSecond=" + it1.next().value + "," + it2.next().value);

const gen3 = Bag.range(2);
console.log("selfIterable=" + ((gen3 as any)[Symbol.iterator]() === gen3));
console.log("ownSymbolIterator=" + Object.prototype.hasOwnProperty.call(gen3, Symbol.iterator));

const own = Object.getPrototypeOf(gen3);
const genProto = Object.getPrototypeOf(own);
const iterProto = Object.getPrototypeOf(genProto);
console.log("ownIsMethodPrototype=" + (own === (Bag.range as any).prototype));
console.log("genProtoOwns=" + ["next", "throw", "return"].map((k) => k + ":" + Object.prototype.hasOwnProperty.call(genProto, k)).join(","));
console.log("iterProtoOwnsSymbol=" + Object.prototype.hasOwnProperty.call(iterProto, Symbol.iterator));
console.log("iterProtoParent=" + (Object.getPrototypeOf(iterProto) === Object.prototype));
console.log("genTag=" + genProto[Symbol.toStringTag]);
console.log("toStringOfGenerator=" + Object.prototype.toString.call(gen3));

function* plain() {
  yield 1;
}
function* other() {
  yield 2;
}
console.log("typeofGenFn=" + typeof plain);
console.log("genFnTag=" + Object.getPrototypeOf(plain)[Symbol.toStringTag]);
console.log("toStringOfGenFn=" + Object.prototype.toString.call(plain));
console.log("genFnHasPrototype=" + typeof plain.prototype);
console.log("notConstructable=" + attempt(() => new (plain as any)()));
console.log("sharedGenFnProto=" + (Object.getPrototypeOf(plain) === Object.getPrototypeOf(other)));
console.log("sharedGenProto=" + (Object.getPrototypeOf(plain.prototype) === Object.getPrototypeOf(other.prototype)));
console.log("methodSharesProto=" + (Object.getPrototypeOf((Bag.range as any).prototype) === Object.getPrototypeOf(plain.prototype)));
console.log("methodEnumerable=" + Object.getOwnPropertyDescriptor(Bag.prototype, "doubled")!.enumerable);
console.log("methodName=" + (Bag.prototype as any).doubled.name);
console.log("iteratorMethodName=" + JSON.stringify(Bag.prototype[Symbol.iterator].name));

const lit = {
  *gen() {
    yield "l1";
    yield "l2";
  },
  *[Symbol.iterator]() {
    yield* (this as any).gen();
  },
};
console.log("literalSpread=" + [...lit].join(","));
console.log("literalMethodName=" + (lit as any).gen.name);

// ── native iterators: values() / keys() / entries() support .next() ─────────
const ai = [1, 2].values();
console.log("arrNext=" + ai.next().value + "," + ai.next().value + "," + JSON.stringify(ai.next()));
console.log("arrKeys=" + [7, 8].keys().next().value);
console.log("arrEntries=" + JSON.stringify([9].entries().next().value));
const set = new Set([5, 6]);
console.log("setValues=" + set.values().next().value + " setKeys=" + set.keys().next().value);
const map = new Map([["k", 9]]);
console.log("mapEntries=" + JSON.stringify(map.entries().next().value));
console.log("mapKeys=" + map.keys().next().value + " mapValues=" + map.values().next().value);

const base = [10, 20];
const itA = base.values();
const itB = base.values();
itA.next();
console.log("independentCursor=" + itB.next().value);

function take(iter: any, n: number): any[] {
  const out: any[] = [];
  for (let i = 0; i < n; i++) {
    const r = iter.next();
    if (r.done) break;
    out.push(r.value);
  }
  return out;
}
console.log("takeNative=" + take([1, 2, 3, 4].values(), 3).join(","));
console.log("takeBeyondEnd=" + take([1, 2].values(), 5).join(","));

const symIt = [1, 2, 3][Symbol.iterator]();
console.log("symIterType=" + typeof symIt);
console.log("symIterFirst=" + symIt.next().value + " second=" + symIt.next().value);
console.log("symIterSpread=" + [...[4, 5][Symbol.iterator]()].join(","));

console.log("plainArrayNext=" + String(([1, 2] as any).next));
console.log("spreadValues=" + [...[1, 2].values()].join(","));
console.log("spreadKeys=" + [...[7, 8].keys()].join(","));
let pairs = "";
for (const [i, v] of [9, 8].entries()) pairs = pairs + i + ":" + v + " ";
console.log("forOfEntries=" + pairs);

// ── an alias of a generator keeps its protocol on every iteration path ──────
function* eagerDecl() {
  yield* [1, 2];
}
const aliasEager = eagerDecl;
console.log("spreadViaAlias=" + [...aliasEager()].join(","));

function* twoValues() {
  yield 1;
  yield 2;
}
const aliasTwo = twoValues;
let aliasSum = 0;
for (const x of aliasTwo()) aliasSum = aliasSum + x;
console.log("forOfViaAlias=" + aliasSum + " nextViaAlias=" + aliasTwo().next().value);

const exprLazy = function* () {
  let i = 1;
  while (i <= 3) {
    yield i;
    i = i + 1;
  }
};
console.log("spreadExprLazy=" + [...exprLazy()].join(","));
console.log("nextExprLazy=" + exprLazy().next().value);
let lazySum = 0;
for (const x of exprLazy()) lazySum = lazySum + x;
console.log("forOfExprLazy=" + lazySum);
const aliasOfAlias = aliasEager;
console.log("aliasChain=" + [...aliasOfAlias()].join(","));

// ── generators crossing dynamic boundaries keep their iterator identity ─────
function* gen() {
  let i = 1;
  while (i <= 3) {
    yield i;
    i = i + 1;
  }
}
console.log("boundaryArray=" + [gen()][0].next().value);
const holderObj = { it: gen() };
console.log("boundaryField=" + holderObj.it.next().value);
function consume(x: any) {
  return x.next();
}
console.log("boundaryArg=" + consume(gen()).value);
const pushed: any[] = [];
pushed.push(gen());
console.log("boundaryPush=" + pushed[0].next().value);
class Holder {
  it: any;
  constructor() {
    this.it = gen();
  }
}
console.log("boundaryClassField=" + new Holder().it.next().value);
const mapHold = new Map();
mapHold.set("k", gen());
console.log("boundaryMap=" + mapHold.get("k").next().value);
const [destructured] = [gen()];
console.log("boundaryDestructure=" + destructured.next().value);

const kept = { it: gen() };
console.log("cursorAdvances=" + (kept.it.next().value + kept.it.next().value + kept.it.next().value));
console.log("boundaryResult=" + JSON.stringify([gen()][0].next()));
const drainedObj = { it: gen() };
drainedObj.it.next();
drainedObj.it.next();
drainedObj.it.next();
console.log("boundaryExhausted=" + JSON.stringify(drainedObj.it.next()));
const closedObj = { it: gen() };
console.log("boundaryReturn=" + JSON.stringify(closedObj.it.return(99)));
console.log("boundaryAfterReturn=" + JSON.stringify(closedObj.it.next()));

// a generator reached through forwarding functions keeps the full protocol
function* counter(n: number) {
  let i = 0;
  while (i < n) {
    yield i;
    i = i + 1;
  }
}
function fwd3() {
  return counter(3);
}
function fwdA() {
  return counter(2);
}
function fwdB() {
  return fwdA();
}
console.log("forwardSpread=" + [...fwd3()].join(","));
let forwardSum = 0;
for (const x of fwd3()) forwardSum = forwardSum + x;
console.log("forwardForOf=" + forwardSum);
console.log("twoLevelForward=" + [...fwdB()].join(","));
const fa1 = fwd3();
const fa2 = fwd3();
fa1.next();
console.log("forwardIndependent=" + fa2.next().value);

function* inner2(o: any) {
  const a = yield o.v;
  yield a * 2;
}
function wrapper() {
  const o = { v: 5 };
  return inner2(o);
}
const wr = wrapper();
console.log("forwardFirst=" + wr.next().value + " sent=" + wr.next(10).value);

// untyped parameters: the lazy and eager shapes must still yield
function* untyped(start: any, count: any) {
  for (let i = 0; i < count; i++) yield start + i;
}
console.log("untypedSpread=" + [...untyped(10, 3)].join(","));
console.log("untypedNext=" + untyped(10, 3).next().value);
const untypedShared = untyped(100, 3);
console.log("untypedCursor=" + (untypedShared.next().value + untypedShared.next().value + untypedShared.next().value));
function* withDefault(start = 1) {
  let n = start;
  while (n < start + 3) {
    yield n;
    n = n + 1;
  }
}
console.log("defaultOmitted=" + [...withDefault()].join(","));
console.log("defaultPassed=" + [...withDefault(5)].join(","));
function* eagerParam(s: any) {
  yield s;
  yield s + 1;
}
console.log("eagerParam=" + [...eagerParam(7)].join(","));

// ── async generators (deterministic: every observation is sequenced by await) ─
async function* ag() {
  yield 1;
  yield 2;
}
(async function () {
  const a = ag();
  const firstCall = a.next();
  console.log("agNextIsPromise=" + (firstCall instanceof Promise));
  console.log("agSelfAsync=" + ((a as any)[Symbol.asyncIterator]() === a));
  console.log("agNoSync=" + ((a as any)[Symbol.iterator] === undefined));
  const agProto = Object.getPrototypeOf(Object.getPrototypeOf(a));
  console.log("agProtoOwns=" + ["next", "throw", "return"].map((k) => k + ":" + Object.prototype.hasOwnProperty.call(agProto, k)).join(","));
  console.log("agTag=" + agProto[Symbol.toStringTag] + " toString=" + Object.prototype.toString.call(a));
  console.log("agFnToString=" + Object.prototype.toString.call(ag));
  console.log("agFirstValue=" + JSON.stringify(await firstCall));

  // yielded promises and thenables are awaited before the consumer sees them
  async function* awaits() {
    yield Promise.resolve("resolved");
    yield { then: (r: (v: string) => void) => r("thenable") };
    yield "plain";
  }
  const seen: string[] = [];
  for await (const v of awaits()) seen.push(String(v));
  console.log("awaitedYields=" + seen.join(","));

  // result objects and exhaustion
  const b = ag();
  const r1 = await b.next();
  const r2 = await b.next();
  const r3 = await b.next();
  console.log("agSequence=" + [JSON.stringify(r1), JSON.stringify(r2), JSON.stringify(r3)].join(""));
  console.log("agPastDone=" + JSON.stringify(await b.next()));

  // queued requests: back-to-back next() calls are served in call order
  async function* source(tag: string) {
    try {
      let i = 0;
      while (i < 5) {
        trace.push(tag + ":before" + i);
        await Promise.resolve();
        yield tag + i;
        i++;
      }
      return tag + "-exhausted";
    } finally {
      trace.push(tag + ":closed");
    }
  }
  trace.length = 0;
  const qa = source("a");
  const pa = [qa.next(), qa.next(), qa.next()];
  const ra = await Promise.all(pa);
  console.log("queueResults=" + ra.map((r) => String(r.value) + ":" + r.done).join(" "));
  console.log("queueTrace=" + trace.join("|"));

  // a return() queued behind two next() calls settles last
  trace.length = 0;
  const qb = source("b");
  const pb1 = qb.next();
  const pb2 = qb.next();
  const pbr = qb.return("stopped");
  const pb3 = qb.next();
  const rb = [await pb1, await pb2, await pbr, await pb3];
  console.log("queueReturn=" + rb.map((r) => String(r.value) + ":" + r.done).join(" "));
  console.log("queueReturnTrace=" + trace.join("|"));

  // a throw() queued behind a next() reaches the generator at its turn
  trace.length = 0;
  async function* guardedAsync() {
    try {
      yield "g1";
      yield "g2";
    } catch (e: any) {
      trace.push("caught " + e.constructor.name);
      yield "recovered";
    } finally {
      trace.push("guarded closed");
    }
  }
  const qc = guardedAsync();
  const pc1 = qc.next();
  const pc2 = qc.throw(new RangeError("late"));
  const pc3 = qc.next();
  const rc = [await pc1, await pc2, await pc3];
  console.log("queueThrow=" + rc.map((r) => String(r.value) + ":" + r.done).join(" "));
  console.log("queueThrowTrace=" + trace.join("|"));

  // return() on a generator suspended inside try runs finally, and a yield in the
  // finally is awaited too
  trace.length = 0;
  async function* finallyYield() {
    try {
      yield await Promise.resolve("a");
      yield "b";
    } finally {
      trace.push("fy:finally");
      yield "fin";
      trace.push("fy:after-fin");
    }
  }
  const fy = finallyYield();
  console.log("fyNext=" + JSON.stringify(await fy.next()));
  console.log("fyReturn=" + JSON.stringify(await fy.return("ret")));
  console.log("fyNext2=" + JSON.stringify(await fy.next()));
  console.log("fyTrace=" + trace.join("|"));

  // a rejected next() does not poison the queue: the generator is done afterwards
  async function* boom() {
    yield "ok";
    throw new TypeError("inside");
  }
  const bm = boom();
  console.log("boomFirst=" + JSON.stringify(await bm.next()));
  const boomSecond = await bm.next().then(
    (r) => "value:" + String(r.value) + ":" + r.done,
    (e: any) => "rejected " + e.constructor.name,
  );
  console.log("boomSecond=" + boomSecond);
  console.log("boomThird=" + JSON.stringify(await bm.next()));

  // throw() that the body does not catch rejects the promise it answers
  const dt = ag();
  await dt.next();
  const outcome = await dt.throw(new Error("x")).then(
    () => "fulfilled",
    (e: any) => "rejected:" + e.constructor.name,
  );
  console.log("asyncThrowRejects=" + outcome + " afterwards=" + JSON.stringify(await dt.next()));

  // a body that catches turns the throw into the next value
  async function* catches() {
    try {
      yield "c1";
    } catch (e: any) {
      yield "caught:" + String(e);
    }
    yield "c2";
  }
  const ct = catches();
  await ct.next();
  console.log("asyncThrowCaught=" + JSON.stringify(await ct.throw("T")));
  console.log("asyncThrowCaughtNext=" + JSON.stringify(await ct.next()));

  // a sync iterable delegated with yield* inside an async generator
  async function* delegating() {
    yield* ["d1", "d2"];
    yield "own";
  }
  const got: string[] = [];
  for await (const v of delegating()) got.push(String(v));
  console.log("asyncDelegatesSync=" + got.join(","));

  // a for-await break closes the async generator; it is dead afterwards
  trace.length = 0;
  const sq = source("e");
  for await (const v of sq) {
    if (v === "e2") break;
  }
  console.log("forAwaitBreakTrace=" + trace.join("|"));
  console.log("forAwaitBreakDead=" + JSON.stringify(await sq.next()));

  // return() on a never-started async generator skips the body
  trace.length = 0;
  const sh = source("h");
  console.log("asyncReturnEarly=" + JSON.stringify(await sh.return("never")));
  console.log("asyncReturnEarlyTrace=" + JSON.stringify(trace.join("|")));

  console.log("end");
})().catch((e: any) => {
  console.log("unexpected=" + e.name + ":" + e.message);
});
