// Iteration protocol contract: the next()/done/value handshake, a fresh iterator
// per consumer, done coercion, malformed results, Symbol.iterator lookup rules,
// and Iterator.from wrapping. One deterministic case per output line.

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

// Bounded manual driver: never loops forever, even on a broken protocol.
function drive(it: any, cap: number): string {
  const parts: string[] = [];
  for (let k = 0; k < cap; k++) {
    const r = it.next();
    if (r.done) {
      parts.push("done");
      return parts.join(",");
    }
    parts.push(String(r.value));
  }
  return parts.join(",") + ",capped";
}

// ---- 1. a fresh iterator is requested by every consumer ----
let created = 0;
const counter: any = {
  [Symbol.iterator]() {
    created++;
    let i = 0;
    return {
      next() {
        i++;
        return i <= 3 ? { value: i * 10, done: false } : { value: undefined, done: true };
      },
    };
  },
};
out("spreadCounter", [...counter].join(","));
const forOfVals: number[] = [];
for (const v of counter) forOfVals.push(v);
out("forOfCounter", forOfVals.join(","));
out("spreadAgain", [...counter].join(","));
out("createdCount", String(created));

// ---- 2. done:true on the first pull means zero iterations; its value is dropped ----
const emptyIt: any = {
  [Symbol.iterator]() {
    return { next() { return { value: 99, done: true }; } };
  },
};
let emptyCount = 0;
for (const _v of emptyIt) emptyCount++;
out("emptyFirstDone", String(emptyCount));
out("emptySpread", String([...emptyIt].length));

const trailing: any = {
  [Symbol.iterator]() {
    let i = 0;
    return {
      next() {
        i++;
        return i === 1 ? { value: "a", done: false } : { value: "IGNORED", done: true };
      },
    };
  },
};
out("trailingValueDropped", [...trailing].join(",") + "|len=" + [...trailing].length);

// ---- 3. done is tested by truthiness, and a missing done is falsy ----
const truthyNum: any = {
  [Symbol.iterator]() {
    let i = 0;
    return { next() { i++; return { value: i, done: i > 2 ? 1 : 0 }; } };
  },
};
out("doneTruthyNumber", [...truthyNum].join(","));

const strDone: any = {
  [Symbol.iterator]() {
    let i = 0;
    return {
      next() {
        i++;
        if (i === 1) return { value: "a", done: "" };
        if (i === 2) return { value: "b", done: 0 };
        return { value: "c", done: "yes" };
      },
    };
  },
};
out("doneTruthyString", [...strDone].join(","));

const missingDone: any = {
  [Symbol.iterator]() {
    let i = 0;
    return { next() { i++; return i <= 2 ? { value: i } : { done: true }; } };
  },
};
out("missingDoneKeepsGoing", [...missingDone].join(","));

// ---- 4. next() calls are exactly values + 1 ----
let nextCalls = 0;
const counted: any = {
  [Symbol.iterator]() {
    let i = 0;
    return {
      next() {
        nextCalls++;
        i++;
        return i <= 4 ? { value: i, done: false } : { value: undefined, done: true };
      },
    };
  },
};
let countedValues = 0;
for (const _v of counted) countedValues++;
out("countedValues", countedValues + "|nextCalls=" + nextCalls);

const manualSrc: any = {
  [Symbol.iterator]() {
    let i = 0;
    return { next() { i++; return i <= 1 ? { value: "m", done: false } : { value: "post", done: true }; } };
  },
};
out("manualDrive", drive(manualSrc[Symbol.iterator](), 5));

const selfIter: any = {
  n: 0,
  next() {
    this.n++;
    return this.n <= 2 ? { value: "s" + this.n, done: false } : { value: undefined, done: true };
  },
  [Symbol.iterator]() {
    return this;
  },
};
out("selfIteratorSpread", [...selfIter].join(","));

// ---- 5. malformed results: a non-object next() result is a TypeError before done is read ----
let pulls = 0;
function makeBad(result: any): any {
  return {
    [Symbol.iterator]() {
      return {
        next() {
          if (++pulls > 50) return { value: undefined, done: true };
          return result;
        },
      };
    },
  };
}
const badKinds: any[] = [5, "s", true, null, undefined, 0, ""];
for (const bad of badKinds) {
  pulls = 0;
  const label = "badNext_" + (bad === null ? "null" : bad === undefined ? "undef" : typeof bad + "_" + String(bad));
  out(label, attempt(() => [...makeBad(bad)].length) + " pulls=" + pulls);
}
pulls = 0;
out("badNextArrayFrom", attempt(() => Array.from(makeBad(5))) + " pulls=" + pulls);
pulls = 0;
out("badNextNewSet", attempt(() => new Set(makeBad(5)).size) + " pulls=" + pulls);
pulls = 0;
out("badNextForOf", attempt(() => { for (const _v of makeBad(5)) { /* never reached */ } return 0; }) + " pulls=" + pulls);
pulls = 0;
out("badNextDestructure", attempt(() => { const [a] = makeBad(5); return a; }) + " pulls=" + pulls);
pulls = 0;
out("badNextYieldStar", attempt(() => { function* g() { yield* makeBad(5); } return [...g()].length; }) + " pulls=" + pulls);

// ---- 6. done and value are plain property reads, performed once per pull ----
let doneReads = 0;
let i3 = 0;
const getterIt: any = {
  next() {
    return { value: i3++, get done() { doneReads++; return i3 > 3; } };
  },
};
const gd: number[] = [];
for (let k = 0; k < 20; k++) {
  const r = getterIt.next();
  if (r.done) break;
  gd.push(r.value);
}
out("doneGetter", gd.join(",") + " reads=" + doneReads);

let throwPulls = 0;
const throwingIt: any = {
  next() {
    throwPulls++;
    return { value: 1, get done(): boolean { throw new RangeError("boom"); } };
  },
};
out("doneGetterThrows", attempt(() => {
  for (let k = 0; k < 20; k++) {
    const r = throwingIt.next();
    if (r.done) break;
  }
  return "no-throw";
}) + " pulls=" + throwPulls);

// ---- 7. Symbol.iterator and next must be callable, and must answer objects ----
const nonCallable: any = { [Symbol.iterator]: 3 };
out("nonCallableSpread", attempt(() => [...nonCallable]));
out("nonCallableArrayFrom", attempt(() => Array.from(nonCallable)));
out("nonCallableForOf", attempt(() => { for (const _x of nonCallable) { /* */ } return 0; }));

const primIter: any = { [Symbol.iterator]() { return 7; } };
out("primitiveIteratorSpread", attempt(() => [...primIter]));
const nullIter: any = { [Symbol.iterator]() { return null; } };
out("nullIteratorSpread", attempt(() => [...nullIter]));

const noNext: any = { [Symbol.iterator]() { return {}; } };
out("missingNextSpread", attempt(() => [...noNext]));
const nonCallableNext: any = { [Symbol.iterator]() { return { next: 42 }; } };
out("nonCallableNextSpread", attempt(() => [...nonCallableNext]));

// ---- 8. where Symbol.iterator is looked up: the prototype chain, shadowing, late edits ----
const parent: any = {
  [Symbol.iterator]() {
    let i = 0;
    return { next() { i++; return i <= 3 ? { value: "p" + i, done: false } : { value: undefined, done: true }; } };
  },
};
const child = Object.create(parent);
out("inheritedIterator", [...child].join(","));
out("ownOnChild", String(Object.prototype.hasOwnProperty.call(child, Symbol.iterator)));
out("ownOnParent", String(Object.prototype.hasOwnProperty.call(parent, Symbol.iterator)));

const shadower = Object.create(parent);
shadower[Symbol.iterator] = function () {
  let i = 0;
  return { next() { i++; return i <= 2 ? { value: "own" + i, done: false } : { value: undefined, done: true }; } };
};
out("shadowed", [...shadower].join(","));
out("parentUnaffected", [...child].join(","));
out("threeLevelsInherited", [...Object.create(Object.create(parent))].join(","));

const late: any = { a: 1 };
const beforeAdd = attempt(() => [...late]);
late[Symbol.iterator] = function () {
  let done = false;
  return {
    next() {
      if (done) return { value: undefined, done: true };
      done = true;
      return { value: "late", done: false };
    },
  };
};
out("lateAddBefore", beforeAdd);
out("lateAddAfter", [...late].join(","));
late[Symbol.iterator] = undefined;
out("lateRemoveAfter", attempt(() => [...late]));

class Range {
  lo: number;
  hi: number;
  constructor(lo: number, hi: number) {
    this.lo = lo;
    this.hi = hi;
  }
  [Symbol.iterator]() {
    let cur = this.lo;
    const hi = this.hi;
    return {
      next() {
        return cur <= hi ? { value: cur++, done: false } : { value: undefined, done: true };
      },
    };
  }
}
class RevRange extends Range {
  [Symbol.iterator]() {
    let cur = this.hi;
    const lo = this.lo;
    return {
      next() {
        return cur >= lo ? { value: cur--, done: false } : { value: undefined, done: true };
      },
    };
  }
}
out("classRange", [...new Range(1, 4)].join(","));
out("classIteratorOnProto", String(Object.prototype.hasOwnProperty.call(new Range(1, 2), Symbol.iterator)));
out("subclassOverride", [...new RevRange(1, 4)].join(","));

const plainArr = [1, 2];
out("arrayIteratorOwn", String(Object.prototype.hasOwnProperty.call(plainArr, Symbol.iterator)));
out("arrayIteratorFromProto", String(plainArr[Symbol.iterator] === Array.prototype[Symbol.iterator]));
const arrChild: any = Object.create([7, 8, 9]);
out("arrayProtoChildSpread", [...arrChild].join(","));

// ---- 9. Iterator.from: wrap a next-only object, adopt an iterable, keep built-ins ----
const trace: string[] = [];
const bare: any = {
  i: 0,
  next() {
    this.i++;
    trace.push("next" + this.i);
    return this.i <= 3 ? { value: this.i * 100, done: false } : { value: undefined, done: true };
  },
};
out("bareHasHelpersBefore", typeof bare.map);
const wrapped: any = Iterator.from(bare);
out("wrapperIsNew", String(wrapped !== bare));
out("wrapperHasHelpers", typeof wrapped.map);
out("wrapperMapToArray", wrapped.map((x: number) => x + 1).toArray().join(","));
out("bareTrace", trace.join(","));

trace.length = 0;
const iterable: any = {
  [Symbol.iterator]() {
    trace.push("symbolIteratorCalled");
    let k = 0;
    return { next() { k++; return k <= 2 ? { value: "v" + k, done: false } : { value: undefined, done: true }; } };
  },
};
const fromIter: any = Iterator.from(iterable);
out("fromIterableTrace", trace.join(","));
out("fromIterableValues", fromIter.toArray().join(","));

const arrIt: any = [1, 2][Symbol.iterator]();
out("fromArrayIteratorIdentity", String(Iterator.from(arrIt) === arrIt));
const strIt: any = "ab"[Symbol.iterator]();
out("fromStringIteratorIdentity", String(Iterator.from(strIt) === strIt));
const mapIt: any = new Map([["k", 1]])[Symbol.iterator]();
out("fromMapIteratorIdentity", String(Iterator.from(mapIt) === mapIt));
function* oneGen() { yield 1; }
const genIt: any = oneGen();
out("fromGeneratorIdentity", String(Iterator.from(genIt) === genIt));

const w5: any = Iterator.from([5, 6, 7]);
out("fromArrayWrapsIterator", Array.isArray(w5) + "|take2=" + w5.take(2).toArray().join(","));

trace.length = 0;
const closable: any = {
  i: 0,
  next() {
    this.i++;
    trace.push("next" + this.i);
    return { value: this.i, done: false };
  },
  return(v: any) {
    trace.push("return:" + String(v));
    return { value: v, done: true };
  },
};
const w6: any = Iterator.from(closable);
out("wrapNext", String(w6.next().value));
out("wrapReturn", JSON.stringify(w6.return("bye")));
out("wrapReturnTrace", trace.join(","));

const noReturn: any = { i: 0, next() { this.i++; return { value: this.i, done: false }; } };
const w7: any = Iterator.from(noReturn);
w7.next();
out("wrapNoReturn", JSON.stringify(w7.return()));

trace.length = 0;
closable.i = 0;
out("takeClosesWrapped", Iterator.from(closable).take(2).toArray().join(","));
out("takeClosesWrappedTrace", trace.join(","));

out("fromNumber", attempt(() => Iterator.from(42 as any)));
out("fromNull", attempt(() => Iterator.from(null as any)));
out("fromEmptyObjectBuilds", (() => { try { Iterator.from({} as any); return "built"; } catch (e: any) { return "threw:" + e.name; } })());
out("fromEmptyObjectNext", attempt(() => (Iterator.from({} as any) as any).next()));
out("fromStringIterable", (Iterator.from("abc" as any) as any).toArray().join("-"));

// ---- 10. spread of a custom iterable in every spread position ----
const spreadLog: string[] = [];
function logged(vals: any[], tag: string): any {
  return {
    [Symbol.iterator]() {
      let i = 0;
      return {
        next() {
          spreadLog.push(tag + ":next" + i);
          return i < vals.length ? { value: vals[i++], done: false } : { value: undefined, done: true };
        },
      };
    },
  };
}
const literalSpread = [...logged([1, 2, 3], "a")];
out("spreadLiteral", literalSpread.join(",") + " calls=" + spreadLog.length);
spreadLog.length = 0;
out("spreadMiddle", [0, ...logged([1, 2], "b"), 9].join(",") + " " + spreadLog.join(","));
spreadLog.length = 0;
out("spreadTwoInOneLiteral", [...logged(["x"], "c"), ...logged(["y", "z"], "d")].join(",") + " calls=" + spreadLog.length);
spreadLog.length = 0;
const sum3 = (a: number, b: number, c: number) => a + b + c;
out("spreadCallArgs", String(sum3(...logged([1, 2, 3], "e"))) + " calls=" + spreadLog.length);
spreadLog.length = 0;
out("spreadEmptyIterable", [...logged([], "f")].length + " " + spreadLog.join(","));
spreadLog.length = 0;
out("drainNextCalls", [...logged([1, 2, 3, 4, 5], "g")].length + " nextCalls=" + spreadLog.length);
const shared = { k: 1 };
out("spreadKeepsIdentity", [...logged([shared, shared], "h")].map((x) => x === shared).join(","));
out("spreadKeepsFalsy", [...logged([undefined, null, 0], "j")].map(String).join(","));
out("spreadOrderKept", [...logged([3, 1, 3, 2], "i")].join(","));
out("setFromCustomIterable", String(new Set(logged([3, 1, 3, 2], "k")).size));
