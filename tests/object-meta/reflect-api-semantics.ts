// The Reflect namespace: its shape (plain object, not callable, thirteen methods with fixed
// name/length/attributes, none constructable), boolean-vs-throw contracts against the Object
// statics, Reflect.apply / Reflect.construct argument lists and newTarget, the receiver rules
// of Reflect.set / Reflect.get, and the basic CRUD-style operations.
export {};

function attempt(label: string, fn: () => unknown): void {
  try {
    console.log(label + "=" + String(fn()));
  } catch (e: any) {
    console.log(label + "=throw:" + e.constructor.name);
  }
}

// ---------- 1. Namespace shape ----------
console.log("typeof=" + typeof Reflect + ",tag=" + Object.prototype.toString.call(Reflect));
console.log("proto_is_object=" + (Object.getPrototypeOf(Reflect) === Object.prototype) + ",extensible=" + Object.isExtensible(Reflect));
attempt("reflect_call", () => (Reflect as any)());
attempt("reflect_new", () => new (Reflect as any)());
console.log("reflect_ctor_prop=" + ((Reflect as any).prototype === undefined));
console.log("reflect_tag_value=" + (Reflect as any)[Symbol.toStringTag]);
const names: string[] = Object.getOwnPropertyNames(Reflect).sort();
console.log("reflect_names=" + names.join("|") + ",count=" + names.length);
console.log("reflect_symbols=" + Object.getOwnPropertySymbols(Reflect).map(String).join("|") + ",enumerable=" + Object.keys(Reflect).length);
for (const n of names) {
  const fn: any = (Reflect as any)[n];
  const d = Object.getOwnPropertyDescriptor(Reflect, n) as any;
  console.log("member:" + n + "=" + typeof fn + ",name=" + fn.name + ",len=" + fn.length +
    ",w=" + d.writable + ",e=" + d.enumerable + ",c=" + d.configurable +
    ",fnproto=" + (Object.getPrototypeOf(fn) === Function.prototype) + ",hasPrototype=" + ("prototype" in fn));
}
for (const n of names) {
  try {
    new ((Reflect as any)[n])({}, "k");
    console.log("new:" + n + "=ok");
  } catch (e: any) {
    console.log("new:" + n + "=throw:" + e.constructor.name);
  }
}
console.log("no_twins=" + ["keys", "values", "entries", "assign", "create", "freeze", "seal", "is"]
  .map((n) => n + ":" + ((Reflect as any)[n] === undefined)).join("|"));

// ---------- 2. Primitives: Reflect refuses, the Object statics coerce ----------
attempt("get_prim", () => Reflect.get(1 as any, "toFixed"));
attempt("set_prim", () => Reflect.set("s" as any, "0", "x"));
attempt("has_prim", () => Reflect.has(1 as any, "x"));
attempt("ownKeys_prim", () => Reflect.ownKeys("ab" as any).join("|"));
attempt("getproto_prim", () => Reflect.getPrototypeOf("ab" as any));
attempt("gopd_prim", () => Reflect.getOwnPropertyDescriptor("ab" as any, "0"));
attempt("delete_prim", () => Reflect.deleteProperty(1 as any, "x"));
attempt("isext_prim", () => Reflect.isExtensible(1 as any));
attempt("prevent_prim", () => Reflect.preventExtensions(1 as any));
attempt("define_prim", () => Reflect.defineProperty(1 as any, "x", { value: 1 }));
attempt("setproto_prim", () => Reflect.setPrototypeOf(1 as any, null));
attempt("apply_nonfn", () => Reflect.apply({} as any, null, []));
attempt("construct_nonfn", () => Reflect.construct({} as any, []));
console.log("object_keys_str=" + Object.keys("ab").join("|") + ",getproto_str=" + (Object.getPrototypeOf("ab") === String.prototype));
console.log("object_isext_num=" + Object.isExtensible(1 as any));
attempt("object_keys_null", () => Object.keys(null as any).length);
attempt("object_freeze_null", () => Object.freeze(null as any));
console.log("object_freeze_num=" + (Object.freeze(7 as any) as any));
console.log("object_gopn_str=" + Object.getOwnPropertyNames("ab" as any).join("|"));

// ---------- 3. Boolean answers versus throws ----------
const frozen: any = Object.freeze({ a: 1 });
attempt("obj_define_frozen", () => Object.defineProperty(frozen, "b", { value: 1 }));
console.log("ref_define_frozen=" + Reflect.defineProperty(frozen, "b", { value: 1 }));
const defTarget: any = {};
console.log("obj_define_returns_target=" + (Object.defineProperty(defTarget, "x", { value: 1 }) === defTarget));
console.log("ref_define_returns_bool=" + Reflect.defineProperty({} as any, "x", { value: 1 }));
const nonExt: any = Object.preventExtensions({});
attempt("obj_setproto_locked", () => Object.setPrototypeOf(nonExt, { p: 1 }));
console.log("ref_setproto_locked=" + Reflect.setPrototypeOf(nonExt, { p: 1 }) + ",same=" + Reflect.setPrototypeOf(nonExt, Object.prototype));
console.log("ref_delete_frozen=" + Reflect.deleteProperty(frozen, "a") + ",still=" + frozen.a + ",missing=" + Reflect.deleteProperty(frozen, "zzz"));
attempt("obj_define_baddesc", () => Object.defineProperty({} as any, "x", 1 as any));
attempt("ref_define_baddesc", () => Reflect.defineProperty({} as any, "x", 1 as any));
attempt("obj_define_both", () => Object.defineProperty({} as any, "x", { value: 1, get() { return 2; } } as any));
attempt("ref_define_both", () => Reflect.defineProperty({} as any, "x", { value: 1, get() { return 2; } } as any));

// Reflect.get / Reflect.has coerce keys exactly like bracket access
const keyed: any = { "1": "one", "true": "T", "[object Object]": "OBJ", undefined: "U" };
console.log("ref_get_keys=" + Reflect.get(keyed, 1 as any) + "," + Reflect.get(keyed, true as any) + "," + Reflect.get(keyed, {} as any) + "," + Reflect.get(keyed, undefined as any));
console.log("ref_has_undef=" + Reflect.has(keyed, undefined as any));
const symK = Symbol("k");
const symHolder: any = { [symK]: "S" };
console.log("ref_get_sym=" + Reflect.get(symHolder, symK) + ",num_key=" + Reflect.get({ 3: "three" } as any, 3 as any));

// ownKeys is the only one that reports strings and symbols together
const both: any = { str: 1, [symK]: 2 };
console.log("ref_ownkeys=" + Reflect.ownKeys(both).map(String).join("|"));
console.log("obj_names=" + Object.getOwnPropertyNames(both).join("|") + ",symbols=" + Object.getOwnPropertySymbols(both).map(String).join("|"));

// ---------- 4. Basic operations ----------
const basic: any = { a: 1, b: 2 };
console.log("get=" + Reflect.get(basic, "a") + ",set=" + Reflect.set(basic, "c", 3) + ",c=" + basic.c);
console.log("has=" + Reflect.has(basic, "a") + "," + Reflect.has(basic, "d") + ",delete=" + Reflect.deleteProperty(basic, "b") + ",has_b=" + Reflect.has(basic, "b"));
console.log("ownKeys=" + Reflect.ownKeys(basic).sort().join(","));
console.log("getproto_arr=" + (Reflect.getPrototypeOf([]) === Array.prototype));
console.log("reflect_set_proto=" + Reflect.setPrototypeOf(basic, null) + ",proto=" + Reflect.getPrototypeOf(basic));

const target: any = { val: 10 };
console.log("rs_get=" + Reflect.get(target, "val"));
Reflect.set(target, "val", 99);
console.log("rs_after=" + target.val);
Reflect.deleteProperty(target, "val");
console.log("rs_in=" + ("val" in target));

const idObj: any = {};
Reflect.defineProperty(idObj, "id", { value: 42, writable: false, enumerable: true });
console.log("define_id=" + idObj.id + ",write=" + Reflect.set(idObj, "id", 1) + ",v=" + idObj.id);
const dsc = Reflect.getOwnPropertyDescriptor(idObj, "id") as any;
console.log("define_desc=" + dsc.value + ",w=" + dsc.writable + ",e=" + dsc.enumerable);
console.log("gopd_missing=" + Reflect.getOwnPropertyDescriptor(idObj, "b"));

// ---------- 5. Reflect.apply and Reflect.construct ----------
function add(this: any, a: number, b: number): number { return a + b; }
console.log("apply_add=" + Reflect.apply(add, null, [3, 4]));
console.log("apply_max=" + Reflect.apply(Math.max, null, [1, 5, 3, 9, 2]));

function sum(this: any, ...args: number[]): string {
  return "this=" + this.tag + ",n=" + args.length + ",sum=" + args.reduce((x, y) => x + y, 0);
}
const T: any = { tag: "T" };
console.log("arr_args=" + Reflect.apply(sum, T, [1, 2, 3]));
console.log("arraylike=" + Reflect.apply(sum, T, { length: 2, 0: 10, 1: 20 } as any));
console.log("holes=" + Reflect.apply(sum, T, { length: 3, 0: 1 } as any));
console.log("len_coerced=" + Reflect.apply(sum, T, { length: "2", 0: 5, 1: 6 } as any));
console.log("len_float=" + Reflect.apply(sum, T, { length: 2.9, 0: 5, 1: 6 } as any));
console.log("len_neg=" + Reflect.apply(sum, T, { length: -1, 0: 5 } as any));
console.log("len_missing=" + Reflect.apply(sum, T, {} as any));
console.log("set_args=" + Reflect.apply(sum, T, new Set([1, 2, 3]) as any));
attempt("args_string", () => Reflect.apply(sum, null, "12" as any));
attempt("args_number", () => Reflect.apply(sum, null, 3 as any));
attempt("args_null", () => Reflect.apply(sum, null, null as any));
attempt("args_missing", () => (Reflect.apply as any)(sum, null));
const probe: any = { tag: "PROBE" };
let sawSame = false;
function identity(this: any): string { sawSame = this === probe; return "tag=" + this.tag; }
console.log("this_identity=" + Reflect.apply(identity, probe, []) + ",same=" + sawSame);
const reads: string[] = [];
const argSource: any = {};
Object.defineProperty(argSource, "length", { get() { reads.push("length"); return 3; } });
Object.defineProperty(argSource, "2", { get() { reads.push("2"); return 30; } });
Object.defineProperty(argSource, "0", { get() { reads.push("0"); return 10; } });
Object.defineProperty(argSource, "1", { get() { reads.push("1"); return 20; } });
console.log("ordered=" + Reflect.apply(sum, T, argSource) + ",read_order=" + reads.join("|"));

class Base {
  constructor() {
    (this as any).from = "Base";
    (this as any).nt = new.target === undefined ? "none" : new.target.name;
  }
}
class Other { }
(Other as any).prototype.marker = "OTHER";
const viaBase: any = Reflect.construct(Base, []);
console.log("construct_plain=" + viaBase.from + "," + viaBase.nt + ",proto=" + (Object.getPrototypeOf(viaBase) === Base.prototype));
const viaOther: any = Reflect.construct(Base, [], Other);
console.log("construct_nt=" + viaOther.from + "," + viaOther.nt + ",proto=" + (Object.getPrototypeOf(viaOther) === Other.prototype));
console.log("construct_marker=" + viaOther.marker + ",instanceof=" + (viaOther instanceof Other) + "," + (viaOther instanceof Base));
attempt("nt_arrow", () => Reflect.construct(Base, [], (() => 1) as any));
attempt("nt_object", () => Reflect.construct(Base, [], {} as any));
attempt("nt_method", () => Reflect.construct(Base, [], ({ m() { return 1; } }).m as any));

function Bindable(this: any): void { (this as any).b = 1; }
(Bindable as any).prototype.tagged = "BINDABLE";
const viaBound: any = Reflect.construct(Base, [], Bindable.bind(null));
console.log("bound_nt=" + viaBound.tagged + ",instanceof=" + (viaBound instanceof Bindable));
function NoProto(this: any): void { /* noop */ }
(NoProto as any).prototype = 7;
const viaNoProto: any = Reflect.construct(Base, [], NoProto);
console.log("noproto_fallback=" + (Object.getPrototypeOf(viaNoProto) === Object.prototype));
attempt("construct_arrow", () => Reflect.construct((() => 1) as any, []));
attempt("construct_method", () => Reflect.construct(({ m() { return 1; } }).m as any, []));
attempt("construct_symbol", () => Reflect.construct(Symbol as any, []));
attempt("construct_parseint", () => Reflect.construct(parseInt as any, ["7"]));
console.log("construct_array=" + JSON.stringify(Reflect.construct(Array, [1, 2, 3])) + ",len=" + (Reflect.construct(Array, [3]) as any[]).length);
console.log("construct_map=" + (Reflect.construct(Map, [[["k", 1]]]) as Map<string, number>).get("k") + ",object=" + (Reflect.construct(Object, [5]) as any).valueOf());
console.log("construct_arraylike=" + JSON.stringify(Reflect.construct(Array, { length: 2, 0: "a", 1: "b" } as any)));
attempt("construct_string_args", () => Reflect.construct(Array, "ab" as any));

// ---------- 6. Reflect.set with a receiver other than the target ----------
const rt: any = { a: 1 };
const rr: any = {};
console.log("recv_set=" + Reflect.set(rt, "a", 2, rr) + ",target=" + rt.a + ",receiver=" + rr.a);
const rrd = Object.getOwnPropertyDescriptor(rr, "a") as any;
console.log("recv_desc=w=" + rrd.writable + ",e=" + rrd.enumerable + ",c=" + rrd.configurable);

const hiddenT: any = {};
Object.defineProperty(hiddenT, "hidden", { value: 1, writable: true, enumerable: false, configurable: true });
const hiddenR: any = {};
console.log("recv_hidden=" + Reflect.set(hiddenT, "hidden", 5, hiddenR));
const hd = Object.getOwnPropertyDescriptor(hiddenR, "hidden") as any;
console.log("recv_hidden_desc=e=" + hd.enumerable + ",c=" + hd.configurable + ",v=" + hd.value);

const lockedR: any = {};
Object.defineProperty(lockedR, "a", { value: "locked", writable: false, configurable: false });
console.log("recv_locked=" + Reflect.set(rt, "a", 9, lockedR) + ",value=" + lockedR.a);

const roT: any = {};
Object.defineProperty(roT, "ro", { value: 1, writable: false, configurable: false });
const roR: any = {};
console.log("target_ro=" + Reflect.set(roT, "ro", 2, roR) + ",has=" + ("ro" in roR));

const accR: any = {};
Object.defineProperty(accR, "a", { get() { return "g"; }, configurable: true });
console.log("recv_accessor=" + Reflect.set(rt, "a", 3, accR) + ",value=" + accR.a);

let where = "none";
const setT: any = {};
Object.defineProperty(setT, "s", {
  set(v: string) { where = v + "@" + ((this as any).tag || "?"); },
  get() { return "target-get@" + ((this as any).tag || "?"); },
  configurable: true,
});
setT.tag = "TARGET";
const setR: any = { tag: "RECEIVER" };
console.log("setter_recv=" + Reflect.set(setT, "s", "v", setR) + ",where=" + where);
console.log("getter_recv=" + Reflect.get(setT, "s", setR) + ",own=" + Object.prototype.hasOwnProperty.call(setR, "s"));

console.log("recv_prim=" + Reflect.set(rt, "a", 4, 7 as any) + "," + Reflect.set(rt, "a", 4, "s" as any));
const extR: any = { a: 0 };
Object.preventExtensions(extR);
console.log("recv_nonext_existing=" + Reflect.set(rt, "a", 8, extR) + ",a=" + extR.a + ",new=" + Reflect.set(rt, "zzz", 8, extR));
const defT: any = { v: 1 };
console.log("default_receiver=" + Reflect.set(defT, "v", 2) + ",v=" + defT.v);

// inherited data properties: a write creates an OWN property unless the inherited one is read-only
const inhP: any = { inh: "p" };
const inhC: any = Object.create(inhP);
console.log("inh_set=" + Reflect.set(inhC, "inh", "c") + ",own=" + Object.prototype.hasOwnProperty.call(inhC, "inh") + ",proto=" + inhP.inh);
const roP: any = {};
Object.defineProperty(roP, "ro", { value: "p", writable: false });
const roC: any = Object.create(roP);
console.log("inh_ro_set=" + Reflect.set(roC, "ro", "c") + ",own=" + Object.prototype.hasOwnProperty.call(roC, "ro") + ",read=" + roC.ro);

// ---------- 7. Reflect.setPrototypeOf cycles ----------
const ca: any = {};
const cb: any = {};
console.log("cycle_1=" + Reflect.setPrototypeOf(ca, cb) + ",linked=" + (Object.getPrototypeOf(ca) === cb));
console.log("cycle_2=" + Reflect.setPrototypeOf(cb, ca) + ",linked=" + (Object.getPrototypeOf(cb) === ca));
