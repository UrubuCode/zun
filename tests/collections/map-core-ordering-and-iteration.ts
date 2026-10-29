// Cross-runtime: Map core surface. Covers size/get/has/delete/clear, overwrite
// versus delete-and-re-add ordering, chaining, the forEach contract under
// mutation, and the live, independent keys()/values()/entries() iterators.

// --- basics ---
const m = new Map<string, number>();
console.log("empty_size=" + m.size);
console.log("set_returns_self=" + (m.set("a", 1) === m));
m.set("b", 2).set("c", 3);
console.log("chain_size=" + m.size);
console.log("get_hit=" + m.get("a") + ":miss=" + String(m.get("zz")));
console.log("has=" + m.has("b") + ":" + m.has("zz"));
console.log("delete_hit=" + m.delete("b") + ":delete_miss=" + m.delete("b"));
console.log("size_after_delete=" + m.size);
console.log("has_after_delete=" + m.has("b"));
console.log("get_after_delete=" + String(m.get("b")));
console.log("clear_returns=" + String(m.clear()) + ":size=" + m.size);

// --- insertion order: overwrite keeps the slot, delete + re-add moves to the end ---
const ow = new Map<string, number>([["a", 1], ["b", 2], ["c", 3]]);
ow.set("a", 99);
console.log("overwrite_keys=" + [...ow.keys()].join(","));
console.log("overwrite_values=" + [...ow.values()].join(","));
console.log("overwrite_size=" + ow.size);
ow.delete("b");
ow.set("b", 20);
console.log("readd_keys=" + [...ow.keys()].join(","));
console.log("readd_values=" + [...ow.values()].join(","));
console.log("entries=" + [...ow.entries()].map((e) => e[0] + ":" + e[1]).join("|"));
ow.clear();
console.log("cleared=" + ow.size + ":spread=" + [...ow].length + ":has_a=" + ow.has("a"));
ow.set("c", 3).set("b", 2).set("a", 1);
console.log("reuse_keys=" + [...ow.keys()].join(","));

const last = new Map<string, number>([["a", 1], ["b", 2], ["c", 3]]);
last.delete("c");
last.set("c", 30);
console.log("last_readd=" + [...last.keys()].join(","));
const cyc = new Map<string, number>([["x", 1], ["y", 2], ["z", 3]]);
cyc.delete("x");
cyc.set("x", 1);
cyc.delete("y");
cyc.set("y", 2);
console.log("cycle=" + [...cyc.keys()].join(","));

// --- forEach: (value, key, collection) triple, thisArg, return value ---
const fe = new Map<string, number>([["a", 1], ["b", 2]]);
const rows: string[] = [];
fe.forEach(function (value, key, coll) {
  rows.push(key + "=" + value + ":same=" + (coll === fe) + ":argc=" + arguments.length);
});
console.log("triple=" + rows.join("|"));
const order: string[] = [];
new Map([["k", "v"]]).forEach((a, b) => order.push("first=" + a + ",second=" + b));
console.log("arg_order=" + order.join(""));
console.log("forEach_returns=" + String(fe.forEach(() => 1)));
const ctx = { tag: "CTX", hits: [] as string[] };
new Map([["x", 1], ["y", 2]]).forEach(function (this: any, v, k) {
  this.hits.push(this.tag + ":" + k + v);
}, ctx);
console.log("thisArg=" + ctx.hits.join("|"));
const outer = { tag: "OUTER" };
let arrowSaw = "";
(function (this: any) {
  new Map([["z", 9]]).forEach(() => { arrowSaw = this.tag; }, { tag: "PASSED" });
}).call(outer);
console.log("arrow_ignores_thisArg=" + arrowSaw);
let emptyCalls = 0;
new Map().forEach(() => { emptyCalls++; });
console.log("empty_calls=" + emptyCalls);
let thrown = "none";
try {
  new Map([["a", 1], ["b", 2]]).forEach((v, k) => { if (k === "b") throw new Error("stop"); });
} catch (e: any) {
  thrown = e.message;
}
console.log("throws=" + thrown);

// --- forEach under mutation ---
const addDuring = new Map([["a", 1]]);
const visitedAdd: string[] = [];
addDuring.forEach((v, k) => {
  visitedAdd.push(k);
  if (k === "a") addDuring.set("b", 2);
});
console.log("added_during=" + visitedAdd.join(","));

const delDuring = new Map([["a", 1], ["b", 2], ["c", 3]]);
const visitedDel: string[] = [];
delDuring.forEach((v, k) => {
  visitedDel.push(k);
  if (k === "a") delDuring.delete("b");
});
console.log("deleted_during=" + visitedDel.join(","));

const liveVals = new Map([["a", 1], ["b", 2]]);
const seenVals: string[] = [];
liveVals.forEach((v, k) => {
  seenVals.push(k + "=" + v);
  if (k === "a") liveVals.set("b", 99);
});
console.log("live_values=" + seenVals.join(","));

const readd = new Map([["a", 1], ["b", 2]]);
const visitedReadd: string[] = [];
readd.forEach((v, k) => {
  visitedReadd.push(k);
  if (k === "a" && visitedReadd.length === 1) { readd.delete("a"); readd.set("a", 1); }
});
console.log("readd_during=" + visitedReadd.join(","));

const clearDuring = new Map<any, any>([["a", 1], ["b", 2], ["c", 3]]);
const seenClear: string[] = [];
clearDuring.forEach((v, k) => {
  seenClear.push(k + v);
  if (k === "a") {
    clearDuring.clear();
    clearDuring.set("d", 4);
    clearDuring.set("e", 5);
  }
});
console.log("clear_during_seen=" + seenClear.join(","));
console.log("clear_during_after=" + [...clearDuring.entries()].map((e) => e[0] + e[1]).join(","));

// --- for-of over a Map, with mutation ---
let forOfOut = "";
for (const [k, v] of new Map([["a", 1], ["b", 2], ["c", 3]])) {
  if (k !== "b") forOfOut += k + ":" + v + ",";
}
console.log("for_of=" + forOfOut);

const fo = new Map<string, number>([["a", 1], ["b", 2], ["c", 3]]);
const seenFo: string[] = [];
for (const [k, v] of fo) {
  seenFo.push(k + v);
  if (k === "a") {
    fo.set("d", 4);
    fo.delete("b");
  }
  if (k === "c") fo.set("e", 5);
}
console.log("for_of_mut_seen=" + seenFo.join(","));
console.log("for_of_mut_keys=" + [...fo.keys()].join(","));

// --- keys()/values()/entries(): independent, live iterators ---
const base = new Map([["a", 1], ["b", 2], ["c", 3]]);
const it1 = base.keys();
const it2 = base.keys();
console.log("it1_first=" + it1.next().value);
console.log("it2_first=" + it2.next().value);
console.log("it1_second=" + it1.next().value);
console.log("it2_still_second=" + it2.next().value);
console.log("iterators_distinct=" + (it1 === it2) + ":" + (base.keys() === base.keys()));
const ks = base.keys(), vs = base.values(), es = base.entries();
console.log("first_k_v_e=" + ks.next().value + ":" + vs.next().value + ":" + es.next().value.join(":"));

const exhaust = new Map([["x", 1]]).keys();
const r1 = exhaust.next();
const r2 = exhaust.next();
const r3 = exhaust.next();
console.log("exhaust=" + r1.value + ":" + r1.done + "|" + String(r2.value) + ":" + r2.done + "|" + String(r3.value) + ":" + r3.done);

const grown = new Map([["a", 1]]);
const gi = grown.keys();
gi.next();
console.log("grown_done=" + gi.next().done);
grown.set("b", 2);
console.log("grown_after=" + String(gi.next().value) + ":" + gi.next().done);

const live = new Map([["a", 1]]);
const li = live.keys();
console.log("live_a=" + li.next().value);
live.set("b", 2);
console.log("live_sees_b=" + li.next().value);
console.log("live_done=" + li.next().done);

const skip = new Map([["a", 1], ["b", 2], ["c", 3]]);
const si = skip.keys();
console.log("skip_a=" + si.next().value);
skip.delete("b");
console.log("skip_next=" + si.next().value);

const partial = new Map([["a", 1], ["b", 2], ["c", 3]]);
const pi = partial.keys();
pi.next();
console.log("rest=" + [...pi].join(","));
console.log("fresh_after=" + [...partial.keys()].join(",") + ":" + [...partial.keys()].join(","));
console.log("iterator_self=" + (ks[Symbol.iterator]() === ks));
console.log("map_iter_is_entries=" + (base[Symbol.iterator] === base.entries));
const defPair = [...base][0];
console.log("default_pair=" + defPair[0] + ":" + defPair[1]);
const p1 = base.entries().next().value;
const p2 = base.entries().next().value;
console.log("pairs_fresh=" + (p1 === p2) + ":" + (p1[0] === p2[0] && p1[1] === p2[1]));

// --- iterator object surface ---
const sample = new Map([["a", 1], ["b", 2]]);
const mk = sample.keys();
const mv = sample.values();
const me = sample.entries();
console.log("tag_keys=" + Object.prototype.toString.call(mk));
console.log("tag_entries=" + Object.prototype.toString.call(me));
const mkp = Object.getPrototypeOf(mk);
console.log("shared_proto=" + (mkp === Object.getPrototypeOf(mv)) + ":" + (mkp === Object.getPrototypeOf(me)));
console.log("proto_not_iterator=" + (mkp === mk));
const iterProto = Object.getPrototypeOf(mkp);
console.log("shared_iter_proto=" + (iterProto === Object.getPrototypeOf(Object.getPrototypeOf([].values()))));
console.log("iter_proto_parent=" + (Object.getPrototypeOf(iterProto) === Object.prototype));
console.log("next_own=" + Object.prototype.hasOwnProperty.call(mkp, "next") + ":return=" + typeof (mkp as any).return + ":throw=" + typeof (mkp as any).throw);
const shape: any = mk.next();
console.log("next_shape=" + Object.getOwnPropertyNames(shape).sort().join(",") + ":proto_object=" + (Object.getPrototypeOf(shape) === Object.prototype));
const entryArr: any = me.next().value;
console.log("entry_is_array=" + Array.isArray(entryArr) + ":" + entryArr.length + ":" + (Object.getPrototypeOf(entryArr) === Array.prototype));
for (const [label, recv] of [["plain", {}], ["map", sample], ["arrayiter", [1].values()]] as [string, any][]) {
  try {
    (mkp as any).next.call(recv);
    console.log("next_on_" + label + "=no_throw");
  } catch (e: any) {
    console.log("next_on_" + label + "=" + e.name);
  }
}
const sv = new Set([10, 20]).values();
console.log("set_iter_tag=" + Object.prototype.toString.call(sv) + ":shared_with_map=" + (Object.getPrototypeOf(sv) === mkp));
