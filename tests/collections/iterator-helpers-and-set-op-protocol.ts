// Cross-runtime: ES2025 iterator helpers reached from Map/Set iterators, and
// the protocol a Set operation follows on its set-like argument (which side is
// walked, which methods are called, and whether an early exit closes it).

const s = new Set([1, 2, 3, 4, 5]);
const m = new Map<string, number>([["a", 1], ["b", 2], ["c", 3]]);

// --- the helpers live on the shared %IteratorPrototype% ---
const helperNames = ["map", "filter", "take", "drop", "flatMap", "reduce", "toArray", "forEach", "some", "every", "find"];
const iterProto: any = Object.getPrototypeOf(Object.getPrototypeOf(s.values()));
let present = "";
for (const n of helperNames) present += n + ":" + typeof iterProto[n] + " ";
console.log("present=" + present.trim());
console.log("shared_with_array=" + (iterProto === Object.getPrototypeOf(Object.getPrototypeOf([].values()))));
console.log("iterator_global=" + typeof (globalThis as any).Iterator + ":" + ((globalThis as any).Iterator.prototype === iterProto));
let arities = "";
for (const n of helperNames) arities += n + ":" + iterProto[n].length + " ";
console.log("arities=" + arities.trim());

// --- eager results through toArray ---
console.log("map=" + (s.values() as any).map((v: number) => v * 2).toArray().join(","));
console.log("filter=" + (s.values() as any).filter((v: number) => v % 2 === 1).toArray().join(","));
console.log("take=" + (s.values() as any).take(2).toArray().join(","));
console.log("drop=" + (s.values() as any).drop(3).toArray().join(","));
console.log("flatMap=" + (s.values() as any).flatMap((v: number) => [v, -v]).take(6).toArray().join(","));
console.log("reduce=" + (s.values() as any).reduce((a: number, b: number) => a + b, 0) + ":" + (s.values() as any).reduce((a: number, b: number) => a + b));
console.log("some=" + (s.values() as any).some((v: number) => v === 3) + ":every=" + (s.values() as any).every((v: number) => v < 10));
console.log("find=" + (s.values() as any).find((v: number) => v > 3) + ":miss=" + String((s.values() as any).find((v: number) => v > 99)));
const pairs: string[] = [];
(s.values() as any).map((v: number, i: number) => { pairs.push(i + ":" + v); return v; }).toArray();
console.log("map_index=" + pairs.join(","));
const mapPairs: string[] = [];
(m.entries() as any).forEach((e: any, i: number) => mapPairs.push(i + ":" + e[0] + "=" + e[1]));
console.log("entries_forEach=" + mapPairs.join(","));
console.log("map_keys_chain=" + (m.keys() as any).map((k: string) => k.toUpperCase()).toArray().join(","));
console.log("map_values_chain=" + (m.values() as any).filter((v: number) => v > 1).toArray().join(","));

// --- chains are themselves iterators ---
const chain = (s.values() as any).map((v: number) => v * 10).filter((v: number) => v > 20).drop(1).take(2);
console.log("chain_tag=" + Object.prototype.toString.call(chain) + ":self=" + (chain[Symbol.iterator]() === chain));
console.log("chain_result=" + [...chain].join(",") + ":exhausted=" + JSON.stringify(chain.next()));

// --- laziness and live cursors ---
let pulled = 0;
const counting = new Set([1, 2, 3, 4, 5, 6]);
const lazy = (counting.values() as any).map((v: number) => { pulled++; return v; });
console.log("pulled_before=" + pulled);
console.log("lazy_first=" + lazy.next().value + ":pulled=" + pulled);
console.log("lazy_two_more=" + lazy.take(2).toArray().join(",") + ":pulled_total=" + pulled);
const live = new Set([1, 2]);
const pending = (live.values() as any).map((v: number) => v * 100);
live.add(3);
console.log("live_seen=" + pending.toArray().join(","));
let closed = "no";
function* closable(): Generator<number> {
  try { yield 1; yield 2; } finally { closed = "yes"; }
}
console.log("take0=" + (closable() as any).take(0).toArray().join(",") + ":closed=" + closed);
const once = (new Set([1]).values() as any).map((v: number) => v);
console.log("once=" + JSON.stringify(once.next()) + ":" + JSON.stringify(once.next()) + ":" + JSON.stringify(once.next()));

// --- argument validation of the helpers ---
function probe(label: string, fn: () => any): void {
  try { console.log(label + "=ok:" + String(fn())); }
  catch (e: any) { console.log(label + "=" + e.name); }
}
probe("map_on_plain", () => iterProto.map.call({}, (v: any) => v));
probe("map_no_callback", () => (s.values() as any).map());
probe("map_non_callable", () => (s.values() as any).map(42));
probe("take_negative", () => (s.values() as any).take(-1).toArray());
probe("take_nan", () => (s.values() as any).take(NaN).toArray());
probe("drop_negative", () => (s.values() as any).drop(-1).toArray());
probe("take_infinity", () => (s.values() as any).take(Infinity).toArray().join(","));
probe("take_string", () => (s.values() as any).take("2").toArray().join(","));
probe("flatMap_non_iterable", () => (s.values() as any).flatMap((v: number) => v).toArray());
probe("reduce_empty_no_seed", () => (new Set() as any).values().reduce((a: any, b: any) => a + b));
let bi = 0;
const bare: any = { next() { return bi < 3 ? { value: bi++, done: false } : { value: undefined, done: true }; } };
const wrapped = (globalThis as any).Iterator.from(bare);
console.log("iterator_from=" + wrapped.map((v: number) => v + 1).toArray().join(","));
console.log("iterator_from_set=" + (globalThis as any).Iterator.from(new Set([7, 8])).toArray().join(","));

// --- Set-operation protocol: a tracked set-like argument ---
function tracker(values: number[], sizeLie?: number): any {
  const calls = { size: 0, has: 0, keys: 0, next: 0, ret: 0 };
  const obj: any = {
    get size() { calls.size++; return sizeLie === undefined ? values.length : sizeLie; },
    has(v: any) { calls.has++; return values.indexOf(v) >= 0; },
    keys() {
      calls.keys++;
      let i = 0;
      return {
        next() {
          calls.next++;
          return i < values.length ? { value: values[i++], done: false } : { value: undefined, done: true };
        },
        return(v: any) { calls.ret++; return { done: true, value: v }; },
      };
    },
  };
  return { obj, calls };
}
function tally(c: any): string {
  return "size=" + c.size + ",has=" + c.has + ",keys=" + c.keys + ",next=" + c.next + ",return=" + c.ret;
}
function show(r: any): string {
  return r instanceof Set ? "{" + [...r].join(",") + "}" : String(r);
}
const receiver = new Set([1, 2, 3]);
const argValues = [2, 3, 4];
for (const op of ["union", "intersection", "difference", "symmetricDifference", "isSubsetOf", "isSupersetOf", "isDisjointFrom"]) {
  const t = tracker(argValues);
  const r = (receiver as any)[op](t.obj);
  console.log(op + "=" + show(r) + " " + tally(t.calls));
}
for (const op of ["intersection", "difference", "isDisjointFrom"]) {
  const small = tracker([2], 1);
  console.log("small_" + op + "=" + show((receiver as any)[op](small.obj)) + " " + tally(small.calls));
  const large = tracker([2, 3, 4, 5, 6, 7], 6);
  console.log("large_" + op + "=" + show((receiver as any)[op](large.obj)) + " " + tally(large.calls));
}
const emptyRecv = new Set<number>();
for (const op of ["union", "intersection", "difference", "isSubsetOf", "isDisjointFrom"]) {
  const t = tracker(argValues);
  (emptyRecv as any)[op](t.obj);
  console.log("empty_" + op + "=" + tally(t.calls));
}
const early = tracker([9, 1, 2], 3);
console.log("superset_false=" + (receiver as any).isSupersetOf(early.obj) + ":" + tally(early.calls));
const earlySym = tracker([1, 2, 3, 4], 4);
console.log("symdiff=" + (receiver as any).symmetricDifference(earlySym.obj).size + ":" + tally(earlySym.calls));
const patched: any = new Set([1, 2]);
let patchedHits = 0;
patched.has = function () { patchedHits++; return true; };
patched.add = function () { patchedHits++; return this; };
patched.keys = function () { patchedHits++; return [][Symbol.iterator](); };
console.log("patched_union=" + show(patched.union(new Set([3]))) + ":subset=" + patched.isSubsetOf(new Set([1, 2, 3])) + ":inter=" + show(patched.intersection(new Set([2]))) + ":hits=" + patchedHits);
class Loud extends Set<number> {
  static hits = 0;
  add(v: number): this { Loud.hits++; return super.add(v); }
  has(v: number): boolean { Loud.hits++; return super.has(v); }
}
const loud = new Loud([1, 2]);
Loud.hits = 0;
const loudUnion = loud.union(new Set([3]));
console.log("subclass_union=" + show(loudUnion) + ":hits=" + Loud.hits + ":plain=" + (Object.getPrototypeOf(loudUnion) === Set.prototype));
console.log("matches_setlike=" + ([...receiver.intersection(new Set([2, 3, 4]))].join(",") === [...(receiver as any).intersection(tracker(argValues).obj)].join(",")));
