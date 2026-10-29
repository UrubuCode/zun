// Functions as objects: the kinds (plain, arrow, method, generator, async,
// async generator, class), their intrinsic prototypes and toStringTag, which of
// them own a `prototype` and with which attributes, which can be constructed,
// new.target in every entry path (call, new, Reflect.construct, bound), user
// properties on a function, freezing, replacing `prototype`, and Function.prototype
// toString in a whitespace-normalised form.
export {};

const show = (label: string, value: unknown): void => {
  console.log(label + "=" + JSON.stringify(value));
};

function plain(a: number): number { return a; }
const arrow = (a: number): number => a;
function* gen(a: number): Generator<number> { yield a; }
async function asyncFn(a: number): Promise<number> { return a; }
async function* asyncGen(a: number): AsyncGenerator<number> { yield a; }
const method = { m(a: number): number { return a; } }.m;
class Klass { constructor(a: number) { void a; } }
class SubKlass extends Klass {}

const kinds: Array<[string, any]> = [
  ["plain", plain],
  ["arrow", arrow],
  ["generator", gen],
  ["async", asyncFn],
  ["asyncGenerator", asyncGen],
  ["method", method],
  ["class", Klass],
];

// ---------------------------------------------------------------------------
// 1. typeof, brand and intrinsic prototype per kind
// ---------------------------------------------------------------------------
show("typeof", kinds.map(([n, f]) => n + ":" + typeof f));
show("brand", kinds.map(([n, f]) => n + ":" + Object.prototype.toString.call(f).slice(8, -1)));

function intrinsicName(f: any): string {
  const proto = Object.getPrototypeOf(f);
  return proto === Function.prototype ? "Function.prototype" : proto.constructor.name;
}
show("intrinsics", kinds.map(([n, f]) => n + ":" + intrinsicName(f)));

const GeneratorFunction: any = Object.getPrototypeOf(gen).constructor;
const AsyncFunction: any = Object.getPrototypeOf(asyncFn).constructor;
show("generator_ctor_global", typeof (globalThis as any).GeneratorFunction);
show("generator_ctor_reachable", [GeneratorFunction.name, AsyncFunction.name]);
show("intrinsic_is_function", GeneratorFunction instanceof Function);
show("generator_fn_tag", (Object.getPrototypeOf(gen) as any)[Symbol.toStringTag]);
show("async_fn_tag", (Object.getPrototypeOf(asyncFn) as any)[Symbol.toStringTag]);
show("gen_fn_proto_not_function_proto", Object.getPrototypeOf(gen) === Function.prototype);
show("class_proto_is_function_proto", Object.getPrototypeOf(Klass) === Function.prototype);
show("subclass_proto_is_parent", Object.getPrototypeOf(SubKlass) === Klass);
show("class_instanceof_function", Klass instanceof Function);

// ---------------------------------------------------------------------------
// 2. Own `prototype`: who has one, and its attributes
// ---------------------------------------------------------------------------
show("has_prototype", kinds.map(([n, f]) =>
  n + ":" + Object.prototype.hasOwnProperty.call(f, "prototype")));
function protoAttrs(f: any): string {
  const d: any = Object.getOwnPropertyDescriptor(f, "prototype");
  if (!d) return "none";
  return "w=" + d.writable + " e=" + d.enumerable + " c=" + d.configurable;
}
show("plain_prototype", protoAttrs(plain));
show("generator_prototype", protoAttrs(gen));
show("class_prototype", protoAttrs(Klass));
show("async_prototype", protoAttrs(asyncFn));
show("arrow_prototype_value", String((arrow as any).prototype));
show("bound_prototype_value", String((plain.bind(null) as any).prototype));

// A declaration's prototype links back by a non-enumerable `constructor`.
show("plain_backlink", plain.prototype.constructor === plain);
show("plain_backlink_enumerable", (Object.getOwnPropertyDescriptor(plain.prototype, "constructor") as any).enumerable);
show("class_backlink", Klass.prototype.constructor === Klass);
show("decl_proto_is_object", Object.getPrototypeOf(plain.prototype) === Object.prototype);
show("decl_proto_keys", Object.getOwnPropertyNames(plain.prototype));
// A generator's prototype has no constructor back-link, and no Object.prototype constructor either.
show("generator_backlink", Object.prototype.hasOwnProperty.call(gen.prototype, "constructor"));
show("generator_proto_parent_is_object", Object.getPrototypeOf(gen.prototype) === Object.prototype);

// ---------------------------------------------------------------------------
// 3. Constructibility and calling without `new`
// ---------------------------------------------------------------------------
function tryNew(f: any): string {
  try {
    const made = new f(1);
    return "ok:" + typeof made;
  } catch (e) {
    return "threw:" + (e as any).name;
  }
}
function tryCall(f: any): string {
  try {
    const out = f(1);
    return "ok:" + (out && typeof out === "object" ? "object" : typeof out);
  } catch (e) {
    return "threw:" + (e as any).name;
  }
}
show("constructible", kinds.map(([n, f]) => n + ":" + tryNew(f)));
show("callable", kinds.map(([n, f]) => n + ":" + tryCall(f)));
show("method_new", tryNew(method));
show("names", kinds.map(([n, f]) => n + ":" + JSON.stringify(f.name)));
show("lengths", kinds.map(([n, f]) => n + ":" + f.length));

// ---------------------------------------------------------------------------
// 4. Generator and async objects
// ---------------------------------------------------------------------------
const it: any = gen(5);
show("generator_object_tag", Object.prototype.toString.call(it).slice(8, -1));
show("generator_self_iterable", it[Symbol.iterator]() === it);
show("generator_first", it.next());
show("generator_second", it.next());
show("generator_proto_link", Object.getPrototypeOf(it) === gen.prototype);
show("generator_proto_has_next", "next" in Object.getPrototypeOf(Object.getPrototypeOf(it)));

const promise: any = asyncFn(3);
show("async_returns_promise", [promise instanceof Promise, promise.constructor.name]);
show("async_tag", Object.prototype.toString.call(promise).slice(8, -1));

const agen: any = asyncGen(1);
show("async_generator_tag", Object.prototype.toString.call(agen).slice(8, -1));
show("async_generator_next_is_promise", agen.next() instanceof Promise);
show("async_generator_has_asyncIterator", typeof agen[Symbol.asyncIterator] === "function");

class WithGen {
  *items(): Generator<number> { yield 1; }
  async load(): Promise<number> { return 1; }
}
show("class_gen_method_intrinsic", intrinsicName(WithGen.prototype.items));
show("class_async_method_intrinsic", intrinsicName(WithGen.prototype.load));
show("class_gen_method_tag", Object.prototype.toString.call(WithGen.prototype.items).slice(8, -1));

// ---------------------------------------------------------------------------
// 5. new.target in each entry path
// ---------------------------------------------------------------------------
class Probe {
  reportOwn: any;
  constructor() {
    this.reportOwn = (): string => (new.target === undefined ? "undefined" : new.target.name);
  }
}
class SubProbe extends Probe {}
show("new_target_direct", new Probe().reportOwn());
show("new_target_subclass", new SubProbe().reportOwn());
show("new_target_reflect", Reflect.construct(Probe, [], SubProbe).reportOwn());

function ordinary(): any {
  return { seen: new.target === undefined ? "called" : "constructed:" + new.target.name };
}
show("ordinary_called", ordinary().seen);
show("ordinary_constructed", new (ordinary as any)().seen);
show("ordinary_via_reflect", Reflect.construct(ordinary as any, []).seen);
show("ordinary_reflect_target", Reflect.construct(ordinary as any, [], Probe).seen);

// An arrow has no new.target of its own: it inherits the enclosing function's.
class Host {
  newTargetProbe(): any { return (): string => String(new.target); }
}
show("new_target_in_method_arrow", new Host().newTargetProbe()());

// A bound constructor ignores the bound `this`, keeps the bound arguments, and
// resolves instanceof against the TARGET's prototype.
function Pair(this: any, a: number, b: number) {
  this.a = a;
  this.b = b;
  this.tagged = (this as any).tagged === undefined ? "fresh" : "reused";
}
(Pair as any).prototype.sum = function (this: any): number { return this.a + this.b; };
(Pair as any).prototype.kind = "Pair";
const bad: any = { a: -1, b: -1, tagged: "bound-this" };
const BoundPair: any = (Pair as any).bind(bad);

BoundPair(7, 8);
show("call_used_bound_this", [bad.a, bad.b, bad.tagged]);
const made = new BoundPair(1, 2);
show("new_ignored_bound_this", [made.a, made.b, made.tagged]);
show("bad_untouched_by_new", [bad.a, bad.b]);
const partial = new ((Pair as any).bind(bad, 10))(20);
show("bound_arg_kept", [partial.a, partial.b, partial.sum()]);
show("both_bound_extra_ignored", new ((Pair as any).bind(bad, 3, 4))(99, 99).sum());
show("instance_of_target", made instanceof (Pair as any));
show("instance_of_bound", made instanceof BoundPair);
show("proto_is_target_proto", Object.getPrototypeOf(made) === (Pair as any).prototype);
show("inherited_kind", made.kind);
show("bound_has_prototype", Object.prototype.hasOwnProperty.call(BoundPair, "prototype"));
const Twice: any = ((Pair as any).bind(bad, 1)).bind(bad, 30);
const twiceMade = new Twice();
show("double_bound", [twiceMade.a, twiceMade.b, twiceMade instanceof (Pair as any)]);

// A bound class stays constructible; a plain call still throws.
class Box {
  v: number;
  constructor(v: number) { this.v = v; }
  read(): string { return "box:" + this.v; }
}
const BoundBox: any = (Box as any).bind(null, 42);
const boxed = new BoundBox();
show("class_bound", boxed.read());
show("class_bound_instanceof", boxed instanceof Box);
show("class_bound_proto", Object.getPrototypeOf(boxed) === Box.prototype);
show("bound_class_call", tryCall(BoundBox));
show("class_call", tryCall(Box));
show("reflect_construct_bound_class", (Reflect.construct(BoundBox, []) as any).v);

// Subclass of a class reached through a bound wrapper.
class Deeper extends Box {
  extra = "d";
  describe(): string { return this.read() + "/" + this.extra; }
}
show("subclass_describe", new Deeper(9).describe());
show("subclass_instanceof_bound_base", new Deeper(9) instanceof BoundBox);

// Symbol.hasInstance on the target is what a bound wrapper consults.
class Branded {
  static [Symbol.hasInstance](x: any): boolean { return x != null && x.brand === "yes"; }
}
const BoundBranded: any = (Branded as any).bind(null);
show("has_instance_target", { brand: "yes" } instanceof Branded);
show("has_instance_bound", ({ brand: "yes" } as any) instanceof BoundBranded);
show("has_instance_bound_no", ({ brand: "no" } as any) instanceof BoundBranded);

// A function's `prototype` can be replaced, and `instanceof` follows the new one.
function Widget(): void {}
const madeBefore: any = new (Widget as any)();
(Widget as any).prototype = { kind: "replaced" };
const madeAfter: any = new (Widget as any)();
show("before_instanceof", madeBefore instanceof Widget);
show("after_instanceof", madeAfter instanceof Widget);
show("after_kind", madeAfter.kind);

// A class's `prototype` is fixed: every write, define or delete is refused.
const beforeWrite = Klass.prototype;
show("class_set_refused", Reflect.set(Klass, "prototype", {}));
show("class_define_refused", Reflect.defineProperty(Klass, "prototype", { value: {} }));
show("class_delete_refused", Reflect.deleteProperty(Klass, "prototype"));
show("class_prototype_unchanged", Klass.prototype === beforeWrite);
show("class_prototype_writable", (Object.getOwnPropertyDescriptor(Klass, "prototype") as any).writable);

// ---------------------------------------------------------------------------
// 6. A function is an object: user properties, freezing, accessors, identity
// ---------------------------------------------------------------------------
function target(a: number, b: number): number { return a + b; }
(target as any).cacheHits = 0;
(target as any).describe = "a function with baggage";
show("enumerable_keys", Object.keys(target));
show("json_omits_function", JSON.stringify({ wrapped: target }));
show("length_is_not_enumerable", Object.keys(target).indexOf("length") >= 0);
show("in_operator", ["length" in target, "cacheHits" in target]);

function counted(): number {
  (counted as any).calls += 1;
  return (counted as any).calls;
}
(counted as any).calls = 0;
const c1 = counted();
const c2 = counted();
const c3 = counted();
show("self_counter", [c1, c2, c3, (counted as any).calls]);

show("delete_fn_prototype", Reflect.deleteProperty(target, "prototype"));
show("still_has_prototype", Object.prototype.hasOwnProperty.call(target, "prototype"));
show("delete_user_property", Reflect.deleteProperty(target, "describe"));
show("user_property_gone", Object.prototype.hasOwnProperty.call(target, "describe"));

function factory(): any { return function inner(): string { return "inner"; }; }
const one: any = factory();
const two: any = factory();
show("distinct_functions", one !== two);
show("distinct_prototypes", one.prototype !== two.prototype);
show("same_name", [one.name === two.name, one.name]);
one.stamp = "first";
show("properties_not_shared", String(two.stamp));
function arrowFactory(): any { return (): string => "arrow"; }
show("distinct_arrows", arrowFactory() !== arrowFactory());
function stable(): string { return "stable"; }
const firstRef = stable;
const secondRef = stable;
show("declaration_identity", firstRef === secondRef);
show("instanceof_function_object", [target instanceof Function, target instanceof Object]);
show("proto_chain", [
  Object.getPrototypeOf(target) === Function.prototype,
  Object.getPrototypeOf(Function.prototype) === Object.prototype,
]);

const spreadCopy: any = { ...(target as any) };
show("spread_copy_keys", Object.keys(spreadCopy));
show("spread_copy_type", typeof spreadCopy);

function frozenFn(): string { return "still-callable"; }
(frozenFn as any).mutable = "before";
Object.freeze(frozenFn);
show("frozen_set", [Reflect.set(frozenFn, "mutable", "after"), (frozenFn as any).mutable]);
show("frozen_call", frozenFn());
show("is_frozen", Object.isFrozen(frozenFn));

function withAccessor(): string { return "base"; }
let slotStore = "x";
Object.defineProperty(withAccessor, "slot", {
  get(): string { return "got:" + slotStore; },
  set(v: string) { slotStore = v.toUpperCase(); },
  enumerable: true,
  configurable: true,
});
(withAccessor as any).slot = "written";
show("accessor_on_function", (withAccessor as any).slot);
show("accessor_enumerable", Object.keys(withAccessor));

const registry = new Map<any, string>();
registry.set(one, "one");
registry.set(two, "two");
show("map_by_identity", [registry.get(one), registry.get(two), registry.get(factory())]);
const set = new Set([target, one, one]);
show("set_identity", [set.size, set.has(target)]);

const enriched: any = Object.assign(function base(): string { return "base"; }, { extra: 1 });
show("assign_onto_function", [typeof enriched, enriched(), enriched.extra]);

const a1 = function (): number { return 1; };
const a2 = function (): number { return 1; };
show("identical_source_differs", [a1 !== a2, a1() === a2()]);

// ---------------------------------------------------------------------------
// 7. Function.prototype.toString, normalised for whitespace
// ---------------------------------------------------------------------------
function norma(s: string): string { return s.replace(/\s+/g, " ").trim(); }
show("String", norma(String.toString()));
show("Array", norma(Array.toString()));
show("Object", norma(Object.toString()));
show("Date", norma(Date.toString()));
function isNative(s: string, name: string): boolean {
  return norma(s) === "function " + name + "() { [native code] }";
}
show("native_String", isNative(String.toString(), "String"));
show("native_Array", isNative(Array.toString(), "Array"));
show("native_wrong_name", isNative(Array.toString(), "String"));
show("source_text_has_name", Function.prototype.toString.call(plain).includes("plain"));
show("arrow_source_has_arrow", arrow.toString().includes("=>"));
show("method_source_starts_with_name", method.toString().startsWith("m("));
