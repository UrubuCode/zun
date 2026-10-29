export {};
// The Proxy constructor as an object, what makes a proxy callable or constructable
// and which construct-trap results are legal, class definitions that extend a
// proxy, instanceof with a proxy on either side, brand checks (internal slots and
// private names do not forward), and how destructuring, for-in and JSON walk a
// proxy.

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
// 1. The Proxy constructor surface
// ---------------------------------------------------------------------------
show("ctor_typeof", typeof Proxy);
show("ctor_name_length", Proxy.name + ",length=" + Proxy.length);
show("ctor_own_names", Object.getOwnPropertyNames(Proxy).sort().join("|"));
show("ctor_has_prototype", "prototype" in Proxy);
show("ctor_proto_is_function_proto", Object.getPrototypeOf(Proxy) === Function.prototype);
const nameDesc = Object.getOwnPropertyDescriptor(Proxy, "name") as any;
show("ctor_name_desc", "w=" + nameDesc.writable + ",e=" + nameDesc.enumerable + ",c=" + nameDesc.configurable);

attempt("ctor_call_no_new", () => (Proxy as any)({}, {}));
attempt("ctor_reflect_apply", () => Reflect.apply(Proxy as any, undefined, [{}, {}]));
show("ctor_reflect_construct", typeof Reflect.construct(Proxy as any, [{}, {}]));

attempt("ctor_null_target", () => new Proxy(null as any, {}));
attempt("ctor_undefined_target", () => new Proxy(undefined as any, {}));
attempt("ctor_number_target", () => new Proxy(1 as any, {}));
attempt("ctor_string_target", () => new Proxy("s" as any, {}));
attempt("ctor_symbol_target", () => new Proxy(Symbol("s") as any, {}));
attempt("ctor_null_handler", () => new Proxy({}, null as any));
attempt("ctor_number_handler", () => new Proxy({}, 1 as any));
attempt("ctor_no_args", () => new (Proxy as any)());
attempt("ctor_one_arg", () => new (Proxy as any)({}));
show("ctor_function_handler", typeof new Proxy({}, function () { /* callable objects are objects */ } as any));
show("ctor_array_handler", typeof new Proxy({}, [] as any));
show("ctor_extra_args_ignored", typeof new (Proxy as any)({ a: 1 }, {}, "ignored"));

const p: any = new Proxy({ a: 1 }, {});
attempt("ctor_instanceof_proxy", () => p instanceof Proxy);
show("ctor_instanceof_object", p instanceof Object);
show("ctor_constructor_is_object", p.constructor === Object);
show("ctor_proto_is_object_proto", Object.getPrototypeOf(p) === Object.prototype);
show("ctor_own_keys", Reflect.ownKeys(p).join("|"));
show("ctor_own_keys_empty_target", Reflect.ownKeys(new Proxy({}, {})).length);
show("ctor_is_extensible", Object.isExtensible(p));
show("ctor_identity", new Proxy({}, {}) === new Proxy({}, {}));

const secretTarget: any = { s: 1 };
const secretHandler: any = { get(t: any, k: any, r: any) { return Reflect.get(t, k, r); } };
const secretive: any = new Proxy(secretTarget, secretHandler);
show("ctor_no_target_prop", String(secretive.target) + ":" + String(secretive["[[Target]]"]));
show("ctor_target_not_equal", secretive === secretTarget);
show("ctor_handler_not_key", Reflect.ownKeys(secretive).indexOf("handler") < 0);

show("revocable_name_length", Proxy.revocable.name + ",length=" + Proxy.revocable.length);
attempt("revocable_no_new", () => typeof Proxy.revocable({}, {}));
attempt("revocable_with_new", () => new (Proxy.revocable as any)({}, {}));
attempt("revocable_bad_target", () => Proxy.revocable(5 as any, {}));
const rv1 = Proxy.revocable({}, {});
const rv2 = Proxy.revocable({}, {});
show("revocable_distinct_revoke", rv1.revoke !== rv2.revoke);
show("revocable_revoke_proto", Object.getPrototypeOf(rv1.revoke) === Function.prototype);
show("revocable_keys", Object.keys(rv1).sort().join("|"));
show("revocable_result_proto", Object.getPrototypeOf(rv1) === Object.prototype);
show("revocable_result_desc", (() => {
  const d = Object.getOwnPropertyDescriptor(rv1, "proxy") as any;
  return "w=" + d.writable + ",e=" + d.enumerable + ",c=" + d.configurable;
})());

// ---------------------------------------------------------------------------
// 2. Callable and constructable: a proxy has [[Call]] / [[Construct]] only when
//    its TARGET does, so a trap on a non-callable target is dead code
// ---------------------------------------------------------------------------
let arrowReached = "no";
const arrowProxy: any = new Proxy(() => 1, { construct() { arrowReached = "yes"; return {}; } });
attempt("arrow_new", () => typeof new arrowProxy());
show("arrow_trap_reached", arrowReached);
show("arrow_call", arrowProxy());

const methodHost = { m() { return "M"; } };
const methodProxy: any = new Proxy(methodHost.m, { construct() { return {}; } });
attempt("method_new", () => typeof new methodProxy());
show("method_call", methodProxy());

let applyReached = "no";
const objProxy: any = new Proxy({} as any, { apply() { applyReached = "yes"; return 1; } });
attempt("object_call", () => objProxy());
show("object_trap_reached", applyReached);
show("object_typeof", typeof objProxy);

function* gen() { yield 1; }
const genProxy: any = new Proxy(gen, { construct() { return {}; } });
show("gen_call", genProxy().next().value);
attempt("gen_new", () => typeof new genProxy());

const asyncProxy: any = new Proxy(async function a() { return 1; }, { construct() { return {}; } });
show("async_typeof", typeof asyncProxy);
attempt("async_new", () => typeof new asyncProxy());

// a construct trap must return an object
function Base(this: any) { this.b = 1; }
attempt("construct_returns_number", () => typeof new (new Proxy(Base, { construct() { return 5 as any; } }) as any)());
attempt("construct_returns_undefined", () => typeof new (new Proxy(Base, { construct() { return undefined as any; } }) as any)());
attempt("construct_returns_null", () => typeof new (new Proxy(Base, { construct() { return null as any; } }) as any)());
show("construct_returns_function", typeof new (new Proxy(Base, { construct() { return function inner() { return 1; }; } }) as any)());
show("construct_returns_array", Array.isArray(new (new Proxy(Base, { construct() { return [1, 2]; } }) as any)()));

// an apply trap has no such rule
show("apply_returns_number", new Proxy(Base, { apply() { return 5; } })());
show("apply_returns_undefined", String(new Proxy(Base, { apply() { return undefined; } })()));

// what the traps receive: thisArg, a real Array of arguments, and newTarget
const trapInfo: string[] = [];
const spy: any = new Proxy(Base, {
  apply(t, thisArg, args) {
    trapInfo.push("apply:this=" + (thisArg === null ? "null" : typeof thisArg) + ",args=" + Array.isArray(args) + ":" + args.length + ":" + args.join("/"));
    return "A";
  },
  construct(t, args, nt) {
    trapInfo.push("construct:args=" + Array.isArray(args) + ":" + args.join("/") + ",nt_is_proxy=" + (nt === spy));
    return { tag: "C" };
  },
});
show("spy_call_ret", spy(1, 2));
show("spy_method_ret", spy.call({ x: 1 }, 3));
show("spy_apply_ret", spy.apply(null, [4, 5]));
show("spy_reflect_apply_ret", Reflect.apply(spy, undefined, [6]));
show("spy_construct_ret", new spy(7, 8).tag);
show("spy_info", trapInfo.join("|"));

// Reflect.construct hands a DIFFERENT newTarget straight to the trap
let sawNewTarget = "none";
const ntSpy: any = new Proxy(Base, {
  construct(t, args, nt) {
    sawNewTarget = nt === OtherCtor ? "Other" : nt === ntSpy ? "self" : "?";
    return Reflect.construct(t as any, args, nt);
  },
});
function OtherCtor(this: any) { /* noop */ }
(OtherCtor as any).prototype.marker = "OTHER";
const viaNt: any = Reflect.construct(ntSpy, [], OtherCtor);
show("nt_seen", sawNewTarget + ",marker=" + viaNt.marker);

// a bound function keeps its target's constructability through a proxy
const boundBase: any = (Base as any).bind(null);
const boundProxy: any = new Proxy(boundBase, {});
show("bound_new", new boundProxy().b);
show("bound_typeof_name", typeof boundProxy + ",name=" + boundProxy.name);
show("proxy_of_proxy_call", new Proxy(new Proxy(Base, { apply() { return "INNER"; } }), {})());

// typeof and structural questions are answered by the target's shape
function plainFn(a: number, b: number): number { return a + b; }
const arrowFn = (x: number) => x * 2;
class Klass { v: number; constructor(v: number) { this.v = v; } }
const plainObj = { a: 1 };
const pPlain: any = new Proxy(plainFn, {});
const pArrow: any = new Proxy(arrowFn, {});
const pKlass: any = new Proxy(Klass, {});
const pObj: any = new Proxy(plainObj, {});

show("tf_plain", typeof pPlain);
show("tf_arrow", typeof pArrow);
show("tf_class", typeof pKlass);
show("tf_obj", typeof pObj);
show("tc_plain", pPlain(2, 3));
show("tc_arrow", pArrow(4));
attempt("tc_obj", () => pObj());
attempt("tn_plain", () => typeof new pPlain(1, 2));
attempt("tn_arrow", () => typeof new pArrow(1));
attempt("tn_class", () => new pKlass(9).v);
attempt("tn_class_without_new", () => pKlass(1));
show("tm_plain", pPlain.length + ":" + pPlain.name);
show("tm_arrow", pArrow.length + ":" + pArrow.name);
show("tm_class", pKlass.length + ":" + pKlass.name);

const klassInst = new Klass(1);
show("ti_inst_of_proxy", klassInst instanceof pKlass);
show("ti_proxy_inst_of_class", new pKlass(1) instanceof Klass);

const arrTarget = [1, 2, 3];
const pArr: any = new Proxy(arrTarget, {});
const pArrArr: any = new Proxy(pArr, {});
show("isarray_arr", Array.isArray(pArr));
show("isarray_nested", Array.isArray(pArrArr));
show("isarray_obj", Array.isArray(pObj));
show("tag_arr", Object.prototype.toString.call(pArr));
show("tag_fn", Object.prototype.toString.call(pPlain));
show("tag_obj", Object.prototype.toString.call(pObj));

pArr.push(4);
show("proxy_push", arrTarget.join("|") + ",len=" + pArr.length);
pArr.length = 2;
show("proxy_truncate", arrTarget.join("|"));
show("proxy_arr_json", JSON.stringify(pArr));
show("proxy_concat_spreads", JSON.stringify([0].concat(pArr)));

show("map_with_proxy", [1, 2, 3].map(pArrow as any).join("|"));
show("bind_through_proxy", (pPlain as any).bind(null, 10)(5));
show("apply_through_proxy", Reflect.apply(pPlain, null, [7, 8]));
attempt("construct_object_proxy", () => Reflect.construct(pObj, []));
show("construct_class_proxy", Reflect.construct(pKlass, [3]).v);

const doubleFn: any = new Proxy(new Proxy(plainFn, {}), {});
show("double_fn", typeof doubleFn + ":" + doubleFn(1, 1));

// ---------------------------------------------------------------------------
// 3. A class whose SUPERCLASS is a proxy: the class definition reads "prototype"
//    through the get trap, super(...) reaches the construct trap with newTarget
//    set to the DERIVED class, and statics resolve through get on every access
// ---------------------------------------------------------------------------
const extLog: string[] = [];

class ExtBase {
  tag: string;
  static kind = "BASE";
  constructor(v: string) { this.tag = "base:" + v; }
  static make(): string { return "static_make"; }
  hello(): string { return "hello:" + this.tag; }
}

const PBase: any = new Proxy(ExtBase, {
  get(t, k, r) { extLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
  construct(t, args, nt) {
    extLog.push("construct:" + args.join("/") + ":nt=" + (nt === PBase ? "proxy" : nt === (Derived as any) ? "Derived" : "?"));
    return Reflect.construct(t as any, args, nt);
  },
});

extLog.length = 0;
class Derived extends PBase {
  extra: string;
  constructor() {
    super("d");
    this.extra = "E";
  }
  hello(): string { return "derived:" + super.hello(); }
}
show("ext_definition_log", extLog.join(","));

extLog.length = 0;
const derivedInst: any = new Derived();
show("ext_new_log", extLog.join(","));
show("ext_instance", derivedInst.tag + "|" + derivedInst.extra);
show("ext_instanceof_derived", derivedInst instanceof Derived);
show("ext_instanceof_base", derivedInst instanceof ExtBase);
show("ext_instanceof_proxy", derivedInst instanceof PBase);
show("ext_proto_chain", Object.getPrototypeOf(Object.getPrototypeOf(derivedInst)) === ExtBase.prototype);
show("ext_super_method", derivedInst.hello());

extLog.length = 0;
show("ext_static_kind", (Derived as any).kind);
show("ext_static_call", (Derived as any).make());
show("ext_static_log", extLog.join(","));
show("ext_class_proto_is_proxy", Object.getPrototypeOf(Derived) === PBase);

extLog.length = 0;
const directInst: any = new PBase("x");
show("ext_direct_log", extLog.join(","));
show("ext_direct_tag", directInst.tag + ",instanceof=" + (directInst instanceof ExtBase));

// a construct trap returning an unrelated object wins over the binding made by super()
class Hijacked extends (new Proxy(ExtBase, { construct() { return { tag: "HIJACKED", extra: "H" }; } }) as any) {
  constructor() { super("ignored"); }
}
const hijackedInst: any = new Hijacked();
show("ext_hijacked", hijackedInst.tag + "|" + hijackedInst.extra);
show("ext_hijacked_instanceof", (hijackedInst instanceof Hijacked) + ":" + (hijackedInst instanceof ExtBase));

// a construct trap returning a primitive breaks the derived constructor too
class BadSuper extends (new Proxy(ExtBase, { construct() { return 5 as any; } }) as any) {
  constructor() { super("x"); }
}
attempt("ext_bad_super", () => { new BadSuper(); return "ok"; });

// the prototype property of a class is non-writable AND non-configurable, so the
// get invariant refuses a substituted prototype; a plain function's is writable,
// so the same lie goes through
function makeWeird(baseFn: any, fake: any): string {
  try {
    const P: any = new Proxy(baseFn, { get(t, k, r) { return k === "prototype" ? fake : Reflect.get(t, k, r); } });
    const C: any = class extends P { constructor() { super("w"); } };
    const inst: any = new C();
    return "chain=" + (Object.getPrototypeOf(C.prototype) === fake) + ",fake=" + inst.fake +
      ",tag=" + inst.tag + ",instanceof_base=" + (inst instanceof baseFn);
  } catch (e) {
    return "throw:" + (e as Error).constructor.name;
  }
}
const fakeProto: any = { fake: "FAKE" };
function PlainBase(this: any, v: string) { this.tag = "plain:" + v; }
show("weird_class", makeWeird(ExtBase, fakeProto));
show("weird_function", makeWeird(PlainBase, fakeProto));
const classProtoDesc = Object.getOwnPropertyDescriptor(ExtBase, "prototype") as any;
show("class_proto_desc", "w=" + classProtoDesc.writable + ",e=" + classProtoDesc.enumerable + ",c=" + classProtoDesc.configurable);
const fnProtoDesc = Object.getOwnPropertyDescriptor(PlainBase, "prototype") as any;
show("fn_proto_desc", "w=" + fnProtoDesc.writable + ",e=" + fnProtoDesc.enumerable + ",c=" + fnProtoDesc.configurable);

attempt("class_proxy_call", () => (PBase as any)("nope"));

// ---------------------------------------------------------------------------
// 4. instanceof with a proxy on the right: a get of @@hasInstance, then (if absent)
//    a get of "prototype" and a walk of the LEFT operand's chain
// ---------------------------------------------------------------------------
const instLog: string[] = [];

function traced(target: any, extra?: any): any {
  return new Proxy(target, {
    get(t, k, r) {
      instLog.push("get:" + String(k));
      if (extra !== undefined && k in extra) return extra[k as any];
      return Reflect.get(t, k, r);
    },
    getPrototypeOf(t) { instLog.push("getProto"); return Reflect.getPrototypeOf(t); },
  });
}

function instRun(label: string, fn: () => string): void {
  instLog.length = 0;
  let out: string;
  try {
    out = fn();
  } catch (e) {
    out = "throw:" + (e as Error).constructor.name;
  }
  show(label, out + "|" + instLog.join(","));
}

class Animal { }
class Dog extends Animal { }
const dog = new Dog();

instRun("io_class_rhs", () => String(dog instanceof traced(Dog)));
instRun("io_base_rhs", () => String(dog instanceof traced(Animal)));
instRun("io_miss", () => String({} instanceof traced(Dog)));
instRun("io_primitive_lhs", () => String((5 as any) instanceof traced(Dog)));
instRun("io_null_lhs", () => String((null as any) instanceof traced(Dog)));
instRun("io_noncallable_rhs", () => String(dog instanceof (new Proxy({}, {}) as any)));
instRun("io_plain_fn_rhs", () => String(dog instanceof traced(function F() { /* noop */ })));

instRun("io_substitute_prototype", () => String(dog instanceof traced(Dog, { prototype: Animal.prototype })));
instRun("io_substitute_unrelated", () => String(dog instanceof traced(Dog, { prototype: {} })));
instRun("io_bad_prototype", () => String(dog instanceof traced(Dog, { prototype: 5 })));

const hiCalls: string[] = [];
instRun("io_hasInstance_true", () => {
  const rhs: any = traced(Dog, {
    [Symbol.hasInstance]: function (this: any, v: any) {
      hiCalls.push("this_is_proxy=" + (this === rhs) + ",arg_is_dog=" + (v === dog));
      return true;
    },
  });
  return String(dog instanceof rhs);
});
show("io_hasInstance_calls", hiCalls.join("|"));
instRun("io_hasInstance_false", () => String(dog instanceof traced(Dog, { [Symbol.hasInstance]: () => false })));
instRun("io_hasInstance_truthy", () => String(dog instanceof traced(Dog, { [Symbol.hasInstance]: () => 1 })));
instRun("io_hasInstance_zero", () => String(dog instanceof traced(Dog, { [Symbol.hasInstance]: () => 0 })));
instRun("io_hasInstance_null", () => String(dog instanceof traced(Dog, { [Symbol.hasInstance]: null })));
instRun("io_hasInstance_noncallable", () => String(dog instanceof traced(Dog, { [Symbol.hasInstance]: 5 })));
// with @@hasInstance the right operand need not be callable at all
instRun("io_hasInstance_on_object", () => String(dog instanceof (new Proxy({}, {
  get(_t, k) { return k === Symbol.hasInstance ? () => true : undefined; },
}) as any)));

class Branded {
  static [Symbol.hasInstance](v: any): boolean { return typeof v === "string"; }
}
instRun("io_static_hasInstance", () => String(("x" as any) instanceof traced(Branded)));
instRun("io_static_hasInstance_miss", () => String((5 as any) instanceof traced(Branded)));

const lhsLog: string[] = [];
const lhsProxy: any = new Proxy(dog, { getPrototypeOf(t) { lhsLog.push("proto"); return Reflect.getPrototypeOf(t); } });
show("io_lhs_dog", (lhsProxy instanceof Dog) + ",steps=" + lhsLog.length);
lhsLog.length = 0;
show("io_lhs_animal", (lhsProxy instanceof Animal) + ",steps=" + lhsLog.length);
lhsLog.length = 0;
show("io_lhs_miss", (lhsProxy instanceof Map) + ",steps=" + lhsLog.length);

const liarLhs: any = new Proxy(dog, { getPrototypeOf() { return Map.prototype; } });
show("io_lying_lhs", (liarLhs instanceof Map) + ",dog=" + (liarLhs instanceof Dog));
show("io_both_proxies", new Proxy(dog, {}) instanceof (new Proxy(Dog, {}) as any));
const boundCtor: any = (Dog as any).bind(null);
show("io_bound_rhs", dog instanceof (new Proxy(boundCtor, {}) as any));

// ---------------------------------------------------------------------------
// 5. Brand checks: a method that reads an INTERNAL SLOT rejects the proxy however
//    faithfully the handler forwards, because `this` is the proxy and the slot is
//    on the target. @@toStringTag does forward, so it still reports the class.
// ---------------------------------------------------------------------------
const map = new Map<string, number>([["a", 1]]);
const set = new Set<number>([1, 2]);
const date = new Date(0);
const re = /ab+/g;
const promise = Promise.resolve(1);
const wm = new WeakMap<object, number>();
const wmKey: any = {};
wm.set(wmKey, 5);

const forward: any = {
  get(t: any, k: any, r: any) { return Reflect.get(t, k, r); },
  has(t: any, k: any) { return Reflect.has(t, k); },
};
const pMap: any = new Proxy(map, forward);
const pSet: any = new Proxy(set, forward);
const pDate: any = new Proxy(date, forward);
const pRe: any = new Proxy(re, forward);
const pProm: any = new Proxy(promise, forward);
const pWm: any = new Proxy(wm, forward);

attempt("brand_map_get", () => pMap.get("a"));
attempt("brand_map_size", () => pMap.size);
attempt("brand_map_iterate", () => { let n = 0; for (const _e of pMap) n++; return n; });
attempt("brand_set_has", () => pSet.has(1));
attempt("brand_date_getTime", () => pDate.getTime());
attempt("brand_date_iso", () => pDate.toISOString());
attempt("brand_regexp_test", () => pRe.test("abb"));
attempt("brand_regexp_source", () => pRe.source);
attempt("brand_promise_then", () => typeof pProm.then(() => 1));
attempt("brand_weakmap_get", () => pWm.get(wmKey));

show("brand_target_map_get", Reflect.apply(pMap.get, map, ["a"]));
show("brand_target_date", Reflect.apply(pDate.getTime, date, []));
show("brand_target_set", Reflect.apply(pSet.has, set, [2]));

// the brand check is on `this`, so a get trap that binds the target repairs it
const boundMap: any = new Proxy(map, {
  get(t: any, k: any, r: any) {
    const v = Reflect.get(t, k, r);
    return typeof v === "function" ? v.bind(t) : v;
  },
});
show("brand_repaired_get", boundMap.get("a"));
attempt("brand_repaired_size", () => boundMap.size);

show("brand_instanceof", (pMap instanceof Map) + ":" + (pDate instanceof Date) + ":" + (pRe instanceof RegExp));
show("brand_proto", Object.getPrototypeOf(pMap) === Map.prototype);
show("brand_has_method", ("get" in pMap) + ":" + typeof pMap.get);

show("tag_map", Object.prototype.toString.call(pMap));
show("tag_set", Object.prototype.toString.call(pSet));
show("tag_promise", Object.prototype.toString.call(pProm));
show("tag_weakmap", Object.prototype.toString.call(pWm));
show("tag_date", Object.prototype.toString.call(pDate));
show("tag_regexp", Object.prototype.toString.call(pRe));
show("tag_error", Object.prototype.toString.call(new Proxy(new Error("x"), forward)));
show("tag_arguments", Object.prototype.toString.call(new Proxy((function () { return arguments; })(), forward)));
show("tag_invented", Object.prototype.toString.call(new Proxy({}, { get(_t, k) { return k === Symbol.toStringTag ? "Invented" : undefined; } })));
show("tag_nonstring_ignored", Object.prototype.toString.call(new Proxy({}, { get(_t, k) { return k === Symbol.toStringTag ? 42 : undefined; } })));

attempt("brand_boxed_valueOf", () => (new Proxy(new Number(7), forward) as any).valueOf());
attempt("brand_boxed_coerce", () => Number(new Proxy(new Number(7), forward)));
show("brand_string_index", (new Proxy(new String("hi"), forward) as any)[0]);

// private names are not property keys: a private method reading `this.#x` with the
// proxy as receiver throws, and no handler can see a private name to repair it
class Counter {
  #count = 0;
  static #shared = "S";
  label: string;

  constructor(label: string) { this.label = label; }

  bump(): number { this.#count++; return this.#count; }
  read(): number { return this.#count; }
  static hasBrand(o: any): boolean { return #count in o; }
  static readShared(): string { return this.#shared; }
  get viaGetter(): number { return this.#count; }
}

const counter = new Counter("real");
counter.bump();
counter.bump();

const forwardingCounter: any = new Proxy(counter, forward);
const bareCounter: any = new Proxy(counter, {});
const nestedCounter: any = new Proxy(bareCounter, {});

show("brand_target_read", counter.read());
show("brand_target_check", Counter.hasBrand(counter));
show("brand_proxy_check", Counter.hasBrand(forwardingCounter));
show("brand_bare_check", Counter.hasBrand(bareCounter));
show("brand_nested_check", Counter.hasBrand(nestedCounter));
show("brand_plain_check", Counter.hasBrand({}));
show("brand_public_forwards", forwardingCounter.label + ":" + ("label" in forwardingCounter));

attempt("brand_proxy_read", () => forwardingCounter.read());
attempt("brand_proxy_bump", () => forwardingCounter.bump());
attempt("brand_proxy_getter", () => forwardingCounter.viaGetter);
attempt("brand_bare_read", () => bareCounter.read());
attempt("brand_nested_read", () => nestedCounter.read());

show("brand_via_apply", Reflect.apply(forwardingCounter.read, counter, []));
show("brand_via_call", (forwardingCounter.read as any).call(counter));
show("brand_target_untouched", counter.read());

const repairedCounter: any = new Proxy(counter, {
  get(t: any, k: any, r: any) {
    const v = Reflect.get(t, k, r);
    return typeof v === "function" ? v.bind(t) : v;
  },
});
show("brand_repaired_read", repairedCounter.read());
show("brand_repaired_bump", repairedCounter.bump());
attempt("brand_repaired_getter", () => repairedCounter.viaGetter);

show("brand_private_ownKeys", Reflect.ownKeys(forwardingCounter).map(String).join("|"));
show("brand_private_keys", Object.keys(forwardingCounter).join("|"));
show("brand_private_json", JSON.stringify(forwardingCounter));
show("brand_private_gopd", String(Object.getOwnPropertyDescriptor(forwardingCounter, "count")));
show("brand_private_get", String(forwardingCounter.count));

const PCounter: any = new Proxy(Counter, forward);
show("brand_static_via_class", Counter.readShared());
attempt("brand_static_via_proxy", () => PCounter.readShared());
attempt("brand_static_check_via_proxy", () => PCounter.hasBrand(counter));

const built: any = new PCounter("built");
show("brand_built_check", Counter.hasBrand(built));
show("brand_built_read", built.read());
show("brand_built_instanceof", built instanceof Counter);

class SubCounter extends Counter { constructor() { super("sub"); } }
const sub = new SubCounter();
show("brand_sub_check", Counter.hasBrand(sub));
show("brand_sub_proxy_check", Counter.hasBrand(new Proxy(sub, {})));

// ---------------------------------------------------------------------------
// 6. Destructuring against a proxy: object patterns are plain [[Get]]s (one per
//    named binding, no ownKeys); a REST element switches to CopyDataProperties
//    (ownKeys, then a descriptor and a get per key it keeps); an array pattern
//    goes through the iterator protocol
// ---------------------------------------------------------------------------
const destLog: string[] = [];

function traced2(target: any): any {
  return new Proxy(target, {
    get(t, k, r) { destLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
    has(t, k) { destLog.push("has:" + String(k)); return Reflect.has(t, k); },
    ownKeys(t) { destLog.push("ownKeys"); return Reflect.ownKeys(t); },
    getOwnPropertyDescriptor(t, k) { destLog.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
  });
}

function destRun(label: string, fn: (p: any) => string): void {
  destLog.length = 0;
  const src: any = { a: 1, b: 2, c: 3 };
  Object.defineProperty(src, "hidden", { value: 4, enumerable: false, configurable: true });
  let out: string;
  try {
    out = fn(traced2(src));
  } catch (e) {
    out = "throw:" + (e as Error).constructor.name;
  }
  show(label, out + "|" + destLog.join(","));
}

destRun("de_named", (p) => { const { a, b } = p; return a + "/" + b; });
destRun("de_renamed", (p) => { const { a: x, c: y } = p; return x + "/" + y; });
destRun("de_missing", (p) => { const { zz } = p; return String(zz); });
destRun("de_default_unused", (p) => { const { a = 99 } = p; return String(a); });
destRun("de_default_used", (p) => { const { zz = 99 } = p; return String(zz); });
destRun("de_computed", (p) => { const key = "b"; const { [key]: v } = p; return String(v); });
destRun("de_rest_after_one", (p) => { const { a, ...rest } = p; return a + "/" + Object.keys(rest).join("+"); });
destRun("de_rest_only", (p) => { const { ...all } = p; return Object.keys(all).join("+"); });
destRun("de_rest_after_two", (p) => { const { a, b, ...rest } = p; return a + b + "/" + Object.keys(rest).join("+"); });
destRun("de_hidden_named", (p) => { const { hidden } = p as any; return String(hidden); });
destRun("de_hidden_in_rest", (p) => { const { ...all } = p; return String((all as any).hidden); });

const restSource: any = new Proxy({ a: 1, b: 2 }, {});
const { a: _restA, ...restObj } = restSource as any;
show("rest_proto", Object.getPrototypeOf(restObj) === Object.prototype);
show("rest_not_the_proxy", restObj === restSource);
const restDesc = Object.getOwnPropertyDescriptor(restObj, "b") as any;
show("rest_desc", "w=" + restDesc.writable + ",e=" + restDesc.enumerable + ",c=" + restDesc.configurable);

let getterCalls = 0;
const withGetter: any = new Proxy({ get live() { getterCalls++; return "L"; }, plain: 1 }, {});
const { ...copied } = withGetter as any;
show("rest_getter_calls", getterCalls);
show("rest_copied_live", (copied as any).live);
show("rest_copied_is_data", (Object.getOwnPropertyDescriptor(copied, "live") as any).get === undefined);

destLog.length = 0;
const exclusion: any = traced2({ keepMe: 1, dropMe: 2 });
const { dropMe: _dropMe, ...kept } = exclusion as any;
show("de_excluded_log", destLog.join(","));
show("de_kept", Object.keys(kept).join("+"));

destLog.length = 0;
const iterable: any = traced2([10, 20, 30]);
const [first, second] = iterable as any[];
show("de_array_values", first + "/" + second);
show("de_array_log", destLog.join(","));

destLog.length = 0;
const [head, ...tail] = traced2([1, 2, 3]) as any[];
show("de_array_rest", head + "/" + tail.join("+"));
show("de_array_rest_log", destLog.join(","));

attempt("de_not_iterable", () => { const [_x] = new Proxy({ a: 1 } as any, {}) as any; return "ok"; });

const fakeIterable: any = new Proxy({} as any, {
  get(_t, k) {
    if (k === Symbol.iterator) return function* () { yield "p"; yield "q"; };
    return undefined;
  },
});
const [p1, p2] = fakeIterable as any[];
show("de_fake_iterable", p1 + "/" + p2);

function takes({ a, ...rest }: any): string { return a + "/" + Object.keys(rest).join("+"); }
show("de_param", takes(new Proxy({ a: 1, b: 2, c: 3 }, {})));

// ---------------------------------------------------------------------------
// 7. for-in over a proxy chain: each level answers ownKeys once, every key is
//    filtered by its own getOwnPropertyDescriptor, a key already seen lower down is
//    skipped even when it was NOT enumerable there, and symbols never appear
// ---------------------------------------------------------------------------
const forinLog: string[] = [];

function forinLevel(target: any, tag: string, keys?: any[]): any {
  return new Proxy(target, {
    ownKeys(t) { forinLog.push("ownKeys" + tag); return keys === undefined ? Reflect.ownKeys(t) : keys; },
    getOwnPropertyDescriptor(t, k) {
      forinLog.push("gopd" + tag + ":" + String(k));
      const d = Reflect.getOwnPropertyDescriptor(t, k);
      if (d === undefined && keys !== undefined) return { value: "V" + tag, enumerable: true, configurable: true, writable: true };
      return d;
    },
    getPrototypeOf(t) { forinLog.push("proto" + tag); return Reflect.getPrototypeOf(t); },
    get(t, k, r) { forinLog.push("get" + tag + ":" + String(k)); return Reflect.get(t, k, r); },
  });
}

function forinNames(o: any): string {
  const acc: string[] = [];
  for (const k in o) acc.push(k);
  return acc.join("|");
}

const flatTarget: any = { a: 1, b: 2 };
Object.defineProperty(flatTarget, "hidden", { value: 3, enumerable: false, configurable: true });
flatTarget[Symbol("s")] = 4;
forinLog.length = 0;
const flat = forinLevel(flatTarget, "1");
show("fi_flat", forinNames(flat));
show("fi_flat_log", forinLog.join(","));

const protoLevelTarget: any = { shared: "P", onlyProto: "P2" };
const childLevelTarget: any = { shared: "C", onlyChild: "C2" };
const protoLevel = forinLevel(protoLevelTarget, "P");
Object.setPrototypeOf(childLevelTarget, protoLevel);
const childLevel = forinLevel(childLevelTarget, "C");
forinLog.length = 0;
show("fi_chain", forinNames(childLevel));
show("fi_chain_log", forinLog.join(","));

const shadowProto: any = { k: "P" };
const shadowChild: any = Object.create(shadowProto);
Object.defineProperty(shadowChild, "k", { value: "C", enumerable: false, configurable: true });
shadowChild.visible = 1;
show("fi_shadow_direct", forinNames(shadowChild));
show("fi_shadow_proxy", forinNames(new Proxy(shadowChild, {})));

forinLog.length = 0;
const invented = forinLevel({}, "I", ["x", "y"]);
show("fi_invented_forin", forinNames(invented));
show("fi_invented_keys", Object.keys(invented).join("|"));
show("fi_invented_log", forinLog.join(","));

const hidingProxy: any = new Proxy({ a: 1, b: 2 }, {
  getOwnPropertyDescriptor(t, k) {
    const d = Reflect.getOwnPropertyDescriptor(t, k) as any;
    if (k === "a" && d) d.enumerable = false;
    return d;
  },
});
show("fi_hiding_forin", forinNames(hidingProxy));
show("fi_hiding_keys", Object.keys(hidingProxy).join("|"));
show("fi_hiding_in", "a" in hidingProxy);
show("fi_hiding_value", hidingProxy.a);

const orderedProxy: any = new Proxy({} as any, {
  ownKeys() { return ["10", "b", "2", "a"]; },
  getOwnPropertyDescriptor() { return { value: 1, enumerable: true, configurable: true, writable: true }; },
});
show("fi_order_forin", forinNames(orderedProxy));
show("fi_order_keys", Object.keys(orderedProxy).join("|"));
show("fi_order_ownKeys", Reflect.ownKeys(orderedProxy).join("|"));

const symKey = Symbol("hidden");
const symGopdLog: string[] = [];
const withSymbols: any = new Proxy({} as any, {
  ownKeys() { return ["s", symKey]; },
  getOwnPropertyDescriptor(_t, k) { symGopdLog.push(String(k)); return { value: 1, enumerable: true, configurable: true, writable: true }; },
});
show("fi_sym_forin", forinNames(withSymbols));
show("fi_sym_gopd_calls", symGopdLog.join(","));

// a proxy PROTOTYPE below an ordinary object: the walk reaches it through the
// ordinary object's [[GetPrototypeOf]], and its ownKeys trap answers once
const belowTarget: any = { fromProxy: 1, alsoShadowed: "P" };
const below: any = new Proxy(belowTarget, { ownKeys(t) { return Reflect.ownKeys(t); } });
const above: any = Object.create(below);
above.own = 1;
above.alsoShadowed = "C";
show("fi_below_forin", forinNames(above));
show("fi_below_keys", Object.keys(above).join("|"));
show("fi_below_value", above.alsoShadowed + ":" + above.fromProxy);

// a getPrototypeOf trap that ends the chain early hides the rest of it
const cut: any = new Proxy(Object.create({ hiddenAbove: 1 }, { visible: { value: 2, enumerable: true, configurable: true } }), {
  getPrototypeOf() { return null; },
});
show("fi_cut_forin", forinNames(cut));
show("fi_cut_read", String(cut.hiddenAbove));

// deleting a key mid-loop through the proxy: the deleted key is not visited
const mutating: any = new Proxy({ a: 1, b: 2, c: 3 } as any, {});
const visited: string[] = [];
for (const k in mutating) {
  visited.push(k);
  Reflect.deleteProperty(mutating, "c");
}
show("fi_mutating", visited.join("|"));

// ---------------------------------------------------------------------------
// 8. JSON.stringify over a proxy: IsArray pierces the proxy, so an array target is
//    walked by length and index (no ownKeys at all), an object target by ownKeys,
//    and a callable target is skipped entirely
// ---------------------------------------------------------------------------
const jsonLog: string[] = [];

function jsonTraced(target: any): any {
  return new Proxy(target, {
    get(t, k, r) { jsonLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
    ownKeys(t) { jsonLog.push("ownKeys"); return Reflect.ownKeys(t); },
    getOwnPropertyDescriptor(t, k) { jsonLog.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
    has(t, k) { jsonLog.push("has:" + String(k)); return Reflect.has(t, k); },
  });
}

function jsonRun(label: string, target: any, replacer?: any, space?: any): void {
  jsonLog.length = 0;
  const out = JSON.stringify(jsonTraced(target), replacer, space);
  show(label, String(out).split("\n").join("\\n") + "|" + jsonLog.join(","));
}

jsonRun("js_object", { a: 1, b: "x" });
jsonRun("js_array", [1, 2]);
jsonRun("js_nested_array", [[1], 2]);
jsonRun("js_empty_array", []);
jsonRun("js_sparse_array", [1, , 3]);
jsonRun("js_function", function f() { return 1; });
jsonRun("js_undefined_member", { a: undefined, b: 1 });
jsonRun("js_symbol_key", { a: 1, [Symbol("s")]: 2 });
jsonRun("js_nonenumerable", Object.defineProperty({ a: 1 }, "hidden", { value: 2, enumerable: false }));
jsonRun("js_replacer_array", { a: 1, b: 2, c: 3 }, ["b", "a"]);
jsonRun("js_space", { a: 1 }, undefined, 2);

const withToJSON: any = { a: 1, toJSON() { return { replaced: true }; } };
jsonRun("js_tojson_own", withToJSON);

jsonLog.length = 0;
const injected: any = new Proxy({ a: 1 }, {
  get(t, k, r) { jsonLog.push("get:" + String(k)); return k === "toJSON" ? () => "INJECTED" : Reflect.get(t, k, r); },
  ownKeys(t) { jsonLog.push("ownKeys"); return Reflect.ownKeys(t); },
  getOwnPropertyDescriptor(t, k) { return Reflect.getOwnPropertyDescriptor(t, k); },
});
show("js_injected_tojson", JSON.stringify(injected) + "|" + jsonLog.join(","));

show("js_tojson_not_callable", JSON.stringify(new Proxy({ a: 1 }, { get(t, k, r) { return k === "toJSON" ? 5 : Reflect.get(t, k, r); } })));

const inventedJson: any = new Proxy({ real: 1 } as any, {
  ownKeys() { return ["real", "ghost"]; },
  getOwnPropertyDescriptor(t, k) {
    if (k === "ghost") return { value: 0, enumerable: true, configurable: true, writable: true };
    return Reflect.getOwnPropertyDescriptor(t, k);
  },
  get(t, k, r) { return k === "ghost" ? "G" : Reflect.get(t, k, r); },
});
show("js_invented", JSON.stringify(inventedJson));
const hiddenJson: any = new Proxy({ a: 1, b: 2 }, {
  getOwnPropertyDescriptor(t, k) {
    const d = Reflect.getOwnPropertyDescriptor(t, k) as any;
    if (k === "b" && d) d.enumerable = false;
    return d;
  },
});
show("js_hidden", JSON.stringify(hiddenJson));

const lyingKeys: any = new Proxy([1, 2, 3], { ownKeys() { return ["0"]; } });
show("js_array_ignores_ownKeys", JSON.stringify(lyingKeys));
const lyingLength: any = new Proxy([1, 2, 3], { get(t, k, r) { return k === "length" ? 1 : Reflect.get(t, k, r); } });
show("js_array_follows_length", JSON.stringify(lyingLength));

show("js_nested_mixed", JSON.stringify({ arr: new Proxy([1, new Proxy({ k: 2 }, {})], {}), obj: new Proxy({ z: 3 }, {}) }));

const holders: string[] = [];
const holderProxy: any = new Proxy({ a: 1, b: 2 }, {});
JSON.stringify(holderProxy, function (this: any, k: string, v: any) {
  holders.push(k + ":" + (this === holderProxy));
  return v;
});
show("js_replacer_holder", holders.join(","));

show("js_boxed_number", JSON.stringify(new Proxy(new Number(7), {})));
show("js_boxed_string", JSON.stringify(new Proxy(new String("s"), {})));
show("js_boxed_bool", JSON.stringify(new Proxy(new Boolean(true), {})));
