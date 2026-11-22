const N: number = 3000000;
let g_state: number = 1;

class Box { s: number = 1; }
class Outer { inner: Box = new Box(); }
class Shape { area(): number { return 1; } }
class Sq extends Shape { area(): number { return 2; } }
class Counter { static total: number = 0; }

function b_local(): number {
  let s: number = 1; let i: number = 0;
  while (i < N) { s = (s * 1664525 + 1013904223) % 4294967296; i = i + 1; }
  return s;
}
function b_static(): number {
  g_state = 1; let i: number = 0;
  while (i < N) { g_state = (g_state * 1664525 + 1013904223) % 4294967296; i = i + 1; }
  return g_state;
}
function b_class_static(): number {
  Counter.total = 1; let i: number = 0;
  while (i < N) { Counter.total = (Counter.total * 1664525 + 1013904223) % 4294967296; i = i + 1; }
  return Counter.total;
}
function b_field(): number {
  const o = new Box(); let i: number = 0;
  while (i < N) { o.s = (o.s * 1664525 + 1013904223) % 4294967296; i = i + 1; }
  return o.s;
}
function b_two_hops(): number {
  const o = new Outer(); let i: number = 0;
  while (i < N) { o.inner.s = (o.inner.s * 1664525 + 1013904223) % 4294967296; i = i + 1; }
  return o.inner.s;
}
function b_literal(): number {
  const o = { s: 1 }; let i: number = 0;
  while (i < N) { o.s = (o.s * 1664525 + 1013904223) % 4294967296; i = i + 1; }
  return o.s;
}
function step(s: number): number { return (s * 1664525 + 1013904223) % 4294967296; }
function b_call(): number {
  let s: number = 1; let i: number = 0;
  while (i < N) { s = step(s); i = i + 1; }
  return s;
}
function b_virtual(): number {
  const a: Shape = new Sq(); let s: number = 0; let i: number = 0;
  while (i < N) { s = s + a.area(); i = i + 1; }
  return s;
}
function b_array(): number {
  const a: number[] = [];
  let i: number = 0;
  while (i < 1000) { a.push(i); i = i + 1; }
  let s: number = 0; let k: number = 0;
  while (k < 3000) { let j: number = 0; while (j < 1000) { s = s + a[j]; j = j + 1; } k = k + 1; }
  return s;
}
function b_alloc(): number {
  let s: number = 0; let i: number = 0;
  while (i < N) { const b = new Box(); s = s + b.s; i = i + 1; }
  return s;
}
function b_string(): number {
  let s: string = ""; let i: number = 0;
  while (i < 200000) { s = s + "x"; i = i + 1; }
  return s.length;
}
function fib(n: number): number { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
function b_fib(): number { return fib(30); }

function timed(label: string, run: () => number): void {
  const t = performance.now(); const r = run(); const e = performance.now() - t;
  console.log(label + " " + e.toFixed(1) + " ms (" + r + ")");
}
timed("local       ", b_local);
timed("mod_static  ", b_static);
timed("class_static", b_class_static);
timed("field       ", b_field);
timed("two_hops    ", b_two_hops);
timed("obj_literal ", b_literal);
timed("call        ", b_call);
timed("virtual     ", b_virtual);
timed("array       ", b_array);
timed("alloc       ", b_alloc);
timed("string      ", b_string);
timed("fib30       ", b_fib);
