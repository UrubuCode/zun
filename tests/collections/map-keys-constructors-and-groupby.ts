// Cross-runtime: Map key identity (SameValueZero), the Map constructor over
// varied iterables, Map versus plain-object key behaviour, and the grouping
// statics Map.groupBy / Object.groupBy.

// --- SameValueZero: -0 normalises to +0 on the stored key ---
const z1 = new Map<any, string>();
z1.set(-0, "v");
const storedZero = [...z1.keys()][0];
console.log("stored_is_plus_zero=" + Object.is(storedZero, 0) + ":div=" + (1 / storedZero));
const z2 = new Map<any, string>([[-0, "first"]]);
z2.set(0, "second");
console.log("zero_pair_size=" + z2.size + ":get_neg=" + z2.get(-0) + ":get_pos=" + z2.get(0));
const z3 = new Map<any, string>([[0, "first"]]);
z3.set(0 * -1, "computed");
console.log("computed_neg_zero=" + z3.size + ":div=" + (1 / [...z3.keys()][0]) + ":has=" + z3.has(0));

// --- SameValueZero: every NaN is one key ---
const nanMap = new Map<any, string>();
nanMap.set(NaN, "a");
nanMap.set(Number.NaN, "b");
nanMap.set(0 / 0, "c");
nanMap.set(Math.sqrt(-1), "d");
nanMap.set(parseFloat("zzz"), "e");
console.log("nan_size=" + nanMap.size + ":get=" + nanMap.get(NaN) + ":key_is_nan=" + Number.isNaN([...nanMap.keys()][0]));
console.log("nan_eq_self=" + (NaN === NaN) + ":has=" + nanMap.has(NaN) + ":has_computed=" + nanMap.has(0 / 0));
const nanFe: string[] = [];
const edgeKeys = new Map<any, string>([[-0, "z"], [NaN, "n"]]);
edgeKeys.forEach((v, k) => { nanFe.push(String(1 / k) + "=" + v); });
console.log("edge_foreach=" + nanFe.join("|"));
console.log("edge_delete=" + edgeKeys.delete(-0) + ":" + edgeKeys.size + ":" + edgeKeys.delete(0 / 0) + ":" + edgeKeys.size);

// --- object keys use identity, not structure ---
const oa = { x: 1 };
const ob = { x: 1 };
const objMap = new Map<any, string>([[oa, "a"], [ob, "b"]]);
console.log("obj_keys=" + objMap.size + ":" + objMap.get(oa) + ":" + objMap.get(ob) + ":" + String(objMap.get({ x: 1 })));
oa.x = 9;
console.log("obj_mutated=" + objMap.get(oa) + ":" + [...objMap.keys()].map((k) => k.x).join(","));
const keyObj: any = { id: 1 };
const idMap = new Map<any, string>([[keyObj, "old"]]);
keyObj.id = 2;
idMap.set({ id: 2 }, "new");
console.log("id_map=" + String(idMap.get(keyObj)) + ":" + String(idMap.get({ id: 2 })) + ":" + idMap.size);

// --- mixed primitive key types stay distinct ---
const mixed = new Map<any, string>([[1, "num"], ["1", "str"], [true, "bool"], ["true", "bool_str"], [null, "null"], [undefined, "undef"]]);
console.log("mixed_size=" + mixed.size + ":" + mixed.get(1) + ":" + mixed.get("1") + ":" + mixed.get(true) + ":" + mixed.get("true"));
console.log("mixed_null_undef=" + mixed.get(null) + ":" + mixed.get(undefined) + ":" + String(mixed.get("null")));
const mixedKeys = [...mixed.keys()].map((k) => typeof k + ":" + String(k));
console.log("mixed_types=" + mixedKeys.join(","));

// --- constructor from iterables ---
const fromPairs = new Map([["a", 1], ["b", 2], ["a", 3]]);
console.log("pairs_dup=" + fromPairs.size + ":" + [...fromPairs.keys()].join(",") + ":a=" + fromPairs.get("a"));
const src = new Map([["x", 10], ["y", 20]]);
const copy = new Map(src);
copy.set("z", 30);
src.set("x", 999);
console.log("copy=" + copy.size + ":" + src.size + ":" + copy.get("x") + ":" + copy.get("z") + ":distinct=" + (copy !== src));
console.log("from_entries=" + [...new Map(new Map([["p", 1], ["q", 2]]).entries())].join("|"));
console.log("from_set_pairs=" + [...new Map(new Set([["s", 1], ["t", 2]] as any))].map((e) => e.join(":")).join("|"));
function* pairGen(): Generator<[string, number]> {
  yield ["g1", 1];
  yield ["g2", 2];
}
console.log("from_gen=" + [...new Map(pairGen())].map((e) => e.join(":")).join("|"));
console.log("from_object_entries=" + [...new Map(Object.entries({ o1: 1, o2: 2 }))].map((e) => e.join(":")).join("|"));
console.log("empty_sources=" + new Map([]).size + ":" + new Map().size + ":" + new Map(null as any).size + ":" + new Map(undefined).size);
const extra = new Map([["k", "v", "ignored"]] as any);
console.log("extra_ignored=" + extra.get("k") + ":" + extra.size);
const short = new Map([["only"]] as any);
console.log("short_entry=" + short.has("only") + ":" + String(short.get("only")) + ":" + short.size);
for (const bad of [[1, 2], ["ab"], [null]]) {
  try {
    new Map(bad as any);
    console.log("bad_entry=no_throw");
  } catch (e: any) {
    console.log("bad_entry=" + e.name);
  }
}
const subSource = new (class Sub extends Map<string, number> {})([["q", 1]]);
const fromSub = new Map(subSource);
console.log("from_subclass=" + fromSub.constructor.name + ":" + fromSub.size);

// --- Map keeps insertion order for every key; objects reorder and coerce ---
const insert = ["2", "10", "1", "b", "a", "0"];
const obj: any = {};
for (const k of insert) obj[k] = k;
console.log("object_keys=" + Object.keys(obj).join(","));
const im = new Map<any, string>();
for (const k of insert) im.set(k, k);
console.log("map_keys=" + [...im.keys()].join(","));
const edge: any = {};
for (const k of ["4294967296", "-1", "01", "1.0", "1e2", "9007199254740992", "3"]) edge[k] = 1;
console.log("object_edge=" + Object.keys(edge).join(","));
const edgeM = new Map<any, number>();
for (const k of ["4294967296", "-1", "01", "1.0", "1e2", "9007199254740992", "3"]) edgeM.set(k, 1);
console.log("map_edge=" + [...edgeM.keys()].join(","));
const coerce: any = {};
coerce[1] = "number";
coerce["1"] = "string";
coerce[true] = "boolean";
console.log("object_coerced=" + Object.keys(coerce).length + ":" + coerce[1]);
const zeroObj: any = {};
zeroObj[-0] = "obj";
console.log("object_neg_zero=" + JSON.stringify(Object.keys(zeroObj)));
const late: any = { z: 1, y: 2 };
late[5] = 3;
console.log("object_late_integer=" + Object.keys(late).join(","));
const lateMap = new Map<any, number>([["z", 1], ["y", 2]]);
lateMap.set(5, 3);
console.log("map_late_integer=" + [...lateMap.keys()].join(","));
const sym = Symbol("s");
const mixedObj: any = {};
mixedObj[sym] = 1;
mixedObj["z"] = 2;
mixedObj["1"] = 3;
console.log("object_ownkeys=" + Reflect.ownKeys(mixedObj).map(String).join(","));
const symMap = new Map<any, number>([[sym, 1], ["z", 2], ["1", 3]]);
console.log("map_ownkeys=" + [...symMap.keys()].map(String).join(","));
const fromMap = Object.fromEntries(im);
console.log("from_entries_keys=" + Object.keys(fromMap).join(","));
const back = new Map(Object.entries(fromMap));
console.log("round_trip_order_same=" + ([...back.keys()].join(",") === [...im.keys()].join(",")));
const numeric = new Map<any, string>([[2, "two"], [1, "one"]]);
const numericObj = Object.fromEntries(numeric);
console.log("numeric_obj=" + JSON.stringify(Object.keys(numericObj)) + ":" + typeof [...new Map(Object.entries(numericObj)).keys()][0]);
const reorder = new Map([["a", 1], ["b", 2], ["c", 3]]);
reorder.delete("a");
reorder.set("a", 9);
const objReorder: any = { a: 1, b: 2, c: 3 };
delete objReorder.a;
objReorder.a = 9;
console.log("readd_map=" + [...reorder.keys()].join(",") + ":readd_obj=" + Object.keys(objReorder).join(","));

// --- Object.groupBy versus Map.groupBy ---
const items = [1, 2, 3, 4, 5];
const og: any = Object.groupBy(items, (x) => (x % 2 === 0 ? "even" : "odd"));
console.log("obj_groups=" + Object.keys(og).join(",") + ":odd=" + og.odd.join(",") + ":even=" + og.even.join(","));
const ogNum: any = Object.groupBy(items, (x) => x % 2);
console.log("obj_num_keys=" + Object.keys(ogNum).join(",") + ":get0=" + ogNum[0].join(",") + ":get_str0=" + ogNum["0"].join(","));
console.log("obj_proto_null=" + (Object.getPrototypeOf(og) === null));
const ogSpecial: any = Object.groupBy([1, 2, 3, 4], (x) => (x === 1 ? -0 : x === 2 ? NaN : x === 3 ? undefined : null));
console.log("obj_coerced_keys=" + Object.keys(ogSpecial).join("|"));
const mg = Map.groupBy(items, (x) => x % 2);
console.log("map_keys=" + [...mg.keys()].join(",") + ":types=" + [...mg.keys()].map((k) => typeof k).join(","));
console.log("map_get_num=" + mg.get(0)!.join(",") + ":str_miss=" + String(mg.get("0" as any)));
const gA = { name: "A" };
const gB = { name: "B" };
const mgObj = Map.groupBy(items, (x) => (x > 3 ? gA : gB));
console.log("map_obj_groups=" + mgObj.size + ":" + mgObj.get(gA)!.join(",") + ":" + mgObj.get(gB)!.join(",") + ":fresh_miss=" + String(mgObj.get({ name: "A" })));
const mgSvz = Map.groupBy([1, 2, 3], (x) => (x === 1 ? -0 : x === 2 ? 0 : NaN));
console.log("svz_groups=" + mgSvz.size + ":zero=" + mgSvz.get(0)!.join(",") + ":nan=" + mgSvz.get(NaN)!.join(",") + ":div=" + (1 / [...mgSvz.keys()][0]));
const mgBool = Map.groupBy(items, (x) => x > 2);
console.log("bool_groups=" + [...mgBool.keys()].join(",") + ":true=" + mgBool.get(true)!.join(","));
const idx: string[] = [];
Map.groupBy(["a", "b"], (v, i) => { idx.push(v + "@" + i + "/" + typeof i); return v; });
console.log("map_cb_args=" + idx.join(","));
const oidx: string[] = [];
Object.groupBy(["a", "b"], (v, i) => { oidx.push(v + "@" + i); return v; });
console.log("obj_cb_args=" + oidx.join(","));
const first = Map.groupBy(["zz", "a", "yyy", "b"], (s) => s.length);
console.log("first_appearance=" + [...first.keys()].join(",") + ":group_is_array=" + Array.isArray(first.get(2)));
const mgOrder = Map.groupBy([3, 1, 3, 2, 1], (x) => x);
console.log("order=" + [...mgOrder.keys()].join(",") + ":groups=" + [...mgOrder.values()].map((v) => v.join("+")).join("|"));
console.log("from_string=" + [...Map.groupBy("banana", (c) => c).keys()].join(","));
console.log("from_set=" + [...Map.groupBy(new Set([1, 2, 3]), (n) => n > 1).keys()].join(","));
console.log("empty_groupby=" + Map.groupBy([], (x: any) => x).size + ":" + Object.keys(Object.groupBy([], (x: any) => x)).length);
let groupCalls = 0;
Map.groupBy([], (_e: any) => { groupCalls++; return 1; });
console.log("empty_calls=" + groupCalls);
console.log("groupby_static=" + typeof Map.groupBy + ":" + ("groupBy" in Map.prototype) + ":" + Map.groupBy.length + ":" + Map.groupBy.name);
const gd: any = Object.getOwnPropertyDescriptor(Map, "groupBy");
console.log("groupby_flags=" + gd.writable + ":" + gd.enumerable + ":" + gd.configurable);
for (const [label, fn] of [
  ["non_callable", () => (Map as any).groupBy([1], 42)],
  ["non_iterable", () => (Map as any).groupBy(42, (x: any) => x)],
  ["null_source", () => (Map as any).groupBy(null, (x: any) => x)],
  ["plain_object", () => (Map as any).groupBy({ a: 1 }, (x: any) => x)],
  ["throwing_cb", () => Map.groupBy([1, 2], (n) => { if (n === 2) throw new RangeError("x"); return n; })],
] as [string, () => any][]) {
  try {
    fn();
    console.log(label + "=no_throw");
  } catch (e: any) {
    console.log(label + "=" + e.name);
  }
}
