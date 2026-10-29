// Cross-runtime: event-loop ordering fixed by the spec. Every line is about ORDER:
// the sync part, the microtask queue (queueMicrotask, then, await share one FIFO),
// the macrotask queue (setTimeout with delay 0 only), and the value each settles with.
// No clock is ever read and no non-zero delay is used.

const tick = (): Promise<void> => new Promise<void>((resolve) => setTimeout(resolve, 0));

async function sectionSyncVsMicroVsMacro(): Promise<void> {
  const out: string[] = [];
  out.push("sync:start");
  setTimeout(() => out.push("macro:timeout"), 0);
  queueMicrotask(() => out.push("micro:qm1"));
  Promise.resolve().then(() => out.push("micro:then1"));
  queueMicrotask(() => out.push("micro:qm2"));
  Promise.resolve().then(() => out.push("micro:then2"));
  out.push("sync:end");
  await tick();
  console.log("A.order=" + out.join("|"));
}

async function sectionChains(): Promise<void> {
  const chain: string[] = [];
  const c1 = Promise.resolve()
    .then(() => { chain.push("1"); return 1; })
    .then(() => { chain.push("2"); return 2; })
    .then(() => { chain.push("3"); });
  const c2 = Promise.resolve()
    .then(() => { chain.push("A"); })
    .then(() => { chain.push("B"); })
    .then(() => { chain.push("C"); });
  await Promise.all([c1, c2]);
  console.log("B.chains=" + chain.join(","));
}

async function sectionInterleave(): Promise<void> {
  const inter: string[] = [];
  async function taskX(tag: string): Promise<void> {
    inter.push(tag + "1");
    await null;
    inter.push(tag + "2");
    await null;
    inter.push(tag + "3");
  }
  await Promise.all([taskX("A"), taskX("B")]);
  console.log("C.interleave=" + inter.join(","));
  const rejected = Promise.reject(new Error("handled"));
  let caught = "none";
  rejected.catch((e: any) => { caught = "caught:" + e.message; });
  await tick();
  console.log("C.rejectionCaught=" + caught);
}

async function sectionAwaitParks(): Promise<void> {
  const park: string[] = [];
  async function f(): Promise<void> {
    park.push("body-start");
    await Promise.resolve();
    park.push("after-await");
  }
  const pf = f();
  park.push("caller-continues");
  Promise.resolve().then(() => park.push("then-attached-later"));
  await pf;
  park.push("main-resumed");
  console.log("D.park=" + park.join("|"));

  let total = 0;
  for (let i = 1; i <= 3; i++) {
    total += await Promise.resolve(i);
  }
  console.log("D.accumulated=" + total);
}

async function sectionSettleAndReject(): Promise<void> {
  const log2: string[] = [];
  async function h(): Promise<number> {
    try {
      await Promise.reject(new Error("boom"));
      log2.push("unreachable");
    } catch (e) {
      log2.push("caught " + (e as Error).message);
    }
    return 1;
  }
  async function thrower(): Promise<void> {
    throw new Error("escaped");
  }
  async function g(): Promise<string> {
    await Promise.resolve();
    return "value";
  }
  const hp = h().then((v) => { log2.push("h-settled " + v); });
  const tp = thrower().catch((e) => { log2.push("thrower-rejected " + (e as Error).message); });
  const gp = g().then((v) => { log2.push("g-settled " + v); });
  await Promise.all([hp, tp, gp]);
  console.log("E.settle=" + log2.join("|"));
}

async function sectionFifo(): Promise<void> {
  const steps: string[] = [];
  async function chain(tag: string, depth: number): Promise<void> {
    for (let i = 1; i <= depth; i++) {
      await null;
      steps.push(tag + i);
    }
  }
  chain("A", 4);
  chain("B", 4);
  function ladder(tag: string, left: number): void {
    queueMicrotask(() => {
      steps.push(tag + (5 - left));
      if (left > 1) ladder(tag, left - 1);
    });
  }
  ladder("Q", 4);
  let chainP: Promise<void> = Promise.resolve();
  for (let i = 1; i <= 4; i++) {
    const k = i;
    chainP = chainP.then(() => { steps.push("T" + k); });
  }
  steps.push("sync");
  await tick();
  console.log("F.interleave=" + steps.join(","));

  steps.length = 0;
  for (let i = 1; i <= 5; i++) {
    const k = i;
    queueMicrotask(() => { steps.push("q" + k); });
    Promise.resolve().then(() => { steps.push("p" + k); });
  }
  await tick();
  console.log("F.strictFifo=" + steps.join(","));

  steps.length = 0;
  queueMicrotask(() => {
    steps.push("m1");
    queueMicrotask(() => { steps.push("m1a"); });
    queueMicrotask(() => { steps.push("m1b"); });
  });
  queueMicrotask(() => { steps.push("m2"); });
  queueMicrotask(() => { steps.push("m3"); });
  await tick();
  console.log("F.tailInsertion=" + steps.join(","));

  steps.length = 0;
  (async () => {
    for (let i = 1; i <= 3; i++) {
      await null;
      steps.push("deep" + i);
    }
  })();
  let rung: Promise<void> = Promise.resolve();
  for (let i = 1; i <= 3; i++) {
    const k = i;
    rung = rung.then(() => { steps.push("rung" + k); });
  }
  await tick();
  console.log("F.oneTickPerAwait=" + steps.join(","));

  steps.length = 0;
  (async () => { await null; steps.push("awaited"); })();
  queueMicrotask(() => { steps.push("qm-after"); });
  await tick();
  console.log("F.awaitIsMicrotask=" + steps.join(","));

  steps.length = 0;
  Promise.resolve().finally(() => {
    steps.push("finallyCb");
    queueMicrotask(() => { steps.push("fromFinally"); });
  });
  queueMicrotask(() => { steps.push("sibling"); });
  await tick();
  console.log("F.finallyReaction=" + steps.join(","));

  let argc = -1;
  queueMicrotask(function () { argc = arguments.length; });
  await tick();
  console.log("F.callbackArgs=" + argc);
  console.log("F.returnValue=" + String(queueMicrotask(() => {})));
  await tick();
}

async function sectionNestedDrain(): Promise<void> {
  const seq: string[] = [];
  seq.push("sync start");
  queueMicrotask(() => { seq.push("qm A"); });
  Promise.resolve().then(() => { seq.push("then A"); });
  queueMicrotask(() => { seq.push("qm B"); });
  Promise.resolve().then(() => { seq.push("then B"); });
  queueMicrotask(() => {
    seq.push("qm C (queues C1, C2)");
    queueMicrotask(() => { seq.push("qm C1"); });
    Promise.resolve().then(() => { seq.push("then C2"); });
  });
  Promise.resolve().then(() => {
    seq.push("then D (queues D1)");
    queueMicrotask(() => { seq.push("qm D1"); });
  });
  Promise.resolve()
    .then(() => { seq.push("chain 1"); })
    .then(() => { seq.push("chain 2"); })
    .then(() => { seq.push("chain 3"); });
  (async () => {
    seq.push("async before await");
    await undefined;
    seq.push("async after first await");
    await undefined;
    seq.push("async after second await");
  })();
  let depth = 0;
  function recur(): void {
    depth++;
    if (depth <= 3) {
      seq.push("recur depth " + depth);
      queueMicrotask(recur);
    } else {
      seq.push("recur done at " + depth);
    }
  }
  queueMicrotask(recur);
  seq.push("sync end");
  await tick();
  console.log("G.drain=" + seq.join("|"));

  const contained: string[] = [];
  queueMicrotask(() => {
    try {
      (() => { throw new RangeError("contained"); })();
    } catch (e: any) {
      contained.push("caught inside microtask: " + e.constructor.name);
    }
  });
  await tick();
  console.log("G.contained=" + contained.join(","));
}

async function sectionTimers(): Promise<void> {
  const steps: string[] = [];
  setTimeout(() => {
    steps.push("timer1");
    queueMicrotask(() => { steps.push("t1-micro"); });
    Promise.resolve().then(() => { steps.push("t1-then"); });
  }, 0);
  setTimeout(() => {
    steps.push("timer2");
    queueMicrotask(() => { steps.push("t2-micro"); });
  }, 0);
  const cleared = setTimeout(() => { steps.push("never"); }, 0);
  clearTimeout(cleared);
  let forwarded = "unset";
  setTimeout(function (a: any, b: any) { forwarded = a + "/" + b + "/" + arguments.length; }, 0, "x", "y");
  const late: string[] = [];
  setTimeout(() => {
    late.push("outer");
    setTimeout(() => { late.push("nested"); }, 0);
  }, 0);
  setTimeout(() => { late.push("sibling"); }, 0);
  await new Promise<void>((resolve) => setTimeout(() => { steps.push("timer3"); resolve(); }, 0));
  console.log("H.timeline=" + steps.join(","));
  console.log("H.forwarded=" + forwarded);
  console.log("H.handleIsTruthy=" + (typeof cleared === "object" || typeof cleared === "number"));
  await tick();
  console.log("H.late=" + late.join(","));
  console.log("H.clearedNeverRan=" + (steps.indexOf("never") < 0));
}

async function sectionPromiseValues(): Promise<void> {
  const vals: string[] = [];
  await Promise.resolve(7).then((v) => { vals.push("num=" + v + ":" + typeof v); });
  await Promise.resolve(-42).then((v) => { vals.push("neg=" + v); });
  await Promise.resolve(3.5).then((v) => { vals.push("frac=" + v); });
  await Promise.resolve(0).then((v) => { vals.push("zero=" + v); });
  await Promise.resolve(5).then((v) => v * 2).then((v) => { vals.push("chain=" + v); });
  await Promise.resolve(true).then((v) => { vals.push("bool=" + v); });
  await Promise.resolve(null).then((v) => { vals.push("null=" + v); });
  await Promise.resolve("oi").then((v) => { vals.push("str=" + v); });
  await Promise.resolve({ a: 1 }).then((v) => { vals.push("obj=" + JSON.stringify(v)); });
  await Promise.resolve([1, 2]).then((v) => { vals.push("arr=" + v.join(",")); });
  async function viaAwait(): Promise<number> { return await Promise.resolve(11); }
  vals.push("await=" + (await viaAwait()));
  console.log("I.values=" + vals.join("|"));
}

async function sectionMessageChannel(): Promise<void> {
  const ch = new MessageChannel();
  const got = await new Promise<string>((resolve) => {
    ch.port1.onmessage = (ev: any) => { resolve("" + ev.data); };
    ch.port2.postMessage("pong");
  });
  ch.port1.close();
  ch.port2.close();
  console.log("J.message=" + got);
}

async function sectionForAwait(): Promise<void> {
  const pulls: string[] = [];
  function makeIterable(values: any[], tag: string): any {
    return {
      [Symbol.iterator]: function () {
        let i = 0;
        return {
          next: function () {
            pulls.push(tag + "-next" + i);
            if (i >= values.length) return { done: true, value: undefined };
            return { done: false, value: values[i++] };
          },
          return: function (v: any) { pulls.push(tag + "-return"); return { done: true, value: v }; },
        };
      },
    };
  }

  const got1: string[] = [];
  for await (const v of makeIterable([1, 2, 3], "a")) got1.push(String(v));
  console.log("K.plainValues=" + got1.join(",") + " pulls=" + pulls.join(","));

  pulls.length = 0;
  const got2: string[] = [];
  for await (const v of makeIterable([Promise.resolve("p1"), Promise.resolve("p2")], "b")) got2.push(String(v));
  console.log("K.promises=" + got2.join(",") + " pulls=" + pulls.join(","));

  pulls.length = 0;
  const th = (v: string) => ({ then: (r: any) => { r(v); } });
  const got3: string[] = [];
  for await (const v of makeIterable([th("t1"), th("t2")], "c")) got3.push(String(v));
  console.log("K.thenables=" + got3.join(",") + " pulls=" + pulls.join(","));

  const marks: string[] = [];
  let rungs: Promise<void> = Promise.resolve();
  for (let i = 1; i <= 8; i++) {
    const k = i;
    rungs = rungs.then(() => { marks.push("r" + k); });
  }
  for await (const v of makeIterable(["x", "y", "z"], "d")) marks.push("it" + v);
  console.log("K.interleaved=" + marks.join(","));

  const got5: string[] = [];
  let caught5 = "none";
  try {
    for await (const v of makeIterable(["ok", Promise.reject("BOOM"), "never"], "e")) got5.push(String(v));
  } catch (e: any) {
    caught5 = String(e);
  }
  console.log("K.rejectedEntry=" + got5.join(",") + " caught=" + caught5);

  pulls.length = 0;
  const got6: string[] = [];
  for await (const v of makeIterable([1, 2, 3, 4], "f")) {
    got6.push(String(v));
    if (v === 2) break;
  }
  console.log("K.break=" + got6.join(",") + " pulls=" + pulls.join(","));

  const got7: string[] = [];
  for await (const v of [Promise.resolve("A"), "B", Promise.resolve("C")]) got7.push(String(v));
  console.log("K.arrayOfPromises=" + got7.join(","));

  const both: any = {
    [Symbol.iterator]: function () { return { next: function () { return { done: true, value: undefined }; } }; },
    [Symbol.asyncIterator]: function () {
      let i = 0;
      return {
        next: function () {
          i++;
          return Promise.resolve(i <= 2 ? { done: false, value: "async" + i } : { done: true, value: undefined });
        },
      };
    },
  };
  const got8: string[] = [];
  for await (const v of both) got8.push(String(v));
  console.log("K.prefersAsyncIterator=" + got8.join(","));

  async function* agen(): AsyncGenerator<string> {
    yield "g1";
    yield Promise.resolve("g2") as any;
    yield "g3";
  }
  const got9: string[] = [];
  for await (const v of agen()) got9.push(String(v));
  console.log("K.asyncGenerator=" + got9.join(","));

  pulls.length = 0;
  let bodyRuns = 0;
  for await (const v of makeIterable([], "g")) { bodyRuns++; }
  console.log("K.empty=" + bodyRuns + " pulls=" + pulls.join(","));

  const got11: string[] = [];
  for await (const entry of new Map([["k1", "v1"], ["k2", "v2"]])) got11.push(entry[0] + "=" + entry[1]);
  for await (const v of new Set(["s1", "s2"])) got11.push(String(v));
  for await (const ch of "abc") got11.push(ch);
  console.log("K.builtins=" + got11.join(","));

  const syncResults: any = {
    [Symbol.asyncIterator]: function () {
      let i = 0;
      return { next: function () { i++; return i <= 2 ? { done: false, value: "r" + i } : { done: true, value: undefined }; } };
    },
  };
  const got12: string[] = [];
  for await (const v of syncResults) got12.push(String(v));
  console.log("K.plainResultObjects=" + got12.join(","));

  pulls.length = 0;
  const got13: string[] = [];
  for await (const v of makeIterable([1, 2, 3, 4], "h")) {
    if ((v as any) % 2 === 0) continue;
    got13.push(String(v));
  }
  console.log("K.continue=" + got13.join(",") + " pulls=" + pulls.join(","));

  pulls.length = 0;
  const returned = await (async function () {
    for await (const v of makeIterable([1, 2, 3], "i")) {
      if ((v as any) === 2) return "left-at-" + v;
    }
    return "drained";
  })();
  console.log("K.returnFromLoop=" + returned + " pulls=" + pulls.join(","));

  let notIterable = "no";
  try {
    for await (const v of {} as any) { notIterable = "iterated " + v; }
  } catch (e: any) {
    notIterable = e.constructor.name;
  }
  console.log("K.notIterable=" + notIterable);
}

async function main(): Promise<void> {
  await sectionSyncVsMicroVsMacro();
  await sectionChains();
  await sectionInterleave();
  await sectionAwaitParks();
  await sectionSettleAndReject();
  await sectionFifo();
  await sectionNestedDrain();
  await sectionTimers();
  await sectionPromiseValues();
  await sectionMessageChannel();
  await sectionForAwait();
  console.log("end");
}

main().catch(() => console.log("UNEXPECTED"));
