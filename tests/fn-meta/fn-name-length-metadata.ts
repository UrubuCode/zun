// Function metadata: `name` and `length` as real own properties.
// Covers declarations, expressions, arrows, methods, accessors, classes, bound
// functions, defaults/rest/destructuring in `length`, name inference across
// binding/property/default positions, descriptor attributes, redefinition and
// deletion, and the "bound " prefix chain.
// The export marks this file as a module so both runtimes run it in strict mode.
export {};

const show = (label: string, value: unknown): void => {
  console.log(label + "=" + JSON.stringify(value));
};

// ---------------------------------------------------------------------------
// 1. Declared, expression, arrow and method shapes
// ---------------------------------------------------------------------------
function foo(a: number, b: number): number { return a + b; }
const anonExpr = function (x: number): number { return x * 2; };
const arrow3 = (a: number, b: number, c: number): number => a + b + c;
const noArgs = (): number => 42;
const holder = {
  m(a: number, b: number, c: number): number { return a + b + c; },
  arrowProp: (a: number): number => a,
};

show("decl_name", foo.name);
show("decl_length", foo.length);
show("expr_name", anonExpr.name);
show("expr_length", anonExpr.length);
show("arrow_name", arrow3.name);
show("arrow_length", arrow3.length);
show("noargs_length", noArgs.length);
show("method_name", holder.m.name);
show("method_length", holder.m.length);
show("prop_arrow_name", holder.arrowProp.name);
show("prop_arrow_length", holder.arrowProp.length);

// Named function expression: its declared name wins over the binding name.
const explicitWins = function realName(): void {};
show("explicit_wins", explicitWins.name);

// ---------------------------------------------------------------------------
// 2. length: stops at the first default, excludes rest, counts a pattern as one
// ---------------------------------------------------------------------------
function three(a: number, b: number, c: number): number { return a + b + c; }
function withDefault(a: number, b: number = 2, c?: number): number { return a + b + (c ?? 0); }
function withRest(a: number, ...rest: number[]): number { return a + rest.length; }
function restOnly(...all: number[]): number { return all.length; }
function patterns({ a }: any, [b]: any, c: number): number { return a + b + c; }
function defaultsThenRest(a: number, b: number = 1, ...rest: number[]): number { return a + b + rest.length; }
show("length_plain", three.length);
show("length_default_stops", withDefault.length);
show("length_rest_excluded", withRest.length);
show("length_rest_only", restOnly.length);
show("length_pattern_counts", patterns.length);
show("length_default_then_rest", defaultsThenRest.length);

// ---------------------------------------------------------------------------
// 3. Accessors, classes and static members
// ---------------------------------------------------------------------------
const accessors: any = {
  get width(): number { return 1; },
  set width(v: number) { void v; },
  get readOnly(): number { return 2; },
};
const wDesc = Object.getOwnPropertyDescriptor(accessors, "width") as any;
show("getter_name", wDesc.get.name);
show("setter_name", wDesc.set.name);
show("getter_length", wDesc.get.length);
show("setter_length", wDesc.set.length);
show("readonly_has_setter", (Object.getOwnPropertyDescriptor(accessors, "readOnly") as any).set === undefined);

class Shaped {
  constructor(a: number, b: number) { void a; void b; }
  static get kind(): string { return "static"; }
  get size(): number { return 3; }
  set size(v: number) { void v; }
  method(x: number): number { return x; }
  static stat(x: number, y: number): number { return x + y; }
}
show("class_name", Shaped.name);
show("class_length", Shaped.length);
show("static_method_name", Shaped.stat.name);
show("static_method_length", Shaped.stat.length);
show("instance_method_name", Shaped.prototype.method.name);
show("ctor_proto_name", Shaped.prototype.constructor.name);
show("static_getter_name", (Object.getOwnPropertyDescriptor(Shaped, "kind") as any).get.name);
show("proto_getter_name", (Object.getOwnPropertyDescriptor(Shaped.prototype, "size") as any).get.name);
show("proto_setter_name", (Object.getOwnPropertyDescriptor(Shaped.prototype, "size") as any).set.name);

// Symbol keys: "[description]"; a description-less symbol gives "".
const described = Symbol("tagName");
const bareSym = Symbol();
const symKeyed: any = {
  [described](): number { return 1; },
  [bareSym](): number { return 2; },
};
show("symbol_described", symKeyed[described].name);
show("symbol_bare", symKeyed[bareSym].name);
class Iterable {
  *[Symbol.iterator](): Generator<number> { yield 1; }
}
show("wellknown_symbol_name", (Iterable.prototype as any)[Symbol.iterator].name);

// Computed string keys take the computed value.
const dyn = "computedName";
const computed: any = { [dyn](): number { return 1; }, [dyn + "2"]: (): number => 2 };
show("computed_method", computed[dyn].name);
show("computed_arrow", computed[dyn + "2"].name);

// ---------------------------------------------------------------------------
// 4. Name inference: where an anonymous function picks a name up, and where not
// ---------------------------------------------------------------------------
const fromConst = function (): void {};
let fromLet = (): void => {};
let assignedLater: any;
assignedLater = (): void => {};
show("const_binding", fromConst.name);
show("let_arrow", fromLet.name);
show("plain_assign_to_existing", assignedLater.name);

const holderInfer: any = {};
holderInfer.member = function (): void {};
const listInfer: any[] = [];
listInfer[0] = (): void => {};
show("member_assign", holderInfer.member.name);
show("index_assign", listInfer[0].name);

show("parenthesised", (function (): void {}).name);
show("comma_expression", (0, function (): void {}).name);

let orTarget: any = null;
orTarget ||= (): void => {};
let nullishTarget: any = null;
nullishTarget ??= function (): void {};
let andTarget: any = 1;
andTarget &&= (): void => {};
show("or_assign", orTarget.name);
show("nullish_assign", nullishTarget.name);
show("and_assign", andTarget.name);

const [arrayDefault = (): void => {}] = [] as any[];
const { objectDefault = function (): void {} }: any = {};
show("array_default", arrayDefault.name);
show("object_default", objectDefault.name);

function paramDefaultNames(cb: any = (): void => {}): string { return cb.name; }
show("param_default", paramDefaultNames());
show("param_supplied", paramDefaultNames(function given(): void {}));

const literal: any = {
  plain: function (): void {},
  arrow: (): void => {},
  method(): void {},
  get acc(): number { return 1; },
  nested: { inner: (): void => {} },
};
show("literal_plain", literal.plain.name);
show("literal_arrow", literal.arrow.name);
show("literal_method", literal.method.name);
show("literal_nested_inner", literal.nested.inner.name);
show("literal_getter", (Object.getOwnPropertyDescriptor(literal, "acc") as any).get.name);

function returnsAnonymous(): any { return (): void => {}; }
function passThrough(f: any): any { return f; }
show("returned_anonymous", returnsAnonymous().name);
show("passed_through", passThrough((): void => {}).name);

const AnonClass = class {};
const NamedClass = class Inner {};
show("anon_class_expr", AnonClass.name);
show("named_class_expr", NamedClass.name);

const inArray: any[] = [(): void => {}, function (): void {}];
show("in_array_arrow", inArray[0].name);
show("in_array_expr", inArray[1].name);

// A name is fixed once, at creation: moving or re-assigning does not rename.
let moved: any = function (): void {};
const copied = moved;
moved = null;
show("copy_keeps_name", copied.name);
let renamedBinding: any = function (): void {};
const firstFn = renamedBinding;
renamedBinding = function (): void {};
show("first_keeps_name", firstFn.name);
show("second_name", renamedBinding.name);

// A function made inside a descriptor literal is named after `value`.
const defined: any = {};
Object.defineProperty(defined, "prop", { value: (): void => {}, configurable: true });
show("define_property_value", defined.prop.name);

// Bound functions prefix whatever name the target had.
show("bound_named", fromConst.bind(null).name);
show("bound_anonymous", (0, function (): void {}).bind(null).name);

// ---------------------------------------------------------------------------
// 5. Descriptors: writable false, enumerable false, configurable true
// ---------------------------------------------------------------------------
function attrs(fn: any, key: string): string {
  const d: any = Object.getOwnPropertyDescriptor(fn, key);
  if (!d) return "absent";
  return "w=" + d.writable + " e=" + d.enumerable + " c=" + d.configurable + " v=" + JSON.stringify(d.value);
}
function two(a: number, b: number): number { return a + b; }
show("decl_name_attrs", attrs(two, "name"));
show("decl_length_attrs", attrs(two, "length"));
show("arrow_length_attrs", attrs((a: number): number => a, "length"));
show("class_name_attrs", attrs(Shaped, "name"));
show("own_trio", ["length", "name", "prototype"]
  .map((k) => k + ":" + Object.prototype.hasOwnProperty.call(two, k)).join(","));
show("enumerable_keys", Object.keys(two));
show("set_name_accepted", Reflect.set(two, "name", "renamed"));
show("name_after_refused_set", two.name);
show("set_length_accepted", Reflect.set(two, "length", 9));
show("length_after_refused_set", two.length);

function redefinable(a: number): number { return a; }
Object.defineProperty(redefinable, "name", { value: "given-a-new-name" });
Object.defineProperty(redefinable, "length", { value: 7 });
show("redefined_name", redefinable.name);
show("redefined_length", redefinable.length);
show("redefined_still_configurable", (Object.getOwnPropertyDescriptor(redefinable, "name") as any).configurable);

function odd(): void {}
Object.defineProperty(odd, "name", { value: 123 });
show("odd_name", [odd.name, typeof odd.name]);

function loosened(): void {}
Object.defineProperty(loosened, "name", { value: "start", writable: true, configurable: true });
show("loosened_set", [Reflect.set(loosened, "name", "changed"), loosened.name]);

// Deleting `name` and `length` falls through to Function.prototype.
function deletable(): void {}
show("before_delete_name", deletable.name);
show("delete_name_accepted", Reflect.deleteProperty(deletable, "name"));
show("has_own_name_after", Object.prototype.hasOwnProperty.call(deletable, "name"));
show("after_delete_name", deletable.name);
show("delete_length_accepted", Reflect.deleteProperty(deletable, "length"));
show("length_after_delete", [deletable.length, Function.prototype.length]);
show("proto_name", Function.prototype.name);

const stored: any = { fn: two };
show("stored_name_length", [stored.fn.name, stored.fn.length]);

// ---------------------------------------------------------------------------
// 6. Bound functions: "bound " prefix stacks, length subtracts and clamps at 0
// ---------------------------------------------------------------------------
const b0 = three.bind(null);
const b1 = three.bind(null, 1);
const b2 = three.bind(null, 1, 2);
const b3 = three.bind(null, 1, 2, 3);
const b5: any = (three as any).bind(null, 1, 2, 3, 4, 5);
show("b0", [b0.name, b0.length]);
show("b1", [b1.name, b1.length]);
show("b2_length", b2.length);
show("b3_length", b3.length);
show("b5_length_clamped", b5.length);
show("b5_result", b5());

const twice = b1.bind(null, 2);
const thrice: any = twice.bind(null, 3);
show("twice", [twice.name, twice.length]);
show("thrice", [thrice.name, thrice.length, thrice()]);

const anon = function (): number { return 1; };
const nameless = (function (): any { return function (): number { return 0; }; })();
show("anon_bound_name", anon.bind(null).name);
show("nameless_bound", nameless.bind(null).name);
show("arrow_bound", [arrow3.bind(null).name, arrow3.bind(null, 2).length]);

function withDefaultLen(a: number, b: number = 2, c?: number): number { return a + b + (c ?? 0); }
show("default_bound_len", [withDefaultLen.bind(null, 1).length, withDefaultLen.bind(null, 1, 2).length]);
show("rest_bound_len", [withRest.bind(null, 1).length, withRest.length]);

const nd = Object.getOwnPropertyDescriptor(b1, "name") as any;
const ld = Object.getOwnPropertyDescriptor(b1, "length") as any;
show("bound_name_attrs", [nd.writable, nd.enumerable, nd.configurable]);
show("bound_length_attrs", [ld.writable, ld.configurable]);
show("bound_own_keys", Object.getOwnPropertyNames(b1).sort());
show("bound_has_prototype", Object.prototype.hasOwnProperty.call(b1, "prototype"));
show("target_has_prototype", Object.prototype.hasOwnProperty.call(three, "prototype"));
show("bound_proto_is_target_proto", Object.getPrototypeOf(b1) === Object.getPrototypeOf(three));

// Class method, bound and stored, keeps its method name.
class Counter {
  n = 3;
  read(): number { return this.n; }
}
const cInst = new Counter();
show("method_bound_name", cInst.read.bind(cInst).name);
show("method_bound_value", cInst.read.bind(cInst)());

// The metadata of a bound function is configurable and never enumerable.
show("bound_keys_enumerable", Object.keys(b1));
