// Generators as iterables and as protocol objects: next arguments, return and
// throw resumption through finally, yield* delegation (return value, forwarding,
// missing-throw closing), lazy generator pipelines, and async generators with
// for-await, error propagation, and return values.

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

// ---- generator as iterable ----
function* three() {
  yield 1;
  yield 2;
  return 99;
}
out("genForOf", (() => { const r: number[] = []; for (const v of three()) r.push(v); return r.join(","); })());
out("genSpreadExcludesReturn", [...three()].join(","));
const g3 = three();
out("genSelfIterable", String(g3[Symbol.iterator]() === g3));
const once = three();
out("genOneShot", [...once].join(",") + "|second=" + [...once].length);
out("genArrayFromMapFn", Array.from(three(), (v) => v * 10).join(","));
out("genTag", Object.prototype.toString.call(three()));
const g3r = three();
g3r.next();
g3r.next();
out("genReturnValueViaDone", JSON.stringify(g3r.next()));
out("genAfterDone", JSON.stringify(g3r.next()));

// ---- next(arg) becomes the value of the suspended yield; the first arg is ignored ----
function* exchange() {
  const a = yield "first";
  const b = yield a + ":second";
  return b + ":done";
}
const ex = exchange();
out("nextArgFirstIgnored", JSON.stringify(ex.next("ignored")));
out("nextArgA", JSON.stringify(ex.next("A")));
out("nextArgB", JSON.stringify(ex.next("B")));
out("nextArgAfterDone", JSON.stringify(ex.next("C")));

// ---- return(): runs finally, and a finally that yields keeps the generator open ----
const rlog: string[] = [];
function* withFinally() {
  try {
    yield 1;
    yield 2;
  } finally {
    rlog.push("finally");
  }
}
const rf = withFinally();
out("returnFirstNext", JSON.stringify(rf.next()));
out("returnValue", JSON.stringify(rf.return(9)));
out("returnRanFinally", rlog.join(","));
out("returnThenNext", JSON.stringify(rf.next()));

function* yieldInFinally() {
  try {
    yield "body";
  } finally {
    yield "cleanup";
  }
}
const yf = yieldInFinally();
out("finYieldBody", JSON.stringify(yf.next()));
out("finYieldReturnPauses", JSON.stringify(yf.return("done")));
out("finYieldResumes", JSON.stringify(yf.next()));
out("finYieldAfter", JSON.stringify(yf.next()));

// return() on a fresh generator never runs the body
function* fresh() {
  yield 1;
}
out("returnFreshGen", JSON.stringify(fresh().return(5)));
const fg = fresh();
out("throwFreshGen", attempt(() => fg.throw(new RangeError("early"))));
out("freshAfterThrow", JSON.stringify(fg.next()));

// ---- throw(): resumes at the suspended yield, catchable; finally return overrides ----
function* guarded() {
  try {
    yield "ready";
  } catch (e: any) {
    yield "caught:" + e.message;
  }
  return "done";
}
const gt = guarded();
out("throwFirst", JSON.stringify(gt.next()));
out("throwCaught", JSON.stringify(gt.throw(new Error("x"))));
out("throwThenNext", JSON.stringify(gt.next()));

function* overrideFinally() {
  try {
    yield "ready";
  } finally {
    return "override";
  }
}
const of2 = overrideFinally();
out("overrideFirst", JSON.stringify(of2.next()));
out("overrideThrow", JSON.stringify(of2.throw(new Error("boom"))));
out("overrideAfter", JSON.stringify(of2.next()));

// re-entrant resumption from inside the body is rejected
let reentrant: any;
function* selfCaller() {
  try {
    reentrant.next();
  } catch (e: any) {
    yield "reentry:" + e.name;
  }
}
reentrant = selfCaller();
out("reentrantNext", JSON.stringify(reentrant.next()));

// ---- yield*: delegates to any iterable, and yields the delegate's return value ----
function* inner() {
  yield 1;
  yield 2;
  return 7;
}
function* outerDelegate() {
  const res = yield* inner();
  yield res * 2;
}
out("yieldStarReturnValue", [...outerDelegate()].join(","));

function* combined() {
  yield* [1, 2];
  yield* "A\ud83d\ude00";
  yield 3;
}
out("yieldStarArrayString", [...combined()].map((v) => String(v).length).join(","));

// delegate without throw(): outer throw() closes the delegate, then TypeError
const dlog: string[] = [];
const delegateNoThrow: any = {
  [Symbol.iterator]() {
    return {
      next() { dlog.push("next"); return { value: 1, done: false }; },
      return() { dlog.push("return"); return { done: true }; },
    };
  },
};
function* deleg() {
  yield* delegateNoThrow;
}
const dg = deleg();
out("yieldStarFirst", JSON.stringify(dg.next()));
out("yieldStarThrowMissing", attempt(() => dg.throw(new Error("x"))));
out("yieldStarThrowLog", dlog.join(","));

// return() on a delegating generator forwards into the delegate
const vlog: string[] = [];
const vdel: any = {
  [Symbol.iterator]() {
    return {
      next() { vlog.push("next"); return { value: 1, done: false }; },
      return(v: any) { vlog.push("return:" + v); return { value: "closed:" + v, done: true }; },
    };
  },
};
function* vouter() {
  const value = yield* vdel;
  return "outer:" + value;
}
const vo = vouter();
out("delegateFirst", JSON.stringify(vo.next()));
out("delegateReturn", JSON.stringify(vo.return("X")));
out("delegateLog", vlog.join(","));

// ---- lazy generator pipelines over an infinite source ----
const plog: string[] = [];
function* naturals() {
  try {
    let i = 0;
    while (true) {
      plog.push("p" + i);
      yield i++;
    }
  } finally {
    plog.push("closed");
  }
}
function* gmap<T, U>(src: Iterable<T>, f: (v: T) => U): Generator<U> {
  for (const v of src) yield f(v);
}
function* gfilter<T>(src: Iterable<T>, pred: (v: T) => boolean): Generator<T> {
  for (const v of src) if (pred(v)) yield v;
}
function* gtake<T>(src: Iterable<T>, n: number): Generator<T> {
  let c = 0;
  if (n <= 0) return;
  for (const v of src) {
    yield v;
    if (++c >= n) return;
  }
}
out("pipelineValues", [...gtake(gfilter(gmap(naturals(), (x) => x * x), (x) => x % 2 === 0), 4)].join(","));
out("pipelinePulls", plog.join(","));

// ---- async generators and for-await ----
async function* paginate(pages: number) {
  for (let i = 1; i <= pages; i++) {
    await Promise.resolve();
    if (i === 3) throw new Error("page 3 failed");
    yield { page: i, data: "content-" + i };
  }
}

async function* mapAsync<T, U>(iter: AsyncIterable<T>, fn: (v: T) => U): AsyncGenerator<U> {
  for await (const v of iter) yield fn(v);
}
async function* filterAsync<T>(iter: AsyncIterable<T>, pred: (v: T) => boolean): AsyncGenerator<T> {
  for await (const v of iter) if (pred(v)) yield v;
}
async function* takeAsync<T>(iter: AsyncIterable<T>, n: number): AsyncGenerator<T> {
  let count = 0;
  for await (const v of iter) {
    if (count++ >= n) return;
    yield v;
  }
}
async function* rangeAsync(start: number, end: number): AsyncGenerator<number> {
  for (let i = start; i <= end; i++) {
    await Promise.resolve();
    yield i;
  }
}
async function* withReturn(): AsyncGenerator<number, string> {
  yield 1;
  yield 2;
  return "final";
}
async function collectAsync<T>(iter: AsyncIterable<T>): Promise<T[]> {
  const acc: T[] = [];
  for await (const v of iter) acc.push(v);
  return acc;
}

async function asyncMain(): Promise<void> {
  const pages: string[] = [];
  try {
    for await (const item of paginate(5)) pages.push(item.data);
  } catch (e: any) {
    pages.push("err:" + e.message);
  }
  out("asyncPaginateWithError", pages.join(","));

  const piped = await collectAsync(takeAsync(filterAsync(mapAsync(rangeAsync(1, 20), (x) => x * x), (x) => x % 2 === 0), 4));
  out("asyncPipeline", piped.join(","));

  const gen = withReturn();
  out("asyncGenNext1", String((await gen.next()).value));
  out("asyncGenNext2", String((await gen.next()).value));
  const ret = await gen.next();
  out("asyncGenReturn", ret.value + "|done=" + ret.done);
  out("asyncGenAfter", JSON.stringify(await gen.next()));

  const vals: string[] = [];
  for await (const v of [Promise.resolve("a"), Promise.resolve("b"), "c"]) vals.push(v);
  out("forAwaitSyncPromises", vals.join(","));

  out("asyncGenTag", Object.prototype.toString.call(withReturn()));
  out("genIsAsyncIterable", String(typeof (withReturn() as any)[Symbol.asyncIterator]));
}

asyncMain().then(() => out("asyncMainDone", "yes"));
