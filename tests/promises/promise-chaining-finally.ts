// Promise chaining: then/catch/finally value and error propagation, returning
// values and promises from handlers, finally transparency and override rules,
// non-callable handler pass-through, species-driven result constructors, and
// the chaining-cycle TypeError. Every case prints exactly one labelled line.

function out(label: string, value: unknown): void {
  console.log(label + "=" + String(value));
}

function settle(p: PromiseLike<unknown>): Promise<string> {
  return Promise.resolve(p).then(
    (v) => "fulfilled:" + String(v),
    (e) => "rejected:" + (e instanceof Error ? e.name + "/" + e.message : String(e)),
  );
}

class Tracked<T> extends Promise<T> {}

class Plain<T> extends Promise<T> {
  static get [Symbol.species](): PromiseConstructor {
    return Promise;
  }
}

async function main(): Promise<void> {
  // 1. Values flow through a then chain; each handler sees the previous result.
  out("valueChain", await Promise.resolve(2)
    .then((n) => n * 3)
    .then((n) => "v" + n)
    .then((s) => s + "!"));

  // 2. A handler that returns a promise is adopted: the next link sees its value.
  out("returnPromise", await Promise.resolve(1).then((n) => Promise.resolve(n + 10)).then((n) => "got " + n));
  out("returnThenable", await Promise.resolve(1).then((n) => ({
    then(r: (v: number) => void) { r(n + 20); },
  }) as any));

  // 3. A handler that returns nothing yields undefined, not its input.
  out("emptyHandler", String(await Promise.resolve("in").then(() => undefined)));

  // 4. Errors propagate past then-links that lack an onRejected handler and are
  //    recovered by the first catch; the recovered value continues the chain.
  out("propagateAndRecover", await Promise.reject(new RangeError("deep"))
    .then((v) => "skipped:" + v)
    .then((v) => "skipped2:" + v)
    .catch((e: Error) => "recovered:" + e.name)
    .then((v) => v + "+continued"));

  // 5. A throw inside a handler becomes the rejection of the next link.
  out("throwInHandler", await settle(Promise.resolve("x").then(() => {
    throw new SyntaxError("bad");
  })));

  // 6. A catch handler that throws replaces the rejection with the new error.
  out("catchRethrows", await settle(Promise.reject(new RangeError("first")).catch((e: Error) => {
    throw new EvalError("second after " + e.name);
  })));

  // 7. catch(f) is then(undefined, f); a fulfilled promise ignores onRejected
  //    and a rejected one ignores onFulfilled.
  out("catchEquivalence", (await Promise.reject("c").catch((e) => "byCatch:" + e)) + "/" +
    (await Promise.reject("c").then(undefined, (e) => "byThen:" + e)));
  let picked = "none";
  await Promise.resolve(1).then(() => { picked = "onFulfilled"; }, () => { picked = "onRejected"; });
  out("fulfilledPicks", picked);
  await Promise.reject(1).then(() => { picked = "onFulfilled"; }, () => { picked = "onRejected"; });
  out("rejectedPicks", picked);

  // 8. Non-callable handlers are a pass-through: the settlement travels on.
  const forms: unknown[] = [undefined, null, 42, "str", true, {}];
  const names = ["undefined", "null", "number", "string", "boolean", "object"];
  for (let i = 0; i < forms.length; i++) {
    out("passThrough." + names[i], await Promise.resolve("V").then(forms[i] as any));
  }
  out("rejectPassThrough", await Promise.reject("R").then(
    (x) => "fulfilled:" + x,
    null as any,
  ).catch((e) => "caught:" + e));
  out("thenNoArgs", await Promise.resolve("ok").then() + "/" +
    (await Promise.reject("bad").then().catch((e) => "caught:" + e)));

  // 9. Each then/catch/finally call answers a distinct new promise.
  const src = Promise.resolve("d");
  out("distinctThen", (src.then() !== src.then()) + "," + (src.then() !== src));
  out("distinctFinally", (src.finally() !== src) + "," + (src.finally(() => {}) !== src.finally(() => {})));

  // 10. finally is transparent: its return value is discarded and the original
  //     settlement survives; it receives no arguments.
  out("finallyKeepsValue", await settle(Promise.resolve("keep").finally(() => "discarded" as any)));
  out("finallyKeepsReason", await settle(Promise.reject(new RangeError("r")).finally(() => "x" as any)));
  let finArgs = -1;
  await Promise.resolve("v").finally(function (..._args: unknown[]) { finArgs = arguments.length; });
  out("finallyArgCount", finArgs);

  // 11. A throw inside finally replaces the settlement, fulfilment or rejection.
  out("finallyThrowOverFulfil", await settle(Promise.resolve("gone").finally(() => {
    throw new TypeError("t");
  })));
  out("finallyThrowOverReject", await settle(Promise.reject(new RangeError("old")).finally(() => {
    throw new EvalError("new");
  })));

  // 12. Returning a rejected promise from finally also replaces the settlement;
  //     returning a fulfilling thenable keeps the original value.
  out("finallyReturnsRejected", await settle(Promise.resolve("gone").finally(
    () => Promise.reject(new URIError("u")) as any,
  )));
  out("finallyThenableValue", await settle(Promise.resolve("original").finally(() => ({
    then(r: (v: string) => void) { r("ignored"); },
  }) as any)));

  // 13. A pending promise returned from finally delays the pass-through; the
  //     value is intact and the callback has run before the next link.
  const marks: string[] = [];
  let deep: Promise<void> = Promise.resolve();
  for (let i = 0; i < 5; i++) deep = deep.then(() => { marks.push("tick"); });
  const delayed = await Promise.resolve("delayed")
    .finally(() => { marks.push("finallyCalled"); return deep as any; })
    .then((v) => { marks.push("next:" + v); return v; });
  out("finallyDelayedValue", delayed);
  out("finallyDelayedOrder", marks.join(","));

  // 14. finally with a non-callable argument is a plain pass-through.
  out("finallyNonCallable", await settle(Promise.resolve("still").finally(null as any)));
  out("finallyNonCallableReject", await settle(Promise.reject(new RangeError("q")).finally(42 as any)));

  // 15. finally runs before anything chained after it, and before a sibling
  //     registered later on the same source.
  const seq: string[] = [];
  const base = Promise.resolve("s");
  const chained = base.finally(() => { seq.push("finally"); }).then(() => { seq.push("afterFinally"); });
  const sibling = base.then(() => { seq.push("sibling"); });
  await Promise.all([chained, sibling]);
  out("finallyOrder", seq.join(","));

  // 16. Shapes of the three methods.
  out("catchShape", Promise.prototype.catch.length + "," + Promise.prototype.catch.name);
  out("thenShape", Promise.prototype.then.length + "," + Promise.prototype.then.name);
  out("finallyShape", Promise.prototype.finally.length + "," + Promise.prototype.finally.name);
  out("finallyOnPrototype", Object.prototype.hasOwnProperty.call(Promise.prototype, "finally"));

  // 17. Chaining cycle: a promise chained to itself rejects with a TypeError.
  let chainedSelf: Promise<unknown> | undefined;
  chainedSelf = Promise.resolve("start").then(() => chainedSelf);
  out("chainingCycle", await chainedSelf.then(
    () => "fulfilled",
    (e) => "rejected:" + (e instanceof TypeError ? "TypeError" : "other"),
  ));

  // 18. The receiver's constructor decides the derived promise: a subclass
  //     answers subclass instances; species pointing at Promise makes them plain.
  const t = Tracked.resolve("v");
  out("trackedThen", t.then((v) => v) instanceof Tracked);
  out("trackedCatch", t.catch(() => 0) instanceof Tracked);
  out("trackedFinally", t.finally(() => {}) instanceof Tracked);
  out("trackedThenIsPromise", t.then((v) => v) instanceof Promise);
  const p = Plain.resolve("w");
  out("plainResolveIsPlain", p instanceof Plain);
  out("plainThenIsPlain", p.then((v) => v) instanceof Plain);
  out("plainThenIsPromise", p.then((v) => v) instanceof Promise);
  out("plainFinallyIsPlain", p.finally(() => {}) instanceof Plain);
  out("promiseSpecies", Promise[Symbol.species] === Promise);
  out("trackedSpeciesInherited", (Tracked as any)[Symbol.species] === Tracked);
  out("trackedProtoChain", Object.getPrototypeOf(Tracked.prototype) === Promise.prototype);
  out("trackedAdopted", await Promise.resolve(Tracked.resolve("flow")));

  console.log("end");
}

main().catch((e) => console.log("UNEXPECTED " + String(e)));
