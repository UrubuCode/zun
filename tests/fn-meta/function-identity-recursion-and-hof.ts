// Function bindings and higher-order patterns. Covers a function declaration as
// a writable binding (the self-replacing memoisation that transpilers emit),
// hoisting, named function expressions whose self-name is immutable and does not
// leak, nested functions that share a name across parents and call each other,
// mutual recursion, and the functional toolkit (memoize, once, partial, curry,
// compose, pipe, reduce-based composition) with deep recursion.
export {};

const show = (label: string, value: unknown): void => {
  console.log(label + "=" + JSON.stringify(value));
};

function tryOf(fn: () => unknown): string {
  try {
    return "ok:" + String(fn());
  } catch (e) {
    return "threw:" + (e as any).name;
  }
}

// ---------------------------------------------------------------------------
// 1. A declaration is a writable binding: self-replacing functions
// ---------------------------------------------------------------------------
function make(): (a: number) => string {
  return function (a: number): string { return "real:" + a; };
}

// The function swaps itself on its first call (lazy initialisation).
function selfSwap(a: number): string {
  selfSwap = make() as typeof selfSwap;
  return (selfSwap as (a: number) => string).apply(null, [a]);
}
show("self_swap_first", selfSwap(1));
show("self_swap_second", selfSwap(2));

// Another function sees the NEW value, not a capture of the original.
function viaOther(a: number): string {
  return (selfSwap as (a: number) => string).apply(null, [a]);
}
show("other_sees_new", viaOther(3));

// Re-assigning a declaration from the top level reaches existing callers.
function base(): string { return "orig"; }
function readsBase(): string { return base(); }
show("before_reassign", readsBase());
base = function (): string { return "replaced"; };
show("after_reassign", readsBase());

// The lazy-init shape from a transpiled async helper: a counter plus a swap.
let initRuns = 0;
function lazy(n: number): number {
  initRuns = initRuns + 1;
  lazy = function (m: number): number { return m + 100; };
  return lazy(n);
}
show("lazy_first", lazy(1));
show("lazy_second", lazy(2));
show("lazy_init_ran", initRuns);

// Hoisting: the binding exists above its declaration.
show("hoisted_call", hoistedLater());
function hoistedLater(): string { return "ok"; }

// A function that is never reassigned behaves the same as always.
function pure(x: number): number { return x * 2; }
show("pure", [pure(21), pure.name, pure.length]);

// Declarations and expressions differ: a `const` holding a function is not
// writable, but the declaration binding is.
function declared(): string { return "original"; }
const keepDeclared = declared;
(declared as any) = function (): string { return "replaced-decl"; };
show("declaration_writable", [declared(), keepDeclared()]);

// ---------------------------------------------------------------------------
// 2. Named function expressions: an immutable, non-leaking self-name
// ---------------------------------------------------------------------------
const factorial = function fact(n: number): number {
  return n < 2 ? 1 : n * fact(n - 1);
};
show("recursion", factorial(6));
show("self_name_leaked", typeof (globalThis as any).fact);
show("self_name_property", factorial.name);

// The self-binding is immutable: a write throws in strict code and changes nothing.
const unchanged = function keeper(): string {
  try {
    (keeper as any) = null;
    return "assigned";
  } catch (e) {
    return "threw:" + (e as any).name;
  }
};
show("self_binding_immutable", [unchanged(), typeof unchanged]);

const stillRecurses = function walk(n: number): number {
  try { (walk as any) = null; } catch { /* refused */ }
  return n <= 0 ? 0 : 1 + walk(n - 1);
};
show("still_recurses", stillRecurses(4));

// The self-binding survives reassignment of the outer variable.
let outerVar: any = function inner(n: number): number {
  return n <= 0 ? 0 : n + inner(n - 1);
};
const captured = outerVar;
outerVar = (): number => -1;
show("survives_reassign", [captured(4), outerVar()]);

// A parameter, a var or a let with the same name shadows the self-binding.
const shadowedByParam = function me(me: any): string { return typeof me; };
show("param_shadows_self", [shadowedByParam(5), shadowedByParam(function (): void {})]);
const shadowedByVar = function me(): string {
  var me: any = "local";
  me = "reassigned";
  return me;
};
show("var_shadows_self", shadowedByVar());
const shadowedByLet = function me(): string {
  let me = "let-value";
  return me;
};
show("let_shadows_self", shadowedByLet());

const nested = function outerName(n: number): string {
  const helper = (): string => typeof outerName;
  return helper() + ":" + n;
};
show("nested_sees_self", nested(1));
const inDefault = function d(x: any = typeof d): string { return String(x); };
show("default_sees_self", inDefault());

const holder = {
  method: function memberName(n: number): number {
    return n === 0 ? 0 : 1 + memberName(n - 1);
  },
};
show("member_recursion", [holder.method(5), holder.method.name, typeof (globalThis as any).memberName]);

show("inline_named_ticker", (function ticker(n: number): string {
  return n === 0 ? "0" : n + ">" + ticker(n - 1);
})(4));

function makeTicker(): () => number {
  let calls = 0;
  return function tick(): number {
    calls += 1;
    return calls < 3 ? tick() : calls;
  };
}
const t1 = makeTicker();
const t2 = makeTicker();
show("ticker_distinct", [t1 !== t2, t1(), t2()]);

// A named class expression binds its name inside, as a constant.
const Klass = class Inner {
  static describe(): string { return "inside:" + Inner.name; }
};
show("class_expr_inside", Klass.describe());
show("class_expr_name", Klass.name);
show("class_expr_leaked", typeof (globalThis as any).Inner);
const Stubborn = class Self {
  static poke(): string {
    try {
      (Self as any) = 1;
      return "assigned";
    } catch (e) {
      return "threw:" + (e as any).name;
    }
  }
};
show("class_self_const", Stubborn.poke());

// ---------------------------------------------------------------------------
// 3. Nested functions: same names under different parents, and sibling calls
// ---------------------------------------------------------------------------
function parentA(): string {
  function helper(): number { return 1; }
  return "a=" + helper();
}
function parentB(): string {
  function helper(): number { return 2; }
  return "b=" + helper();
}
show("same_name_helpers", [parentA(), parentB()]);

function outerSiblings(): string {
  let available = 8;
  function availableFrames(): number { return available; }
  function consume(n: number): void { available = available - n; }
  function tick(): string {
    let drained = 0;
    let guard = 0;
    while (availableFrames() > 0 && guard < 50) {
      const got = availableFrames();
      consume(got);
      drained = drained + got;
      guard = guard + 1;
    }
    return "drained=" + drained + " remaining=" + availableFrames() + " iters=" + guard;
  }
  return tick();
}
show("sibling_calls", outerSiblings());

function engine(): string {
  const capacity = 20;
  let queued = 0;
  function freeSpace(): number { return capacity - queued; }
  function fill(n: number): void { queued = queued + n; }
  function pump(): string {
    let filled = 0;
    let rounds = 0;
    const block = 5;
    while (freeSpace() > 0 && rounds < 100) {
      let n = freeSpace();
      if (n > block) n = block;
      fill(n);
      filled = filled + n;
      rounds = rounds + 1;
    }
    return "filled=" + filled + " final_queued=" + queued + " rounds=" + rounds;
  }
  return pump();
}
show("producer_consumer", engine());

// ---------------------------------------------------------------------------
// 4. Hoisting between mutually recursive functions
// ---------------------------------------------------------------------------
function isEven(n: number): boolean {
  if (n === 0) return true;
  return isOdd(n - 1);
}
function isOdd(n: number): boolean {
  if (n === 0) return false;
  return isEven(n - 1);
}
show("mutual_declarations", [isEven(10), isOdd(10), isEven(7), isOdd(7), isEven(0), isOdd(1)]);

const evenA = (n: number): boolean => n === 0 || oddA(n - 1);
const oddA = (n: number): boolean => n !== 0 && evenA(n - 1);
show("mutual_arrows_even", [0, 1, 7, 12].map((n) => evenA(n)));
show("mutual_arrows_odd", [0, 1, 7, 12].map((n) => oddA(n)));

// ---------------------------------------------------------------------------
// 5. Recursion depth and a deep, branching call tree
// ---------------------------------------------------------------------------
function ackermann(m: number, n: number): number {
  if (m === 0) return n + 1;
  if (n === 0) return ackermann(m - 1, 1);
  return ackermann(m - 1, ackermann(m, n - 1));
}
show("ackermann", [ackermann(0, 0), ackermann(1, 1), ackermann(2, 3), ackermann(3, 3)]);

// ---------------------------------------------------------------------------
// 6. Higher-order toolkit
// ---------------------------------------------------------------------------
function memoize<T>(fn: (...args: number[]) => T): (...args: number[]) => T {
  const cache = new Map<string, T>();
  return (...args: number[]): T => {
    const key = args.join(",");
    if (cache.has(key)) return cache.get(key)!;
    const result = fn(...args);
    cache.set(key, result);
    return result;
  };
}
let fibCalls = 0;
const fib = memoize(function f(n: number): number {
  fibCalls++;
  if (n <= 1) return n;
  return f(n - 1) + f(n - 2);
});
show("memo_fib", [fib(10), fib(10), fibCalls]);

function once<T>(fn: () => T): () => T {
  let called = false;
  let result: T;
  return () => {
    if (!called) { called = true; result = fn(); }
    return result;
  };
}
let initCount = 0;
const init = once(() => { initCount++; return "initialized"; });
show("once", [init(), init(), init(), initCount]);

function partial<T>(fn: (...args: any[]) => T, ...preArgs: any[]): (...args: any[]) => T {
  return (...args: any[]) => fn(...preArgs, ...args);
}
const add3 = (a: number, b: number, c: number): number => a + b + c;
const add10 = partial(add3, 10);
show("partial", [add10(1, 2), add10(5, 5)]);

function curry(fn: (...args: number[]) => number): (...args: number[]) => any {
  const arity = fn.length;
  return function curried(...args: number[]): any {
    if (args.length >= arity) return fn(...args);
    return (...more: number[]) => curried(...args, ...more);
  };
}
const multiply = curry((a: number, b: number, c: number) => a * b * c);
show("curry_forms", [multiply(2)(3)(4), multiply(2, 3)(4), multiply(2, 3, 4)]);

function adder(a: number) {
  return function (b: number) {
    return function (c: number) {
      return function (d: number): number { return a + b + c + d; };
    };
  };
}
const step1 = adder(10);
const step2 = step1(20);
show("curry_chain", [adder(1)(2)(3)(4), step2(30)(40), step2(5)(6)]);

function compose(f: (x: number) => number, g: (x: number) => number) {
  return function (x: number): number { return f(g(x)); };
}
const inc = (n: number): number => n + 1;
const dbl = (n: number): number => n * 2;
show("compose_order", [compose(dbl, inc)(5), compose(inc, dbl)(5), compose(compose(inc, inc), inc)(0)]);

// A pipe built by re-binding a variable: each step captures the previous value.
let pipe = (x: number): number => x;
const ops = [inc, dbl, inc];
for (let i = 0; i < ops.length; i++) {
  const prev = pipe;
  const op = ops[i];
  pipe = (x: number): number => op(prev(x));
}
show("pipe_rebound", pipe(1));

const reduced = [inc, dbl, inc].reduce((acc, f) => (x: number) => f(acc(x)), (x: number) => x);
show("reduce_pipe", reduced(1));

function makeAccumulators(): Array<(x: number) => number> {
  const list: Array<(x: number) => number> = [];
  for (let i = 0; i < 3; i++) {
    let total = 0;
    list.push(function (x: number): number {
      total += x * (i + 1);
      return total;
    });
  }
  return list;
}
const accs = makeAccumulators();
show("accumulators", [accs[0](1), accs[0](1), accs[1](1), accs[2](1), accs[2](1)]);

// The function's own `length` drives currying, so a default would change it.
show("curry_length_drives", [multiply.length, curry.length]);

// A non-function argument is refused the same way in every runtime.
show("apply_non_function", tryOf(() => (1 as any)()));
