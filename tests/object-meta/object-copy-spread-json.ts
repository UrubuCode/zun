// Copying and serialising objects: Object.assign (getter/setter order, partial abort, skipped
// sources), object spread (getter-to-data, override order, nullish and string sources),
// Object.fromEntries (Map, arrays, opaque sources, duplicate and numeric keys), destructuring
// defaults with side-effect order, and JSON.stringify (replacer, indent, toJSON, special values).
export {};

function attempt(label: string, fn: () => unknown): void {
  try {
    console.log(label + "=ok:" + String(fn()));
  } catch (e: any) {
    console.log(label + "=throw:" + e.constructor.name);
  }
}

// ---------- 1. Object.assign ----------
const tgt: any = { a: 1, b: 2 };
const src1: any = { b: 3, c: 4 };
const result = Object.assign(tgt, src1);
console.log("assign_same=" + (result === tgt) + ",a=" + result.a + ",b=" + result.b + ",c=" + result.c);
console.log("assign_multi=" + JSON.stringify(Object.assign({}, { x: 1 }, { y: 2 }, { z: 3 })));
console.log("assign_override=" + JSON.stringify(Object.assign({}, { a: 1, b: 2 }, { b: 3, c: 4 }, { a: 5 })));

// getters on the source run in enumeration order; setters on the target run in the same order
const log1: string[] = [];
const sourceG: any = {
  get a() { log1.push("get-a"); return 1; },
  get b() { log1.push("get-b"); return 2; },
};
const targetS: any = { set a(v: number) { log1.push("set-a:" + v); } };
const out1 = Object.assign(targetS, sourceG, { a: 3 });
console.log("assign_order=" + log1.join("|") + ",same=" + (out1 === targetS) + ",b=" + out1.b);

// a source getter that throws aborts the copy; earlier keys stay copied
const order2: string[] = [];
const boomSrc: any = {};
Object.defineProperty(boomSrc, "a", { get() { order2.push("a"); return 1; }, enumerable: true });
Object.defineProperty(boomSrc, "b", { get() { order2.push("b"); return 2; }, enumerable: true });
Object.defineProperty(boomSrc, "boom", { get() { order2.push("boom"); throw new RangeError("x"); }, enumerable: true });
Object.defineProperty(boomSrc, "c", { get() { order2.push("c"); return 3; }, enumerable: true });
const dest2: any = {};
attempt("assign_src_throw", () => Object.assign(dest2, boomSrc));
console.log("assign_partial_order=" + order2.join("|") + ",keys=" + Object.keys(dest2).join("|"));

// a target setter that throws stops at that key
const order3: string[] = [];
const dest3: any = { plain: 0 };
Object.defineProperty(dest3, "bad", { set() { order3.push("set-bad"); throw new TypeError("y"); }, get() { return "G"; }, enumerable: true, configurable: true });
attempt("assign_dest_throw", () => Object.assign(dest3, { first: 1, bad: 2, last: 3 }));
console.log("assign_dest_log=" + order3.join("|") + ",keys=" + Object.keys(dest3).join("|") + ",first=" + dest3.first + ",last=" + dest3.last);

// a later source is never read once an earlier one threw
const readLog: string[] = [];
const s1: any = { get ok() { readLog.push("s1.ok"); return 1; }, get bad() { readLog.push("s1.bad"); throw new Error("z"); } };
const s2: any = { get never() { readLog.push("s2.never"); return 2; } };
attempt("assign_multi_throw", () => Object.assign({}, s1, s2));
console.log("assign_readlog=" + readLog.join("|"));

// nullish sources are skipped; nullish targets are errors
console.log("assign_nullish=" + JSON.stringify(Object.assign({ k: 1 }, null, undefined, { j: 2 })));
attempt("assign_null_target", () => Object.assign(null as any, { a: 1 }));
attempt("assign_undef_target", () => Object.assign(undefined as any, { a: 1 }));

// primitive sources are wrapped; only strings contribute own enumerable keys
console.log("assign_str_src=" + JSON.stringify(Object.assign({}, "ab")));
console.log("assign_num_src=" + JSON.stringify(Object.assign({}, 42)) + ",bool=" + JSON.stringify(Object.assign({}, true)));
console.log("assign_sym_src=" + JSON.stringify(Object.assign({}, Symbol("s") as any)));

// inherited and non-enumerable source props are not copied; enumerable symbols are
const protoS: any = { inherited: "no" };
const mixed: any = Object.create(protoS);
mixed.visible = "yes";
Object.defineProperty(mixed, "hidden", { value: "no", enumerable: false });
console.log("assign_filtered=" + JSON.stringify(Object.assign({}, mixed)));
const se = Symbol("enum");
const sh = Symbol("hidden");
const symSrc: any = { [se]: 1 };
Object.defineProperty(symSrc, sh, { value: 2, enumerable: false });
const symDest: any = Object.assign({}, symSrc);
console.log("assign_symbols=" + (symDest[se] === 1) + ",hidden_skipped=" + (symDest[sh] === undefined) + ",keys=" + Object.getOwnPropertySymbols(symDest).map(String).join("|"));

// Object.assign uses [[Set]]: a non-writable own target property throws; frozen with nothing to copy is fine
const locked: any = {};
Object.defineProperty(locked, "ro", { value: "keep", writable: false, enumerable: true, configurable: false });
attempt("assign_locked", () => Object.assign(locked, { ro: "new" }));
console.log("assign_locked_value=" + locked.ro);
const frozenT: any = Object.freeze({ a: 1 });
attempt("assign_frozen_empty", () => Object.assign(frozenT, {}));
attempt("assign_frozen_nonempty", () => Object.assign(frozenT, { b: 2 }));

// Object.assign onto a prototype setter routes through it (see the setter fixture)
// ---------- 2. Object spread ----------
const a1: any = { x: 1, y: 2 };
const b1: any = { y: 3, z: 4 };
console.log("spread_override=" + JSON.stringify({ ...a1, y: 99, ...b1 }) + ",keys=" + Object.keys({ ...a1, y: 99, ...b1 }).join(","));

const spreadLog: string[] = [];
const getterSrc: any = {
  get p() { spreadLog.push("p"); return 1; },
  get q() { spreadLog.push("q"); return 2; },
};
const spreadOut: any = { first: 0, ...getterSrc, last: 3 };
console.log("spread_order=" + Object.keys(spreadOut).join(",") + ",log=" + spreadLog.join(","));

// spread turns a getter into a data property on the copy
let calls = 0;
const liveSrc = {
  get a() { calls++; return 1; },
  get b() { calls++; return 2; },
};
const copy: any = { ...liveSrc };
console.log("spread_values=" + copy.a + "," + copy.b + ",calls=" + calls);
const cd = Object.getOwnPropertyDescriptor(copy, "a") as any;
console.log("spread_desc=get_undef:" + (cd.get === undefined) + ",v=" + cd.value + ",w=" + cd.writable + ",e=" + cd.enumerable + ",c=" + cd.configurable);

// spread skips non-enumerable and inherited props, copies enumerable symbols
const protoSp: any = { inh: 1 };
const spSrc: any = Object.create(protoSp);
spSrc.vis = 2;
Object.defineProperty(spSrc, "hid", { value: 3, enumerable: false });
const spSym = Symbol("e");
spSrc[spSym] = 4;
const spCopy: any = { ...spSrc };
console.log("spread_filter=" + JSON.stringify(spCopy) + ",sym=" + spCopy[spSym] + ",proto=" + (Object.getPrototypeOf(spCopy) === Object.prototype));

// nullish and string sources
console.log("spread_nullish=" + JSON.stringify({ ...null, ...undefined, ...{ a: 1 }, ...{} }));
console.log("spread_string=" + JSON.stringify({ ..."ab" }));
console.log("spread_number=" + JSON.stringify({ ...(5 as any) }) + ",bool=" + JSON.stringify({ ...(true as any) }));

// spread into an array-like object does not inherit array-ness
const arrSpread: any = { ...[7, 8] };
console.log("spread_array=" + JSON.stringify(arrSpread) + ",isArray=" + Array.isArray(arrSpread));

// ---------- 3. Object.fromEntries ----------
const m = new Map<string, number>([["a", 1], ["b", 2]]);
console.log("fe_map=" + JSON.stringify(Object.fromEntries(m)));
console.log("fe_array=" + JSON.stringify(Object.fromEntries([["x", 7], ["y", 8]])));
console.log("fe_empty=" + JSON.stringify(Object.fromEntries(new Map())) + "," + JSON.stringify(Object.fromEntries([])));
// the source is opaque at the call site: the function does not know it is a Map
function viaParam(src: any): any { return Object.fromEntries(src); }
console.log("fe_opaque_map=" + JSON.stringify(viaParam(m)) + ",opaque_array=" + JSON.stringify(viaParam([["y", 7]])));
console.log("fe_roundtrip=" + JSON.stringify(Object.fromEntries(Object.entries({ p: 1, q: 2 }))));
console.log("fe_numeric_key=" + JSON.stringify(Object.fromEntries([[1, "um"]])));
// the last duplicate wins, but the key keeps its FIRST position
const dupFe: any = Object.fromEntries([["a", 1], ["b", 2], ["a", 3], ["2", "x"], ["1", "y"]]);
console.log("fe_dup=" + JSON.stringify(dupFe) + ",keys=" + Object.keys(dupFe).join(","));
const feEntries = Object.entries({ c: 3, a: 1, b: 2 });
console.log("fe_entries=" + feEntries.map(([kk, v]) => kk + "=" + v).join(","));
console.log("fe_entries_keys_rt=" + Object.keys(Object.fromEntries(feEntries)).join(","));
attempt("fe_not_iterable", () => Object.fromEntries(5 as any));
attempt("fe_bad_entry", () => Object.fromEntries([1] as any));

// ---------- 4. Destructuring defaults and side-effect order ----------
const dOrder: string[] = [];
function dflt(tag: string, val: unknown): unknown { dOrder.push(tag); return val; }
const dObj: any = { x: 10, z: undefined };
const { x = dflt("x", -1), y = dflt("y", -2), z = dflt("z", -3) } = dObj;
console.log("destr_obj=" + x + "," + y + "," + z + ",order=" + dOrder.join("|"));
dOrder.length = 0;
const [p = dflt("p", 100), q = dflt("q", 200), r = dflt("r", 300)] = [5, undefined] as any[];
console.log("destr_arr=" + p + "," + q + "," + r + ",order=" + dOrder.join("|"));

const trace: string[] = [];
function tn(name: string): number { trace.push(name); return name.length; }
const config: any = { a: { b: 1 } };
const { a: { b = tn("b"), c = tn("c") } = {}, d: { e = tn("e") } = {} } = config;
console.log("destr_nested=" + b + "," + c + "," + e + ",trace=" + trace.join("|"));
trace.length = 0;
const { m: mv = tn("m1"), n: { o = tn("o") } = (trace.push("default-n"), {}) } = { m: 5 } as any;
console.log("destr_nested2=" + mv + "," + o + ",trace=" + trace.join("|"));

// ---------- 5. JSON.stringify ----------
console.log("json_special_obj=" + JSON.stringify({ a: undefined, b: function () { /* noop */ }, c: 1 }));
console.log("json_special_arr=" + JSON.stringify([undefined, function () { /* noop */ }, 1]));
console.log("json_nonfinite=" + JSON.stringify({ a: NaN, b: Infinity, c: -Infinity }) + "," + JSON.stringify([NaN, Infinity]));
console.log("json_top=" + JSON.stringify(undefined) + "," + JSON.stringify(function () { /* noop */ }) + "," + JSON.stringify(NaN) + "," + JSON.stringify(null));
console.log("json_null_prop=" + JSON.stringify({ a: null }));

const repObj = { b: 1, a: 2, "2": 3, "1": 4 };
console.log("json_list=" + JSON.stringify(repObj, ["a", "1", "missing"]));
console.log("json_indent=" + JSON.stringify({ k: [1, 2] }, null, 2).split("\n").join("|"));
console.log("json_tab=" + JSON.stringify({ a: [1, 2] }, null, "\t").split("\n").join("|"));
const withSecret = { x: 1, secret: "hide", y: 2 };
console.log("json_replacer_drop=" + JSON.stringify(withSecret, (k, v) => (k === "secret" ? undefined : v)));
console.log("json_replacer_map=" + JSON.stringify({ a: 1, b: 2, c: 3 }, (_k, v) => (typeof v === "number" ? v * 10 : v)));
console.log("json_replacer_order=" + JSON.stringify({ z: 1, 10: 2, 9: 3, y: 4 }));

const hook = {
  value: 42,
  toJSON() { return { wrapped: this.value, tag: "custom" }; },
};
console.log("json_tojson=" + JSON.stringify(hook) + ",nested=" + JSON.stringify({ nested: hook, plain: 1 }));
console.log("json_tojson_arr=" + JSON.stringify([hook, hook]));
console.log("json_tojson_date=" + JSON.stringify({ when: new Date(0) }));
console.log("json_tojson_num=" + JSON.stringify({ toJSON() { return 7; } }));
const tjArr: any = [1, 2, 3];
tjArr.toJSON = function () { return "ARR"; };
console.log("json_tojson_on_arr=" + JSON.stringify({ a: tjArr }));

// null-prototype objects serialise like any other own-key bag
const nullObj: any = Object.create(null);
nullObj.b = 2;
nullObj.a = 1;
nullObj.toString = "own";
console.log("json_nullproto=" + JSON.stringify(nullObj) + ",keys=" + Object.keys(nullObj).join(","));

// JSON.parse creates an own __proto__ key instead of setting the prototype
const parsed: any = JSON.parse('{"__proto__":{"polluted":true},"safe":1}');
console.log("json_parse_proto=" + (Object.getPrototypeOf(parsed) === Object.prototype) + ",own=" + Object.getOwnPropertyNames(parsed).join("|"));
console.log("json_parse_clean=" + (({} as any).polluted === undefined));

// ---------- 6. Object rest with a computed key: the key is evaluated once and excluded ----------
let keyCalls = 0;
const pickKey = () => { keyCalls++; return "b"; };
const restSrc = { a: 1, b: 2, c: 3 };
const { [pickKey()]: picked, ...restOut } = restSrc;
console.log("rest_computed=" + picked + "," + JSON.stringify(restOut) + ",calls=" + keyCalls);
