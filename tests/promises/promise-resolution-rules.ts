// Promise resolution rules: executor semantics, single settlement, thenable
// assimilation (including misbehaving thenables), identity of Promise.resolve /
// Promise.reject, the two-step self-resolution cycle, and Promise.withResolvers.
// Every case prints exactly one labelled line; the final line is "end".

function out(label: string, value: unknown): void {
  console.log(label + "=" + String(value));
}

function describe(p: Promise<unknown>): Promise<string> {
  return p.then(
    (v) => "fulfilled:" + String(v),
    (e) => "rejected:" + (e instanceof Error ? e.name : String(e)),
  );
}

async function main(): Promise<void> {
  // 1. The executor runs synchronously, before the constructor returns, and
  //    receives two functions of arity 1.
  let ran = "no";
  let resFn: ((v: unknown) => void) | undefined;
  let rejFn: ((e: unknown) => void) | undefined;
  const shaped = new Promise((res, rej) => {
    ran = "yes";
    resFn = res;
    rejFn = rej;
  });
  out("executorSync", ran);
  out("resolveArity", resFn!.length);
  out("rejectArity", rejFn!.length);
  out("resolveHasPrototype", resFn!.prototype !== undefined);
  out("shapedIsPromise", shaped instanceof Promise);

  // 2. Only the first settlement counts; later resolve/reject calls are ignored.
  out("firstSettlementWins", await describe(new Promise<string>((res, rej) => {
    res("first");
    res("second");
    rej(new RangeError("late"));
  })));
  out("rejectThenResolve", await describe(new Promise<string>((_res, rej) => {
    rej(new EvalError("won"));
    _res("lost");
  })));

  // 3. A throw after a settlement is swallowed; a throw before one rejects.
  out("throwAfterResolve", await describe(new Promise<string>((res) => {
    res("kept");
    throw new URIError("ignored");
  })));
  out("throwBeforeResolve", await describe(new Promise<string>(() => {
    throw new TypeError("boom");
  })));

  // 4. A non-callable executor and calling Promise without new both throw
  //    synchronously.
  let nonCallable = "no";
  try { new (Promise as any)(42); } catch (e) { nonCallable = (e as Error).name; }
  out("nonCallableExecutor", nonCallable);
  let withoutNew = "no";
  try { (Promise as any)(() => {}); } catch (e) { withoutNew = (e as Error).name; }
  out("calledWithoutNew", withoutNew);

  // 5. Promise.resolve returns its argument only for a native promise whose
  //    constructor is the receiver; an executor that resolves WITH a promise
  //    adopts it and never returns that object.
  const base = Promise.resolve(1);
  out("resolveSameObject", Promise.resolve(base) === base);
  out("resolveViaCall", Promise.resolve.call(Promise, base) === base);
  const wrapped = new Promise((res) => res(base));
  out("executorAdoptsNotSame", wrapped === base);
  out("executorAdoptsValue", await wrapped);

  // 6. Promise.reject never unwraps: the reason is the very object passed in.
  const inner = Promise.resolve("in");
  const thenable = { tag: "thenable", then(r: (v: string) => void) { r("th"); } };
  out("rejectKeepsPromiseReason", await Promise.reject(inner).then(() => "no", (r) => r === inner));
  out("rejectKeepsThenableReason", await Promise.reject(thenable).then(() => "no", (r) => r === thenable));
  out("rejectLength", Promise.reject.length);
  out("resolveLength", Promise.resolve.length);

  // 7. Thenable assimilation: the first call wins, later calls and throws are
  //    discarded, and a throw before any call becomes the rejection.
  out("thenableResolveThenThrow", await describe(Promise.resolve({
    then(res: (v: string) => void) { res("A"); throw new Error("late"); },
  } as any)));
  out("thenableThrowOnly", await describe(Promise.resolve({
    then() { throw "T"; },
  } as any)));
  out("thenableResolveTwice", await describe(Promise.resolve({
    then(res: (v: string) => void) { res("first"); res("second"); },
  } as any)));
  out("thenableResolveThenReject", await describe(Promise.resolve({
    then(res: (v: string) => void, rej: (e: string) => void) { res("kept"); rej("dropped"); },
  } as any)));
  out("thenableRejectThenResolve", await describe(Promise.resolve({
    then(res: (v: string) => void, rej: (e: string) => void) { rej("kept-reason"); res("dropped"); },
  } as any)));

  // 8. `then` is called once, with the thenable as `this` and exactly two
  //    arguments; the then getter of an object is read exactly once.
  const probe: any = {
    seen: "",
    then(this: any, res: any, rej: any) {
      this.seen = "this=" + (this === probe) + ",args=" + arguments.length +
        ",types=" + typeof res + "/" + typeof rej;
      res("done");
    },
  };
  await Promise.resolve(probe);
  out("thenableCallShape", probe.seen);

  let reads = 0;
  const getterObj = {
    get then() {
      reads++;
      return (resolve: (v: string) => void) => resolve("ok");
    },
  };
  out("thenGetterValue", await Promise.resolve(getterObj));
  out("thenGetterReads", reads);

  // 9. A nested thenable is assimilated again; a thenable that resolves with a
  //    promise yields that promise's value; an inherited `then` counts; a
  //    non-callable `then` makes the object an ordinary value.
  out("nestedThenable", await describe(Promise.resolve({
    then(res: (v: unknown) => void) { res({ then(r2: (v: string) => void) { r2("inner"); } }); },
  } as any)));
  out("thenableWithPromise", await Promise.resolve({
    then(res: (v: unknown) => void) { res(Promise.resolve("unwrapped")); },
  } as any));
  const proto = { then(res: (v: string) => void) { res("inherited"); } };
  out("inheritedThen", await describe(Promise.resolve(Object.create(proto))));
  const notThenable: any = { then: 42, tag: "plain" };
  const asValue = await Promise.resolve(notThenable);
  out("nonCallableThenIsValue", (asValue === notThenable) + ":" + asValue.tag + ":" + asValue.then);

  // 10. The cycle check: resolving a promise with ITSELF is a TypeError
  //     delivered as a rejection, not a throw.
  let resSelf: ((v: unknown) => void) | undefined;
  const selfP: Promise<unknown> = new Promise((res) => { resSelf = res; });
  let selfThrew = "no";
  try { resSelf!(selfP); } catch (e) { selfThrew = "threw"; }
  out("selfResolveThrowsSync", selfThrew);
  out("selfResolveRejects", await selfP.then(
    () => "fulfilled",
    (e) => (e instanceof TypeError ? "TypeError" : "other") + ":" + (e as Error).name,
  ));

  // 11. Rejecting with itself is allowed (no cycle check on that path).
  let rejSelf: ((e: unknown) => void) | undefined;
  const selfRejP: Promise<unknown> = new Promise((_r, j) => { rejSelf = j; });
  rejSelf!(selfRejP);
  out("selfRejectReasonIsSelf", await selfRejP.then(() => "no", (e) => e === selfRejP));

  // 12. A two-promise cycle is NOT caught: both promises stay pending forever.
  let ra: ((v: unknown) => void) | undefined;
  let rb: ((v: unknown) => void) | undefined;
  const pa: Promise<unknown> = new Promise((r) => { ra = r; });
  const pb: Promise<unknown> = new Promise((r) => { rb = r; });
  ra!(pb);
  rb!(pa);
  let mutual = "pending";
  pa.then(() => { mutual = "settled"; }, () => { mutual = "settled"; });

  // 13. A thenable that never calls its callbacks leaves the promise pending.
  let stuck = "pending";
  Promise.resolve({ then() {} } as any).then(
    () => { stuck = "fulfilled"; },
    () => { stuck = "rejected"; },
  );

  // 14. Promise.withResolvers: a plain object with promise/resolve/reject,
  //     whose functions settle the promise once.
  const kit: any = Promise.withResolvers();
  out("withResolversKeys", Object.keys(kit).sort().join(","));
  out("withResolversPromise", kit.promise instanceof Promise);
  out("withResolversArity", kit.resolve.length + "," + kit.reject.length);
  kit.resolve("first");
  kit.reject("ignored");
  out("withResolversValue", await kit.promise);
  const kit2: any = Promise.withResolvers();
  const detached = kit2.resolve;
  detached("detached-ok");
  out("withResolversDetached", await kit2.promise);
  const kit3: any = Promise.withResolvers();
  kit3.reject("why");
  kit3.resolve("too-late");
  out("withResolversRejected", await kit3.promise.then(() => "no", (e: unknown) => "rejected:" + e));
  out("withResolversFresh", Promise.withResolvers() !== Promise.withResolvers());

  // Drain well past every settlement above, then report the pending states.
  for (let i = 0; i < 20; i++) await null;
  out("neverCallingThenable", stuck);
  out("twoPromiseCycle", mutual);
  console.log("end");
}

main().catch((e) => console.log("UNEXPECTED " + String(e)));
