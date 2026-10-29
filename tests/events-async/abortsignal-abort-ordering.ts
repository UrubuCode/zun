// Cross-runtime: AbortController / AbortSignal as an event source with a fixed ordering.
// Covers the abort reason (default, explicit, falsy), when the abort event fires
// (synchronously inside abort()), second-abort and late-listener behaviour,
// throwIfAborted, onabort, AbortSignal.abort() and AbortSignal.any() composition, and
// how an abort interleaves with once/signal listeners and microtasks. No timers, so
// nothing here can race. AbortSignal.timeout() is deliberately not used: its firing
// time is a clock, not an ordering.

function sectionReasonAndEvent(): void {
  const c1 = new AbortController();
  console.log("R1.before=" + c1.signal.aborted + " reason=" + String(c1.signal.reason));
  console.log("R1.ctors=" + c1.signal.constructor.name + "," + c1.constructor.name + "," + (c1.signal instanceof EventTarget));
  let throwBefore = "no-throw";
  try { c1.signal.throwIfAborted(); } catch (e: any) { throwBefore = e.name; }
  console.log("R1.throwIfBefore=" + throwBefore);

  const seen: string[] = [];
  c1.signal.addEventListener("abort", (ev: Event) => {
    seen.push("listener:" + ev.type + ":" + (ev.target === c1.signal));
  });
  c1.abort();
  console.log("R2.afterAborted=" + c1.signal.aborted);
  console.log("R2.reasonName=" + c1.signal.reason.name + " isError=" + (c1.signal.reason instanceof Error));
  console.log("R2.listenerRan=" + seen.join(","));
  console.log("R2.synchronousAbort=" + (seen.length === 1));

  let thrown: any = null;
  try { c1.signal.throwIfAborted(); } catch (e: any) { thrown = e; }
  console.log("R3.throwIfAfter=" + (thrown === c1.signal.reason) + " name=" + thrown.name);

  seen.length = 0;
  c1.abort("ignored");
  console.log("R4.secondAbortReason=" + c1.signal.reason.name + " fired=" + seen.length);
  c1.signal.addEventListener("abort", () => { seen.push("late"); });
  console.log("R4.lateListenerRuns=" + seen.length);

  const c2 = new AbortController();
  const token = { why: "stop" };
  c2.abort(token);
  console.log("R5.customSame=" + (c2.signal.reason === token) + " why=" + c2.signal.reason.why);

  const c3 = new AbortController();
  c3.abort(undefined);
  console.log("R5.undefinedIsDefault=" + c3.signal.reason.name);

  const c4 = new AbortController();
  c4.abort(null);
  console.log("R5.nullReason=" + String(c4.signal.reason));

  const c5 = new AbortController();
  c5.abort(0);
  console.log("R5.zeroReason=" + c5.signal.reason);

  const c6 = new AbortController();
  c6.abort("stop");
  console.log("R5.stringReason=" + String(c6.signal.reason));

  const c7 = new AbortController();
  c7.abort(new RangeError("propagated"));
  let compositeThrow = "no-throw";
  try { c7.signal.throwIfAborted(); } catch (e: any) { compositeThrow = e.constructor.name; }
  console.log("R5.throwIfAbortedRangeError=" + compositeThrow);

  const c8 = new AbortController();
  c8.signal.onabort = () => { seen.push("onabort"); };
  console.log("R6.onabortIsFunction=" + (typeof c8.signal.onabort));
  seen.length = 0;
  c8.abort();
  console.log("R6.onabortRan=" + seen.join(","));

  const c9 = new AbortController();
  let ev9: any = null;
  c9.signal.addEventListener("abort", (ev: Event) => { ev9 = ev; });
  c9.abort();
  console.log("R7.abortEvent=" + ev9.type + " isEvent=" + (ev9 instanceof Event) + " bubbles=" + ev9.bubbles + " cancelable=" + ev9.cancelable);

  const c10 = new AbortController();
  let shape = "unset";
  c10.signal.addEventListener("abort", (ev: Event) => {
    shape = "bubbles=" + ev.bubbles + " cancelable=" + ev.cancelable + " currentTargetIsSignal=" + (ev.currentTarget === c10.signal) + " phase=" + ev.eventPhase;
  });
  c10.abort();
  console.log("R7.realAbortShape=" + shape);

  const a = new AbortController();
  const fakeCount = { n: 0 };
  a.signal.addEventListener("abort", () => { fakeCount.n++; });
  const dispatched = a.signal.dispatchEvent(new Event("abort"));
  console.log("R8.handDispatch=" + dispatched + " aborted=" + a.signal.aborted + " listenerCalls=" + fakeCount.n);
  a.abort("real");
  console.log("R8.afterRealAbort=" + a.signal.aborted + " reason=" + a.signal.reason + " listenerCalls=" + fakeCount.n);
  fakeCount.n = 0;
  a.signal.dispatchEvent(new Event("abort"));
  console.log("R8.dispatchAfterAbort=" + fakeCount.n);

  const c11 = new AbortController();
  let onceCalls = 0;
  c11.signal.addEventListener("abort", () => { onceCalls++; }, { once: true });
  c11.signal.dispatchEvent(new Event("abort"));
  c11.abort();
  console.log("R9.onceCalls=" + onceCalls + " aborted=" + c11.signal.aborted);

  const c12 = new AbortController();
  let removedCalls = 0;
  const fn12 = () => { removedCalls++; };
  c12.signal.addEventListener("abort", fn12);
  c12.signal.removeEventListener("abort", fn12);
  c12.abort();
  console.log("R9.removedCalls=" + removedCalls + " aborted=" + c12.signal.aborted);

  const c13 = new AbortController();
  const order: string[] = [];
  c13.signal.addEventListener("abort", () => { order.push("first"); });
  c13.signal.addEventListener("abort", () => { order.push("second"); });
  c13.signal.onabort = () => { order.push("onabort"); };
  c13.signal.addEventListener("abort", () => { order.push("third"); });
  c13.abort();
  console.log("R10.abortListenerOrder=" + order.join(","));

  const c14 = new AbortController();
  const replaced: string[] = [];
  c14.signal.onabort = () => { replaced.push("A"); };
  c14.signal.onabort = () => { replaced.push("B"); };
  c14.abort();
  console.log("R10.onabortReplaced=" + replaced.join(","));

  const c15 = new AbortController();
  let cleared = 0;
  c15.signal.onabort = () => { cleared++; };
  c15.signal.onabort = null;
  c15.abort();
  console.log("R10.onabortCleared=" + cleared + " value=" + String(c15.signal.onabort));

  const s1 = AbortSignal.abort("already");
  let staticCalls = 0;
  s1.addEventListener("abort", () => { staticCalls++; });
  console.log("R11.staticAborted=" + s1.aborted + " reason=" + s1.reason + " listenerCalls=" + staticCalls);
  s1.dispatchEvent(new Event("abort"));
  console.log("R11.afterManualDispatch=" + staticCalls + " stillAborted=" + s1.aborted);
  const s2 = AbortSignal.abort();
  console.log("R11.defaultReasonName=" + s2.aborted + " " + s2.reason.name);
}

function sectionAnySignals(): void {
  console.log("A0.hasAny=" + (typeof AbortSignal.any));

  const empty = AbortSignal.any([]);
  console.log("A1.empty=" + empty.aborted + " reason=" + String(empty.reason) + " isSignal=" + (empty instanceof AbortSignal));

  const done = AbortSignal.abort("early");
  const pending = new AbortController().signal;
  const composed = AbortSignal.any([pending, done]);
  const composedSeen: string[] = [];
  composed.addEventListener("abort", () => { composedSeen.push("composedLate"); });
  console.log("A2.composedAborted=" + composed.aborted + " reason=" + composed.reason + " listener=" + composedSeen.length);

  const a = new AbortController();
  const b = new AbortController();
  const race = AbortSignal.any([a.signal, b.signal]);
  const raceEvents: string[] = [];
  race.addEventListener("abort", (ev: Event) => {
    raceEvents.push("raceAbort:" + ev.type + ":" + (ev.target === race));
  });
  console.log("A3.raceBefore=" + race.aborted);
  b.abort("b-first");
  console.log("A3.raceAfterB=" + race.aborted + " reason=" + race.reason);
  a.abort("a-second");
  console.log("A3.raceAfterA reason=" + race.reason + " events=" + raceEvents.join("|"));
  console.log("A3.sources=" + a.signal.reason + "," + b.signal.reason);

  const c = new AbortController();
  const d = new AbortController();
  const comp = AbortSignal.any([c.signal, d.signal]);
  c.abort("c-only");
  console.log("A4.compAborted=" + comp.aborted + " dUntouched=" + d.signal.aborted);

  const e = new AbortController();
  const one = AbortSignal.any([e.signal]);
  const two = AbortSignal.any([e.signal]);
  console.log("A5.distinct=" + (one !== two));
  e.abort("shared");
  console.log("A5.one=" + one.reason + " two=" + two.reason);

  const f = new AbortController();
  const inner = AbortSignal.any([f.signal]);
  const outer = AbortSignal.any([inner]);
  console.log("A6.nestedBefore=" + outer.aborted);
  f.abort("deep");
  console.log("A6.nestedAfter=" + outer.aborted + " reason=" + outer.reason);

  const g = new AbortController();
  const dup = AbortSignal.any([g.signal, g.signal]);
  let dupCount = 0;
  dup.addEventListener("abort", () => { dupCount++; });
  g.abort("dup");
  console.log("A7.dupAborted=" + dup.aborted + " events=" + dupCount);

  let badMember = "no";
  try { AbortSignal.any([42 as any]); } catch (err: any) { badMember = err.constructor.name; }
  console.log("A8.badMember=" + badMember);
  let nonIterable = "no";
  try { AbortSignal.any(7 as any); } catch (err: any) { nonIterable = err.constructor.name; }
  console.log("A8.nonIterable=" + nonIterable);

  console.log("A9.hasAbortMethod=" + typeof (composed as any).abort);

  const h = new AbortController();
  const hi = AbortSignal.any([h.signal]);
  const cause = new RangeError("propagated");
  h.abort(cause);
  let compositeThrow = "no-throw";
  try { hi.throwIfAborted(); } catch (err: any) { compositeThrow = err.constructor.name; }
  console.log("A10.compositeThrow=" + compositeThrow + " sameReason=" + (hi.reason === h.signal.reason));
}

async function sectionAbortAndMicrotasks(): Promise<void> {
  const target = new EventTarget();
  const controller = new AbortController();
  const seen: string[] = [];
  target.addEventListener("ping", () => seen.push("normal"));
  target.addEventListener("ping", () => seen.push("once"), { once: true });
  target.addEventListener("ping", () => seen.push("abortable"), { signal: controller.signal });
  target.dispatchEvent(new Event("ping"));
  controller.abort();
  target.dispatchEvent(new Event("ping"));
  console.log("Z1.orderAcrossAbort=" + seen.join(","));

  const ctl = new AbortController();
  const steps: string[] = [];
  ctl.signal.addEventListener("abort", () => {
    steps.push("abort-listener");
    queueMicrotask(() => { steps.push("abort-micro"); });
    Promise.resolve().then(() => { steps.push("abort-then"); });
  });
  steps.push("beforeAbort");
  ctl.abort();
  steps.push("afterAbort");
  console.log("Z2.syncPart=" + steps.join(","));
  await null;
  await null;
  console.log("Z2.drained=" + steps.join(","));

  const waiter = new AbortController();
  const resumed = new Promise<string>((resolve) => {
    waiter.signal.addEventListener("abort", () => resolve("resolved-by-abort"), { once: true });
  });
  waiter.abort();
  console.log("Z3.waiter=" + (await resumed));
}

async function main(): Promise<void> {
  sectionReasonAndEvent();
  sectionAnySignals();
  await sectionAbortAndMicrotasks();
  console.log("end");
}

main().catch(() => console.log("UNEXPECTED"));
