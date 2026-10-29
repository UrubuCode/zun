// WHATWG EventTarget listener semantics: registration, ordering, once,
// removal, stop propagation flags, handleEvent objects, and mutation during dispatch.

const log = (label: string, value: unknown) => console.log(label + "=" + String(value));

// --- A. Basic registration and dispatch order ---------------------------------
{
  const t = new EventTarget();
  const order = [];
  const a = () => order.push("a");
  const b = () => order.push("b");
  const c = () => order.push("c");
  log("A.emptyDispatch", t.dispatchEvent(new Event("nothing")));
  t.addEventListener("ping", a);
  t.addEventListener("ping", b);
  t.addEventListener("ping", c);
  const r = t.dispatchEvent(new Event("ping"));
  log("A.order", order.join(","));
  log("A.returnValue", r);
  // Adding the same function for the same type+capture is a no-op.
  t.addEventListener("ping", a);
  order.length = 0;
  t.dispatchEvent(new Event("ping"));
  log("A.duplicateIgnored", order.join(","));
  // Other event types do not reach these listeners.
  order.length = 0;
  t.dispatchEvent(new Event("pong"));
  log("A.otherType", order.length);
  // Removing a listener that was never added is a silent no-op.
  t.removeEventListener("never-added", a);
  log("A.unknownRemoveOk", "yes");
}

// --- B. once option -----------------------------------------------------------
{
  const t = new EventTarget();
  let calls = 0;
  t.addEventListener("tick", () => calls++, { once: true });
  t.dispatchEvent(new Event("tick"));
  t.dispatchEvent(new Event("tick"));
  log("B.onceCalls", calls);
  // once listener removed before dispatch never fires.
  let skipped = 0;
  const s = () => skipped++;
  t.addEventListener("tock", s, { once: true });
  t.removeEventListener("tock", s);
  t.dispatchEvent(new Event("tock"));
  log("B.onceRemovedBeforeDispatch", skipped);
  // once + persistent listener on the same type: both run on first dispatch, only persistent on second.
  const seq = [];
  t.addEventListener("mix", () => seq.push("once"), { once: true });
  t.addEventListener("mix", () => seq.push("keep"));
  t.dispatchEvent(new Event("mix"));
  t.dispatchEvent(new Event("mix"));
  log("B.mixed", seq.join(","));
}

// --- C. removeEventListener and capture keys ----------------------------------
{
  const t = new EventTarget();
  const order = [];
  const a = () => order.push("a");
  const b = () => order.push("b");
  const c = () => order.push("c");
  t.addEventListener("r", a);
  t.addEventListener("r", b);
  t.addEventListener("r", c);
  t.removeEventListener("r", b);
  t.dispatchEvent(new Event("r"));
  log("C.afterRemoveMiddle", order.join(","));
  // capture is part of the listener identity.
  order.length = 0;
  const f = () => order.push("f");
  t.addEventListener("cap", f, true);
  t.removeEventListener("cap", f);
  t.dispatchEvent(new Event("cap"));
  log("C.captureNotRemovedWithoutFlag", order.join(","));
  // Capturing listener on the target itself still fires.
  let capFired = 0;
  t.addEventListener("cap2", () => capFired++, true);
  t.dispatchEvent(new Event("cap2"));
  log("C.captureOnTargetFires", capFired);
  // options object form of capture.
  let optFired = 0;
  t.addEventListener("cap3", () => optFired++, { capture: true });
  t.dispatchEvent(new Event("cap3"));
  log("C.captureOptionObjectFires", optFired);
}

// --- D. stopPropagation / stopImmediatePropagation ----------------------------
{
  const t = new EventTarget();
  const order = [];
  t.addEventListener("stop", (e) => { order.push("s1"); e.stopPropagation(); });
  t.addEventListener("stop", () => order.push("s2"));
  t.dispatchEvent(new Event("stop"));
  log("D.stopPropagationSameTarget", order.join(","));

  const u = new EventTarget();
  const seen = [];
  u.addEventListener("imm", (e) => { seen.push("i1"); e.stopImmediatePropagation(); });
  u.addEventListener("imm", () => seen.push("i2"));
  u.addEventListener("imm", () => seen.push("i3"));
  u.dispatchEvent(new Event("imm"));
  log("D.stopImmediate", seen.join(","));

  // stopImmediatePropagation only affects the current dispatch.
  let runs = 0;
  u.addEventListener("again", (e) => { runs++; e.stopImmediatePropagation(); });
  u.addEventListener("again", () => runs += 100);
  u.dispatchEvent(new Event("again"));
  u.dispatchEvent(new Event("again"));
  log("D.immediateEveryDispatch", runs);
}

// --- E. handleEvent objects ----------------------------------------------------
{
  const t = new EventTarget();
  const obj = {
    count: 0,
    seen: [],
    handleEvent(e) {
      this.count++;
      this.seen.push(e.type + ":" + (this === obj));
    },
  };
  t.addEventListener("obj", obj);
  t.dispatchEvent(new Event("obj"));
  t.dispatchEvent(new Event("obj"));
  log("E.handleEventCount", obj.count);
  log("E.handleEventThis", obj.seen.join(","));
  t.removeEventListener("obj", obj);
  t.dispatchEvent(new Event("obj"));
  log("E.handleEventRemoved", obj.count);

  const once = { hits: 0, handleEvent() { this.hits++; } };
  t.addEventListener("objOnce", once, { once: true });
  t.dispatchEvent(new Event("objOnce"));
  t.dispatchEvent(new Event("objOnce"));
  log("E.handleEventOnce", once.hits);
}

// --- F. Mutation of the listener list during dispatch ------------------------
{
  const t = new EventTarget();
  const order = [];
  const late = () => order.push("late");
  const victim = () => order.push("victim");
  t.addEventListener("mut", () => {
    order.push("first");
    t.addEventListener("mut", late);
    t.removeEventListener("mut", victim);
  });
  t.addEventListener("mut", victim);
  t.dispatchEvent(new Event("mut"));
  // victim was removed mid-dispatch, so it must not run in that dispatch.
  log("F.victimRanFirstDispatch", order.includes("victim"));
  order.length = 0;
  t.dispatchEvent(new Event("mut"));
  log("F.secondDispatch", order.join(","));
}

// --- G. Invalid arguments ------------------------------------------------------
{
  const t = new EventTarget();
  try {
    t.dispatchEvent({ type: "not-an-event" });
    log("G.plainObjectDispatch", "no-throw");
  } catch (err) {
    log("G.plainObjectDispatch", err instanceof TypeError ? "TypeError" : "other");
  }
}
