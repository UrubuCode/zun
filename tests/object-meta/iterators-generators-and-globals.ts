// The iterator protocol (custom iterables, generators with next/throw/return, zip and
// flatten helpers, spread and destructuring over iterators) and the globalThis surface
// (built-in identities, presence of standard globals, idempotent global registration).
export {};

// ---------- 1. Infinite generators consumed by a bounded helper ----------
function* naturals(start = 1) {
  let n = start;
  while (true) yield n++;
}
function take<T>(iter: Iterator<T>, n: number): T[] {
  const out: T[] = [];
  for (let i = 0; i < n; i++) {
    const { value, done } = iter.next();
    if (done) break;
    out.push(value);
  }
  return out;
}
console.log("take_1=" + take(naturals(), 5).join(","));
console.log("take_10=" + take(naturals(10), 4).join(","));

// ---------- 2. A hand-written iterable and iterator ----------
const range = {
  from: 3,
  to: 6,
  [Symbol.iterator]() {
    let cur = this.from;
    const end = this.to;
    return {
      next() { return cur <= end ? { value: cur++, done: false } : { value: undefined, done: true }; },
      return() { console.log("range_return_called"); return { value: undefined, done: true }; },
    };
  },
};
console.log("range_spread=" + [...range].join(","));
const [first, second] = range as any;
console.log("range_destructure=" + first + "," + second);
for (const v of range) { if (v === 4) break; }

// ---------- 3. zip and flatten as generators ----------
function* zip<A, B>(a: Iterable<A>, b: Iterable<B>): Generator<[A, B]> {
  const ia = a[Symbol.iterator](), ib = b[Symbol.iterator]();
  while (true) {
    const ra = ia.next(), rb = ib.next();
    if (ra.done || rb.done) return;
    yield [ra.value, rb.value];
  }
}
console.log("zip=" + [...zip([1, 2, 3], ["a", "b", "c"])].map(([n, s]) => n + s).join(","));
console.log("zip_short=" + [...zip([1, 2, 3, 4], ["x"])].length);

function* flatten(iter: Iterable<any>, depth = Infinity): Generator<any> {
  for (const item of iter) {
    if (depth > 0 && typeof item !== "string" && typeof item[Symbol.iterator] === "function") {
      yield* flatten(item, depth - 1);
    } else {
      yield item;
    }
  }
}
console.log("flatten_deep=" + [...flatten([1, [2, [3, [4]]], 5])].join(","));
console.log("flatten_depth1=" + [...flatten([1, [2, [3]], 4], 1)].map((v) => (Array.isArray(v) ? "arr" : v)).join(","));
console.log("flatten_strings=" + [...flatten(["ab", ["cd"]])].join(","));

// ---------- 4. Generator control: throw, return, done ----------
function* safeGen() {
  try {
    const v = yield "first";
    yield "second:" + v;
  } catch (e: any) {
    yield "caught:" + e.message;
  }
}
const g = safeGen();
console.log("gen_first=" + g.next().value);
console.log("gen_throw=" + g.throw(new Error("oops")).value);

function* countUp() { yield 1; yield 2; yield 3; }
const it = countUp();
console.log("gen_next=" + it.next().value);
console.log("gen_return=" + it.return(99).value + ",after_done=" + it.next().done);
console.log("gen_once_more=" + JSON.stringify(it.next()));

function* withFinally() {
  try { yield "a"; yield "b"; } finally { console.log("finally_ran"); }
}
const wf = withFinally();
console.log("finally_first=" + wf.next().value);
console.log("finally_return=" + JSON.stringify(wf.return("R")));

function* delegating() {
  const inner = yield* countUp();
  yield "inner-result:" + inner;
}
console.log("delegate=" + [...delegating()].join(","));

const sent: string[] = [];
function* echo() {
  while (true) { const v = yield; sent.push(String(v)); }
}
const ec = echo();
ec.next();
ec.next("x");
ec.next("y");
console.log("echo_sent=" + sent.join("|"));

// a generator object is its own iterator and reports the generator tag
const gen = countUp();
console.log("gen_self=" + (gen[Symbol.iterator]() === gen) + ",tag=" + Object.prototype.toString.call(gen));

// Array.from accepts an iterable with a mapping function
console.log("from_gen=" + Array.from(countUp(), (v) => v * 10).join(","));

// ---------- 5. globalThis ----------
console.log("global_type=" + typeof globalThis + ",self=" + (globalThis === globalThis));
const standard = ["Math", "JSON", "Promise", "Map", "Set", "WeakMap", "Proxy", "Reflect", "Symbol",
  "BigInt", "parseInt", "parseFloat", "isNaN", "isFinite", "encodeURIComponent"];
console.log("globals=" + standard.map((n) => n + ":" + typeof (globalThis as any)[n]).join(","));
console.log("identity=" + (globalThis.Array === Array) + "," + (globalThis.Object === Object) + "," + (globalThis.Error === Error));

// registration helper: the first registration wins, later factories are not run
function ensureGlobal(key: string, factory: () => any): any {
  if (!(key in globalThis)) (globalThis as any)[key] = factory();
  return (globalThis as any)[key];
}
const lib = ensureGlobal("__zunObjectMetaLib", () => ({ version: "1.0", ready: true }));
const lib2 = ensureGlobal("__zunObjectMetaLib", () => ({ version: "2.0", ready: false }));
console.log("ensure=" + lib.version + "," + lib.ready + "," + lib2.version + ",same=" + (lib === lib2));
console.log("ensure_defined=" + ("__zunObjectMetaLib" in globalThis) + ",own=" + Object.hasOwn(globalThis, "__zunObjectMetaLib"));
