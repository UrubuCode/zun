// Functional composition, recursion that does not grow the stack, lifted
// recursive closures, monads and parsers. Every output line is a labelled
// assertion so a divergence points at exactly one case.

type Fn = (x: any) => any;
type Reducer<A, B> = (acc: A, val: B) => A;

// ---------------------------------------------------------------- composition

const pipe = (...fns: Fn[]) => (x: any) => fns.reduce((v, f) => f(v), x);
const compose = (...fns: Fn[]) => (x: any) => fns.reduceRight((v, f) => f(v), x);

const double = (x: number) => x * 2;
const addOne = (x: number) => x + 1;
const square = (x: number) => x * x;
const toStr = (x: number) => "val:" + x;

console.log("pipe.3=" + pipe(double, addOne, square, toStr)(3)); // (3*2+1)^2 = 49
console.log("compose.3=" + compose(toStr, square, addOne, double)(3)); // same chain, reversed
console.log("pipe.empty=" + pipe()(42)); // zero stages is the identity
console.log("compose.empty=" + compose()("same"));
console.log("pipe.single=" + pipe(addOne)(-1));
console.log("order.pipe=" + pipe(addOne, double)(5) + " order.compose=" + compose(addOne, double)(5));

const partial = (fn: (...a: any[]) => any, ...pre: any[]) => (...rest: any[]) => fn(...pre, ...rest);
const mul = (a: number, b: number) => a * b;
const triple = partial(mul, 3);
console.log("partial.7=" + triple(7));
console.log("partial.10=" + triple(10));
console.log("partial.none=" + partial(mul)(4, 5));

const flip = (fn: (...a: any[]) => any) => (a: any, b: any, ...rest: any[]) => fn(b, a, ...rest);
const sub = (a: number, b: number) => a - b;
console.log("sub.10.3=" + sub(10, 3));
console.log("flip.sub.10.3=" + flip(sub)(10, 3));
console.log("flip.flip.sub.10.3=" + flip(flip(sub))(10, 3));

// ---------------------------------------------------------------- transducers

function transduce<A, B>(
  xf: (r: Reducer<A, B>) => Reducer<A, B>,
  reducer: Reducer<A, B>,
  init: A,
  coll: B[]
): A {
  return coll.reduce(xf(reducer), init);
}
const pushInto: Reducer<number[], number> = (acc, v) => {
  acc.push(v);
  return acc;
};
const mapXf = (fn: (x: number) => number) =>
  (next: Reducer<number[], number>): Reducer<number[], number> =>
    (acc, val) => next(acc, fn(val));
const filterXf = (pred: (x: number) => boolean) =>
  (next: Reducer<number[], number>): Reducer<number[], number> =>
    (acc, val) => (pred(val) ? next(acc, val) : acc);
const evenSquares = (r: Reducer<number[], number>) =>
  filterXf((x) => x % 2 === 0)(mapXf((x) => x * x)(r));

console.log("transduce.evenSquares=" + transduce(evenSquares, pushInto, [] as number[], [1, 2, 3, 4, 5, 6]).join(","));
console.log("transduce.empty=" + transduce(evenSquares, pushInto, [] as number[], []).length);

const mapR = <A>(fn: (x: number) => number) =>
  (next: Reducer<A, number>): Reducer<A, number> => (acc, v) => next(acc, fn(v));
const filterR = <A>(pred: (x: number) => boolean) =>
  (next: Reducer<A, number>): Reducer<A, number> => (acc, v) => (pred(v) ? next(acc, v) : acc);
const oddSquareSum = transduce<number, number>(
  (r) => filterR<number>((x) => x % 2 === 1)(mapR<number>((x) => x * x)(r)),
  (acc, v) => acc + v,
  0,
  [1, 2, 3, 4, 5]
);
console.log("transduce.oddSquareSum=" + oddSquareSum); // 1 + 9 + 25

// ---------------------------------------------------------------- memoization

function memoize(fn: (n: number) => number) {
  const cache = new Map<number, number>();
  let misses = 0;
  const wrapped = (n: number): number => {
    if (cache.has(n)) return cache.get(n) as number;
    misses += 1;
    const v = fn(n);
    cache.set(n, v);
    return v;
  };
  return { wrapped, misses: () => misses, size: () => cache.size };
}
const slowSquare = memoize((n) => n * n);
console.log("memo.4=" + slowSquare.wrapped(4));
console.log("memo.4.again=" + slowSquare.wrapped(4));
console.log("memo.5=" + slowSquare.wrapped(5));
console.log("memo.misses=" + slowSquare.misses());
console.log("memo.size=" + slowSquare.size());

function memoFib() {
  const cache = new Map<number, number>([[0, 0], [1, 1]]);
  function fib(n: number): number {
    if (cache.has(n)) return cache.get(n) as number;
    const v = fib(n - 1) + fib(n - 2);
    cache.set(n, v);
    return v;
  }
  return { fib, cache };
}
const mf = memoFib();
console.log("memoFib.10=" + mf.fib(10));
console.log("memoFib.keys=" + [...mf.cache.keys()].join(","));

// ---------------------------------------------------------------- trampolines and CPS

// Keep calling returned thunks until a non-function value appears.
function trampoline<T>(fn: (...args: any[]) => any): (...args: any[]) => T {
  return function (...args: any[]): T {
    let result: any = fn(...args);
    while (typeof result === "function") result = result();
    return result;
  };
}

const factorial = trampoline(function fact(n: number, acc = 1): any {
  if (n <= 1) return acc;
  return () => fact(n - 1, n * acc);
});
console.log("fact.10=" + factorial(10));
console.log("fact.0=" + factorial(0));
console.log("fact.20=" + factorial(20));

const fib = trampoline(function f(n: number, a = 0, b = 1): any {
  if (n === 0) return a;
  return () => f(n - 1, b, a + b);
});
console.log("tfib.0=" + fib(0));
console.log("tfib.1=" + fib(1));
console.log("tfib.10=" + fib(10));
console.log("tfib.20=" + fib(20));
console.log("tfib.50=" + fib(50));

const sum = trampoline(function s(n: number, acc = 0): any {
  if (n <= 0) return acc;
  return () => s(n - 1, acc + n);
});
console.log("tsum.0=" + sum(0));
console.log("tsum.1000=" + sum(1000));
console.log("tsum.10000=" + sum(10000));

// Mutual recursion in continuation-passing style, driven by a loop.
function isEvenCPS(n: number, k: (v: boolean) => any): any {
  if (n === 0) return k(true);
  return () => isOddCPS(n - 1, k);
}
function isOddCPS(n: number, k: (v: boolean) => any): any {
  if (n === 0) return k(false);
  return () => isEvenCPS(n - 1, k);
}
function runCPS<T>(fn: (k: (v: T) => T) => any): T {
  let result: any;
  let thunk: any = fn((v: T) => {
    result = v;
    return v;
  });
  while (typeof thunk === "function") thunk = thunk();
  return result;
}
console.log("cps.even10=" + runCPS<boolean>((k) => isEvenCPS(10, k)));
console.log("cps.even11=" + runCPS<boolean>((k) => isEvenCPS(11, k)));
console.log("cps.odd7=" + runCPS<boolean>((k) => isOddCPS(7, k)));
console.log("cps.even0=" + runCPS<boolean>((k) => isEvenCPS(0, k)));

// ---------------------------------------------------------------- recursive closures lifted over captures
// Recursive functions that capture an outer variable and recurse from inside a
// switch, a nested switch, a labelled loop, a do-while, and a returned closure.

function comSwitch(cap: number): number {
  function z(t: number): number {
    switch (t) {
      case 0: return cap;
      case 1: return z(0);
      case 2: return z(1) + z(0);
    }
    return t + cap;
  }
  return z(2);
}
console.log("lift.switch=" + comSwitch(10)); // 10 + 10

function switchAninhado(cap: number): number {
  function z(t: number, k: number): number {
    switch (t) {
      case 0:
        switch (k) {
          case 9: return cap;
          case 8: return z(0, 9);
        }
        return k;
    }
    return t;
  }
  return z(0, 8);
}
console.log("lift.nested=" + switchAninhado(7));

function comLabeled(cap: number): number {
  function z(n: number): number {
    let acc = 0;
    L: for (let i = 0; i < n; i = i + 1) {
      if (i === 0 && n > 1) { acc = acc + z(1); continue L; }
      acc = acc + cap;
    }
    return acc;
  }
  return z(3);
}
console.log("lift.labeled=" + comLabeled(10)); // 10 + 10 + 10

function comDoWhile(cap: number): number {
  function z(n: number): number {
    let s = 0;
    let i = 0;
    do {
      if (i === 0 && n > 1) { s = s + z(1); }
      else { s = s + cap; }
      i = i + 1;
    } while (i < n);
    return s;
  }
  return z(2);
}
console.log("lift.dowhile=" + comDoWhile(10));

function build(base: number): (n: number) => number {
  function fib2(n: number): number {
    switch (n) {
      case 0: return base;
      case 1: return base + 1;
    }
    return fib2(n - 1) + fib2(n - 2) - base;
  }
  return fib2;
}
console.log("lift.fib5=" + build(0)(5));
console.log("lift.fib6.base3=" + build(3)(6));

// ---------------------------------------------------------------- monads

class Option<T> {
  #value: T | null;
  private constructor(v: T | null) { this.#value = v; }
  static some<T>(v: T): Option<T> { return new Option<T>(v); }
  static none<T>(): Option<T> { return new Option<T>(null); }
  static of<T>(v: T | null | undefined): Option<T> {
    return v == null ? Option.none<T>() : Option.some(v);
  }
  isSome() { return this.#value !== null; }
  isNone() { return this.#value === null; }
  map<U>(fn: (v: T) => U): Option<U> {
    return this.isSome() ? Option.some(fn(this.#value as T)) : Option.none<U>();
  }
  flatMap<U>(fn: (v: T) => Option<U>): Option<U> {
    return this.isSome() ? fn(this.#value as T) : Option.none<U>();
  }
  getOrElse(def: T): T { return this.isSome() ? (this.#value as T) : def; }
  toString() { return this.isSome() ? "Some(" + this.#value + ")" : "None"; }
}

const users: Record<number, { name: string; age: number }> = {
  1: { name: "Alice", age: 30 },
  2: { name: "Bob", age: 17 },
};
const getUser = (id: number) => Option.of(users[id]);
const getName = (u: { name: string }) => u.name;
const getAdult = (u: { age: number; name: string }) =>
  u.age >= 18 ? Option.some(u) : Option.none<typeof u>();

console.log("option.alice=" + getUser(1).map(getName).toString());
console.log("option.missing=" + getUser(99).map(getName).toString());
console.log("option.adult.alice=" + getUser(1).flatMap(getAdult).map(getName).toString());
console.log("option.adult.bob=" + getUser(2).flatMap(getAdult).map(getName).toString());
console.log("option.getOrElse=" + getUser(99).getOrElse({ name: "Guest", age: 0 }).name);
console.log("option.of.zero=" + Option.of(0).toString() + " of.null=" + Option.of(null).toString());
console.log("option.of.false=" + Option.of(false).isSome());

class Result<T, E> {
  private _ok: T | null;
  private _err: E | null;
  private constructor(ok: T | null, err: E | null) {
    this._ok = ok;
    this._err = err;
  }
  static ok<T, E>(v: T): Result<T, E> { return new Result<T, E>(v, null); }
  static err<T, E>(e: E): Result<T, E> { return new Result<T, E>(null, e); }
  isOk() { return this._err === null; }
  isErr() { return this._err !== null; }
  map<U>(fn: (v: T) => U): Result<U, E> {
    return this.isOk() ? Result.ok<U, E>(fn(this._ok as T)) : Result.err<U, E>(this._err as E);
  }
  flatMap<U>(fn: (v: T) => Result<U, E>): Result<U, E> {
    return this.isOk() ? fn(this._ok as T) : Result.err<U, E>(this._err as E);
  }
  mapErr<F>(fn: (e: E) => F): Result<T, F> {
    return this.isErr() ? Result.err<T, F>(fn(this._err as E)) : Result.ok<T, F>(this._ok as T);
  }
  unwrap(): T {
    if (this.isOk()) return this._ok as T;
    throw new Error("unwrap on Err: " + this._err);
  }
  unwrapOr(def: T): T { return this.isOk() ? (this._ok as T) : def; }
  toString() { return this.isOk() ? "Ok(" + this._ok + ")" : "Err(" + this._err + ")"; }
}

function parseNum(s: string): Result<number, string> {
  const n = Number(s);
  return s.trim() === "" || isNaN(n) ? Result.err<number, string>("not a number: " + s) : Result.ok<number, string>(n);
}
function safeDivide(a: number, b: number): Result<number, string> {
  return b === 0 ? Result.err<number, string>("division by zero") : Result.ok<number, string>(a / b);
}

const r1 = parseNum("42").flatMap((n) => safeDivide(100, n));
const r2 = parseNum("0").flatMap((n) => safeDivide(100, n));
const r3 = parseNum("abc").flatMap((n) => safeDivide(100, n));
const r4 = parseNum("   ").flatMap((n) => safeDivide(100, n));
console.log("result.r1=" + r1.toString());
console.log("result.r1.rounded=" + Math.round((r1.unwrap()) * 100) / 100);
console.log("result.r2=" + r2.toString());
console.log("result.r3=" + r3.toString());
console.log("result.r4=" + r4.toString());
console.log("result.r2.unwrapOr=" + r2.unwrapOr(-1));
console.log("result.mapErr=" + r3.mapErr((e) => e.length).toString());
console.log("result.map.chain=" + parseNum("4").map((n) => n * 10).map((n) => n + 2).toString());
try {
  r2.unwrap();
  console.log("unwrap.err=none");
} catch (e: any) {
  console.log("unwrap.err.name=" + e.name);
  console.log("unwrap.err.message=" + e.message);
}

// ---------------------------------------------------------------- parsers

// Recursive-descent arithmetic over integers with parentheses.
function parseExpr(src: string): number {
  let i = 0;
  function skip() { while (src[i] === " ") i++; }
  function num(): number {
    skip();
    let s = "";
    while (/[0-9]/.test(src[i] || "")) s += src[i++];
    return Number(s);
  }
  function factor(): number {
    skip();
    if (src[i] === "(") {
      i++;
      const v = expr();
      skip();
      i++;
      return v;
    }
    return num();
  }
  function term(): number {
    let v = factor();
    for (;;) {
      skip();
      if (src[i] === "*") { i++; v *= factor(); }
      else break;
    }
    return v;
  }
  function expr(): number {
    let v = term();
    for (;;) {
      skip();
      if (src[i] === "+") { i++; v += term(); }
      else if (src[i] === "-") { i++; v -= term(); }
      else break;
    }
    return v;
  }
  return expr();
}
console.log("parse.2+3*4=" + parseExpr("2+3*4"));
console.log("parse.(2+3)*4-5=" + parseExpr("(2+3)*4-5"));
console.log("parse.10+6*(7-4)=" + parseExpr("10 + 6 * (7 - 4)"));
console.log("parse.nested=" + parseExpr("(((1)))"));
console.log("parse.single=" + parseExpr("7"));

// Backtracking over a fixed alphabet: each alternative restores the cursor.
function parseBacktrack(input: string): string {
  let i = 0;
  function eat(s: string) {
    if (input.slice(i, i + s.length) === s) { i += s.length; return true; }
    return false;
  }
  function alt(): string {
    const save = i;
    if (eat("ab") && eat("c")) return "abc";
    i = save;
    if (eat("ab")) return "ab";
    i = save;
    return eat("a") ? "a" : "none";
  }
  const first = alt();
  const pos = i;
  const second = alt();
  return first + "@" + pos + "|" + second + "@" + i;
}
console.log("backtrack.abcab=" + parseBacktrack("abcab"));
console.log("backtrack.ababca=" + parseBacktrack("ababca"));
console.log("backtrack.empty=" + parseBacktrack(""));

// Token pipeline: regex matching, reduction over a Map of operators.
const operators = new Map<string, (a: number, b: number) => number>([
  ["+", (a, b) => a + b],
  ["*", (a, b) => a * b],
]);
function evaluate(input: string): string {
  const tokens = [...input.matchAll(/\d+|[+*]/g)].map((m) => m[0]);
  let value = Number(tokens[0]);
  const trace: string[] = [String(value)];
  for (let i = 1; i < tokens.length; i += 2) {
    const op = operators.get(tokens[i]) as (a: number, b: number) => number;
    value = op(value, Number(tokens[i + 1]));
    trace.push(tokens[i] + tokens[i + 1] + "=" + value);
  }
  return value + "|" + trace.join(",");
}
console.log("pipeline.a=" + evaluate("2 + 3 * 4 + 5"));
console.log("pipeline.b=" + evaluate("10 * 2 + 7"));
console.log("pipeline.single=" + evaluate("9"));

// ---------------------------------------------------------------- state machine

// A turnstile driven by a transition table; unknown events keep the state.
type TState = "locked" | "unlocked";
const table: Record<TState, Record<string, TState>> = {
  locked: { coin: "unlocked", push: "locked" },
  unlocked: { coin: "unlocked", push: "locked" },
};
function drive(events: string[]): string {
  let state: TState = "locked";
  const visited: string[] = [state];
  for (const ev of events) {
    state = table[state][ev] ?? state;
    visited.push(state);
  }
  return visited.join(">");
}
console.log("fsm.basic=" + drive(["coin", "push", "push", "coin"]));
console.log("fsm.unknown=" + drive(["kick", "coin", "kick"]));
console.log("fsm.empty=" + drive([]));
