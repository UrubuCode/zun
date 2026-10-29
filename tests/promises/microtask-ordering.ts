// Microtask ordering: synchronous code versus reactions, FIFO across promises,
// reentrant scheduling, await and adoption costs measured against a ruler chain,
// thenable call order inside combinators, finally with a thenable, and the
// guarantee that every microtask runs before a timer. Each case prints one
// labelled line; the tick numbers are differences of ruler positions.

function out(label: string, value: unknown): void {
  console.log(label + "=" + String(value));
}

const marks: string[] = [];
function mark(tag: string): void {
  marks.push(tag);
}

// Ruler: R1..R14, each link one microtask after the previous one. Every
// ruler link is queued before any other work in this file.
let ruler: Promise<void> = Promise.resolve();
for (let i = 1; i <= 14; i++) {
  const k = i;
  ruler = ruler.then(() => mark("R" + k));
}

// Number of ruler links that ran before `tag` was recorded.
function tickOf(tag: string): number {
  const at = marks.indexOf(tag);
  if (at < 0) return -1;
  let ticks = 0;
  for (let k = 0; k < at; k++) if (/^R\d+$/.test(marks[k])) ticks++;
  return ticks;
}

async function main(): Promise<void> {
  // ---- A. synchronous code always precedes reactions ---------------------
  const syncLog: string[] = [];
  const pre = new Promise<void>((resolve) => {
    syncLog.push("executor");
    resolve();
    syncLog.push("afterResolve");
  });
  pre.then(() => { syncLog.push("reaction"); });
  syncLog.push("syncEnd");

  // A reaction on an already-settled promise still defers one tick.
  const deferred: string[] = [];
  deferred.push("before");
  Promise.resolve().then(() => { deferred.push("reaction"); });
  deferred.push("after");

  // ---- B. FIFO across promises: reactions run in registration order ------
  const order: string[] = [];
  const pa = Promise.resolve("a");
  const pb = Promise.resolve("b");
  pa.then(() => { order.push("a1"); });
  pb.then(() => { order.push("b1"); });
  pa.then(() => { order.push("a2"); });
  pb.then(() => { order.push("b2"); });
  pa.then(() => { order.push("a3"); });

  // ---- C. reentrant scheduling joins the tail of the queue ---------------
  const re: string[] = [];
  const p = Promise.resolve();
  p.then(() => {
    re.push("a");
    p.then(() => re.push("c"));
  });
  p.then(() => re.push("b"));
  Promise.resolve().then(() => re.push("d"));

  const fifo: string[] = [];
  Promise.resolve().then(() => {
    fifo.push("x");
    Promise.resolve().then(() => fifo.push("z"));
  });
  Promise.resolve().then(() => {
    fifo.push("y");
    Promise.resolve().then(() => fifo.push("w"));
  });
  Promise.resolve()
    .then(() => Promise.resolve())
    .then(() => Promise.resolve())
    .then(() => { fifo.push("chainEnd"); });

  // ---- D. await costs, measured against the ruler ------------------------
  (async () => { await 1; mark("D.plain"); })();
  (async () => { await Promise.resolve(2); mark("D.native"); })();
  (async () => { await { then(r: (v: number) => void) { r(3); } }; mark("D.thenable"); })();
  class MyP extends Promise<any> {}
  (async () => { await MyP.resolve(4); mark("D.subclass"); })();
  (async () => {
    await 5;
    mark("D.double1");
    await 5;
    mark("D.double2");
  })();
  Promise.resolve(6).then(() => { mark("D.bareThen"); });
  (async () => Promise.resolve(7))().then(() => { mark("D.returnPromise"); });
  (async () => 8)().then(() => { mark("D.returnValue"); });

  // ---- E. adoption costs: resolving with a promise or thenable ------------
  const donor = Promise.resolve("donor");
  new Promise((r) => r("plain")).then(() => { mark("E.execValue"); });
  new Promise((r) => r(donor)).then(() => { mark("E.execPromise"); });
  new Promise((r) => r({ then(rr: (v: string) => void) { rr("t"); } })).then(() => { mark("E.execThenable"); });
  Promise.resolve().then(() => "v").then(() => { mark("E.handlerValue"); });
  Promise.resolve().then(() => donor).then(() => { mark("E.handlerPromise"); });
  Promise.resolve().then(() => ({ then(rr: (v: string) => void) { rr("t"); } })).then(() => { mark("E.handlerThenable"); });
  Promise.resolve().then(() => Promise.resolve(donor)).then(() => { mark("E.doubleAdopt"); });

  // ---- F. thenable `then` calls happen in microtasks, in iteration order --
  const thenOrder: string[] = [];
  const slowT = { then(r: (v: string) => void) { thenOrder.push("slow-then"); Promise.resolve().then(() => r("S")); } };
  const fastT = { then(r: (v: string) => void) { thenOrder.push("fast-then"); r("F"); } };
  const allResult: string[] = [];
  Promise.all([slowT as any, fastT as any, Promise.resolve("P")]).then((vals) => {
    allResult.push(vals.join(","));
  });
  const raceFirst = { then(r: (v: string) => void) { thenOrder.push("race-first"); r("A"); } };
  const raceSecond = { then(r: (v: string) => void) { thenOrder.push("race-second"); r("B"); } };
  let raceWinner = "none";
  Promise.race([raceFirst as any, raceSecond as any]).then((v) => { raceWinner = v as string; });

  // ---- G. finally waits for a thenable returned by its callback ----------
  const fin: string[] = [];
  Promise.resolve("value")
    .finally(() => {
      fin.push("finally");
      return { then(r: () => void) { fin.push("thenable"); r(); } } as any;
    })
    .then((v) => { fin.push("got:" + v); });

  // ---- H. a timer runs only after every pending microtask ----------------
  let timerSnapshot: string[] = [];
  setTimeout(() => { timerSnapshot = marks.slice(); }, 0);

  // Drain well past every chain above.
  for (let i = 0; i < 40; i++) await null;
  await new Promise<void>((resolve) => setTimeout(resolve, 5));

  out("A.syncOrder", syncLog.join("|"));
  out("A.deferred", deferred.join("|"));
  out("B.registrationOrder", order.join(","));
  out("C.reentrant", re.join(","));
  out("C.fifo", fifo.join(","));
  out("D.arrivalOrder", marks.filter((m) => m.startsWith("D.")).join(","));
  out("D.plainTick", tickOf("D.plain"));
  out("D.nativeTick", tickOf("D.native"));
  out("D.thenableTick", tickOf("D.thenable"));
  out("D.subclassTick", tickOf("D.subclass"));
  out("D.double1Tick", tickOf("D.double1"));
  out("D.double2Tick", tickOf("D.double2"));
  out("D.bareThenTick", tickOf("D.bareThen"));
  out("D.returnValueTick", tickOf("D.returnValue"));
  out("D.returnPromiseTick", tickOf("D.returnPromise"));
  out("E.arrivalOrder", marks.filter((m) => m.startsWith("E.")).join(","));
  out("E.execPromiseCost", tickOf("E.execPromise") - tickOf("E.execValue"));
  out("E.handlerPromiseCost", tickOf("E.handlerPromise") - tickOf("E.handlerValue"));
  out("E.doubleAdoptCost", tickOf("E.doubleAdopt") - tickOf("E.handlerPromise"));
  out("F.thenCallOrder", thenOrder.join("|"));
  out("F.allResult", allResult.join(","));
  out("F.raceWinner", raceWinner);
  out("G.finallyThenable", fin.join("|"));
  out("H.timerSawMicrotasks", ["D.double2", "E.doubleAdopt"].every((m) => timerSnapshot.includes(m)) &&
    fifo.includes("chainEnd") && fin.includes("got:value") && allResult.length === 1);
  out("H.timerAfterRuler", timerSnapshot.includes("R14"));
  console.log("end");
}

main().catch((e) => console.log("UNEXPECTED " + String(e)));
