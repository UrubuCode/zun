// Cross-runtime: EventTarget dispatch as a synchronous, ordered process. Covers listener
// registration order and de-duplication, once, stopImmediatePropagation, capture-phase
// identity, the `signal` option, re-entrant dispatch, Event/CustomEvent fields and
// subclassing, and where microtasks fall relative to a dispatch. Ordering only: no timers.

const tick = (): Promise<void> => new Promise<void>((resolve) => setTimeout(resolve, 0));

function sectionListenerOrder(): void {
  const t = new EventTarget();
  let seen: string[] = [];
  const reset = () => { seen = []; };
  const fire = (type: string) => t.dispatchEvent(new Event(type));

  const a = () => { seen.push("a"); };
  const b = () => { seen.push("b"); };
  const c = () => { seen.push("c"); };
  t.addEventListener("p", a);
  t.addEventListener("p", b);
  t.addEventListener("p", c);
  reset(); fire("p");
  console.log("L1.order=" + seen.join(","));

  t.addEventListener("p", a);
  reset(); fire("p");
  console.log("L2.duplicateIgnored=" + seen.join(","));

  t.removeEventListener("p", b);
  reset(); fire("p");
  console.log("L3.afterRemove=" + seen.join(","));

  t.removeEventListener("p", () => {});
  t.removeEventListener("nosuch", a);
  reset(); fire("p");
  console.log("L4.noopRemovals=" + seen.join(","));

  const late = () => { seen.push("late"); };
  const adder = () => { seen.push("adder"); t.addEventListener("p", late); };
  t.addEventListener("p", adder);
  reset(); fire("p");
  console.log("L5.addDuringDispatch=" + seen.join(","));
  reset(); fire("p");
  console.log("L5.nextDispatch=" + seen.join(","));
  t.removeEventListener("p", adder);
  t.removeEventListener("p", late);

  const tr = new EventTarget();
  const victim = () => { seen.push("victim"); };
  tr.addEventListener("p", () => { seen.push("remover"); tr.removeEventListener("p", victim); });
  tr.addEventListener("p", victim);
  tr.addEventListener("p", () => { seen.push("survivor"); });
  reset(); tr.dispatchEvent(new Event("p"));
  console.log("L6.removeDuringDispatch=" + seen.join(","));

  const only = () => { seen.push("once"); };
  t.addEventListener("q", only, { once: true });
  t.addEventListener("q", () => { seen.push("plain"); });
  reset(); fire("q");
  console.log("L7.onceFirst=" + seen.join(","));
  reset(); fire("q");
  console.log("L7.onceSecond=" + seen.join(","));

  const t2 = new EventTarget();
  t2.addEventListener("r", () => { seen.push("r1"); });
  t2.addEventListener("r", (ev: Event) => { seen.push("r2"); ev.stopImmediatePropagation(); });
  t2.addEventListener("r", () => { seen.push("r3"); });
  reset();
  const ok8 = t2.dispatchEvent(new Event("r"));
  console.log("L8.stopImmediate=" + seen.join(",") + " returned=" + ok8);

  const t3 = new EventTarget();
  t3.addEventListener("s", (ev: Event) => { seen.push("s1"); ev.stopPropagation(); });
  t3.addEventListener("s", () => { seen.push("s2"); });
  reset(); t3.dispatchEvent(new Event("s"));
  console.log("L9.stopPropagationSameTarget=" + seen.join(","));

  const t4 = new EventTarget();
  t4.addEventListener("u", () => { seen.push("u1"); });
  reset();
  const okUnknown = t4.dispatchEvent(new Event("unlistened"));
  console.log("L10.unknownType=" + okUnknown + " seen=" + seen.length);

  const t5 = new EventTarget();
  const handler: any = { calls: 0, handleEvent(ev: Event) { seen.push("handleEvent:" + ev.type); this.calls++; } };
  t5.addEventListener("v", handler);
  reset(); t5.dispatchEvent(new Event("v"));
  console.log("L11.handleEvent=" + seen.join(",") + " calls=" + handler.calls);

  const t6 = new EventTarget();
  reset();
  t6.addEventListener("w", () => { seen.push("inListener"); });
  t6.dispatchEvent(new Event("w"));
  seen.push("afterDispatch");
  console.log("L12.synchronous=" + seen.join(","));

  const t7 = new EventTarget();
  const ev7 = new Event("x");
  t7.dispatchEvent(ev7);
  let reuse = "no";
  try { t7.dispatchEvent(ev7); } catch (e: any) { reuse = e.constructor.name; }
  console.log("L13.reuseDispatch=" + reuse);
}

function sectionCapture(): void {
  let seen: string[] = [];
  const reset = () => { seen = []; };

  const t1 = new EventTarget();
  const f = () => { seen.push("f"); };
  t1.addEventListener("p", f, true);
  t1.addEventListener("p", f, false);
  reset(); t1.dispatchEvent(new Event("p"));
  console.log("M1.bothEntries=" + seen.length);
  t1.removeEventListener("p", f);
  reset(); t1.dispatchEvent(new Event("p"));
  console.log("M1.removeBubbleOnly=" + seen.join(","));
  t1.removeEventListener("p", f, { capture: true });
  reset(); t1.dispatchEvent(new Event("p"));
  console.log("M1.removeCaptureOnly=" + JSON.stringify(seen.join(",")));

  const t2 = new EventTarget();
  const g = () => { seen.push("g"); };
  t2.addEventListener("q", g, true);
  t2.addEventListener("q", g, { capture: true });
  reset(); t2.dispatchEvent(new Event("q"));
  console.log("M2.booleanEqualsOptions=" + seen.join(","));
  t2.removeEventListener("q", g, { capture: true });
  reset(); t2.dispatchEvent(new Event("q"));
  console.log("M2.removedByOptions=" + JSON.stringify(seen.join(",")));

  const t3 = new EventTarget();
  const h = () => { seen.push("h"); };
  t3.addEventListener("r", h, false);
  t3.addEventListener("r", h, undefined);
  t3.addEventListener("r", h, {});
  t3.addEventListener("r", h, { capture: false });
  reset(); t3.dispatchEvent(new Event("r"));
  console.log("M3.oneEntry=" + seen.join(","));
  t3.removeEventListener("r", h);
  reset(); t3.dispatchEvent(new Event("r"));
  console.log("M3.oneRemoveClears=" + JSON.stringify(seen.join(",")));

  const t4 = new EventTarget();
  const objs: Event[] = [];
  const collect = (ev: Event) => { objs.push(ev); };
  t4.addEventListener("s", collect, { capture: true });
  t4.addEventListener("s", collect, { capture: false });
  t4.dispatchEvent(new Event("s"));
  console.log("M4.sameEventObject=" + (objs.length === 2 && objs[0] === objs[1]));

  const t5 = new EventTarget();
  const phases: string[] = [];
  t5.addEventListener("u", function (ev: Event) {
    phases.push("phase=" + ev.eventPhase);
    phases.push("atTarget=" + (ev.eventPhase === Event.AT_TARGET));
    phases.push("target=" + (ev.target === t5));
    phases.push("current=" + (ev.currentTarget === t5));
  }, true);
  t5.dispatchEvent(new Event("u"));
  console.log("M5.atTarget=" + phases.join("|"));
  console.log("M5.constants=" + Event.NONE + "," + Event.CAPTURING_PHASE + "," + Event.AT_TARGET + "," + Event.BUBBLING_PHASE);

  const t6 = new EventTarget();
  let captured: any = null;
  t6.addEventListener("v", (ev: Event) => { captured = ev; });
  t6.dispatchEvent(new Event("v"));
  console.log("M6.afterDispatch=" + String(captured.currentTarget) + "," + (captured.target === t6) + "," + captured.eventPhase);

  const t7 = new EventTarget();
  const both = () => { seen.push("both"); };
  t7.addEventListener("w", both, { capture: true, once: true });
  t7.addEventListener("w", both, { capture: false });
  reset(); t7.dispatchEvent(new Event("w"));
  console.log("M7.onceCaptureFirst=" + seen.join(","));
  reset(); t7.dispatchEvent(new Event("w"));
  console.log("M7.onceCaptureSecond=" + seen.join(","));

  const t8 = new EventTarget();
  t8.addEventListener("x", () => { seen.push("passive"); }, { passive: true });
  reset(); t8.dispatchEvent(new Event("x"));
  console.log("M8.passive=" + seen.join(","));

  const t9 = new EventTarget();
  const opts: any = { capture: false };
  t9.addEventListener("y", () => { seen.push("y"); }, opts);
  opts.capture = true;
  t9.removeEventListener("y", () => {}, { capture: false });
  reset(); t9.dispatchEvent(new Event("y"));
  console.log("M9.optionsSnapshot=" + seen.join(","));
}

function sectionSubclassAndInit(): void {
  class Signal extends Event {
    detail: any;
    constructor(type: string, detail: any) {
      super(type, { cancelable: true });
      this.detail = detail;
    }
  }
  class Bus extends EventTarget {
    name: string;
    count: number;
    constructor(name: string) {
      super();
      this.name = name;
      this.count = 0;
    }
    emit(type: string, detail: any): boolean {
      this.count++;
      return this.dispatchEvent(new Signal(type, detail));
    }
  }

  const bus = new Bus("main");
  const seen: string[] = [];
  bus.addEventListener("ping", (ev: any) => {
    seen.push("type=" + ev.type);
    seen.push("detail=" + ev.detail);
    seen.push("isSignal=" + (ev instanceof Signal));
    seen.push("isEvent=" + (ev instanceof Event));
    seen.push("targetIsBus=" + (ev.target === bus));
    seen.push("currentTargetIsBus=" + (ev.currentTarget === bus));
    seen.push("busName=" + ev.target.name);
  });
  const ok = bus.emit("ping", "P1");
  console.log("S1.dispatchReturned=" + ok + " count=" + bus.count);
  console.log("S1.listenerSaw=" + seen.join(" "));

  const sig = new Signal("keep", "K");
  let inside: any = null;
  bus.addEventListener("keep", (ev: any) => { inside = ev; });
  bus.dispatchEvent(sig);
  console.log("S2.sameObject=" + (inside === sig) + " detailAfter=" + sig.detail);
  console.log("S2.protoChain=" + (Object.getPrototypeOf(sig) === Signal.prototype) + "," + (Object.getPrototypeOf(Signal.prototype) === Event.prototype));

  const bare = new Event("bare");
  console.log("S3.bareFlags=" + [bare.bubbles, bare.cancelable, bare.composed, bare.defaultPrevented].join(","));
  const full = new Event("full", { bubbles: true, cancelable: true, composed: true });
  console.log("S3.fullFlags=" + [full.bubbles, full.cancelable, full.composed].join(","));
  const extra = new Event("x", { bubbles: true, nonsense: 1 } as any);
  console.log("S3.extraIgnored=" + extra.bubbles + " hasNonsense=" + ("nonsense" in extra));
  let noType = "no";
  try { new (Event as any)(); } catch (e: any) { noType = e.constructor.name; }
  console.log("S3.noType=" + noType);

  console.log("S4.typeIsOwn=" + Object.prototype.hasOwnProperty.call(bare, "type"));
  const desc: any = Object.getOwnPropertyDescriptor(Event.prototype, "type");
  console.log("S4.typeAccessor=" + typeof desc.get + "," + typeof desc.set + "," + desc.enumerable + "," + desc.configurable);
  console.log("S4.detailIsOwn=" + Object.prototype.hasOwnProperty.call(sig, "detail"));
  console.log("S4.eventTag=" + Object.prototype.toString.call(bare));
  console.log("S4.busIsEventTarget=" + (bus instanceof EventTarget) + " ctorName=" + bus.constructor.name);
  console.log("S4.addIsInherited=" + (Object.prototype.hasOwnProperty.call(bus, "addEventListener") === false));

  const t9 = new EventTarget();
  let pathInside = "unset";
  t9.addEventListener("c", (ev: Event) => {
    const p = ev.composedPath();
    pathInside = "len=" + p.length + " first=" + (p[0] === t9);
  });
  const ev9 = new Event("c");
  console.log("S5.pathBefore=" + ev9.composedPath().length);
  t9.dispatchEvent(ev9);
  console.log("S5.pathDuring=" + pathInside);
  console.log("S5.pathAfter=" + ev9.composedPath().length);

  const t10 = new EventTarget();
  t10.addEventListener("p", (ev: Event) => { ev.preventDefault(); });
  const cancelable = new Event("p", { cancelable: true });
  const plain = new Event("p");
  console.log("S6.cancelableDispatch=" + t10.dispatchEvent(cancelable) + " defaultPrevented=" + cancelable.defaultPrevented);
  console.log("S6.plainDispatch=" + t10.dispatchEvent(plain) + " defaultPrevented=" + plain.defaultPrevented);

  const t3 = new EventTarget();
  t3.addEventListener("a", (ev: Event) => { ev.preventDefault(); });
  const e3 = new Event("a", { cancelable: false });
  console.log("S6.nonCancelablePrevented=" + t3.dispatchEvent(e3) + " defaultPrevented=" + e3.defaultPrevented);

  const t5 = new EventTarget();
  t5.addEventListener("c", () => false as any);
  const e5 = new Event("c", { cancelable: true });
  console.log("S6.returnFalseIgnored=" + t5.dispatchEvent(e5) + " defaultPrevented=" + e5.defaultPrevented);

  const cv = new CustomEvent("d");
  console.log("S7.customDefaultDetail=" + String(cv.detail) + " isEvent=" + (cv instanceof Event));
  console.log("S7.customProto=" + (Object.getPrototypeOf(CustomEvent.prototype) === Event.prototype));
  const payload = { k: 1, nested: { deep: true } };
  const c2 = new CustomEvent("e", { detail: payload, cancelable: true });
  console.log("S7.detailSameObject=" + (c2.detail === payload));
  const t11 = new EventTarget();
  t11.addEventListener("e", (ev: any) => { ev.detail.k = 99; });
  t11.dispatchEvent(c2);
  console.log("S7.mutatedDetail=" + payload.k);
  console.log("S7.detailValues=" + new CustomEvent("f", { detail: "s" }).detail + "," + new CustomEvent("f", { detail: 0 }).detail + "," + String(new CustomEvent("f", { detail: undefined }).detail));
  console.log("S7.numericType=" + JSON.stringify(new Event(1 as any).type));

  let plainObj = "no";
  try { (t11 as any).dispatchEvent({ type: "z" }); } catch (e: any) { plainObj = e.constructor.name; }
  console.log("S8.dispatchPlainObject=" + plainObj);
}

function sectionSignalOption(): void {
  let seen: string[] = [];
  const reset = () => { seen = []; };

  const t1 = new EventTarget();
  const c1 = new AbortController();
  t1.addEventListener("e", () => { seen.push("tied"); }, { signal: c1.signal });
  t1.addEventListener("e", () => { seen.push("plain"); });
  reset(); t1.dispatchEvent(new Event("e"));
  console.log("G1.beforeAbort=" + seen.join(","));
  c1.abort();
  reset(); t1.dispatchEvent(new Event("e"));
  console.log("G1.afterAbort=" + seen.join(","));

  const t2 = new EventTarget();
  t2.addEventListener("e", () => { seen.push("never"); }, { signal: AbortSignal.abort() });
  t2.addEventListener("e", () => { seen.push("survivor"); });
  reset(); t2.dispatchEvent(new Event("e"));
  console.log("G2.alreadyAborted=" + seen.join(","));

  const t4 = new EventTarget();
  const c4 = new AbortController();
  let onceCalls = 0;
  t4.addEventListener("e", () => { onceCalls++; }, { signal: c4.signal, once: true });
  t4.dispatchEvent(new Event("e"));
  t4.dispatchEvent(new Event("e"));
  c4.abort();
  t4.dispatchEvent(new Event("e"));
  console.log("G3.onceWithSignal=" + onceCalls);

  const ta = new EventTarget();
  const tb = new EventTarget();
  const c5 = new AbortController();
  ta.addEventListener("e", () => { seen.push("a1"); }, { signal: c5.signal });
  ta.addEventListener("e", () => { seen.push("a2"); }, { signal: c5.signal });
  tb.addEventListener("e", () => { seen.push("b1"); }, { signal: c5.signal });
  tb.addEventListener("e", () => { seen.push("b2"); });
  reset(); ta.dispatchEvent(new Event("e")); tb.dispatchEvent(new Event("e"));
  console.log("G4.manyBefore=" + seen.join(","));
  c5.abort();
  reset(); ta.dispatchEvent(new Event("e")); tb.dispatchEvent(new Event("e"));
  console.log("G4.manyAfter=" + seen.join(","));

  const t6 = new EventTarget();
  const c6 = new AbortController();
  const fn6 = () => { seen.push("f6"); };
  t6.addEventListener("e", fn6, { signal: c6.signal });
  t6.removeEventListener("e", fn6);
  reset(); t6.dispatchEvent(new Event("e"));
  console.log("G5.removedManually=" + JSON.stringify(seen.join(",")));
  c6.abort();
  reset(); t6.dispatchEvent(new Event("e"));
  console.log("G5.abortAfterRemoval=" + JSON.stringify(seen.join(",")));

  const t7 = new EventTarget();
  const c7a = new AbortController();
  const c7b = new AbortController();
  const fn7 = () => { seen.push("f7"); };
  t7.addEventListener("e", fn7, { signal: c7a.signal });
  c7a.abort();
  t7.addEventListener("e", fn7, { signal: c7b.signal });
  reset(); t7.dispatchEvent(new Event("e"));
  console.log("G6.reAdded=" + seen.join(","));
  c7b.abort();
  reset(); t7.dispatchEvent(new Event("e"));
  console.log("G6.secondSignalAborts=" + JSON.stringify(seen.join(",")));

  const t8 = new EventTarget();
  let badSignal = "no";
  try { t8.addEventListener("e", () => {}, { signal: {} } as any); } catch (e: any) { badSignal = e.constructor.name; }
  console.log("G7.badSignal=" + badSignal);

  const t9 = new EventTarget();
  const c9 = new AbortController();
  c9.signal.addEventListener("abort", () => { seen.push("signal-abort"); });
  t9.addEventListener("e", () => { seen.push("tied9"); }, { signal: c9.signal });
  reset();
  c9.abort();
  t9.dispatchEvent(new Event("e"));
  console.log("G8.orderOnAbort=" + seen.join(",") + " aborted=" + c9.signal.aborted);
}

function sectionNestedDispatch(): void {
  let seen: string[] = [];
  const reset = () => { seen = []; };

  const t1 = new EventTarget();
  t1.addEventListener("outer", () => { seen.push("outer1-start"); t1.dispatchEvent(new Event("inner")); seen.push("outer1-end"); });
  t1.addEventListener("outer", () => { seen.push("outer2"); });
  t1.addEventListener("inner", () => { seen.push("inner1"); });
  t1.addEventListener("inner", () => { seen.push("inner2"); });
  reset(); t1.dispatchEvent(new Event("outer"));
  console.log("N1.nestedOrder=" + seen.join(","));

  const t2 = new EventTarget();
  let depth = 0;
  t2.addEventListener("re", () => {
    depth++;
    seen.push("enter" + depth);
    if (depth < 3) t2.dispatchEvent(new Event("re"));
    seen.push("leave" + depth);
    depth--;
  });
  reset(); t2.dispatchEvent(new Event("re"));
  console.log("N2.reentrant=" + seen.join(","));

  const t3 = new EventTarget();
  t3.addEventListener("o", () => { seen.push("o1"); t3.dispatchEvent(new Event("i")); });
  t3.addEventListener("o", () => { seen.push("o2"); });
  t3.addEventListener("i", (ev: Event) => { seen.push("i1"); ev.stopImmediatePropagation(); });
  t3.addEventListener("i", () => { seen.push("i2"); });
  reset(); t3.dispatchEvent(new Event("i"));
  console.log("N3.innerAlone=" + seen.join(","));
  reset(); t3.dispatchEvent(new Event("o"));
  console.log("N3.innerStopDoesNotLeak=" + seen.join(","));

  const t4 = new EventTarget();
  const outerEv = new Event("o4", { cancelable: true });
  let innerPrevented = "unset";
  t4.addEventListener("o4", () => {
    const innerEv = new Event("i4", { cancelable: true });
    t4.dispatchEvent(innerEv);
    innerPrevented = String(innerEv.defaultPrevented);
  });
  t4.addEventListener("i4", (ev: Event) => { ev.preventDefault(); });
  const outerResult = t4.dispatchEvent(outerEv);
  console.log("N4.outerUntouched=" + outerEv.defaultPrevented + " returned=" + outerResult + " innerPrevented=" + innerPrevented);

  const src = new EventTarget();
  const dst = new EventTarget();
  src.addEventListener("go", (ev: Event) => {
    seen.push("srcSees=" + (ev.currentTarget === src));
    dst.dispatchEvent(new Event("relay"));
    seen.push("afterRelay=" + (ev.currentTarget === src));
  });
  dst.addEventListener("relay", (ev: Event) => { seen.push("dstSees=" + (ev.currentTarget === dst)); });
  reset(); src.dispatchEvent(new Event("go"));
  console.log("N5.crossTarget=" + seen.join(","));

  const t6 = new EventTarget();
  const selfRemoving = () => {
    seen.push("self");
    t6.removeEventListener("s", selfRemoving);
    seen.push("selfAfterRemove");
  };
  t6.addEventListener("s", selfRemoving);
  t6.addEventListener("s", () => { seen.push("other"); });
  reset(); t6.dispatchEvent(new Event("s"));
  console.log("N6.selfRemovalFirst=" + seen.join(","));
  reset(); t6.dispatchEvent(new Event("s"));
  console.log("N6.selfRemovalSecond=" + seen.join(","));

  const t7 = new EventTarget();
  const added = () => { seen.push("added"); };
  t7.addEventListener("a", () => { seen.push("a1"); t7.dispatchEvent(new Event("b")); });
  t7.addEventListener("b", () => { seen.push("b1"); t7.addEventListener("a", added); });
  reset(); t7.dispatchEvent(new Event("a"));
  console.log("N7.firstOuter=" + seen.join(","));
  reset(); t7.dispatchEvent(new Event("a"));
  console.log("N7.secondOuter=" + seen.join(","));

  const t9 = new EventTarget();
  let onceCalls = 0;
  t9.addEventListener("o9", () => {
    onceCalls++;
    if (onceCalls < 5) t9.dispatchEvent(new Event("o9"));
  }, { once: true });
  t9.dispatchEvent(new Event("o9"));
  console.log("N8.onceNoReentry=" + onceCalls);

  const outerT = new EventTarget();
  const innerT = new EventTarget();
  let snapshot = "unset";
  outerT.addEventListener("x", (ev: Event) => {
    const before = ev.eventPhase + ":" + (ev.currentTarget === outerT);
    innerT.dispatchEvent(new Event("y"));
    snapshot = "before=" + before + " after=" + ev.eventPhase + ":" + (ev.currentTarget === outerT);
  });
  innerT.addEventListener("y", () => {});
  outerT.dispatchEvent(new Event("x"));
  console.log("N9.outerFieldsIntact=" + snapshot);

  const t11 = new EventTarget();
  let reuse = "unset";
  const shared = new Event("s11");
  t11.addEventListener("s11", (ev: Event) => {
    try { t11.dispatchEvent(ev); reuse = "no-throw"; } catch (e: any) { reuse = e.constructor.name; }
  });
  t11.dispatchEvent(shared);
  console.log("N10.reDispatchInFlight=" + reuse);

  const t13 = new EventTarget();
  t13.addEventListener("L1", () => { seen.push("L1-in"); t13.dispatchEvent(new Event("L2")); seen.push("L1-out"); });
  t13.addEventListener("L2", () => { seen.push("L2-in"); t13.dispatchEvent(new Event("L3")); seen.push("L2-out"); });
  t13.addEventListener("L3", () => { seen.push("L3"); });
  reset(); t13.dispatchEvent(new Event("L1"));
  console.log("N11.threeLevels=" + seen.join(","));
}

async function sectionMicrotasksAroundDispatch(): Promise<void> {
  let seen: string[] = [];
  const t = new EventTarget();
  t.addEventListener("go", () => {
    seen.push("L1");
    queueMicrotask(() => { seen.push("L1-qm"); });
    Promise.resolve().then(() => { seen.push("L1-then"); });
  });
  t.addEventListener("go", () => {
    seen.push("L2");
    Promise.resolve().then(() => { seen.push("L2-then"); });
    queueMicrotask(() => { seen.push("L2-qm"); });
  });
  t.addEventListener("go", () => { seen.push("L3"); });

  seen.push("beforeDispatch");
  t.dispatchEvent(new Event("go"));
  seen.push("afterDispatch");
  queueMicrotask(() => { seen.push("tail-qm"); });
  console.log("T1.synchronousPart=" + seen.join(","));
  await tick();
  console.log("T1.afterDrain=" + seen.join(","));

  seen = [];
  t.dispatchEvent(new Event("go"));
  t.dispatchEvent(new Event("go"));
  console.log("T2.twoDispatchesSync=" + seen.length);
  await tick();
  console.log("T2.twoDispatchesDrained=" + seen.join(","));

  seen = [];
  queueMicrotask(() => { seen.push("mt1-start"); t.dispatchEvent(new Event("go")); seen.push("mt1-end"); });
  queueMicrotask(() => { seen.push("mt2"); });
  await tick();
  console.log("T3.dispatchInsideMicrotask=" + seen.join(","));

  seen = [];
  const t3 = new EventTarget();
  t3.addEventListener("a", async () => {
    seen.push("async-start");
    await null;
    seen.push("async-after-await");
  });
  t3.addEventListener("a", () => { seen.push("sync-second"); });
  const ret = t3.dispatchEvent(new Event("a"));
  seen.push("dispatchReturned=" + ret);
  console.log("T4.asyncListenerSync=" + seen.join(","));
  await tick();
  console.log("T4.asyncListenerDrained=" + seen.join(","));

  seen = [];
  t3.dispatchEvent(new Event("a"));
  t3.dispatchEvent(new Event("a"));
  console.log("T5.twoAsyncSync=" + seen.join(","));
  await tick();
  console.log("T5.twoAsyncDrained=" + seen.join(","));

  const t5 = new EventTarget();
  t5.addEventListener("c", async (ev: Event) => { await null; ev.preventDefault(); });
  const ev5 = new Event("c", { cancelable: true });
  const r5 = t5.dispatchEvent(ev5);
  console.log("T6.beforeAwaitPrevent=" + r5 + "," + ev5.defaultPrevented);
  await tick();
  console.log("T6.afterAwaitPrevent=" + ev5.defaultPrevented);

  seen = [];
  const t6 = new EventTarget();
  t6.addEventListener("i", () => {
    seen.push("i-start");
    queueMicrotask(() => { seen.push("i-micro"); });
    seen.push("i-end");
  });
  t6.dispatchEvent(new Event("i"));
  t6.dispatchEvent(new Event("i"));
  console.log("T7.noInterleave=" + seen.join(","));
  await tick();
  console.log("T7.noInterleaveDrained=" + seen.join(","));

  seen = [];
  const ctl = new AbortController();
  ctl.signal.addEventListener("abort", () => {
    seen.push("abort-listener");
    queueMicrotask(() => { seen.push("abort-micro"); });
  });
  seen.push("beforeAbort");
  ctl.abort();
  seen.push("afterAbort");
  console.log("T8.abortIsSynchronous=" + seen.join(","));
  await tick();
  console.log("T8.abortDrained=" + seen.join(","));

  seen = [];
  const t8 = new EventTarget();
  t8.addEventListener("k", async () => { seen.push("l1-sync"); await Promise.resolve(); seen.push("l1-resumed"); });
  t8.addEventListener("k", async () => { seen.push("l2-sync"); await Promise.resolve(); seen.push("l2-resumed"); });
  t8.addEventListener("k", () => { seen.push("l3-sync"); });
  t8.dispatchEvent(new Event("k"));
  await tick();
  console.log("T9.resumeAfterDispatch=" + seen.join(","));
}

async function main(): Promise<void> {
  sectionListenerOrder();
  sectionCapture();
  sectionSubclassAndInit();
  sectionSignalOption();
  sectionNestedDispatch();
  await sectionMicrotasksAroundDispatch();
  console.log("end");
}

main().catch(() => console.log("UNEXPECTED"));
