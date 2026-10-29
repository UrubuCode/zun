// Generator protocol: next() argument passing, return() and throw() at every
// suspension point, finally semantics, completion records, and re-entrancy.

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

// ── next() and sent values ──────────────────────────────────────────────────
function* simple() {
  yield 1;
  yield 2;
  return 3;
}
const s = simple();
console.log("simple1=" + JSON.stringify(s.next()));
console.log("simple2=" + JSON.stringify(s.next()));
console.log("simple3=" + JSON.stringify(s.next()));
console.log("simple4=" + JSON.stringify(s.next()));
console.log("simpleIterSelf=" + (s[Symbol.iterator]() === s));

function* accumulator() {
  let total = 0;
  while (true) {
    const add = yield total;
    if (add === undefined) return "total:" + total;
    total = total + add;
  }
}
const acc = accumulator();
console.log("acc0=" + JSON.stringify(acc.next("ignored")));
console.log("acc1=" + JSON.stringify(acc.next(5)));
console.log("acc2=" + JSON.stringify(acc.next(10)));
console.log("acc3=" + JSON.stringify(acc.next()));
console.log("acc4=" + JSON.stringify(acc.next(99)));

function* twoWay() {
  const a = yield "first";
  const b = yield "second:" + a;
  return "got:" + a + "/" + b;
}
const tw = twoWay();
console.log("twoWay1=" + JSON.stringify(tw.next("x")));
console.log("twoWay2=" + JSON.stringify(tw.next("A")));
console.log("twoWay3=" + JSON.stringify(tw.next("B")));

// ── guarded body: throw() and return() before, during and after start ────────
function* guarded(tag: string) {
  trace.push(tag + ":entered");
  try {
    trace.push(tag + ":try");
    yield tag + "-1";
  } catch (e: any) {
    trace.push(tag + ":caught " + String(e));
    yield tag + "-recovered";
  } finally {
    trace.push(tag + ":finally");
  }
  return tag + "-end";
}

// throw() before the first next(): the body never runs, so nothing catches it
const before = guarded("A");
console.log("throwBeforeStart=" + attempt(() => before.throw("E0")));
console.log("throwBeforeStartTrace=" + JSON.stringify(trace.join(",")));
console.log("throwBeforeStartNext=" + JSON.stringify(before.next()));

// return() before the first next(): completes at once, body never entered
trace.length = 0;
const early = guarded("B");
console.log("returnBeforeStart=" + JSON.stringify(early.return("RB")));
console.log("returnBeforeStartTrace=" + JSON.stringify(trace.join(",")));
console.log("returnBeforeStartNext=" + JSON.stringify(early.next()));

// throw() after the first next(): the body's catch receives it
trace.length = 0;
const mid = guarded("C");
console.log("midFirst=" + JSON.stringify(mid.next()));
console.log("midThrow=" + JSON.stringify(mid.throw("E1")));
console.log("midTrace=" + trace.join(","));
console.log("midNext=" + JSON.stringify(mid.next()));
console.log("midTraceFinal=" + trace.join(","));

// throw() with no argument: the catch sees undefined
trace.length = 0;
const und = guarded("U");
und.next();
console.log("throwUndefined=" + JSON.stringify(und.throw(undefined)));
console.log("throwUndefinedTrace=" + trace.join(","));

// throw() on a completed generator throws straight back out
trace.length = 0;
const finished = guarded("D");
finished.next();
finished.next();
console.log("completedThrow=" + attempt(() => finished.throw("E2")) + " trace=" + JSON.stringify(trace.join(",")));
console.log("completedReturn=" + JSON.stringify(finished.return("RD")));

// an uncaught throw still runs finally, then propagates, then the generator is dead
function* noCatch() {
  try {
    yield 1;
    yield 2;
  } finally {
    trace.push("noCatch:finally");
  }
}
trace.length = 0;
const nc = noCatch();
nc.next();
let ncOutcome = "none";
try {
  nc.throw(new RangeError("E3"));
  ncOutcome = "returned";
} catch (e: any) {
  ncOutcome = "threw " + e.name + ":" + e.message;
}
console.log("uncaughtThrow=" + ncOutcome + " trace=" + trace.join(","));
console.log("uncaughtAfter=" + JSON.stringify(nc.next()));

// ── return() through try / finally ──────────────────────────────────────────
function* plainFinally() {
  try {
    yield "a";
    yield "b";
  } finally {
    trace.push("plain:finally");
  }
  trace.push("plain:unreached");
}
trace.length = 0;
const pf = plainFinally();
pf.next();
console.log("plainReturn=" + JSON.stringify(pf.return("asked")));
console.log("plainNext=" + JSON.stringify(pf.next()));
console.log("plainTrace=" + trace.join(","));

// a finally that YIELDS suspends the return; the return value survives
function* yieldingFinally() {
  try {
    yield "a";
  } finally {
    trace.push("yf:start");
    yield "cleanup";
    trace.push("yf:end");
  }
}
trace.length = 0;
const yf = yieldingFinally();
yf.next();
console.log("yfReturn=" + JSON.stringify(yf.return("R")));
console.log("yfNext=" + JSON.stringify(yf.next()));
console.log("yfNext2=" + JSON.stringify(yf.next()));
console.log("yfTrace=" + trace.join(","));

// a finally that RETURNS replaces the requested value
function* overriding() {
  try {
    yield "a";
  } finally {
    return "fromFinally";
  }
}
const ov = overriding();
ov.next();
console.log("overrideReturn=" + JSON.stringify(ov.return("asked")));
console.log("overrideNext=" + JSON.stringify(ov.next()));

// a finally that THROWS replaces the completion with a throw
function* throwingFinally() {
  try {
    yield "a";
  } finally {
    throw new RangeError("fromFinally");
  }
}
const tf = throwingFinally();
tf.next();
let tfOutcome = "returned";
try {
  tf.return("asked");
} catch (e: any) {
  tfOutcome = "threw " + e.name + ":" + e.message;
}
console.log("throwingFinally=" + tfOutcome);
console.log("throwingFinallyNext=" + JSON.stringify(tf.next()));

// nested finally blocks unwind inside out; a catch is NOT entered by return()
function* nestedFinally() {
  try {
    try {
      yield "deep";
    } finally {
      trace.push("inner");
    }
  } catch (e) {
    trace.push("catch-entered");
  } finally {
    trace.push("outer");
  }
}
trace.length = 0;
const nf = nestedFinally();
nf.next();
console.log("nestedReturn=" + JSON.stringify(nf.return("v")));
console.log("nestedTrace=" + trace.join(","));

// return() on a generator that has not started skips the body and its finally
function* lazyBody() {
  trace.push("lazy:body");
  try {
    yield 1;
  } finally {
    trace.push("lazy:finally");
  }
}
trace.length = 0;
const lb = lazyBody();
console.log("lazyReturn=" + JSON.stringify(lb.return("early")));
console.log("lazyTrace=" + JSON.stringify(trace.join(",")));

// return() on a completed generator echoes the value quietly
function* oneShot() {
  yield "a";
}
const os = oneShot();
os.next();
os.next();
console.log("doneReturn=" + JSON.stringify(os.return("echo")));
console.log("doneReturnUndef=" + JSON.stringify(os.return(undefined)));

// ── a finally that runs on return() with a value ────────────────────────────
function* finallyWithValue() {
  try {
    yield 1;
  } finally {
    trace.push("fv:cleanup");
  }
}
trace.length = 0;
const fv = finallyWithValue();
fv.next();
console.log("finallyWithValueReturn=" + JSON.stringify(fv.return(7)));
console.log("finallyWithValueTrace=" + trace.join(","));

// ── driving another generator is fine; driving SELF is a TypeError ──────────
function* numbers() {
  yield "n1";
  yield "n2";
  yield "n3";
}
const target = numbers();
function* driver(t: Generator<string, void, unknown>) {
  yield "d:" + t.next().value;
  yield "d:" + t.next().value;
}
const dr = driver(target);
console.log("driven=" + dr.next().value + "," + dr.next().value);
console.log("targetLeft=" + JSON.stringify(target.next()));

// Re-entrancy: each probe runs while its own generator is executing.
function* probeNext(): Generator<string, void, unknown> {
  yield "next:" + attempt(() => (probeNextGen as any).next());
}
const probeNextGen = probeNext();
console.log("reNext=" + probeNextGen.next().value);

function* probeThrow(): Generator<string, void, unknown> {
  yield "throw:" + attempt(() => (probeThrowGen as any).throw(new Error("x")));
}
const probeThrowGen = probeThrow();
console.log("reThrow=" + probeThrowGen.next().value);

function* probeReturn(): Generator<string, void, unknown> {
  yield "return:" + attempt(() => (probeReturnGen as any).return("r"));
}
const probeReturnGen = probeReturn();
console.log("reReturn=" + probeReturnGen.next().value);

// re-entry through a callback the body invokes is the same situation
function* probeCallback(): Generator<string, void, unknown> {
  const poke = () => (probeCbGen as any).next();
  yield "callback:" + attempt(poke);
}
const probeCbGen = probeCallback();
console.log("reCallback=" + probeCbGen.next().value);

// re-entry from inside a finally that a return() is running
let fromFinally = "unset";
function* probeFinally(): Generator<string, void, unknown> {
  try {
    yield "a";
  } finally {
    fromFinally = attempt(() => (probeFinGen as any).next());
  }
}
const probeFinGen = probeFinally();
probeFinGen.next();
console.log("reFinallyReturn=" + JSON.stringify(probeFinGen.return("R")));
console.log("reFinallyInside=" + fromFinally);

// the refused call leaves the generator's position untouched: it resumes normally
function* refusedThenContinues(): Generator<string, string, unknown> {
  yield "one";
  yield "re:" + attempt(() => (refusedGen as any).next());
  return "tail";
}
const refusedGen = refusedThenContinues();
console.log("refusedA=" + refusedGen.next().value);
console.log("refusedB=" + refusedGen.next().value);
console.log("refusedC=" + JSON.stringify(refusedGen.next()));

// a COMPLETED generator is not running: next() is quiet, not an error
function* short() {
  yield 1;
}
const sh = short();
sh.next();
sh.next();
console.log("completedNext=" + JSON.stringify(sh.next()) + " completedReturn=" + JSON.stringify(sh.return("q")));

// a spread over a generator that pokes itself surfaces the same TypeError
let spreadProbe = "unset";
function* pokes(): Generator<number, void, unknown> {
  yield 1;
  spreadProbe = attempt(() => (pokesGen as any).next());
  yield 2;
}
const pokesGen = pokes();
console.log("spreadPokes=" + [...pokesGen].join(",") + " probe=" + spreadProbe);

console.log("end");
