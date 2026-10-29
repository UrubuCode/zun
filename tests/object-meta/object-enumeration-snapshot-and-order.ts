// Enumeration as a single agreed order and as a snapshot-vs-live question: every own-key API
// reaches the same sequence; the canonical-index boundary (2^32-2, non-canonical numerics);
// Object.keys/values/entries snapshot the key list before reading values, while for..in is live;
// a non-enumerable own key shadows an enumerable inherited name; mutation during for..in.
export {};

function forinOf(x: any): string {
  const out: string[] = [];
  for (const k in x) out.push(k);
  return out.join("|");
}

// ---------- 1. The canonical integer-index boundary ----------
const idx: any = {};
idx["zzz"] = "s0";
idx["4294967295"] = "not-index";
idx["4294967294"] = "last-index";
idx["10"] = "ten";
idx["-1"] = "neg";
idx["01"] = "leading-zero";
idx["1"] = "one";
idx["1e2"] = "exp";
idx["100"] = "hundred";
idx["-0"] = "neg-zero";
idx["0"] = "zero";
idx["+1"] = "plus-one";
idx["1.0"] = "dot-zero";
idx[" 1"] = "space-one";
idx["0x1"] = "hex";
idx["NaN"] = "nan";
idx["Infinity"] = "inf";
idx["9007199254740993"] = "big";
console.log("boundary_keys=" + Object.keys(idx).join("|"));
console.log("boundary_names=" + Object.getOwnPropertyNames(idx).join("|"));
console.log("boundary_reflect=" + Reflect.ownKeys(idx).join("|"));
console.log("boundary_values=" + Object.values(idx).join("|"));
console.log("boundary_forin_eq=" + (forinOf(idx) === Object.keys(idx).join("|")));
console.log("boundary_json=" + JSON.stringify(idx).slice(0, 40));
console.log("boundary_assign=" + Object.keys(Object.assign({}, idx)).join("|"));
console.log("boundary_spread_first=" + Object.keys({ ...idx })[0]);
console.log("boundary_entries_first=" + Object.entries(idx)[0].join("="));

// numeric keys inserted out of order still ascend
const outOfOrder: any = {};
outOfOrder[9] = "n9";
outOfOrder[3] = "n3";
outOfOrder[11] = "n11";
outOfOrder[2] = "n2";
console.log("numeric_order=" + Object.keys(outOfOrder).join("|"));

// number and string forms of a key are the same own key
const coerce: any = {};
coerce[1] = "num";
coerce["1"] = "str";
console.log("coerce_same=" + Object.keys(coerce).length + ":" + coerce[1]);
const minus: any = {};
minus[-0] = "minus-zero";
console.log("minus_zero_key=" + Object.keys(minus).join("|") + ",read=" + minus["0"]);

// integer-like keys on an array: the float-looking key stays a string key
const arrF: any = [];
arrF["1.5"] = "half";
arrF[2] = "two";
console.log("arr_float=" + arrF.length + ",keys=" + Object.keys(arrF).join("|"));
const arrG: any = [1, 2];
arrG.tail = "T";
console.log("arr_gopn=" + Object.getOwnPropertyNames(arrG).join("|"));
const sparse: any = [];
sparse[5] = "five";
sparse[2] = "two";
sparse.beta = "b";
sparse[0] = "zero";
console.log("sparse_keys=" + Object.keys(sparse).join("|") + ",length=" + sparse.length);

// ---------- 2. One agreed order across all own-key APIs ----------
const agree: any = {};
agree["b"] = 1;
agree["3"] = 2;
agree["a"] = 3;
agree["1"] = 4;
agree["c"] = 5;
agree["2"] = 6;
const canonical = Object.keys(agree).join("|");
console.log("agree_canonical=" + canonical);
const checks: Record<string, string> = {
  forin: forinOf(agree),
  json: Object.keys(JSON.parse(JSON.stringify(agree))).join("|"),
  assign: Object.keys(Object.assign({}, agree)).join("|"),
  spread: Object.keys({ ...agree }).join("|"),
  entries: Object.entries(agree).map((e) => e[0]).join("|"),
  names: Object.getOwnPropertyNames(agree).join("|"),
  reflect: Reflect.ownKeys(agree).join("|"),
  descs: Object.keys(Object.getOwnPropertyDescriptors(agree)).join("|"),
  fromEntries: Object.keys(Object.fromEntries(Object.entries(agree))).join("|"),
};
for (const name of Object.keys(checks)) {
  console.log("agree_" + name + "=" + (checks[name] === canonical));
}

// delete then re-add: a string key moves to the end, an index key keeps its numeric slot
delete agree["a"];
agree["a"] = 9;
console.log("readd_string=" + Object.keys(agree).join("|"));
delete agree["1"];
agree["1"] = 8;
console.log("readd_index=" + Object.keys(agree).join("|"));
agree["b"] = 100;
console.log("overwrite_keeps=" + Object.keys(agree).join("|"));

// defineProperty adds at the end of the string group; redefining does not move a key
const dp: any = { x: 1, z: 2 };
Object.defineProperty(dp, "y", { value: 3, enumerable: true, configurable: true, writable: true });
console.log("define_end=" + Object.keys(dp).join("|"));
Object.defineProperty(dp, "x", { value: 10 });
console.log("redefine_keeps=" + Object.keys(dp).join("|"));

// a non-enumerable key keeps its slot in ownKeys and vanishes from the enumerable views
const hide: any = { p: 1, q: 2, r: 3 };
Object.defineProperty(hide, "q", { enumerable: false });
console.log("hide_names=" + Object.getOwnPropertyNames(hide).join("|"));
console.log("hide_keys=" + Object.keys(hide).join("|") + ",forin=" + forinOf(hide) + ",json=" + JSON.stringify(hide));

// symbols come after every string key, in insertion order, and only ownKeys reports them
const s1 = Symbol("one");
const s2 = Symbol("two");
const syms: any = {};
syms[s2] = 1;
syms["late"] = 2;
syms[s1] = 3;
syms["9"] = 4;
console.log("sym_own=" + Reflect.ownKeys(syms).map(String).join("|"));
console.log("sym_names=" + Object.getOwnPropertyNames(syms).join("|") + ",symbols=" + Object.getOwnPropertySymbols(syms).map(String).join("|"));

// class instances: constructor assignment order rules as well
class Rec {
  constructor() {
    (this as any).zeta = 1;
    (this as any)["7"] = 2;
    (this as any).alpha = 3;
    (this as any)["2"] = 4;
  }
}
console.log("class_keys=" + Object.keys(new Rec()).join("|"));

// an object built from a Map keeps the Map's insertion order for string keys
const fromMap = new Map<string, number>([["m", 1], ["5", 2], ["k", 3], ["0", 4]]);
console.log("frommap=" + Object.keys(Object.fromEntries(fromMap)).join("|") + ",iter=" + Array.from(fromMap.keys()).join("|"));

// a null-prototype object orders identically
const bare: any = Object.create(null);
bare["b"] = 1;
bare["3"] = 2;
bare["a"] = 3;
bare["1"] = 4;
console.log("bare_keys=" + Object.keys(bare).join("|") + ",forin=" + forinOf(bare));

// JSON.stringify with a key list uses the list's order
console.log("json_list=" + JSON.stringify(agree, ["c", "b", "3"]));

// ---------- 3. Snapshot semantics: key list taken first, values read lazily ----------
function makeSubject(mutate: (self: any) => void): any {
  const o: any = {};
  Object.defineProperty(o, "a", { get() { return "A"; }, enumerable: true, configurable: true });
  Object.defineProperty(o, "b", { get() { mutate(o); return "B"; }, enumerable: true, configurable: true });
  Object.defineProperty(o, "c", { get() { return "C"; }, enumerable: true, configurable: true });
  Object.defineProperty(o, "d", { get() { return "D"; }, enumerable: true, configurable: true });
  return o;
}
console.log("del_entries=" + Object.entries(makeSubject((s) => { delete s.c; })).map((e) => e[0] + ":" + e[1]).join("|"));
console.log("del_values=" + Object.values(makeSubject((s) => { delete s.c; })).join("|"));
console.log("del_keys=" + Object.keys(makeSubject((s) => { delete s.c; })).join("|"));
console.log("del_json=" + JSON.stringify(makeSubject((s) => { delete s.c; })));
console.log("del_assign=" + JSON.stringify(Object.assign({}, makeSubject((s) => { delete s.c; }))));
console.log("del_spread=" + JSON.stringify({ ...makeSubject((s) => { delete s.c; }) }));
console.log("hide_later=" + Object.entries(makeSubject((s) => { Object.defineProperty(s, "c", { enumerable: false }); })).map((e) => e[0]).join("|"));
console.log("add_entries=" + Object.entries(makeSubject((s) => { s.zz = "NEW"; })).map((e) => e[0]).join("|"));
const addSubject = makeSubject((s) => { s.zz = "NEW"; });
Object.entries(addSubject);
console.log("add_after=" + Object.keys(addSubject).join("|"));
console.log("earlier_del=" + Object.entries(makeSubject((s) => { delete s.a; })).map((e) => e[0] + ":" + e[1]).join("|"));

// for..in is LIVE: a key deleted before it is reached is not visited; an added key is not guaranteed
const live: any = { a: 1, b: 2, c: 3, d: 4 };
const visited: string[] = [];
for (const k in live) {
  visited.push(k);
  if (k === "b") delete live.c;
}
console.log("forin_live=" + visited.join("|"));

const mutating: any = { a: 1, b: 2, c: 3 };
const mutVisited: string[] = [];
for (const k in mutating) {
  mutVisited.push(k);
  Reflect.deleteProperty(mutating, "c");
}
console.log("forin_delete_later=" + mutVisited.join("|"));

// ---------- 4. Non-enumerable shadowing across the prototype chain ----------
const grand: any = { a: "grand-a", b: "grand-b", c: "grand-c", d: "grand-d" };
const mid: any = Object.create(grand);
mid.b = "mid-b";
Object.defineProperty(mid, "c", { value: "mid-c", enumerable: false, configurable: true, writable: true });
const leaf: any = Object.create(mid);
leaf.a = "leaf-a";
const seen: string[] = [];
for (const k in leaf) seen.push(k + ":" + leaf[k]);
console.log("shadow_forin=" + seen.join("|"));
console.log("shadow_c=" + leaf.c + ",in=" + ("c" in leaf) + ",d_seen=" + (seen.join("|").indexOf("d:") >= 0));
const midSeen: string[] = [];
for (const k in mid) midSeen.push(k);
console.log("shadow_mid=" + midSeen.join("|"));
Object.defineProperty(mid, "c", { enumerable: true });
const seen2: string[] = [];
for (const k in leaf) seen2.push(k + ":" + leaf[k]);
console.log("shadow_reenabled=" + seen2.join("|"));
console.log("shadow_keys=" + Object.keys(leaf).join("|") + ",mid_keys=" + Object.keys(mid).join("|"));

// hiding an inherited enumerable name on the prototype hides it everywhere
const hideProto: any = { visible: 1, gone: 2 };
Object.defineProperty(hideProto, "gone", { enumerable: false });
const hideChild: any = Object.create(hideProto);
console.log("proto_hidden_forin=" + forinOf(hideChild) + ",read=" + hideChild.gone);

// an enumerable own name reported once, from the child, with the child's value
const dupProto: any = { dup: "P" };
const dupChild: any = Object.create(dupProto);
dupChild.dup = "C";
const dupSeen: string[] = [];
for (const k in dupChild) dupSeen.push(k + "=" + dupChild[k]);
console.log("dup_forin=" + dupSeen.join("|") + ",count=" + dupSeen.length);

// a getter on the prototype is enumerated by name and read through the getter
const accProto: any = {};
Object.defineProperty(accProto, "acc", { get() { return "from-getter"; }, enumerable: true, configurable: true });
const accChild: any = Object.create(accProto);
accChild.own = "o";
const accSeen: string[] = [];
for (const k in accChild) accSeen.push(k + "=" + accChild[k]);
console.log("proto_getter_forin=" + accSeen.join("|"));

// removing the prototype mid-walk stops the inherited part
const cutProto: any = { pa: 1, pb: 2 };
const cut: any = Object.create(cutProto);
cut.own = 0;
const cutSeen: string[] = [];
for (const k in cut) {
  cutSeen.push(k);
  if (k === "own") Object.setPrototypeOf(cut, null);
}
console.log("cut_forin=" + cutSeen.join("|"));

// for..in over arrays, strings, holes, primitives and nullish values
const arrIn: any = [10, 20];
arrIn.extra = 30;
console.log("forin_arr=" + (() => { const out: string[] = []; for (const k in arrIn) out.push(typeof k + ":" + k); return out.join("|"); })());
const holed: any = [1, , 3];
console.log("forin_hole=" + forinOf(holed));
console.log("forin_string=" + forinOf("ab"));
const numIn: string[] = [];
for (const k in 7 as any) numIn.push(k);
const nullIn: string[] = [];
for (const k in null as any) nullIn.push(k);
for (const k in undefined as any) nullIn.push(k);
console.log("forin_prims=" + numIn.length + ":" + nullIn.length);
const keyTypes: string[] = [];
for (const k in { 1: "a", x: "b" } as any) keyTypes.push(typeof k);
console.log("forin_keytypes=" + keyTypes.join("|"));
const sym = Symbol("hidden");
const withSym: any = { visible: 1 };
withSym[sym] = 2;
console.log("forin_sym=" + forinOf(withSym) + ",own=" + Reflect.ownKeys(withSym).map(String).join("|"));

// ---------- 5. Snapshot-style readers on primitives and odd receivers ----------
console.log("str_keys=" + Object.keys("hey" as any).join("|") + ",values=" + Object.values("hey" as any).join("|"));
console.log("str_entries=" + Object.entries("hey" as any).map((e) => e[0] + ":" + e[1]).join("|") + ",empty=" + Object.keys("" as any).length);
console.log("num_keys=" + Object.keys(7 as any).length + ",bool_values=" + Object.values(true as any).length);
try {
  console.log("null_keys=" + Object.keys(null as any).length);
} catch (e: any) {
  console.log("null_keys=throw:" + e.constructor.name);
}
const arrayLike: any = { length: 3, 2: "two", 0: "zero", 1: "one" };
console.log("array_like=" + Object.entries(arrayLike).map((e) => e[0] + ":" + e[1]).join("|"));
const sparseOut: any = [1, , 3];
sparseOut.tail = "T";
console.log("sparse_out_keys=" + Object.keys(sparseOut).join("|") + ",values=" + Object.values(sparseOut).join("|"));
const boom: any = {};
Object.defineProperty(boom, "ok", { get() { return 1; }, enumerable: true });
Object.defineProperty(boom, "bad", { get(): number { throw new RangeError("b"); }, enumerable: true });
try {
  console.log("boom_entries=" + Object.entries(boom).length);
} catch (e: any) {
  console.log("boom_entries=throw:" + e.constructor.name);
}
console.log("boom_keys=" + Object.keys(boom).join("|"));

// ---------- 6. Property key coercion: ToPropertyKey runs toString first (string hint) ----------
const keyLog: string[] = [];
const keyObj: any = {
  toString() { keyLog.push("toString"); return "7"; },
  valueOf() { keyLog.push("valueOf"); return 8; },
};
const kobj: any = { 7: "seven", true: "bool", "[object Object]": "plain" };
console.log("key_obj=" + kobj[keyObj] + ",log=" + keyLog.join(","));
console.log("key_bool=" + kobj[true as any] + ",key_plain=" + kobj[{} as any]);
const karr: any[] = ["zero", "one", "two"];
karr[keyObj] = "seven-index";
console.log("key_arr=" + karr.length + ":" + karr[7] + ":" + Object.keys(karr).join("|") + ",log=" + keyLog.join(","));
