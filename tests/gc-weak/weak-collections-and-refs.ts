// Cross-runtime: WeakMap, WeakSet and WeakRef behavior that is observable
// without any collection. Keys and targets are compared by identity; every
// exotic object (Proxy, revoked Proxy, bound function, class, symbol) is an
// ordinary target, and holding one fires no trap. Nothing here depends on
// garbage collection timing, and no deref() result is ever expected to be
// undefined.

function attempt(label: string, fn: () => any): void {
  try {
    console.log(label + "=ok:" + String(fn()));
  } catch (e: any) {
    console.log(label + "=" + e.name);
  }
}

// --- WeakMap basics: set returns the map, get/has/delete answer by identity ---
const a: any = { id: "a" };
const b: any = { id: "b" };
const wm = new WeakMap<any, string>();
console.log("wm_set_returns_map=" + (wm.set(a, "alpha") === wm));
wm.set(b, "beta");
console.log("wm_get_a=" + wm.get(a));
console.log("wm_get_b=" + wm.get(b));
console.log("wm_get_missing=" + String(wm.get({ id: "a" } as any)));
console.log("wm_has_a=" + wm.has(a));
console.log("wm_has_missing=" + wm.has({ id: "a" } as any));
console.log("wm_overwrite=" + (wm.set(a, "alpha2") === wm) + ":" + wm.get(a));
console.log("wm_delete_a=" + wm.delete(a));
console.log("wm_delete_a_again=" + wm.delete(a));
console.log("wm_has_a_after_delete=" + wm.has(a));
console.log("wm_size=" + (wm as any).size);
console.log("wm_no_iterator=" + String(typeof (wm as any)[Symbol.iterator]));
console.log("wm_no_keys=" + String(typeof (wm as any).keys));
console.log("wm_tag=" + Object.prototype.toString.call(wm));
console.log("wm_instanceof=" + (wm instanceof WeakMap));

// --- WeakMap with primitive keys: every primitive is refused ---
attempt("wm_set_number", () => wm.set(1 as any, "x"));
attempt("wm_set_string", () => wm.set("s" as any, "x"));
attempt("wm_set_boolean", () => wm.set(true as any, "x"));
attempt("wm_set_null", () => wm.set(null as any, "x"));
attempt("wm_set_undefined", () => wm.set(undefined as any, "x"));
attempt("wm_set_bigint", () => wm.set(10n as any, "x"));
attempt("wm_set_registered_symbol", () => wm.set(Symbol.for("wm-key") as any, "x"));
attempt("wm_get_primitive", () => wm.get(1 as any));
attempt("wm_has_primitive", () => wm.has("s" as any));
attempt("wm_delete_primitive", () => wm.delete(null as any));
attempt("wm_no_new", () => (WeakMap as any)());
attempt("wm_iterable_init_primitive", () => new WeakMap([[1 as any, "x"]]));

// --- WeakMap accepts unique symbols as keys ---
const uniq: any = Symbol("uniq");
const symMap = new WeakMap<any, string>();
symMap.set(uniq, "sym");
console.log("wm_unique_symbol=" + symMap.get(uniq) + ":" + symMap.has(Symbol("uniq")));
attempt("wm_well_known_symbol", () => symMap.set(Symbol.iterator, "wk"));

// --- WeakSet basics: add returns the set, has/delete answer by identity ---
const ws = new WeakSet<any>();
console.log("ws_add_returns_set=" + (ws.add(a) === ws));
console.log("ws_has_a=" + ws.has(a));
console.log("ws_has_b=" + ws.has(b));
console.log("ws_add_again=" + (ws.add(a) === ws));
console.log("ws_delete_a=" + ws.delete(a));
console.log("ws_delete_a_again=" + ws.delete(a));
console.log("ws_has_a_after_delete=" + ws.has(a));
console.log("ws_has_primitive=" + ws.has(1 as any));
console.log("ws_delete_primitive=" + ws.delete("s" as any));
console.log("ws_size=" + (ws as any).size);
console.log("ws_no_iterator=" + String(typeof (ws as any)[Symbol.iterator]));
console.log("ws_no_values=" + String(typeof (ws as any).values));
console.log("ws_tag=" + Object.prototype.toString.call(ws));
attempt("ws_add_number", () => ws.add(1 as any));
attempt("ws_add_string", () => ws.add("s" as any));
attempt("ws_add_null", () => ws.add(null as any));
attempt("ws_add_undefined", () => ws.add(undefined as any));
attempt("ws_add_registered_symbol", () => ws.add(Symbol.for("ws-key") as any));
attempt("ws_add_unique_symbol", () => ws.add(Symbol("ws-unique")));
attempt("ws_iterable_init_primitive", () => new WeakSet([1 as any]));
attempt("ws_no_new", () => (WeakSet as any)());

// --- WeakRef construction and identity ---
const target: any = { id: 7 };
const ref = new WeakRef(target);
console.log("wr_tag=" + Object.prototype.toString.call(ref));
console.log("wr_typeof_deref=" + typeof ref.deref());
console.log("wr_deref_identity=" + (ref.deref() === target));
console.log("wr_deref_repeat=" + (ref.deref() === ref.deref()));
console.log("wr_deref_value=" + (ref.deref() as any).id);
target.id = 8;
console.log("wr_deref_sees_mutation=" + (ref.deref() as any).id);
const ref2 = new WeakRef(target);
console.log("wr_refs_distinct=" + (ref === ref2));
console.log("wr_targets_same=" + (ref.deref() === ref2.deref()));
console.log("wr_instanceof=" + (ref instanceof WeakRef));
console.log("wr_own_keys=" + Reflect.ownKeys(ref).length);
console.log("wr_proto_ctor=" + (WeakRef.prototype.constructor === WeakRef));
console.log("wr_ctor_length=" + WeakRef.length + ":" + WeakRef.name);
console.log("wr_deref_length=" + WeakRef.prototype.deref.length + ":" + WeakRef.prototype.deref.name);
const wrTag: any = Object.getOwnPropertyDescriptor(WeakRef.prototype, Symbol.toStringTag);
console.log("wr_tag_desc=" + wrTag.value + ":" + wrTag.writable + ":" + wrTag.enumerable + ":" + wrTag.configurable);
console.log("wr_deref_via_call=" + (WeakRef.prototype.deref.call(ref) === target));

// --- WeakRef targets: functions, arrays, instances, symbols, proxies ---
function fnTarget(): number { return 1; }
const arrTarget: any = [1, 2];
class Inst { v = 3; }
const instTarget = new Inst();
console.log("wr_function_target=" + (new WeakRef(fnTarget).deref() === fnTarget));
console.log("wr_array_target=" + (new WeakRef(arrTarget).deref() === arrTarget));
console.log("wr_class_target=" + (new WeakRef(instTarget).deref() as any).v);
console.log("wr_proxy_target_typeof=" + typeof new WeakRef(new Proxy({}, {})).deref());
const symTarget: any = Symbol("weak");
console.log("wr_unique_symbol_target=" + (new WeakRef(symTarget).deref() === symTarget));
console.log("wr_well_known_symbol_target=" + (new WeakRef(Symbol.iterator as any).deref() === Symbol.iterator));
attempt("wr_registered_symbol", () => new WeakRef(Symbol.for("wr-key") as any));

// --- WeakRef refuses every other primitive and a missing target ---
attempt("wr_number", () => new WeakRef(1 as any));
attempt("wr_string", () => new WeakRef("s" as any));
attempt("wr_boolean", () => new WeakRef(true as any));
attempt("wr_null", () => new WeakRef(null as any));
attempt("wr_undefined", () => new WeakRef(undefined as any));
attempt("wr_bigint", () => new WeakRef(10n as any));
attempt("wr_no_argument", () => new (WeakRef as any)());
attempt("wr_call_without_new", () => (WeakRef as any)({}));

// --- deref brand check ---
attempt("wr_deref_on_plain", () => WeakRef.prototype.deref.call({} as any));
attempt("wr_deref_on_object_target", () => WeakRef.prototype.deref.call(target as any));
attempt("wr_deref_on_prototype", () => (WeakRef.prototype as any).deref());

// --- subclass keeps the internal slot ---
class TrackedRef extends WeakRef<any> {
  label = "tracked";
}
const tracked = new TrackedRef(target);
console.log("wr_subclass_deref=" + (tracked.deref() === target));
console.log("wr_subclass_label=" + tracked.label);
console.log("wr_subclass_tag=" + Object.prototype.toString.call(tracked));
console.log("wr_subclass_instanceof=" + (tracked instanceof WeakRef) + ":" + (tracked instanceof TrackedRef));

// --- a WeakRef is an ordinary object: usable as a Map, Set and WeakMap key ---
const holder = new WeakMap<any, string>();
holder.set(ref, "held");
console.log("wr_as_weakmap_key=" + holder.get(ref));
console.log("wr_as_set_member=" + new Set([ref, ref]).size);

// --- exotic targets: a Proxy is its own identity, and no trap fires ---
const traps: string[] = [];
const inner: any = { id: "target" };
const proxy: any = new Proxy(inner, {
  get(t: any, k: any, r: any) { traps.push("get:" + String(k)); return Reflect.get(t, k, r); },
  has(t: any, k: any) { traps.push("has:" + String(k)); return Reflect.has(t, k); },
  ownKeys(t: any) { traps.push("ownKeys"); return Reflect.ownKeys(t); },
  getPrototypeOf(t: any) { traps.push("getProto"); return Reflect.getPrototypeOf(t); },
  getOwnPropertyDescriptor(t: any, k: any) { traps.push("gopd:" + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
});
const pwm = new WeakMap<any, string>();
const pws = new WeakSet<any>();
const pref = new WeakRef(proxy);
pwm.set(proxy, "via_proxy");
pws.add(proxy);
console.log("proxy_weakmap_get=" + pwm.get(proxy));
console.log("proxy_weakset_has=" + pws.has(proxy));
console.log("proxy_deref_is_proxy=" + (pref.deref() === proxy));
console.log("proxy_deref_not_target=" + (pref.deref() === inner));
console.log("proxy_target_not_key=" + pwm.has(inner) + ":" + pws.has(inner));
pwm.set(inner, "via_target");
console.log("proxy_both_held=" + pwm.get(proxy) + ":" + pwm.get(inner));
console.log("proxy_traps=" + JSON.stringify(traps.join("|")));

// --- a revoked Proxy is still an object and still a working key ---
const rev = Proxy.revocable({ id: "revocable" }, {});
const revWm = new WeakMap<any, string>();
revWm.set(rev.proxy, "before");
const revRef = new WeakRef(rev.proxy);
rev.revoke();
console.log("revoked_typeof=" + typeof rev.proxy);
console.log("revoked_weakmap_get=" + revWm.get(rev.proxy));
console.log("revoked_weakmap_has=" + revWm.has(rev.proxy));
console.log("revoked_deref_same=" + (revRef.deref() === rev.proxy));
console.log("revoked_set_returns_map=" + (revWm.set(rev.proxy, "after") === revWm) + ":" + revWm.get(rev.proxy));
console.log("revoked_weakset=" + new WeakSet([rev.proxy]).has(rev.proxy));
console.log("revoked_new_weakref_typeof=" + typeof new WeakRef(rev.proxy).deref());
attempt("revoked_read", () => (rev.proxy as any).id);
console.log("revoked_delete=" + revWm.delete(rev.proxy) + ":" + revWm.has(rev.proxy));

// --- every callable kind is a distinct identity ---
function plain(): number { return 1; }
const bound = plain.bind(null);
const arrow = () => 1;
class Klass { static tag = "k"; }
function* genFn(): any { yield 1; }
async function asyncFn(): Promise<void> { /* shape only */ }
const callables: any[] = [plain, bound, arrow, Klass, genFn, asyncFn, Math.max];
const fnMap = new WeakMap<any, number>();
callables.forEach((f, i) => fnMap.set(f, i));
console.log("callables_held=" + callables.map((f) => String(fnMap.get(f))).join(","));
console.log("bound_is_distinct=" + (bound === plain) + ":" + fnMap.get(bound) + ":" + fnMap.get(plain));
console.log("rebind_is_new_key=" + fnMap.has(plain.bind(null)));

// --- exotic non-callables are ordinary keys too ---
const nullProto: any = Object.create(null);
const argumentsObj: any = (function () { return arguments; })(1);
const frozen: any = Object.freeze({ f: 1 });
const arr: any = [1, 2];
const typed: any = new Uint8Array(2);
const err: any = new TypeError("x");
const gen: any = genFn();
const promise: any = Promise.resolve(1);
const boxed: any = new Number(1);
const exotics: any[] = [nullProto, argumentsObj, frozen, arr, typed, err, gen, promise, boxed, Math, JSON, globalThis];
const exoticSet = new WeakSet<any>(exotics);
console.log("exotics_all_held=" + exotics.every((o) => exoticSet.has(o)));
console.log("frozen_as_weakmap_key=" + (function () { const w = new WeakMap(); w.set(frozen, "ok"); return w.get(frozen); })());
console.log("null_proto_deref=" + (new WeakRef(nullProto).deref() === nullProto));
console.log("globalthis_deref=" + (new WeakRef(globalThis as any).deref() === globalThis));

// --- a WeakRef target stays stable while a strong reference is held ---
const strong: any = { n: 0 };
const stable = new WeakRef(strong);
for (let i = 0; i < 5; i++) strong.n = i;
console.log("stable_identity=" + (stable.deref() === strong) + ":" + (stable.deref() as any).n);
console.log("stable_repeat=" + (stable.deref() === stable.deref()));
console.log("stable_typeof=" + typeof stable.deref());
