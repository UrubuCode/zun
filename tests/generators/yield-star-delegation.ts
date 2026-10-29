// yield* delegation: any iterable as delegate, the delegate's return value as the
// value of the yield* expression, forwarding of next / throw / return into the
// delegate, closing rules when the delegate lacks throw or return, recursive
// delegation, and consumer-side "take" patterns over infinite generators.

// Runs fn and reports how it ended: "no-throw" or the thrown constructor name.
function attempt(fn: () => unknown): string {
  try {
    fn();
    return "no-throw";
  } catch (e: any) {
    return e.constructor.name;
  }
}

const trace: string[] = [];

// ── basics: the delegate's return value is the yield* result ────────────────
function* inner() {
  yield 1;
  yield 2;
  return 9;
}
function* outer() {
  const r = yield* inner();
  yield r + 1;
  return r + 2;
}
const g = outer();
console.log(
  "steps=" +
    [g.next(), g.next(), g.next(), g.next()].map((x) => String(x.value) + ":" + x.done).join("|"),
);

function* counter() {
  yield 1;
  yield 2;
  return "done";
}
function* delegator() {
  const result = yield* counter();
  yield "result:" + result;
}
console.log("delegatorValues=" + [...delegator()].join(","));

function* range(start: number, count: number) {
  for (let i = 0; i < count; i++) yield start + i;
}
function* mixed() {
  yield 0;
  yield* range(10, 3);
  yield 20;
  yield* [30, 31];
  yield* "ab";
  yield* new Set([40]);
  yield* new Map([[50, "v"]]).keys();
  const arrayReturn = yield* [7];
  yield "arrayStar=" + String(arrayReturn);
}
console.log("mixed=" + [...mixed()].join(","));

// a delegate that is itself a generator object, returning a value the outer uses
function* producer() {
  const first = yield "p1";
  return "p-received:" + first;
}
function* consumerOfProducer() {
  const got = yield* producer();
  return "consumer:" + got;
}
const cp = consumerOfProducer();
console.log("cp1=" + JSON.stringify(cp.next()));
console.log("cp2=" + JSON.stringify(cp.next("hello")));

// ── recursive delegation: tree traversal ────────────────────────────────────
type Tree = { val: number; children: Tree[] };
function* traverse(node: Tree): Generator<number> {
  yield node.val;
  for (const child of node.children) yield* traverse(child);
}
const tree: Tree = {
  val: 1,
  children: [
    { val: 2, children: [{ val: 4, children: [] }, { val: 5, children: [] }] },
    { val: 3, children: [{ val: 6, children: [] }] },
  ],
};
console.log("traverse=" + [...traverse(tree)].join(","));

// ── next(v), throw(e), return(v) reach the INNER generator while delegating ──
function* inner1() {
  const a = yield "i1";
  trace.push("inner got " + a);
  const b = yield "i2";
  trace.push("inner got " + b);
  return "innerReturn";
}
function* outer1() {
  const before = yield "o1";
  trace.push("outer got " + before);
  const fromStar = yield* inner1();
  trace.push("star produced " + fromStar);
  const after = yield "o2";
  trace.push("outer got " + after);
  return "outerReturn";
}
const g1 = outer1();
const steps1 = [
  g1.next("ignored-first"),
  g1.next("A"),
  g1.next("B"),
  g1.next("C"),
  g1.next("D"),
  g1.next("E"),
];
console.log("forward=" + steps1.map((r) => String(r.value) + ":" + r.done).join("|"));
console.log("forwardTrace=" + trace.join("|"));

// throw() lands inside the INNER generator, which catches it and carries on
trace.length = 0;
function* inner2() {
  try {
    yield "i1";
    trace.push("inner2 not reached");
  } catch (e: any) {
    trace.push("inner2 caught " + e.constructor.name);
    yield "i-recovered";
  } finally {
    trace.push("inner2 finally");
  }
  return "innerDone";
}
function* outer2() {
  try {
    const v = yield* inner2();
    trace.push("star produced " + v);
  } catch (e: any) {
    trace.push("outer2 caught " + e.constructor.name);
  }
  yield "o-last";
  return "outerDone";
}
const g2 = outer2();
console.log("throwInner=" + JSON.stringify(g2.next()));
console.log("throwInnerRecovered=" + JSON.stringify(g2.throw(new RangeError("boom"))));
console.log("throwInnerNext=" + JSON.stringify(g2.next()));
console.log("throwInnerDone=" + JSON.stringify(g2.next()));
console.log("throwInnerTrace=" + trace.join("|"));

// an inner generator with no catch lets the throw escape to the OUTER catch
trace.length = 0;
function* inner3() {
  try {
    yield "i1";
  } finally {
    trace.push("inner3 finally");
  }
}
function* outer3() {
  try {
    yield* inner3();
  } catch (e: any) {
    trace.push("outer3 caught " + e.constructor.name);
    yield "o-recovered";
  }
  return "outer3Done";
}
const g3 = outer3();
console.log("escape1=" + JSON.stringify(g3.next()));
console.log("escape2=" + JSON.stringify(g3.throw(new TypeError("t"))));
console.log("escape3=" + JSON.stringify(g3.next()));
console.log("escapeTrace=" + trace.join("|"));

// return() closes the inner generator first; both finally blocks run
trace.length = 0;
function* inner4() {
  try {
    yield "i1";
    yield "i2";
  } finally {
    trace.push("inner4 finally");
  }
}
function* outer4() {
  try {
    yield* inner4();
  } finally {
    trace.push("outer4 finally");
  }
}
const g4 = outer4();
console.log("ret1=" + JSON.stringify(g4.next()));
console.log("ret2=" + JSON.stringify(g4.return("closed")));
console.log("ret3=" + JSON.stringify(g4.next()));
console.log("retTrace=" + trace.join("|"));

// ── nested delegation forwards through every level ──────────────────────────
trace.length = 0;
function* deep() {
  const x = yield "d1";
  trace.push("deep got " + x);
  return "deepReturn";
}
function* mid() {
  return "mid:" + (yield* deep());
}
function* top() {
  return "top:" + (yield* mid());
}
const gt = top();
console.log("nest1=" + JSON.stringify(gt.next()));
console.log("nest2=" + JSON.stringify(gt.next("through")));
console.log("nestTrace=" + trace.join("|"));

// ── delegates that lack throw / return: the closing rules ───────────────────
// makeDelegate yields tag0, tag1, tag2 and records every protocol call.
function makeDelegate(tag: string, hasThrow: boolean, hasReturn: boolean, returnAnswer?: unknown) {
  let i = 0;
  const it: any = {
    [Symbol.iterator]() {
      return this;
    },
    next() {
      trace.push(tag + ".next");
      return i < 3 ? { done: false, value: tag + i++ } : { done: true, value: undefined };
    },
  };
  if (hasThrow) {
    it.throw = function (e: any) {
      trace.push(tag + ".throw");
      return { done: false, value: "handled:" + String(e) };
    };
  }
  if (hasReturn) {
    it.return = function (v: any) {
      trace.push(tag + ".return(" + String(v) + ")");
      return returnAnswer === undefined ? { done: true, value: "closed-" + tag } : returnAnswer;
    };
  }
  return it;
}

// throw() into a delegate with NO throw: it is closed (its return is called with
// no argument), then a TypeError is raised at the yield* site
trace.length = 0;
function* outerA() {
  try {
    yield* makeDelegate("A", false, true);
  } finally {
    trace.push("outerA finally");
  }
}
const ga = outerA();
const firstA = ga.next();
console.log("noThrowFirst=" + firstA.value + ":" + firstA.done);
console.log("noThrowOutcome=" + attempt(() => ga.throw(new Error("x"))));
console.log("noThrowTrace=" + trace.join("|"));
console.log("noThrowAfter=" + JSON.stringify(ga.next()));

// same, with neither throw nor return: still a TypeError, nothing to close
trace.length = 0;
function* outerB() {
  yield* makeDelegate("B", false, false);
}
const gb = outerB();
gb.next();
console.log("bareThrowOutcome=" + attempt(() => gb.throw(new Error("x"))));
console.log("bareThrowTrace=" + trace.join("|"));
console.log("bareThrowAfter=" + JSON.stringify(gb.next()));

// return() into a delegate WITHOUT return: the outer just finishes with the value
trace.length = 0;
function* outerC() {
  try {
    yield* makeDelegate("C", true, false);
  } finally {
    trace.push("outerC finally");
  }
}
const gc = outerC();
gc.next();
console.log("returnNoReturnMethod=" + JSON.stringify(gc.return("R")));
console.log("returnNoReturnTrace=" + trace.join("|"));

// return() into a delegate WITH return: it receives the value, and the value of
// its answer becomes the outer completion value (not the value that was sent)
trace.length = 0;
function* outerD() {
  yield* makeDelegate("D", true, true);
}
const gd = outerD();
gd.next();
console.log("returnWithMethod=" + JSON.stringify(gd.return("RD")));
console.log("returnWithMethodTrace=" + trace.join("|"));

// a delegate whose return answers a NON-object: TypeError on the return path
trace.length = 0;
function* outerE() {
  yield* makeDelegate("E", true, true, 42);
}
const ge = outerE();
ge.next();
console.log("badReturnAnswer=" + attempt(() => ge.return("X")));
console.log("badReturnTrace=" + trace.join("|"));

// contrast: a generator delegate HAS throw, so the throw is forwarded and caught
trace.length = 0;
function* innerF() {
  try {
    yield "f0";
    yield "f1";
  } catch (e: any) {
    trace.push("innerF caught " + String(e));
    yield "recovered";
  }
}
function* outerF() {
  const r = yield* innerF();
  trace.push("delegationValue=" + String(r));
  yield "after";
}
const gf = outerF();
console.log("fFirst=" + gf.next().value);
console.log("fThrown=" + gf.throw("BOOM").value);
console.log("fNext=" + JSON.stringify(gf.next()));
console.log("fTrace=" + trace.join("|"));

// the outer generator completed by a TypeError answers done afterwards
console.log("afterTypeErrorNext=" + JSON.stringify(gb.next()));
console.log("afterTypeErrorReturn=" + JSON.stringify(gb.return("Z")));

// ── take patterns over infinite generators ──────────────────────────────────
function* naturals() {
  let n = 1;
  try {
    while (true) yield n++;
  } finally {
    trace.push("naturals closed");
  }
}
function* takeN<T>(src: Iterable<T>, n: number): Generator<T, number, undefined> {
  let taken = 0;
  if (n <= 0) return taken;
  for (const v of src) {
    yield v;
    taken++;
    if (taken >= n) return taken;
  }
  return taken;
}
function* mapG<T, U>(src: Iterable<T>, f: (x: T) => U): Generator<U> {
  for (const x of src) yield f(x);
}
function* filterG<T>(src: Iterable<T>, p: (x: T) => boolean): Generator<T> {
  for (const x of src) if (p(x)) yield x;
}

trace.length = 0;
const t3 = takeN(naturals(), 3);
console.log("take1=" + JSON.stringify(t3.next()));
console.log("take2=" + JSON.stringify(t3.next()));
console.log("take3=" + JSON.stringify(t3.next()));
console.log("take4=" + JSON.stringify(t3.next()));
console.log("takeTrace=" + trace.join("|"));

trace.length = 0;
console.log("takeZero=" + JSON.stringify(takeN(naturals(), 0).next()) + " trace=" + trace.join("|"));

function* pipeline() {
  yield* takeN(naturals(), 5);
}
trace.length = 0;
console.log("pipeline=" + [...pipeline()].join(",") + " trace=" + trace.join("|"));

// filter then map then take: closing propagates down the whole chain
trace.length = 0;
const chain = takeN(mapG(filterG(naturals(), (x) => x % 2 === 0), (x) => x * x), 4);
console.log("chain=" + [...chain].join(","));
console.log("chainTrace=" + trace.join("|"));

// an external return() closes an infinite source mid-stream
trace.length = 0;
const nat = naturals();
nat.next();
nat.next();
console.log("externalReturn=" + JSON.stringify(nat.return("stop")));
console.log("externalReturnTrace=" + trace.join("|"));
console.log("externalReturnAfter=" + JSON.stringify(nat.next()));

// a delegated, lazily produced sequence is still lazy: only the pulled prefix runs
trace.length = 0;
function* lazyDelegate() {
  yield* naturals();
}
const ld = lazyDelegate();
console.log("lazy=" + ld.next().value + "," + ld.next().value);
console.log("lazyTrace=" + trace.join("|"));
ld.return(undefined);
console.log("lazyClosed=" + trace.join("|"));

console.log("end");
