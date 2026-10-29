export {};
// Proxies as prototypes and as receivers: which object an accessor or a
// forwarded write lands on, how Reflect.get / Reflect.set substitute a receiver,
// what the traps see when a proxy sits in a prototype chain, array and typed
// array targets (which keep their exotic behaviour behind the proxy), nested
// proxy targets, and the getPrototypeOf / setPrototypeOf traps.

function show(label: string, value: unknown): void {
  console.log(label + "=" + String(value));
}

function attempt(label: string, fn: () => unknown): void {
  try {
    show(label, fn());
  } catch (e) {
    console.log(label + "=throw:" + (e as Error).constructor.name);
  }
}

// ---------------------------------------------------------------------------
// 1. A proxy used as a PROTOTYPE: traps fire for lookups that miss on the child,
//    the receiver handed to the traps is the CHILD, and an own property stops
//    the walk before the proxy is reached
// ---------------------------------------------------------------------------
const protoLog: string[] = [];
const protoTarget: any = { inherited: "T" };

function tagOf(v: any): string {
  if (v === protoChild) return "child";
  if (v === protoProxy) return "proxy";
  if (v === protoTarget) return "target";
  return "other";
}

const protoProxy: any = new Proxy(protoTarget, {
  get(t, k, r) { protoLog.push("get:" + String(k) + ":recv=" + tagOf(r)); return Reflect.get(t, k, r); },
  set(t, k, v, r) { protoLog.push("set:" + String(k) + ":recv=" + tagOf(r)); return Reflect.set(t, k, v, r); },
  has(t, k) { protoLog.push("has:" + String(k)); return Reflect.has(t, k); },
  getOwnPropertyDescriptor(t, k) { protoLog.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
  ownKeys(t) { protoLog.push("ownKeys"); return Reflect.ownKeys(t); },
  getPrototypeOf(t) { protoLog.push("getProto"); return Reflect.getPrototypeOf(t); },
});

const protoChild: any = Object.create(protoProxy);
protoChild.own = "C";

function protoRun(label: string, fn: () => void): void {
  protoLog.length = 0;
  fn();
  show(label, protoLog.join(","));
}

protoRun("pc_miss_on_child", () => { void protoChild.inherited; });
protoRun("pc_hit_on_child", () => { void protoChild.own; });
protoRun("pc_absent_everywhere", () => { void protoChild.nowhere; });
protoRun("pc_in_operator", () => { void ("inherited" in protoChild); });
protoRun("pc_in_own", () => { void ("own" in protoChild); });
protoRun("pc_hasOwn_child", () => { Object.hasOwn(protoChild, "inherited"); });
protoRun("pc_keys_child", () => { Object.keys(protoChild); });
protoRun("pc_forin_child", () => { for (const _k in protoChild) { void _k; } });
show("pc_value", protoChild.inherited + ":" + protoChild.own);

const forinNames: string[] = [];
for (const k in protoChild) forinNames.push(k);
show("pc_forin_names", forinNames.join("|"));

// a write that misses on the child reaches the proxy's set trap with the child as
// receiver; Reflect.set then creates the property on the CHILD, not the target
protoLog.length = 0;
show("pc_write_miss", Reflect.set(protoChild, "fresh", "F"));
show("pc_write_log", protoLog.join(","));
show("pc_fresh_on_child", Object.hasOwn(protoChild, "fresh") + ",target=" + Object.hasOwn(protoTarget, "fresh"));

// a set trap on a prototype that reports success claims the write: no own property
const claimTarget: any = {};
const claimProto: any = new Proxy(claimTarget, { set() { return true; } });
const claimChild: any = Object.create(claimProto);
show("pc_claim_set", Reflect.set(claimChild, "k", 1));
show("pc_claim_child_own", Object.getOwnPropertyNames(claimChild).join("|"));
show("pc_claim_target_own", Object.getOwnPropertyNames(claimTarget).join("|"));

// an accessor behind the proxy runs with `this` bound to the child
const accTarget: any = {
  get doubled() { return "this_is_" + accTag(this) + ":" + (this as any).own; },
  set doubled(v: any) { (this as any).stored = "from_" + accTag(this) + ":" + v; },
};
function accTag(v: any): string {
  if (v === accChild) return "accChild";
  if (v === accProxy) return "accProxy";
  if (v === accTarget) return "accTarget";
  return "other";
}
const accProxy: any = new Proxy(accTarget, {
  get(t, k, r) { return Reflect.get(t, k, r); },
  set(t, k, v, r) { return Reflect.set(t, k, v, r); },
});
const accChild: any = Object.create(accProxy);
accChild.own = "C2";
show("acc_get", accChild.doubled);
show("acc_set", Reflect.set(accChild, "doubled", "V"));
show("acc_stored", accChild.stored + ",own_on_child=" + Object.hasOwn(accChild, "stored"));
show("acc_target_clean", Object.getOwnPropertyNames(accTarget).sort().join("|"));

// a proxy deep in a chain still sees the original child as receiver
const deepTarget: any = { deep: "D" };
let deepRecv = "none";
const deepProxy: any = new Proxy(deepTarget, {
  get(t, k, r) { deepRecv = r === grandchild ? "grandchild" : "other"; return Reflect.get(t, k, r); },
});
const middle: any = Object.create(deepProxy);
const grandchild: any = Object.create(middle);
show("deep_value", grandchild.deep);
show("deep_receiver", deepRecv);

// ---------------------------------------------------------------------------
// 2. Receivers: the default receiver of a direct read is the proxy; Reflect.get
//    and Reflect.set can substitute any object; accessors see the substitute
// ---------------------------------------------------------------------------
const recvLog: string[] = [];
const otherRecv: any = { tag: "other" };
const direct: any = new Proxy({ v: 1 }, {
  get(t, k, r) {
    recvLog.push(String(k) + ":" + (r === direct ? "proxy" : r === otherRecv ? "other" : "?"));
    return Reflect.get(t, k, r);
  },
});
void direct.v;
Reflect.get(direct, "v", otherRecv);
show("receivers", recvLog.join(","));

class Reader {
  static tagOf(v: any): string {
    if (v === null) return "null";
    if (typeof v !== "object" && typeof v !== "function") return typeof v + ":" + String(v);
    return String((v as any).tag);
  }
}

const rProto: any = {
  get who() { return "who:" + Reader.tagOf(this); },
  get slot() { return "slot:" + String((this as any).value); },
  plain: "DATA",
};
const rOwner: any = Object.create(rProto);
rOwner.tag = "owner";
rOwner.value = 1;
const substitute: any = { tag: "substitute", value: 2 };

show("rg_default", Reflect.get(rOwner, "who"));
show("rg_explicit_owner", Reflect.get(rOwner, "who", rOwner));
show("rg_substituted", Reflect.get(rOwner, "who", substitute));
show("rg_slot_owner", Reflect.get(rOwner, "slot"));
show("rg_slot_substituted", Reflect.get(rOwner, "slot", substitute));
show("rg_getter_on_proto_direct", Reflect.get(rProto, "who", substitute));
show("rg_data_ignores_receiver", Reflect.get(rOwner, "plain", substitute));
show("rg_own_data_ignores_receiver", Reflect.get(rOwner, "tag", substitute));
show("rg_missing", String(Reflect.get(rOwner, "nope", substitute)));

// a primitive receiver reaches a class-body getter unboxed, since class bodies
// are always strict
class TypeProbe {
  get typeofThis(): string { return typeof this + ":" + String(this); }
}
const typeProbe: any = new TypeProbe();
function describeWith(recv: any): string {
  return Reflect.get(typeProbe, "typeofThis", recv);
}
show("rg_recv_number", describeWith(5));
show("rg_recv_string", describeWith("s"));
show("rg_recv_boolean", describeWith(true));
show("rg_recv_null", describeWith(null));
show("rg_recv_undefined", describeWith(undefined));

// a proxy target receives the receiver verbatim, even an unrelated object or a
// primitive
const seenRecv: string[] = [];
const verbatim: any = new Proxy({ k: "T" }, {
  get(t, k, r) {
    const which = r === verbatim ? "proxy" : r === substitute ? "substitute" : typeof r + ":" + String(r);
    seenRecv.push(String(k) + ":" + which);
    return Reflect.get(t, k, r);
  },
});
Reflect.get(verbatim, "k");
Reflect.get(verbatim, "k", substitute);
Reflect.get(verbatim, "k", 7 as any);
show("proxy_receivers_verbatim", seenRecv.join("|"));

// forwarding the receiver keeps an inherited getter bound to the ORIGINAL one;
// dropping it changes which object the getter reads
const computedBase: any = { get computed() { return "computed:" + String((this as any).value); } };
const withProto: any = Object.create(computedBase);
withProto.value = "target";
const forwarding: any = new Proxy(withProto, { get(t, k, r) { return Reflect.get(t, k, r); } });
const dropping: any = new Proxy(withProto, { get(t, k) { return Reflect.get(t, k); } });
show("forward_getter_default", forwarding.computed);
show("forward_getter_substituted", Reflect.get(forwarding, "computed", { value: "sub" }));
show("drop_getter_substituted", Reflect.get(dropping, "computed", { value: "sub" }));

show("rg_array_length", Reflect.get([1, 2, 3], "length"));
show("rg_array_index", Reflect.get([1, 2, 3], "1"));
show("rg_boxed_string_index", Reflect.get(new String("hi"), "0"));
attempt("rg_primitive_target", () => Reflect.get("hi" as any, "0"));

// Reflect.set with a receiver runs the setter with that receiver and creates no
// own property on the owner
const setterLog: string[] = [];
const setterProto: any = { set record(v: any) { setterLog.push(Reader.tagOf(this) + "<-" + v); } };
const setterOwner: any = Object.create(setterProto);
setterOwner.tag = "setterOwner";
show("rs_default", Reflect.set(setterOwner, "record", 1));
show("rs_substituted", Reflect.set(setterOwner, "record", 2, substitute));
show("rs_log", setterLog.join("|"));
show("rs_no_own_created", Object.getOwnPropertyNames(setterOwner).sort().join("|"));

// set through a proxy with a receiver that loops back into the same proxy
const loopTarget: any = { a: 1 };
const loopLog: string[] = [];
const loop: any = new Proxy(loopTarget, {
  set(t, k, v, r) {
    loopLog.push("set:" + String(k) + ":recv_is_proxy=" + (r === loop));
    return Reflect.set(t, k, v, r);
  },
  defineProperty(t, k, d) { loopLog.push("define:" + String(k)); return Reflect.defineProperty(t, k, d); },
  getOwnPropertyDescriptor(t, k) { loopLog.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
});
show("loop_set", Reflect.set(loop, "a", 7));
show("loop_log", loopLog.join(","));
show("loop_target", loopTarget.a);

// a setter reached through a proxy prototype with an accessor on the proxy target
const pSetTarget: any = { set value(v: any) { this._v = v + ":set"; }, get value() { return this._v + ":get"; } };
const pSetProxy: any = new Proxy(pSetTarget, {
  get(t, k, r) { return Reflect.get(t, k, r); },
  set(t, k, v, r) { return Reflect.set(t, k, v, r); },
});
const pSetChild: any = Object.create(pSetProxy);
pSetChild._v = "base";
pSetChild.value = "new";
show("setter_chain_child_v", pSetChild._v);
show("setter_chain_target_has_v", Object.hasOwn(pSetTarget, "_v"));
show("setter_chain_read", pSetChild.value);

// the 416 shape: a prototype accessor with a proxy target that forwards the receiver
const proto416: any = {
  get value() { return this._v + ":get"; },
  set value(v: string) { this._v = v + ":set"; },
};
const target416 = Object.create(proto416);
target416._v = "base";
const proxy416 = new Proxy(target416, {
  get(t, k, r) { return Reflect.get(t, k, r); },
  set(t, k, v, r) { return Reflect.set(t, k, v, r); },
});
proxy416.value = "new";
show("p416_target_v", target416._v);
show("p416_read", proxy416.value);
show("p416_own_v", Object.prototype.hasOwnProperty.call(target416, "_v"));

// ---------------------------------------------------------------------------
// 3. Prototype traps: getPrototypeOf / setPrototypeOf over an EXTENSIBLE target
//    may answer anything (the target is never consulted for the answer)
// ---------------------------------------------------------------------------
const protoA: any = { tag: "A" };
const protoB: any = { tag: "B" };

const openTarget: any = Object.create(protoA);
const lyingProto: any = new Proxy(openTarget, { getPrototypeOf() { return protoB; } });
show("open_lie", Object.getPrototypeOf(lyingProto) === protoB);
show("open_target_intact", Object.getPrototypeOf(openTarget) === protoA);
show("open_lookup_forwards_to_target", lyingProto.tag);
show("open_instanceof_object", lyingProto instanceof Object);

show("gp_null", String(Object.getPrototypeOf(new Proxy({}, { getPrototypeOf() { return null; } }))));
attempt("gp_number_throws", () => Object.getPrototypeOf(new Proxy({}, { getPrototypeOf() { return 1 as any; } })));
attempt("gp_string_throws", () => Object.getPrototypeOf(new Proxy({}, { getPrototypeOf() { return "p" as any; } })));
attempt("gp_undefined_throws", () => Object.getPrototypeOf(new Proxy({}, { getPrototypeOf() { return undefined as any; } })));
show("gp_object_result", typeof Object.getPrototypeOf(new Proxy({}, { getPrototypeOf() { return protoA; } })));

// setPrototypeOf: refusing is always legal; Reflect reports false, Object throws
const refusesProto: any = new Proxy({}, { setPrototypeOf() { return false; } });
show("sp_refuse_reflect", Reflect.setPrototypeOf(refusesProto, protoA));
attempt("sp_refuse_object", () => { Object.setPrototypeOf(refusesProto, protoA); return "ok"; });

// reporting success without moving the target is legal while it is extensible
const idleProtoTarget: any = {};
const idleProto: any = new Proxy(idleProtoTarget, { setPrototypeOf() { return true; } });
show("sp_idle_reports", Reflect.setPrototypeOf(idleProto, protoA));
show("sp_idle_target", Object.getPrototypeOf(idleProtoTarget) === Object.prototype);
show("sp_idle_read", Object.getPrototypeOf(idleProto) === Object.prototype);

// forwarding traps behave like no trap at all
const fwdTarget: any = {};
const fwdProto: any = new Proxy(fwdTarget, {
  getPrototypeOf(t) { return Reflect.getPrototypeOf(t); },
  setPrototypeOf(t, v) { return Reflect.setPrototypeOf(t, v); },
});
show("fwd_set", Reflect.setPrototypeOf(fwdProto, protoA));
show("fwd_target_moved", Object.getPrototypeOf(fwdTarget) === protoA);
show("fwd_read", fwdProto.tag);
show("fwd_null", Reflect.setPrototypeOf(fwdProto, null));
show("fwd_after_null", String(Object.getPrototypeOf(fwdProto)));

// __proto__ is an accessor on Object.prototype, so it reaches the traps only while
// the proxy still inherits from Object.prototype
const dunder: any = new Proxy({} as any, {
  getPrototypeOf() { return protoB; },
  setPrototypeOf() { return false; },
});
show("dunder_get", dunder.__proto__ === protoB);
attempt("dunder_set_refused", () => Reflect.set(dunder, "__proto__", protoA));

let protoCalls = 0;
const counted: any = new Proxy({}, { getPrototypeOf() { protoCalls++; return protoA; } });
Object.getPrototypeOf(counted);
Object.getPrototypeOf(counted);
void counted.tag;
show("gp_calls", protoCalls);

// ---------------------------------------------------------------------------
// 4. An ARRAY target: length and indices go through the traps, and the
//    Array.prototype methods reach the proxy as a plain sequence of get / has
// ---------------------------------------------------------------------------
const arrLog: string[] = [];

function watched(arr: any[]): any {
  return new Proxy(arr, {
    get(t, k, r) { arrLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
    set(t, k, v, r) { arrLog.push("set:" + String(k) + "=" + String(v)); return Reflect.set(t, k, v, r); },
    has(t, k) { arrLog.push("has:" + String(k)); return Reflect.has(t, k); },
    deleteProperty(t, k) { arrLog.push("del:" + String(k)); return Reflect.deleteProperty(t, k); },
    defineProperty(t, k, d) { arrLog.push("def:" + String(k)); return Reflect.defineProperty(t, k, d); },
  });
}

function arrRun(label: string, fn: (p: any, raw: any[]) => string): void {
  arrLog.length = 0;
  const raw: any[] = [10, 20, 30];
  const p = watched(raw);
  const out = fn(p, raw);
  show(label, out + "|" + arrLog.join(","));
}

arrRun("arr_read_index", (p) => String(p[1]));
arrRun("arr_read_length", (p) => String(p.length));
arrRun("arr_write_in_range", (p, raw) => { Reflect.set(p, "0", 99); return String(raw[0]); });
arrRun("arr_write_past_end", (p, raw) => { Reflect.set(p, "5", 1); return raw.length + ":" + String(raw[4]); });
arrRun("arr_shrink_length", (p, raw) => { Reflect.set(p, "length", 1); return raw.join("/"); });
arrRun("arr_delete_index", (p, raw) => { Reflect.deleteProperty(p, "1"); return raw.length + ":" + String(raw[1]); });
arrRun("arr_push", (p, raw) => { Array.prototype.push.call(p, 40); return raw.join("/"); });
arrRun("arr_pop", (p, raw) => { const v = Array.prototype.pop.call(p); return v + ":" + raw.join("/"); });
arrRun("arr_join", (p) => Array.prototype.join.call(p, "-"));
arrRun("arr_indexOf", (p) => String(Array.prototype.indexOf.call(p, 20)));
arrRun("arr_for_of", (p) => { const acc: number[] = []; for (const v of p) acc.push(v as number); return acc.join("/"); });
arrRun("arr_spread", (p) => [...(p as number[])].join("/"));
arrRun("arr_concat", (p) => (([] as any[]).concat(p as any) as any[]).join("/"));

// a hole in the target shows through `has`, and the trap can invent the element
const holed: any = [1, , 3];
const holeProxy: any = new Proxy(holed, {
  has(t, k) { return k === "1" ? true : Reflect.has(t, k); },
  get(t, k, r) { return k === "1" ? "INVENTED" : Reflect.get(t, k, r); },
});
show("arr_hole_has", ("1" in holeProxy) + ",value=" + holeProxy[1]);
show("arr_hole_map", (Array.prototype.map.call(holeProxy, (v: any) => String(v)) as any[]).join("/"));
show("arr_hole_target_map_len", holed.map((v: any) => String(v)).length);

// the length a get trap reports drives the generic methods
const liar: any = new Proxy([1, 2, 3, 4, 5], { get(t, k, r) { return k === "length" ? 2 : Reflect.get(t, k, r); } });
show("arr_lie_length", liar.length);
show("arr_lie_join", Array.prototype.join.call(liar, ","));
show("arr_lie_slice", (Array.prototype.slice.call(liar) as any[]).join(","));
show("arr_lie_isArray", Array.isArray(liar));
show("arr_lie_ownKeys", Reflect.ownKeys(liar).join("|"));

// the array exotic [[DefineOwnProperty]] still guards length: a non-configurable
// element blocks shrinking, and that rule belongs to the TARGET
const guarded: any = [1, 2, 3];
Object.defineProperty(guarded, "1", { value: 2, configurable: false, writable: true, enumerable: true });
const guardProxy: any = new Proxy(guarded, {});
show("arr_shrink_guarded", Reflect.set(guardProxy, "length", 0));
show("arr_guarded_state", guarded.length + ",contents=" + guarded.join("/"));

const sliced: any = Array.prototype.slice.call(new Proxy([1, 2, 3], {}));
show("arr_slice_is_array", Array.isArray(sliced) + ",proto=" + (Object.getPrototypeOf(sliced) === Array.prototype));
show("arr_nested_json", JSON.stringify([new Proxy([1, 2], {}), new Proxy({ a: 1 }, {})]));

// ---------------------------------------------------------------------------
// 5. A TYPED ARRAY target: indices forward as ordinary string keys, but every
//    accessor that needs the [[TypedArrayName]] slot refuses the proxy
// ---------------------------------------------------------------------------
const ta = new Uint8Array([10, 20, 30]);
const taLog: string[] = [];
const taProxy: any = new Proxy(ta, {
  get(t, k, r) { taLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
  set(t, k, v, r) { taLog.push("set:" + String(k)); return Reflect.set(t, k, v, r); },
  has(t, k) { taLog.push("has:" + String(k)); return Reflect.has(t, k); },
  ownKeys(t) { taLog.push("ownKeys"); return Reflect.ownKeys(t); },
});

taLog.length = 0;
show("ta_index_read", taProxy[1] + "|" + taLog.join(","));
taLog.length = 0;
show("ta_index_write", Reflect.set(taProxy, "0", 99) + ",target=" + ta[0] + "|" + taLog.join(","));
taLog.length = 0;
show("ta_out_of_range_write", Reflect.set(taProxy, "9", 1) + ",len=" + ta.length + "|" + taLog.join(","));
taLog.length = 0;
show("ta_ownKeys", Reflect.ownKeys(taProxy).join("|") + "|" + taLog.join(","));
show("ta_keys", Object.keys(taProxy).join("|"));
show("ta_has_index", ("2" in taProxy) + ",has_missing=" + ("7" in taProxy));

attempt("ta_length", () => taProxy.length);
attempt("ta_byteLength", () => taProxy.byteLength);
attempt("ta_byteOffset", () => taProxy.byteOffset);
attempt("ta_buffer", () => typeof taProxy.buffer);
attempt("ta_iterate", () => { let n = 0; for (const _v of taProxy) n++; return n; });
attempt("ta_join", () => taProxy.join("-"));
attempt("ta_subarray", () => taProxy.subarray(0, 1).length);
attempt("ta_set_method", () => { taProxy.set([1]); return "ok"; });
attempt("ta_array_from", () => Array.from(taProxy as any).join("-"));

const taLengthed: any = new Proxy(ta, { get(t, k, r) { return k === "length" ? (t as any).length : Reflect.get(t, k, r); } });
show("ta_generic_join", Array.prototype.join.call(taLengthed, "-"));
show("ta_generic_slice", (Array.prototype.slice.call(taLengthed) as any[]).join("-"));
show("ta_generic_map", (Array.prototype.map.call(taLengthed, (v: any) => v + 1) as any[]).join("-"));

show("ta_isView_proxy", ArrayBuffer.isView(taProxy));
show("ta_isView_target", ArrayBuffer.isView(ta));
show("ta_isArray_proxy", Array.isArray(taProxy));
show("ta_tag_proxy", Object.prototype.toString.call(taProxy));
show("ta_tag_target", Object.prototype.toString.call(ta));
show("ta_instanceof", taProxy instanceof Uint8Array);
show("ta_proto_is_uint8", Object.getPrototypeOf(taProxy) === Uint8Array.prototype);

show("ta_define_index", Reflect.defineProperty(taProxy, "1", { value: 7, configurable: true, writable: true, enumerable: true }));
show("ta_define_index_value", ta[1]);
show("ta_define_out_of_range", Reflect.defineProperty(taProxy, "5", { value: 7, configurable: true, writable: true, enumerable: true }));
show("ta_delete_index", Reflect.deleteProperty(taProxy, "0") + ",still=" + ta[0]);
show("ta_named_prop", Reflect.set(taProxy, "note", "N") + ",read=" + taProxy.note);
show("ta_json_proxy", JSON.stringify(taProxy));
show("ta_json_target", JSON.stringify(ta));

const dv = new DataView(new ArrayBuffer(4));
dv.setUint8(0, 42);
const dvProxy: any = new Proxy(dv, {});
attempt("dv_method_on_proxy", () => dvProxy.getUint8(0));
show("dv_method_via_target", Reflect.apply(dvProxy.getUint8, dv, [0]));
show("dv_isView_proxy", ArrayBuffer.isView(dvProxy));

// ---------------------------------------------------------------------------
// 6. NESTED targets: a proxy whose target is another proxy. Each level's trap
//    sees the next proxy as its target, the receiver stays the OUTERMOST proxy,
//    and the structural questions pierce every level
// ---------------------------------------------------------------------------
const layerLog: string[] = [];

function layer(target: any, tag: string): any {
  return new Proxy(target, {
    get(t, k, r) { layerLog.push("get" + tag + ":" + String(k)); return Reflect.get(t, k, r); },
    set(t, k, v, r) { layerLog.push("set" + tag + ":" + String(k)); return Reflect.set(t, k, v, r); },
    has(t, k) { layerLog.push("has" + tag + ":" + String(k)); return Reflect.has(t, k); },
    ownKeys(t) { layerLog.push("keys" + tag); return Reflect.ownKeys(t); },
    getOwnPropertyDescriptor(t, k) { layerLog.push("gopd" + tag + ":" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
    deleteProperty(t, k) { layerLog.push("del" + tag + ":" + String(k)); return Reflect.deleteProperty(t, k); },
    getPrototypeOf(t) { layerLog.push("proto" + tag); return Reflect.getPrototypeOf(t); },
  });
}

const nestRoot: any = { a: 1, b: 2 };
const nestOne: any = layer(nestRoot, "1");
const nestTwo: any = layer(nestOne, "2");
const nestThree: any = layer(nestTwo, "3");

function layerRun(label: string, fn: () => void): void {
  layerLog.length = 0;
  fn();
  show(label, layerLog.join(","));
}

layerRun("nest_read", () => { void nestThree.a; });
layerRun("nest_write", () => { Reflect.set(nestThree, "c", 3); });
layerRun("nest_has", () => { void ("a" in nestThree); });
layerRun("nest_delete", () => { Reflect.deleteProperty(nestThree, "c"); });
layerRun("nest_ownKeys", () => { Reflect.ownKeys(nestThree); });
layerRun("nest_keys", () => { Object.keys(nestThree); });
layerRun("nest_proto", () => { Object.getPrototypeOf(nestThree); });

show("nest_value", nestThree.a + ":" + nestThree.b);
show("nest_root_after", Object.keys(nestRoot).join("|"));

const seenTargets: string[] = [];
const innerNest: any = new Proxy(nestRoot, {
  get(t, k, r) { seenTargets.push("inner_target_is_root:" + (t === nestRoot)); return Reflect.get(t, k, r); },
});
const outerNest: any = new Proxy(innerNest, {
  get(t, k, r) { seenTargets.push("outer_target_is_inner:" + (t === innerNest)); return Reflect.get(t, k, r); },
});
void outerNest.a;
show("nest_targets", seenTargets.join(","));

const seenReceivers: string[] = [];
const rInner: any = new Proxy({ z: 1 }, {
  get(t, k, r) { seenReceivers.push("inner:" + (r === rOuter)); return Reflect.get(t, k, r); },
});
const rOuter: any = new Proxy(rInner, {
  get(t, k, r) { seenReceivers.push("outer:" + (r === rOuter)); return Reflect.get(t, k, r); },
});
void rOuter.z;
show("nest_receivers", seenReceivers.join(","));

// a trap at ONE level intercepts even though the levels around it forward
const midOverride: any = new Proxy(new Proxy({ x: 1 }, { get() { return "MIDDLE"; } }), {});
show("nest_middle_wins", midOverride.x);

const nArr: any = [1, 2, 3];
const na1: any = new Proxy(nArr, {});
const na2: any = new Proxy(na1, {});
const na3: any = new Proxy(na2, {});
show("nest_isArray", Array.isArray(na1) + ":" + Array.isArray(na2) + ":" + Array.isArray(na3));
show("nest_arr_tag", Object.prototype.toString.call(na3));
show("nest_arr_len", na3.length);
show("nest_arr_json", JSON.stringify(na3));

function nestFn(x: number): number { return x + 1; }
const nf1: any = new Proxy(nestFn, {});
const nf2: any = new Proxy(nf1, {});
const nf3: any = new Proxy(nf2, {});
show("nest_typeof", typeof nf1 + ":" + typeof nf2 + ":" + typeof nf3);
show("nest_call", nf3(1));
show("nest_fn_meta", nf3.name + ",len=" + nf3.length);

class NestK { }
const nk1: any = new Proxy(NestK, {});
const nk2: any = new Proxy(nk1, {});
show("nest_new", new nk2() instanceof NestK);

const mid = Proxy.revocable({ q: 1 }, {});
const outsideMid: any = new Proxy(mid.proxy, {});
show("nest_revoke_before", outsideMid.q);
mid.revoke();
attempt("nest_revoke_after", () => outsideMid.q);
