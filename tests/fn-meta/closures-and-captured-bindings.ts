// Closures capture bindings, not copies. Covers counters and factories, sibling
// closures sharing one cell, live reads after later writes, a parameter captured
// and rewritten, per-iteration `let` in for / for-of / nested loops against the
// shared `var`, closures created inside try / catch / finally, module-pattern
// IIFEs, direct eval mutating a captured binding, and recursion whose frames share
// an outer accumulator while each frame keeps its own locals.
export {};

const show = (label: string, value: unknown): void => {
  console.log(label + "=" + JSON.stringify(value));
};

// ---------------------------------------------------------------------------
// 1. Cells: one per call, shared between siblings, mutated from either side
// ---------------------------------------------------------------------------
function makeCounter(start: number): () => number {
  let value = start;
  return function (): number {
    value += 1;
    return value;
  };
}
const counterA = makeCounter(3);
show("counter_pair", [counterA(), counterA()]);
const counterB = makeCounter(0);
show("factory_independent", [counterA(), counterB(), counterA(), counterB()]);

function pair(): { add(n: number): number; read(): number } {
  let value = 0;
  return {
    add(n: number): number { value += n; return value; },
    read(): number { return value; },
  };
}
const p = pair();
show("sibling_cell", [p.add(2), p.read(), p.add(5), p.read()]);

function makePair(): { get(): number; set(x: number): void } {
  let v = 0;
  const get = (): number => v;
  const set = (x: number): void => { v = x; };
  return { get, set };
}
const pp = makePair();
const pq = makePair();
pp.set(99);
show("pairs_independent", [pp.get(), pq.get()]);

// Module pattern: several functions over one private binding.
const counterModule = (function () {
  let count = 0;
  let history: number[] = [];
  function bump(by: number): number {
    count += by;
    history.push(count);
    return count;
  }
  return {
    get: (): number => count,
    inc: (): number => bump(1),
    add: (n: number): number => bump(n),
    reset: (): number => { count = 0; history = []; return count; },
    size: (): number => history.length,
    trail: (): string => history.join("|"),
  };
})();
show("counterModule_inc", [counterModule.inc(), counterModule.inc()]);
show("counterModule_add", counterModule.add(10));
show("counterModule_state", [counterModule.get(), counterModule.size(), counterModule.trail()]);
show("counterModule_reset", [counterModule.reset(), counterModule.get(), counterModule.size()]);
show("counterModule_inc_after_reset", counterModule.inc());

function makeModule(start: number) {
  let value = start;
  return {
    get: (): number => value,
    inc: (): number => { value += 1; return value; },
    set: (v: number): number => { value = v; return value; },
  };
}
const m1 = makeModule(0);
const m2 = makeModule(100);
m1.inc();
m1.inc();
m2.inc();
show("counterModule_instances", [m1.get(), m2.get()]);
m1.set(50);
show("counterModule_after_set", [m1.get(), m2.get()]);

const looseInc = m2.inc;
const looseGet = m2.get;
looseInc();
show("detached_alias_shares_cell", [looseGet(), m2.get()]);

const isolated = [0, 1].map((seed) =>
  (function () {
    let v = seed;
    return { bump: (): number => { v += 10; return v; }, peek: (): number => v };
  })()
);
isolated[0].bump();
show("iife_siblings_isolated", [isolated[0].peek(), isolated[1].peek()]);

// ---------------------------------------------------------------------------
// 2. Live bindings: a closure reads the variable, not the value it had
// ---------------------------------------------------------------------------
let liveX = 1;
const readX = (): number => liveX;
show("live_before", readX());
liveX = 2;
liveX = liveX + 40;
show("live_after", readX());

let snapY = 1;
const snap = snapY;
const readSnap = (): number => snap;
const readLive = (): number => snapY;
snapY = 99;
show("snapshot_vs_live", [readSnap(), readLive()]);

function paramLive(q: number): () => number {
  const read = (): number => q;
  q = q * 3;
  return read;
}
show("param_mutated_after", paramLive(5)());

let early = 0;
const earlyFn = (): number => early;
early = 7;
const lateFn = (): number => early;
early = 8;
show("early_late_same_binding", [earlyFn(), lateFn()]);

function hoistedVar(): string {
  const read = (): number => hoistedW;
  var hoistedW = 3;
  const first = read();
  hoistedW = 4;
  return first + ":" + read();
}
show("var_hoisted_live", hoistedVar());

let impl = (): string => "first";
const callImpl = (): string => impl();
const captured = impl;
impl = (): string => "second";
show("reassign_binding", [callImpl(), captured()]);

let acc = "";
let tick = 0;
const record = (): void => { acc += tick; };
record();
tick = 1;
record();
tick = 2;
record();
show("interleaved_writes", acc);

let box = { n: 1 };
const readBox = (): number => box.n;
box.n = 2;
show("mutate_content", readBox());
box = { n: 3 };
show("rebind_binding", readBox());

// ---------------------------------------------------------------------------
// 3. Closures that write: a captured parameter is one binding
// ---------------------------------------------------------------------------
function makeParamCounter(n: number): () => number {
  return (): number => { n += 1; return n; };
}
const pc = makeParamCounter(10);
show("closure_writes_param", [pc(), pc(), pc()]);

function bodyMutates(s: string): string {
  const read = (): string => s;
  const first = read();
  s = s.toUpperCase();
  return first + ":" + read();
}
show("body_mutates_param", bodyMutates("ab"));

function paramAccessor(seed: number) {
  return {
    get: (): number => seed,
    set: (x: number): void => { seed = x; },
    bump: (): number => { seed += 5; return seed; },
  };
}
const acc2 = paramAccessor(1);
const first = acc2.get();
acc2.set(20);
show("accessor_pair", [first, acc2.get(), acc2.bump(), acc2.get()]);

function closureWrites(v: string): string {
  const set = (x: string): void => { v = x; };
  set("novo");
  return v;
}
show("closure_writes_outer", closureWrites("velho"));

function twoClosures(p0: number): number {
  const inc = (): void => { p0 = p0 + 1; };
  const read = (): number => p0;
  inc();
  inc();
  return read();
}
show("two_closures_one_param", twoClosures(0));

function destructured({ a, b }: { a: number; b: number }) {
  const read = (): string => a + "/" + b;
  a = a + 100;
  return { read, bumpB: (): number => { b += 1; return b; } };
}
const de = destructured({ a: 1, b: 2 });
show("destructured_before", de.read());
show("destructured_bump", [de.bumpB(), de.read()]);

function sharedParam(total: number) {
  return {
    addA: (x: number): void => { total += x; },
    addB: (x: number): void => { total += x * 10; },
    get: (): number => total,
  };
}
const sp = sharedParam(0);
sp.addA(1);
sp.addB(2);
sp.addA(3);
show("shared_param", sp.get());

let callerValue = 5;
function mutateArg(q: number): () => number {
  q = 999;
  return (): number => q;
}
const ma = mutateArg(callerValue);
show("arg_by_value", [callerValue, ma()]);

// ---------------------------------------------------------------------------
// 4. Loops: per-iteration let, shared var, nested loops, destructured heads
// ---------------------------------------------------------------------------
const forLet: Array<() => number> = [];
for (let i = 0; i < 3; i++) forLet.push(() => i);
show("for_let", [forLet[0](), forLet[1](), forLet[2]()]);

const forVar: Array<() => number> = [];
for (var j = 0; j < 3; j++) forVar.push(() => j);
show("for_var", [forVar[0](), forVar[1](), forVar[2]()]);

const forConst: Array<() => number> = [];
for (let q = 0; q < 3; q++) {
  const k = q;
  forConst.push(() => k);
}
show("for_const_snapshot", forConst.map((f) => f()));

const nestedLoops: Array<() => number> = [];
for (let i = 0; i < 3; i++) {
  for (let j2 = 0; j2 < 2; j2++) nestedLoops.push(() => i * 10 + j2);
}
show("nested_loops", nestedLoops.map((f) => f()));

const bodyMutated: Array<() => number> = [];
for (let i = 0; i < 3; i++) {
  i += 0;
  bodyMutated.push(() => i);
}
show("body_mutation_per_iteration", bodyMutated.map((f) => f()));

const perIterMutable: Array<(x: number) => number> = [];
for (let i = 0; i < 3; i++) {
  let total = 0;
  perIterMutable.push((x: number): number => {
    total += x * (i + 1);
    return total;
  });
}
show("per_iteration_state", [perIterMutable[0](1), perIterMutable[0](1), perIterMutable[1](1), perIterMutable[2](1), perIterMutable[2](1)]);

const forOf: Array<() => string> = [];
for (const ch of ["x", "y", "z"]) forOf.push(() => ch);
show("for_of_const", forOf.map((f) => f()).join(""));

const forOfLet: Array<() => number> = [];
for (let n of [1, 2, 3]) {
  n = n * 10;
  forOfLet.push(() => n);
}
show("for_of_let_mutated", forOfLet.map((f) => f()));

const forOfVar: Array<() => number> = [];
for (var vv of [1, 2, 3]) forOfVar.push(() => vv);
show("for_of_var", forOfVar.map((f) => f()));

const forOfDestructure: Array<() => string> = [];
for (const [key, val] of [["a", 1], ["b", 2]] as Array<[string, number]>) {
  forOfDestructure.push(() => key + val);
}
show("for_of_destructure", forOfDestructure.map((f) => f()));

const forOfNested: Array<() => string> = [];
for (const i of [1, 2]) {
  for (const j3 of ["a", "b"]) forOfNested.push(() => i + j3);
}
show("for_of_nested", forOfNested.map((f) => f()));

const getters: Array<() => number> = [];
const setters: Array<(x: number) => void> = [];
for (const seed of [10, 20]) {
  let cell = seed;
  getters.push(() => cell);
  setters.push((x: number): void => { cell = x; });
}
setters[0](111);
show("for_of_independent_cells", [getters[0](), getters[1]()]);

// ---------------------------------------------------------------------------
// 5. try / catch / finally
// ---------------------------------------------------------------------------
function tryThenFinally(): () => string {
  let state = "try";
  try {
    return () => state;
  } finally {
    state = "finally";
  }
}
show("mutated_in_finally", tryThenFinally()());

function returnValueVsBinding(): string {
  let n = 1;
  const read = (): number => n;
  function inner(): number {
    try {
      return n;
    } finally {
      n = 100;
    }
  }
  const returned = inner();
  return "returned=" + returned + " closure=" + read();
}
show("return_value_vs_binding", returnValueVsBinding());

function catchBinding(): string {
  let outer = "outer";
  let readErr: () => string = () => "none";
  try {
    throw new Error("boom");
  } catch (e) {
    const msg = (e as Error).message;
    readErr = () => msg + "/" + outer;
  }
  outer = "changed";
  return readErr();
}
show("catch_binding", catchBinding());

function catchShadow(): string {
  let e = "outer_e";
  let read: () => string = () => "x";
  try {
    throw "thrown_e";
  } catch (e) {
    read = () => String(e);
  }
  return read() + ":" + e;
}
show("catch_shadow", catchShadow());

function finallyNoReturn(): string {
  let tag = "a";
  try {
    tag = "b";
    return "from_try:" + tag;
  } finally {
    tag = "c";
  }
}
show("finally_no_return", finallyNoReturn());

function madeInFinally(): () => string {
  let v = "start";
  let f: () => string = () => "unset";
  try {
    v = "in_try";
  } finally {
    f = () => v;
    v = "in_finally";
  }
  return f;
}
show("made_in_finally", madeInFinally()());

const tryFns: Array<() => string> = [];
for (let i = 0; i < 3; i++) {
  try {
    if (i === 1) throw new Error("skip");
    tryFns.push(() => "ok" + i);
  } catch {
    tryFns.push(() => "err" + i);
  }
}
show("loop_try", tryFns.map((f) => f()));

function partial(): string {
  const log: string[] = [];
  let count = 0;
  try {
    for (const step of ["a", "b", "c"]) {
      if (step === "c") throw new Error("stop");
      count += 1;
      log.push(step);
    }
  } catch {
    log.push("caught");
  } finally {
    log.push("count=" + count);
  }
  return log.join(",");
}
show("partial", partial());

// ---------------------------------------------------------------------------
// 6. Shadowing, nesting and direct eval
// ---------------------------------------------------------------------------
const shadowX = "outer";
function makeShadow(x: string) {
  return function middle() {
    const y = x + "-middle";
    return function inner(x: string) {
      return [x, y].join("|");
    };
  };
}
show("nearest_shadow", makeShadow("param")()("inner"));
show("outer_untouched", shadowX);

function multilevel(a: number) {
  return function middle(b: number) {
    return function inner(c: number): number { return a + b + c; };
  };
}
show("multilevel", [multilevel(1)(2)(3), multilevel(10)(20)(30)]);
const mm = multilevel(100);
show("multilevel_shared_outer", [mm(5)(1), mm(6)(2)]);

function makeEvalCell() {
  let x = 1;
  return {
    inc(): number { eval("x += 2"); return x; },
    read(): number { return x; },
  };
}
const evalCell = makeEvalCell();
show("eval_mutates_outer", [evalCell.inc(), evalCell.inc(), evalCell.read()]);

// ---------------------------------------------------------------------------
// 7. Recursion: frames share an outer accumulator, not a per-frame copy
// ---------------------------------------------------------------------------
function sumTo(n: number): number {
  let total = 0;
  function go(k: number): void {
    if (k <= 0) return;
    total += k;
    go(k - 1);
  }
  go(n);
  return total;
}
show("shared_total", sumTo(5));

function countCalls(n: number): string {
  let calls = 0;
  function fib(k: number): number {
    calls += 1;
    return k < 2 ? k : fib(k - 1) + fib(k - 2);
  }
  const v = fib(n);
  return v + "/" + calls;
}
show("fib_calls", countCalls(6));

function trail(n: number): string {
  const out: string[] = [];
  function walk(k: number): void {
    out.push("in" + k);
    if (k > 0) walk(k - 1);
    out.push("out" + k);
  }
  walk(n);
  return out.join(",");
}
show("trail", trail(2));

function maxDepth(n: number): number {
  let depth = 0;
  let best = 0;
  function go(k: number): void {
    depth += 1;
    if (depth > best) best = depth;
    if (k > 0) {
      go(k - 1);
      go(k - 1);
    }
    depth -= 1;
  }
  go(n);
  return best;
}
show("max_depth", maxDepth(3));

function memoFib(n: number): string {
  const memo: Record<number, number> = {};
  let hits = 0;
  function f(k: number): number {
    if (k in memo) {
      hits += 1;
      return memo[k];
    }
    const r = k < 2 ? k : f(k - 1) + f(k - 2);
    memo[k] = r;
    return r;
  }
  const v = f(n);
  return v + "/hits=" + hits;
}
show("memo", memoFib(10));

function mutual(n: number): string {
  const log: string[] = [];
  function even(k: number): boolean {
    log.push("e" + k);
    return k === 0 ? true : odd(k - 1);
  }
  function odd(k: number): boolean {
    log.push("o" + k);
    return k === 0 ? false : even(k - 1);
  }
  const r = even(n);
  return r + ":" + log.join("");
}
show("mutual_shared_log", mutual(4));

function perFrame(n: number): number {
  function go(k: number): number {
    const local = k;
    if (k > 0) go(k - 1);
    return local;
  }
  return go(n);
}
show("per_frame_local", perFrame(3));

let globalCalls = 0;
function tickDown(k: number): number {
  globalCalls += 1;
  return k > 0 ? tickDown(k - 1) : globalCalls;
}
show("counterModule_accumulator", [tickDown(2), tickDown(2), globalCalls]);
