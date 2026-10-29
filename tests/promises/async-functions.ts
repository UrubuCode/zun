// async functions and await: what an async function returns (always a fresh
// native Promise), synchronous throws becoming rejections, return versus
// return-await inside try/catch, await on values, thenables and subclasses,
// sequential versus parallel composition, retry loops, async generators and
// for-await iteration with early exit. Every case prints one labelled line.

function out(label: string, value: unknown): void {
  console.log(label + "=" + String(value));
}

const inner = Promise.resolve("I");

async function returnsPromise(): Promise<string> { return inner; }
async function returnsAwaited(): Promise<string> { return await inner; }
async function returnsValue(): Promise<string> { return "V"; }
async function throwsSync(): Promise<never> { throw "S"; }
async function returnsRejected(): Promise<string> { return Promise.reject("RJ"); }

// `return p` inside try does NOT catch p's rejection; `return await p` does.
async function returnUncaught(): Promise<string> {
  try { return Promise.reject(new RangeError("late")); } catch (e) { return "caughtHere"; }
}
async function returnAwaitCaught(): Promise<string> {
  try { return await Promise.reject(new RangeError("now")); } catch (e) { return "caught:" + (e as Error).name; }
}

class MyP extends Promise<any> {}

class Counter {
  async m(): Promise<number> { return 3; }
  static async s(): Promise<number> { return 4; }
}

function sleep(ms: number, value: string): Promise<string> {
  return new Promise((resolve) => setTimeout(() => resolve(value), ms));
}

async function sequential(): Promise<number> {
  const a = await Promise.resolve(1);
  const b = await Promise.resolve(2);
  const c = await Promise.resolve(3);
  return a + b + c;
}

async function parallel(): Promise<number> {
  const [a, b, c] = await Promise.all([Promise.resolve(10), Promise.resolve(20), Promise.resolve(30)]);
  return a + b + c;
}

async function settledSummary(): Promise<string[]> {
  const results = await Promise.allSettled([
    Promise.resolve("ok"),
    Promise.reject(new Error("fail")),
    Promise.resolve("also ok"),
  ]);
  return results.map((r) => r.status + ":" + (r.status === "fulfilled" ? r.value : (r as any).reason.message));
}

async function retry<T>(fn: () => Promise<T>, times: number): Promise<T> {
  let last: unknown;
  for (let i = 0; i < times; i++) {
    try { return await fn(); } catch (e) { last = e; }
  }
  throw last;
}

async function* asyncRange(start: number, end: number): AsyncGenerator<number> {
  for (let i = start; i <= end; i++) {
    await Promise.resolve();
    yield i;
  }
}

async function* withCleanup(): AsyncGenerator<string> {
  try {
    yield "first";
    yield "second";
    yield "third";
  } finally {
    cleanups.push("generator-finally");
  }
}
const cleanups: string[] = [];

async function main(): Promise<void> {
  // 1. Identity and freshness: the returned promise is native, new per call,
  //    and never the promise that was returned.
  const rp = returnsPromise();
  out("returnedIsInner", rp === inner);
  out("returnedIsPromise", rp instanceof Promise);
  out("returnedCtor", rp.constructor.name);
  out("freshEachCall", returnsPromise() !== returnsPromise());
  out("valueCallIsPromise", returnsValue() instanceof Promise);
  out("returnPromiseValue", await returnsPromise());
  out("returnAwaitValue", await returnsAwaited());
  out("returnValueValue", await returnsValue());

  // 2. A synchronous throw in an async body becomes a rejection, never a throw
  //    at the call site.
  let threw = "no";
  let syncResult: Promise<unknown> | undefined;
  try { syncResult = throwsSync(); } catch (e) { threw = "threw"; }
  out("syncThrowAtCallSite", threw);
  out("syncThrowIsPromise", syncResult instanceof Promise);
  out("syncThrowReason", await syncResult!.then(() => "fulfilled", (e) => "rejected:" + e));
  out("returnsRejected", await returnsRejected().then(() => "fulfilled", (e) => "rejected:" + e));

  // 3. Methods, arrows and static methods answer native promises too; a
  //    subclass instance returned from an async function is wrapped.
  out("methodCtor", new Counter().m().constructor.name + "," + Counter.s().constructor.name);
  out("arrowCtor", (async () => 2)().constructor.name);
  out("objectMethodCtor", ({ async m() { return 1; } }).m().constructor.name);
  const wrappedSub = (async () => MyP.resolve("sub"))();
  out("subclassReturnIsMyP", wrappedSub instanceof MyP);
  out("subclassReturnIsPlain", wrappedSub.constructor === Promise);
  out("subclassReturnValue", await wrappedSub);
  out("counterMethod", await new Counter().m());

  // 4. return versus return-await inside try/catch.
  out("returnUncaught", await returnUncaught().then((v) => v, (e) => "escaped:" + (e as Error).name));
  out("returnAwaitCaught", await returnAwaitCaught());

  // 5. await on non-promise values: plain values, plain objects (identity),
  //    and thenables (called with resolve/reject).
  const plain = { tag: "plain" };
  out("awaitPlainIdentity", (await plain) === plain);
  out("awaitPlainValue", await 42);
  out("awaitThenable", await { then(r: (v: string) => void) { r("thenable"); } });
  out("awaitThenableReject", await (async () => {
    try {
      await { then(_r: unknown, j: (e: string) => void) { j("nope"); } };
      return "fulfilled";
    } catch (e) {
      return "caught:" + e;
    }
  })());

  // 6. Awaiting the same already-fulfilled promise twice is fine.
  const twice: string[] = [];
  await inner;
  twice.push(await inner);
  twice.push(await inner);
  out("awaitTwice", twice.join(","));

  // 7. Sequential and parallel composition.
  out("sequential", await sequential());
  out("parallel", await parallel());
  out("settledSummary", (await settledSummary()).join("|"));
  const timed = await Promise.race([sleep(50, "slow"), sleep(1, "fast")]);
  out("raceOfTimers", timed);

  // 8. Error handling with try/catch/finally inside an async body.
  const trace: string[] = [];
  async function guarded(fail: boolean): Promise<string> {
    try {
      trace.push("try");
      if (fail) await Promise.reject(new TypeError("bad input"));
      trace.push("ok");
      return "returned";
    } catch (e) {
      trace.push("catch:" + (e as Error).name);
      return "caught:" + (e as Error).message;
    } finally {
      trace.push("finally");
    }
  }
  out("guardedFail", await guarded(true));
  out("guardedOk", await guarded(false));
  out("guardedTrace", trace.join(","));

  // 9. Retry: fails twice, succeeds on the third attempt.
  let attempts = 0;
  const retried = await retry(() => {
    attempts++;
    if (attempts < 3) return Promise.reject(new Error("not yet"));
    return Promise.resolve("success");
  }, 5);
  out("retryResult", retried);
  out("retryAttempts", attempts);
  let exhausted = "no";
  try {
    await retry(() => Promise.reject(new RangeError("always")), 2);
  } catch (e) {
    exhausted = "exhausted:" + (e as Error).name;
  }
  out("retryExhausted", exhausted);

  // 10. Async generators and for-await.
  const vals: number[] = [];
  for await (const v of asyncRange(1, 5)) vals.push(v);
  out("asyncRange", vals.join(","));

  const iterated: string[] = [];
  for await (const v of [Promise.resolve("p1"), "plain", Promise.resolve("p3")]) iterated.push(v);
  out("forAwaitArray", iterated.join(","));

  cleanups.length = 0;
  const early: string[] = [];
  for await (const v of withCleanup()) {
    early.push(v);
    if (v === "second") break;
  }
  out("forAwaitEarlyExit", early.join(","));
  out("forAwaitCleanup", cleanups.join(","));

  // 11. Promises returned from a handler inside a loop are adopted, not nested.
  let chained: Promise<number> = Promise.resolve(0);
  for (let i = 1; i <= 3; i++) {
    chained = chained.then((n) => Promise.resolve(n + i));
  }
  out("chainedAdoption", await chained);

  // 12. An async function that awaits itself through a chain of calls.
  async function countdown(n: number): Promise<number> {
    if (n === 0) return 0;
    return 1 + (await countdown(n - 1));
  }
  out("recursiveAsync", await countdown(20));

  console.log("end");
}

main().catch((e) => console.log("UNEXPECTED " + String(e)));
