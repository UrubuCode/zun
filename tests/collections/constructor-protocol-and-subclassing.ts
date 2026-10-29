// Cross-runtime: how the Map/Set constructors consume their argument, and how
// subclasses participate. Covers the adder (`set` / `add`) read exactly once
// from the new instance, the iterator-close rules on a failed build, the
// non-callable adder refusal, subclass overrides seen during construction,
// new.target prototype selection, and species being ignored by Set operations.

// --- the adder is read once, not once per entry ---
let mapGets = 0;
class CountingMap extends Map<any, any> {
  get set(): any {
    mapGets++;
    return Map.prototype.set;
  }
}
const cm = new CountingMap([["a", 1], ["b", 2], ["c", 3]]);
console.log("map_adder_gets=" + mapGets + ":entries=" + [...cm.keys()].join(","));
mapGets = 0;
const cmEmpty = new CountingMap([]);
console.log("map_adder_gets_empty=" + mapGets + ":size=" + cmEmpty.size);
mapGets = 0;
const cmNone = new CountingMap();
console.log("map_adder_gets_noarg=" + mapGets + ":size=" + cmNone.size);

let setGets = 0;
class CountingSet extends Set<any> {
  get add(): any {
    setGets++;
    return Set.prototype.add;
  }
}
const cs = new CountingSet([1, 2, 3, 4]);
console.log("set_adder_gets=" + setGets + ":entries=" + [...cs].join(","));

// --- a non-callable adder is refused, even for an empty iterable ---
class BadMap extends Map<any, any> {
  get set(): any { return 42; }
}
for (const [label, make] of [
  ["bad_adder", () => new BadMap([["a", 1]])],
  ["bad_adder_empty", () => new BadMap([])],
  ["bad_adder_noarg", () => new BadMap()],
] as [string, () => any][]) {
  try {
    const b = make();
    console.log(label + "=no_throw:" + b.size);
  } catch (e: any) {
    console.log(label + "=" + e.name);
  }
}

// --- subclass overrides are what the constructor calls ---
class UpperMap extends Map<any, any> {
  set(k: any, v: any): any {
    return super.set(String(k).toUpperCase(), v);
  }
}
const um = new UpperMap([["a", 1], ["b", 2]]);
um.set("c", 3);
console.log("upper_keys=" + [...um.keys()].join(",") + ":size=" + um.size);
class DoublingSet extends Set<any> {
  add(v: any): any { return super.add(v * 2); }
}
console.log("doubled=" + [...new DoublingSet([1, 2, 3])].join(","));
let sawThis = "";
class ThisMap extends Map<any, any> {
  set(k: any, v: any): any {
    sawThis = (this instanceof ThisMap) + ":" + (this instanceof Map);
    return super.set(k, v);
  }
}
new ThisMap([["k", "v"]]);
console.log("adder_this=" + sawThis);
const seq: string[] = [];
class SeqSet extends Set<any> {
  add(v: any): any { seq.push(String(v)); return super.add(v); }
}
const seqSet = new SeqSet(["c", "a", "b", "a"]);
console.log("adder_sequence=" + seq.join(",") + ":count=" + seq.length + ":size=" + seqSet.size + ":stored=" + [...seqSet].join(","));
class ThrowingSet extends Set<any> {
  add(v: any): any { if (v === 2) throw new RangeError("no"); return super.add(v); }
}
try {
  const ts = new ThrowingSet([1, 2, 3]);
  console.log("throwing_adder=no_throw:" + ts.size);
} catch (e: any) {
  console.log("throwing_adder=" + e.name);
}
class OwnAdder extends Map<any, any> {
  constructor(src?: any) {
    super();
    (this as any).set = function (k: any, v: any) { return Map.prototype.set.call(this, "own:" + k, v); };
    if (src) for (const e of src) (this as any).set(e[0], e[1]);
  }
}
const oa = new OwnAdder([["p", 1]]);
console.log("own_adder=" + [...oa.keys()].join(",") + ":is_own=" + Object.prototype.hasOwnProperty.call(oa, "set"));
const realAdd = Set.prototype.add;
const patchedSeen: string[] = [];
(Set.prototype as any).add = function (v: any) {
  patchedSeen.push(String(v));
  return realAdd.call(this, v);
};
const plain = new Set([7, 8, 9]);
(Set.prototype as any).add = realAdd;
console.log("patched_seen=" + patchedSeen.join(",") + ":result=" + [...plain].join(",") + ":restored=" + (Set.prototype.add === realAdd));

// --- iterator close: a failed build closes the source; a clean walk does not ---
function source(values: any[], seen: string[]): any {
  let i = 0;
  return {
    [Symbol.iterator]() { seen.push("iter"); return this; },
    next() {
      seen.push("next" + i);
      return i < values.length ? { value: values[i++], done: false } : { value: undefined, done: true };
    },
    return(v: any) { seen.push("return"); return { done: true, value: v }; },
  };
}
function run(label: string, values: any[], build: (it: any) => any): void {
  const seen: string[] = [];
  let outcome: string;
  try {
    const r = build(source(values, seen));
    outcome = "ok:" + (r.size !== undefined ? r.size : String(r));
  } catch (e: any) {
    outcome = e.name;
  }
  console.log(label + "=" + outcome + " [" + seen.join(",") + "]");
}
run("map_ok", [["a", 1], ["b", 2]], (it) => new Map(it));
run("set_ok", [1, 2], (it) => new Set(it));
run("weakset_ok", [{}, {}], (it) => new WeakSet(it));
run("map_primitive_entry", [["a", 1], 7, ["c", 3]], (it) => new Map(it));
run("map_null_entry", [null], (it) => new Map(it));
run("map_string_entry", ["ab"], (it) => new Map(it));
run("weakset_primitive", [{}, 5], (it) => new WeakSet(it));
run("weakmap_primitive_key", [[{}, 1], [5, 2]], (it) => new WeakMap(it));
const boomEntry: any = { get 0() { throw new EvalError("k"); }, get 1() { return 1; } };
run("map_entry_getter_throws", [boomEntry], (it) => new Map(it));
run("subclass_add_throws", [1, 2, 3], (it) => new ThrowingSet(it));
class NoAdder extends Set<any> {}
(NoAdder.prototype as any).add = 42;
run("adder_not_callable", [1, 2], (it) => new NoAdder(it));
run("no_arg", [], () => new Map());
console.log("null_arg=" + new Map(null).size + ":" + new Set(null).size);

// a source whose return() itself throws: the ORIGINAL error still wins
function badReturn(values: any[], seen: string[]): any {
  let i = 0;
  return {
    [Symbol.iterator]() { return this; },
    next() { seen.push("next"); return i < values.length ? { value: values[i++], done: false } : { value: undefined, done: true }; },
    return() { seen.push("return"); throw new URIError("from_return"); },
  };
}
const seenBad: string[] = [];
try { new Map(badReturn([7], seenBad)); console.log("bad_return=no_throw"); }
catch (e: any) { console.log("bad_return=" + e.name + " [" + seenBad.join(",") + "]"); }

// a source with no return() at all is simply left alone
function noReturn(values: any[], seen: string[]): any {
  let i = 0;
  return {
    [Symbol.iterator]() { return this; },
    next() { seen.push("next"); return i < values.length ? { value: values[i++], done: false } : { value: undefined, done: true }; },
  };
}
const seenNo: string[] = [];
try { new Map(noReturn([7], seenNo)); console.log("no_return=no_throw"); }
catch (e: any) { console.log("no_return=" + e.name + " [" + seenNo.join(",") + "]"); }

// generators are closed the same way; a closed generator stays done
let genState = "fresh";
function* gen(): Generator<any> {
  try {
    genState = "running";
    yield ["a", 1];
    yield 9;
    yield ["c", 3];
  } finally {
    genState = "finalised";
  }
}
const g = gen();
try { new Map(g as any); console.log("gen_build=no_throw"); }
catch (e: any) { console.log("gen_build=" + e.name); }
console.log("gen_state=" + genState + ":gen_after=" + JSON.stringify(g.next()));

// --- subclassing: prototype from new.target, field timing, species ---
class MyMap extends Map<any, any> {
  tag = "mine";
  first(): any { return [...this.keys()][0]; }
}
class MySet extends Set<any> {
  tag = "mine";
}
const mm = new MyMap([["a", 1], ["b", 2]]);
console.log("mm=" + mm.size + ":" + mm.tag + ":" + mm.first() + ":" + [...mm.keys()].join(","));
console.log("mm_checks=" + (mm instanceof MyMap) + ":" + (mm instanceof Map) + ":" + (Object.getPrototypeOf(mm) === MyMap.prototype) + ":" + mm.constructor.name);
console.log("mm_own=" + Object.getOwnPropertyNames(mm).join(",") + ":tostring=" + Object.prototype.toString.call(mm));
console.log("static=" + (Object.getPrototypeOf(MyMap) === Map) + ":" + (MyMap[Symbol.species] === MyMap) + ":" + typeof (MyMap as any).groupBy);
const rc: any = Reflect.construct(Map, [[["k", 9]]], MyMap);
console.log("rc=" + (Object.getPrototypeOf(rc) === MyMap.prototype) + ":" + (rc instanceof Map) + ":" + rc.size + ":" + rc.get("k") + ":hasField=" + ("tag" in rc) + ":first=" + rc.first());
function Alien() { /* only its .prototype is read */ }
(Alien as any).prototype = { kind: "alien" };
const rc2: any = Reflect.construct(Map, [], Alien as any);
console.log("rc2=" + rc2.kind + ":" + (rc2 instanceof Map) + ":" + Map.prototype.set.call(rc2, "z", 1).size + ":" + Map.prototype.get.call(rc2, "z"));
function NoProto() { /* placeholder */ }
(NoProto as any).prototype = 7;
const rc3: any = Reflect.construct(Set, [[1, 2]], NoProto as any);
console.log("rc3=" + (Object.getPrototypeOf(rc3) === Set.prototype) + ":" + rc3.size);
const ms = new MySet([1, 2, 3]);
const msUnion = ms.union(new Set([4]));
console.log("union_species=" + msUnion.constructor.name + ":" + (msUnion instanceof MySet) + ":" + [...msUnion].join(","));
console.log("other_species=" + ms.difference(new Set([1])).constructor.name + ":" + ms.intersection(new Set([2])).constructor.name + ":" + ms.symmetricDifference(new Set([9])).constructor.name);
class SpeciesSet extends Set<any> {
  static get [Symbol.species]() { return Set; }
}
console.log("declared_species=" + (SpeciesSet[Symbol.species] === Set) + ":" + new SpeciesSet([1]).union(new Set([2])).constructor.name);
const copy = new Map(mm);
console.log("copy_ctor=" + copy.constructor.name + ":" + copy.size + ":distinct=" + (copy !== mm));
class Broken extends Map<any, any> {
  constructor() {
    super();
  }
}
console.log("broken_ok=" + new Broken().size);
try { (MyMap as any)(); console.log("call_subclass=no_throw"); }
catch (e: any) { console.log("call_subclass=" + e.name); }
