// Cross-runtime: Set core surface and the ES2025 set-algebra family.
// Covers dedup, SameValueZero, insertion order, forEach and for-of under
// mutation, union/intersection/difference/symmetricDifference result order,
// the predicates, set-like argument validation (GetSetRecord), and brand checks.

// --- basics: dedup, size, has, chained add, delete, clear ---
const s = new Set([1, 2, 3, 2, 1]);
console.log("dedup_size=" + s.size + ":has2=" + s.has(2) + ":has9=" + s.has(9));
console.log("add_returns_self=" + (s.add(4) === s) + ":chain=" + (s.add(5).add(6) === s));
console.log("dup_add_size=" + s.add(2).size);
console.log("delete_hit=" + s.delete(1) + ":delete_miss=" + s.delete(1) + ":size=" + s.size + ":has1=" + s.has(1));
console.log("values=" + [...s].join(","));
console.log("set_clear=" + String(s.clear()) + ":size=" + s.size + ":spread=" + [...s].length);
s.add("x").add("y");
console.log("reuse=" + [...s].join(","));
console.log("chain_inline=" + [...new Set<string>().add("a").add("b").add("a")].join(","));
console.log("chain_size_inline=" + new Set<number>().add(9).add(8).size);
const mixedChain = new Set<any>();
mixedChain.add(0).add("0").add(false).add(null).add(undefined).add(NaN);
console.log("mixed_chain=" + mixedChain.size + ":" + mixedChain.has(0) + mixedChain.has("0") + mixedChain.has(false) + mixedChain.has(null) + mixedChain.has(undefined) + mixedChain.has(NaN));

// --- SameValueZero on set members ---
const sv = new Set<any>([NaN, NaN, -0, 0, "0", [1], [1]]);
console.log("svz_size=" + sv.size + ":nan=" + sv.has(NaN) + ":zero=" + sv.has(-0) + ":string_zero=" + sv.has("0"));
const svFirst = [...new Set<any>([-0, 0 * -1, -(0)])][0];
console.log("svz_stored_zero=" + (1 / svFirst));
console.log("svz_delete_nan=" + sv.delete(NaN) + ":" + sv.size);
console.log("svz_typeof=" + [...new Set<any>([NaN, "s", 1])].map((x) => typeof x).join(","));

// --- insertion order ---
const ord = new Set(["a", "b", "c", "a", "b"]);
console.log("first_order=" + [...ord].join(","));
ord.add("a");
console.log("readd_noop=" + [...ord].join(",") + ":" + ord.size);
ord.delete("b");
ord.add("b");
console.log("delete_readd=" + [...ord].join(","));
ord.delete("a");
ord.add("a");
console.log("cycle=" + [...ord].join(","));
const cs = new Set([1, 2, 3, 4]);
cs.delete(2);
cs.add(2);
console.log("moved_to_end=" + [...cs].join(",") + ":" + cs.has(2) + ":" + cs.delete(9) + ":" + cs.size);
const objA = { n: "a" }, objB = { n: "b" };
const objSet = new Set<any>([objA, objB]);
objSet.delete(objA);
objSet.add(objA);
console.log("obj_order=" + [...objSet].map((k) => k.n).join(","));

// --- forEach: (value, value, set) triple, thisArg, mutation ---
const fe = new Set(["p", "q"]);
const rows: string[] = [];
fe.forEach(function (v1, v2, coll) {
  rows.push(v1 + ":" + (v1 === v2) + ":same=" + (coll === fe) + ":argc=" + arguments.length);
});
console.log("forEach_triple=" + rows.join("|"));
const feCtx = { n: 0 };
new Set([1, 2, 3]).forEach(function (this: any, v) { this.n += v; }, feCtx);
console.log("forEach_thisArg=" + feCtx.n + ":returns=" + String(new Set([1]).forEach(() => 1)));
const fmut = new Set([1, 2, 3]);
const seenFm: string[] = [];
fmut.forEach((value, same, set) => {
  seenFm.push(value + ":" + same + ":" + (set === fmut));
  if (value === 1) {
    fmut.delete(2);
    fmut.add(4);
  }
  if (value === 3) fmut.add(5);
});
console.log("forEach_mut_seen=" + seenFm.join("|"));
console.log("forEach_mut_final=" + [...fmut].join(","));
const appended = new Set([1, 2]);
const seenApp: number[] = [];
appended.forEach((v) => {
  seenApp.push(v);
  if (v === 1) appended.add(3);
});
console.log("forEach_append=" + seenApp.join(",") + ":" + [...appended].join(","));
const forOfSet = new Set<number>([1, 2, 3]);
const seenFo: number[] = [];
for (const v of forOfSet) {
  seenFo.push(v);
  if (v === 1) {
    forOfSet.delete(2);
    forOfSet.add(4);
  }
  if (v === 3) forOfSet.add(5);
}
console.log("for_of_seen=" + seenFo.join(",") + ":final=" + [...forOfSet].join(","));

// --- iterator surface and live cursor ---
const itSet = new Set(["a", "b", "c"]);
const it = itSet.values();
console.log("it_first=" + JSON.stringify(it.next()));
itSet.delete("b");
console.log("it_skip=" + JSON.stringify(it.next()));
console.log("it_done=" + JSON.stringify(it.next()));
console.log("entries_pair=" + JSON.stringify(new Set(["v"]).entries().next().value));
console.log("set_keys_is_values=" + (Set.prototype.keys === Set.prototype.values) + ":iter_is_values=" + ((Set.prototype as any)[Symbol.iterator] === Set.prototype.values));

// --- set operations: result order and result identity ---
const a = new Set([1, 2, 3, 4]);
const b = new Set([3, 4, 5, 6]);
console.log("union=" + [...a.union(b)].join(",") + ":reverse=" + [...b.union(a)].join(","));
console.log("inter=" + [...a.intersection(b)].join(",") + ":reverse=" + [...b.intersection(a)].join(","));
console.log("diff=" + [...a.difference(b)].join(",") + ":reverse=" + [...b.difference(a)].join(","));
console.log("symdiff=" + [...a.symmetricDifference(b)].join(",") + ":reverse=" + [...b.symmetricDifference(a)].join(","));
const u = a.union(b);
console.log("results_fresh=" + (u === a) + ":" + (u === b) + ":" + (u instanceof Set) + ":sources=" + a.size + ":" + b.size);
console.log("empty_inter=" + a.intersection(new Set([99])).size + ":empty_diff=" + a.difference(a).size + ":symdiff_self=" + a.symmetricDifference(a).size);
const e = new Set<number>();
console.log("empty_receiver=" + [...e.union(b)].join(",") + ":" + e.intersection(b).size + ":" + e.difference(b).size + ":" + [...e.symmetricDifference(b)].join(","));
const base = new Set([1, 2]);
const derived = base.union(new Set([3]));
base.add(99);
console.log("derived_stable=" + [...derived].join(",") + ":base=" + [...base].join(","));
console.log("chained=" + [...a.union(b).difference(new Set([1, 6]))].join(",") + ":" + [...a.intersection(b).union(new Set([0]))].join(","));
const nz = new Set([NaN, -0]);
console.log("svz_ops=" + [...nz.intersection(new Set([NaN])).values()].length + ":" + (1 / [...nz.intersection(new Set([0]))][0]) + ":" + nz.difference(new Set([NaN])).size);

// --- predicates ---
console.log("subset=" + new Set([2, 3]).isSubsetOf(a) + ":" + new Set([2, 9]).isSubsetOf(a) + ":" + a.isSubsetOf(a) + ":" + new Set().isSubsetOf(a));
console.log("superset=" + a.isSupersetOf(new Set([1, 2])) + ":" + a.isSupersetOf(new Set([1, 99])) + ":" + a.isSupersetOf(new Set()));
console.log("disjoint=" + a.isDisjointFrom(new Set([7, 8])) + ":" + a.isDisjointFrom(b) + ":" + a.isDisjointFrom(new Set()));

// --- set-like arguments: Map and custom objects ---
const mapArg = new Map([[3, "x"], [4, "y"], [9, "z"]]);
console.log("map_arg=" + [...a.union(mapArg as any)].join(",") + ":" + [...a.intersection(mapArg as any)].join(",") + ":" + [...a.difference(mapArg as any)].join(","));
console.log("map_arg_preds=" + new Set([3, 4]).isSubsetOf(mapArg as any) + ":" + new Set([1, 2]).isDisjointFrom(mapArg as any));
const setLike = {
  size: 2,
  has: (v: number) => v === 2 || v === 3,
  keys: function* () { yield 2; yield 3; },
};
console.log("setlike=" + [...a.union(setLike as any)].join(",") + ":" + [...a.intersection(setLike as any)].join(",") + ":" + new Set([2]).isSubsetOf(setLike as any));
try {
  (a as any).union([5, 6]);
  console.log("array_arg=no_throw");
} catch (err: any) {
  console.log("array_arg=" + err.name);
}

// --- validation order: size, then has, then keys ---
const base3 = new Set([1, 2, 3]);
function record(size: any, has: any, keys: any, seen: string[]): any {
  return {
    get size() { seen.push("size"); return size; },
    get has() { seen.push("has"); return has; },
    get keys() { seen.push("keys"); return keys; },
  };
}
const goodHas = (v: any) => v === 2 || v === 9;
const goodKeys = function () { let i = 0; const vals = [2, 9]; return { next() { return i < vals.length ? { value: vals[i++], done: false } : { value: undefined, done: true }; } }; };
const seenOk: string[] = [];
console.log("valid_union=" + [...(base3 as any).union(record(2, goodHas, goodKeys, seenOk))].join(",") + ":order=" + seenOk.join(","));
function probe(label: string, fn: () => any): void {
  try {
    const r = fn();
    console.log(label + "=ok:" + (r instanceof Set ? "{" + [...r].join(",") + "}" : String(r)));
  } catch (err: any) {
    console.log(label + "=" + err.name);
  }
}
const seenHas: string[] = [];
probe("has_not_callable", () => (base3 as any).union(record(2, 42, goodKeys, seenHas)));
console.log("has_stop=" + seenHas.join(","));
const seenKeys: string[] = [];
probe("keys_not_callable", () => (base3 as any).union(record(2, goodHas, "nope", seenKeys)));
console.log("keys_stop=" + seenKeys.join(","));
const seenSize: string[] = [];
probe("size_undefined_stop", () => (base3 as any).union(record(undefined, goodHas, goodKeys, seenSize)));
console.log("size_stop=" + seenSize.join(","));
function withSize(sz: any): any {
  return { size: sz, has: goodHas, keys: goodKeys };
}
probe("size_nan", () => (base3 as any).union(withSize(NaN)));
probe("size_negative", () => (base3 as any).union(withSize(-1)));
probe("size_neg_infinity", () => (base3 as any).union(withSize(-Infinity)));
probe("size_negative_zero", () => (base3 as any).union(withSize(-0)));
probe("size_infinity", () => (base3 as any).union(withSize(Infinity)));
probe("size_fractional", () => (base3 as any).union(withSize(1.9)));
probe("size_bigint", () => (base3 as any).union(withSize(10n)));
probe("size_symbol", () => (base3 as any).union(withSize(Symbol("s"))));
probe("size_string_numeric", () => (base3 as any).union(withSize("2")));
probe("size_string_junk", () => (base3 as any).union(withSize("two")));
probe("size_null", () => (base3 as any).union(withSize(null)));
probe("size_valueOf", () => (base3 as any).union(withSize({ valueOf() { return 2; } })));
probe("size_getter_throws", () => (base3 as any).union({ get size(): any { throw new EvalError("size"); }, has: goodHas, keys: goodKeys }));
probe("arg_number", () => (base3 as any).union(3));
probe("arg_null", () => (base3 as any).union(null));
probe("arg_missing", () => (base3 as any).union());
const opNames = ["union", "intersection", "difference", "symmetricDifference", "isSubsetOf", "isSupersetOf", "isDisjointFrom"];
let allNeg = "";
for (const op of opNames) {
  try { (base3 as any)[op](withSize(-1)); allNeg += op + ":ok;"; }
  catch (err: any) { allNeg += op + ":" + err.name + ";"; }
}
console.log("negative_all_ops=" + allNeg);
const liar = { size: 0, has: (v: any) => v === 2, keys: goodKeys };
console.log("lie_small=" + [...base3.intersection(liar as any)].join(",") + ":" + base3.isDisjointFrom(liar as any));
const liarBig = { size: 1000, has: (v: any) => v === 2, keys: goodKeys };
console.log("lie_big=" + [...base3.intersection(liarBig as any)].join(",") + ":" + base3.isDisjointFrom(liarBig as any) + ":" + base3.isSubsetOf(liarBig as any));

// --- brand checks and constructor ---
probe("receiver_array", () => (Set.prototype as any).union.call([1, 2], new Set([1])));
probe("receiver_map", () => (Set.prototype as any).union.call(new Map(), new Set([1])));
probe("receiver_proto", () => (Set.prototype as any).union(new Set([1])));
probe("set_add_plain", () => Set.prototype.add.call({} as any, 1));
probe("set_has_map", () => Set.prototype.has.call(new Map() as any, 1));
probe("set_no_new", () => (Set as any)([1]));
probe("set_from_number", () => new Set(7 as any));
probe("set_from_string", () => [...new Set("aab")].join(","));
probe("set_from_null", () => new Set(null as any).size);
const sctx = new Set([1, 2, 3]);
console.log("set_from_set=" + [...new Set(new Set([1, 2]))].join(",") + ":" + [...new Set(sctx.keys())].join(","));
function* sgen(): Generator<number> { yield 1; yield 2; yield 1; }
console.log("set_from_gen=" + [...new Set(sgen())].join(","));
console.log("set_from_chain=" + [...new Set(new Set([1, 2]).add(3))].join(","));
