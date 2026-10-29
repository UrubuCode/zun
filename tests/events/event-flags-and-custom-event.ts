// WHATWG Event / CustomEvent flags: cancelation, defaultPrevented, phase,
// currentTarget, detail payloads, and constants.

const log = (label: string, value: unknown) => console.log(label + "=" + String(value));

// --- A. Event defaults and constants ------------------------------------------
{
  const ev = new Event("plain");
  log("A.type", ev.type);
  log("A.bubbles", ev.bubbles);
  log("A.cancelable", ev.cancelable);
  log("A.composed", ev.composed);
  log("A.defaultPreventedInitial", ev.defaultPrevented);
  log("A.isTrustedDispatchedFalse", ev.isTrusted);
  log("A.phaseBeforeDispatch", ev.eventPhase);
  log("A.currentTargetBeforeDispatch", ev.currentTarget);
  log("A.constants", [Event.NONE, Event.CAPTURING_PHASE, Event.AT_TARGET, Event.BUBBLING_PHASE].join(","));
  const init = new Event("opts", { bubbles: true, cancelable: true, composed: true });
  log("A.initBubbles", init.bubbles);
  log("A.initCancelable", init.cancelable);
  log("A.initComposed", init.composed);
  log("A.instanceofEvent", ev instanceof Event);
}

// --- B. Cancelation and defaultPrevented --------------------------------------
{
  const t = new EventTarget();
  t.addEventListener("cancel", (e) => e.preventDefault());
  const cancelable = new Event("cancel", { cancelable: true });
  log("B.cancelableBefore", cancelable.defaultPrevented);
  const r1 = t.dispatchEvent(cancelable);
  log("B.cancelableReturn", r1);
  log("B.cancelableAfter", cancelable.defaultPrevented);

  // preventDefault has no effect on a non-cancelable event.
  const nonCancel = new Event("cancel");
  const r2 = t.dispatchEvent(nonCancel);
  log("B.nonCancelableReturn", r2);
  log("B.nonCancelableDefaultPrevented", nonCancel.defaultPrevented);

  // A later listener observes that the event was already canceled.
  const u = new EventTarget();
  const seen = [];
  u.addEventListener("c2", (e) => { e.preventDefault(); });
  u.addEventListener("c2", (e) => seen.push(e.defaultPrevented));
  u.dispatchEvent(new Event("c2", { cancelable: true }));
  log("B.laterListenerSeesCanceled", seen.join(","));

  // A listener that does not cancel leaves the return value true.
  const v = new EventTarget();
  v.addEventListener("ok", () => {});
  log("B.noCancelReturn", v.dispatchEvent(new Event("ok", { cancelable: true })));
}

// --- C. Phase and currentTarget during dispatch -------------------------------
{
  const t = new EventTarget();
  const ev = new Event("phase");
  const trace = [];
  t.addEventListener("phase", (e) => {
    trace.push("phase=" + e.eventPhase);
    trace.push("current=" + (e.currentTarget === t));
    trace.push("target=" + (e.target === t));
  });
  t.dispatchEvent(ev);
  log("C.trace", trace.join(","));
  log("C.phaseAfter", ev.eventPhase);
  log("C.currentTargetAfter", ev.currentTarget);
  log("C.targetAfter", ev.target === t);
}

// --- D. CustomEvent detail payloads -------------------------------------------
{
  const t = new EventTarget();
  const got = [];
  t.addEventListener("job", (e) => {
    got.push(e instanceof CustomEvent);
    got.push(e instanceof Event);
    got.push(e.detail.id);
    got.push(e.detail.tags.join("|"));
  });
  t.dispatchEvent(new CustomEvent("job", { detail: { id: 7, tags: ["x", "y"] } }));
  log("D.customEventFields", got.join(","));

  const bare = new CustomEvent("bare");
  log("D.defaultDetailIsNull", bare.detail === null);
  log("D.bareType", bare.type);

  const numeric = new CustomEvent("num", { detail: 42 });
  log("D.numericDetail", numeric.detail + 1);

  const falsy = new CustomEvent("falsy", { detail: 0 });
  log("D.zeroDetail", falsy.detail === 0);

  const withFlags = new CustomEvent("flags", { detail: "d", bubbles: true, cancelable: true });
  log("D.flagsBubbles", withFlags.bubbles);
  log("D.flagsCancelable", withFlags.cancelable);
  log("D.flagsDetail", withFlags.detail);
}

// --- E. Same event object re-dispatched after completion ----------------------
{
  const t = new EventTarget();
  let hits = 0;
  t.addEventListener("again", () => hits++);
  const ev = new Event("again");
  t.dispatchEvent(ev);
  t.dispatchEvent(ev);
  log("E.redispatchHits", hits);
  log("E.redispatchPhaseAfter", ev.eventPhase);
}
