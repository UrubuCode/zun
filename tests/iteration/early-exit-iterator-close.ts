// IteratorClose: when break, label-break, throw, destructuring and rest/spread
// leave a loop or pattern early. Covers return() presence/absence, getter-once
// lookup, non-callable and non-object return results, throwing return, and
// for-await awaiting an asynchronous return().

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

// Finite or infinite traced source. `extra` is merged onto the iterator object,
// so it can supply return (absent, callable, null, non-callable, ...).
function source(log: string[], tag: string, limit: number, extra: Record<string, unknown> = {}): any {
  return {
    [Symbol.iterator]() {
      let n = 0;
      return {
        next() {
          n++;
          log.push(tag + ":next" + n);
          return n <= limit ? { value: n, done: false } : { value: undefined, done: true };
        },
        ...extra,
      };
    },
  };
}

function breakAt(it: any, at: number): string {
  for (const v of it) {
    if (v === at) break;
  }
  return "completed";
}

function throwAt(it: any, at: number, err: Error): string {
  for (const v of it) {
    if (v === at) throw err;
  }
  return "completed";
}

function withReturn(log: string[], tag: string): Record<string, unknown> {
  return {
    return() {
      log.push(tag + ":return");
      return { done: true };
    },
  };
}

// ---- for-of: break, continue, normal completion ----
let log: string[] = [];
out("breakCallsReturn", breakAt(source(log, "a", Infinity, withReturn(log, "a")), 2) + " " + log.join(","));

log = [];
let kept: number[] = [];
for (const v of source(log, "b", 3, withReturn(log, "b"))) {
  if (v === 2) continue;
  kept.push(v);
}
out("continueNoClose", kept.join(",") + " " + log.join(","));

log = [];
out("breakWithoutReturn", breakAt(source(log, "c", Infinity), 2) + " " + log.join(","));

log = [];
out("returnNullIsAbsent", breakAt(source(log, "d", Infinity, { return: null }), 1) + " " + log.join(","));

log = [];
out("returnNotCallable", attempt(() => breakAt(source(log, "e", Infinity, { return: 42 }), 2)) + " " + log.join(","));

log = [];
out("returnNonObjectResult", attempt(() => breakAt(source(log, "f", Infinity, {
  return() { log.push("f:return"); return "notAnObject"; },
}), 1)) + " " + log.join(","));

log = [];
out("returnThrowsOnBreak", attempt(() => breakAt(source(log, "g", Infinity, {
  return() { log.push("g:return"); throw new EvalError("fromReturn"); },
}), 1)) + " " + log.join(","));

log = [];
out("bodyThrowWinsOverBadReturn", attempt(() => throwAt(source(log, "h", Infinity, {
  return() { log.push("h:return"); return "notAnObject"; },
}), 1, new RangeError("body"))) + " " + log.join(","));

log = [];
out("bodyThrowWinsOverThrowingReturn", attempt(() => throwAt(source(log, "i", Infinity, {
  return() { log.push("i:return"); throw new EvalError("fromReturn"); },
}), 1, new URIError("body"))) + " " + log.join(","));

log = [];
out("bodyThrowClosesFirst", attempt(() => throwAt(source(log, "j", Infinity, withReturn(log, "j")), 1, new Error("boom"))) + " " + log.join(","));

log = [];
out("returnResultValueDiscarded", breakAt(source(log, "k", Infinity, {
  return() { log.push("k:return"); return { value: "discarded", done: true }; },
}), 1) + " " + log.join(","));

// ---- return getter is read once per loop exit, and called with the iterator as receiver ----
const getLog: string[] = [];
let getReads = 0;
const getterIter: any = {
  i: 0,
  next() {
    this.i++;
    getLog.push("g:next" + this.i);
    return { value: this.i, done: false };
  },
  get return() {
    getReads++;
    return function (this: any) {
      getLog.push("g:call:" + (this === getterIter));
      return { done: true };
    };
  },
};
const getterIterable = { [Symbol.iterator]() { return getterIter; } };
breakAt(getterIterable, 2);
out("returnGetterOnce", getReads + " " + getLog.join(","));

// ---- labeled break across nested loops closes both, inner first ----
const nestLog: string[] = [];
const outerSrc = source(nestLog, "X", Infinity, withReturn(nestLog, "X"));
const innerSrc = source(nestLog, "Y", Infinity, withReturn(nestLog, "Y"));
outerLoop: for (const _a of outerSrc) {
  for (const b of innerSrc) {
    if (b === 2) break outerLoop;
  }
}
out("labeledBreakCloses", nestLog.join(","));

// Plain nested break closes only the inner iterator; the outer runs to completion.
const plainLog: string[] = [];
const outerPlain = source(plainLog, "O", 2, withReturn(plainLog, "O"));
const innerPlain = source(plainLog, "I", Infinity, withReturn(plainLog, "I"));
for (const _a of outerPlain) {
  for (const b of innerPlain) {
    if (b === 1) break;
  }
}
out("nestedBreakInnerOnly", plainLog.join(","));

// ---- destructuring pulls only what it binds, then closes if not exhausted ----
const dlog: string[] = [];
const inf = () => source(dlog, "d", Infinity, withReturn(dlog, "d"));
const [p1, p2] = inf();
out("destructTwoOfInfinite", p1 + "," + p2 + " " + dlog.join(","));

dlog.length = 0;
const [q1] = inf();
out("destructOneOfInfinite", q1 + " " + dlog.join(","));

dlog.length = 0;
const [] = inf();
out("destructEmptyPattern", dlog.join(","));

dlog.length = 0;
const [, h2] = inf();
out("destructHolePulls", h2 + " " + dlog.join(","));

dlog.length = 0;
const [e1, e2, e3] = source(dlog, "e", 2, withReturn(dlog, "e"));
out("destructExactOverFinite", e1 + "," + e2 + "," + String(e3) + " " + dlog.join(","));

dlog.length = 0;
const [r1, ...rr] = source(dlog, "r", 3, withReturn(dlog, "r"));
out("destructRestDrains", r1 + "|" + rr.join(",") + " " + dlog.join(","));

const undefLog: string[] = [];
const undefIter: any = {
  [Symbol.iterator]() {
    let n = 0;
    return {
      next() { n++; undefLog.push("u:next" + n); return { value: undefined, done: false }; },
      return() { undefLog.push("u:return"); return { done: true }; },
    };
  },
};
out("defaultThrowCloses", attempt(() => {
  const [x = (() => { throw new RangeError("default"); })()] = undefIter;
  return x;
}) + " " + undefLog.join(","));

out("defaultsOnlyForUndefined", (() => {
  const [d1 = "DEF", d2 = "DEF", d3 = "DEF"] = [undefined, null, 5] as any[];
  return [d1, d2, d3].map(String).join(",");
})());

const nestLog2: string[] = [];
const nested: any = {
  [Symbol.iterator]() {
    let i = 0;
    return {
      next() { i++; nestLog2.push("outer" + i); return { value: [i, i * 100], done: false }; },
      return() { nestLog2.push("outerReturn"); return { done: true }; },
    };
  },
};
const [[n1, n2]] = nested;
out("nestedDestructureLazy", n1 + "," + n2 + " " + nestLog2.join(","));

// ---- spread always drains to done, and never calls return ----
const slog: string[] = [];
out("spreadToDone", [...source(slog, "s", 2, withReturn(slog, "s"))].join(",") + " " + slog.join(","));

// ---- async: for-await break awaits an asynchronous return() ----
async function asyncCases(): Promise<void> {
  const alog: string[] = [];
  const asyncIter = {
    [Symbol.asyncIterator]() {
      let n = 0;
      return {
        async next() {
          n++;
          alog.push("next" + n);
          return { value: n, done: false };
        },
        async return() {
          alog.push("return-start");
          await Promise.resolve();
          alog.push("return-end");
          return { done: true };
        },
      };
    },
  };
  let sum = 0;
  for await (const x of asyncIter) {
    sum += x;
    if (x === 2) break;
  }
  out("forAwaitBreakAwaitsReturn", sum + " " + alog.join(","));

  const tlog: string[] = [];
  const throwingAsync = {
    [Symbol.asyncIterator]() {
      return {
        async next() { tlog.push("t:next"); return { value: 1, done: false }; },
        async return() { tlog.push("t:return"); return { done: true }; },
      };
    },
  };
  try {
    for await (const _x of throwingAsync) {
      throw new RangeError("async body");
    }
  } catch (e: any) {
    out("forAwaitThrowCloses", e.name + " " + tlog.join(","));
  }
}

asyncCases().then(() => out("asyncCasesDone", "yes"));
