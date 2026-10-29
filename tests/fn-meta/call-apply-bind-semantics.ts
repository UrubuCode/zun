// Function.prototype.call / apply / bind and Reflect.apply in strict code.
// Covers receivers (primitives unboxed, null/undefined kept), array-like
// argument lists and their coercions, TypeError/RangeError cases, bound
// functions (partial application, rebinding rules, bind of bind), the
// "uncurried this" idiom built from call.bind, and the metadata of the
// derived functions.
export {};

const show = (label: string, value: unknown): void => {
  console.log(label + "=" + JSON.stringify(value));
};

function tryOf(fn: () => unknown): string {
  try {
    return "ok:" + String(fn());
  } catch (e) {
    return "threw:" + (e as any).name;
  }
}

// ---------------------------------------------------------------------------
// 1. call and apply: explicit receivers and positional arguments
// ---------------------------------------------------------------------------
function greet(this: any, greeting: string, name: string): string {
  return greeting + " " + name + (this && this.suffix ? this.suffix : "");
}
const obj = { suffix: "!" };
show("call", greet.call(obj, "Hello", "World"));
show("apply", greet.apply(obj, ["Hi", "There"]));
show("call_no_suffix", greet.call({}, "Yo", "You"));
show("call_null_receiver", greet.call(null, "Hey", "You"));
show("apply_null_receiver", greet.apply(undefined, ["Hey", "All"]));

function describeThis(this: any, a: number, b: number): string {
  return this.name + ":" + (a + b);
}
show("call_context", describeThis.call({ name: "ctx" }, 2, 5));
show("call_other_context", describeThis.call({ name: "other" }, -1, 4));
show("call_and_apply_same", describeThis.call({ name: "R" }, 1, 2) === describeThis.apply({ name: "R" }, [1, 2]));

// The receiver of a method reaches its fields through call and apply.
class Named {
  label = "instance";
  show(): string { return "label=" + (this as any).label; }
}
const namedProto: any = Named.prototype;
show("instance_receiver", namedProto.show.call(new Named()));
show("foreign_receiver", namedProto.show.call({ label: "foreign" }));
show("undefined_receiver", tryOf(() => namedProto.show.call(undefined)));
show("call_is_shared", Function.prototype.call === Function.prototype.call);
show("call_meta", [Function.prototype.call.name, Function.prototype.call.length, Function.prototype.apply.length]);
show("bind_meta", [Function.prototype.bind.name, Function.prototype.bind.length]);

// ---------------------------------------------------------------------------
// 2. Receivers in strict code: primitives are not boxed, null stays null
// ---------------------------------------------------------------------------
class Receiver {
  method(...tail: any[]): string {
    const self: any = this;
    return typeof self + ":" + String(self) + (tail.length ? "|args=" + tail.join(",") : "");
  }
  static describe(...tail: any[]): string {
    const self: any = this;
    return typeof self + ":" + String(self) + (tail.length ? "|args=" + tail.join(",") : "");
  }
}
const proto: any = Receiver.prototype;
show("prim_number", proto.method.call(7));
show("prim_string", proto.method.call("s"));
show("prim_boolean", proto.method.call(true));
show("prim_symbol_type", typeof proto.method.call(Symbol("k")));
show("null_receiver", proto.method.call(null));
show("undefined_receiver", proto.method.call(undefined));
show("no_receiver", proto.method.call());

class Identity {
  same(v: any): boolean { return (this as any) === v; }
}
const idProto: any = Identity.prototype;
show("primitive_not_boxed", idProto.same.call(7, 7));
show("string_not_boxed", idProto.same.call("s", "s"));
const str = "shared";
show("string_identity", idProto.same.call(str, str));
const sym = Symbol("id");
show("symbol_identity", idProto.same.call(sym, sym));
show("object_identity", idProto.same.call(proto, proto));

const bag: any = { toString(): string { return "[bag]"; } };
show("call_object", proto.method.call(bag, 1, 2));
show("apply_object", proto.method.apply(bag, [3, 4]));
show("static_default", Receiver.describe().slice(0, 8));
show("static_call_null", Receiver.describe.call(null));
show("call_empty", proto.method.call(bag));
show("apply_empty", proto.method.apply(bag, []));

// ---------------------------------------------------------------------------
// 3. apply's second argument: array-likes, null/undefined, and refusals
// ---------------------------------------------------------------------------
function count(...args: any[]): string {
  return args.length + ":[" + args.map((a) => String(a)).join(",") + "]";
}
show("apply_array", count.apply(null, [1, 2, 3] as any));
show("apply_arraylike", count.apply(null, { 0: "a", 1: "b", length: 2 } as any));
show("apply_extra_ignored", count.apply(null, { 0: "a", 1: "b", 2: "c", length: 2 } as any));
show("apply_holes_undefined", count.apply(null, { 0: "a", length: 3 } as any));
show("length_string", count.apply(null, { 0: "a", 1: "b", length: "2" } as any));
show("length_float", count.apply(null, { 0: "a", 1: "b", length: 2.9 } as any));
show("length_negative", count.apply(null, { 0: "a", length: -1 } as any));
show("length_missing", count.apply(null, { 0: "a" } as any));
show("length_nan", count.apply(null, { 0: "a", length: NaN } as any));
show("length_bool", count.apply(null, { 0: "a", 1: "b", length: true } as any));
show("set_is_not_arraylike", count.apply(null, new Set([1, 2, 3]) as any));
show("null_args", count.apply(null, null as any));
show("undefined_args", count.apply(null, undefined as any));
show("omitted_args", count.apply(null));

function applyWith(args: any): string {
  try {
    (count as any).apply(null, args);
    return "ok";
  } catch (e) {
    return (e as any).name;
  }
}
show("apply_number", applyWith(1));
show("apply_boolean", applyWith(true));
show("apply_string", applyWith("abc"));
show("apply_symbol", applyWith(Symbol("s")));
show("apply_bigint", applyWith(10n));
show("apply_boxed_string_ok", applyWith(Object("abc")));
show("apply_huge_length", applyWith({ length: 2 ** 53 - 1 }));
show("apply_infinite_length", applyWith({ length: Infinity }));

// A getter on `length` runs exactly once; index getters run in order.
let lengthReads = 0;
const watched: any = {
  0: "x",
  1: "y",
  get length(): number { lengthReads += 1; return 2; },
};
show("length_getter", count.apply(null, watched));
show("length_reads", lengthReads);
const order: string[] = [];
const ordered: any = {
  get 0() { order.push("i0"); return "a"; },
  get 1() { order.push("i1"); return "b"; },
  get 2() { order.push("i2"); return "c"; },
  length: 3,
};
show("ordered", count.apply(null, ordered));
show("index_order", order);

// Inherited indices are visible through the prototype chain.
const base: any = { 1: "inherited" };
const derived: any = Object.create(base);
derived[0] = "own";
derived.length = 2;
show("inherited_index", count.apply(null, derived));

// `arguments` is itself an array-like that apply accepts.
function forward(): string { return count.apply(null, arguments as any); }
show("forward_arguments", forward(1, 2 as any, 3 as any));

// apply and call with the same receiver, short and long lists.
function receiverPair(this: any, a: any, b: any): string {
  return this.tag + "|" + a + "|" + b;
}
show("pair_call", receiverPair.call({ tag: "R" }, 1, 2));
show("pair_apply", receiverPair.apply({ tag: "R" }, [1, 2]));
show("pair_apply_short", receiverPair.apply({ tag: "R" }, [1] as any));
show("pair_apply_long", receiverPair.apply({ tag: "R" }, [1, 2, 3] as any));

// Reflect.apply: the same operation, and it needs a real array-like.
show("reflect_apply", Reflect.apply(Math.max, null, [2, 7]));
show("reflect_arraylike", Reflect.apply(count, null, { 0: 1, length: 1 } as any));
show("reflect_null_list", tryOf(() => (Reflect as any).apply(count, null, null)));

// ---------------------------------------------------------------------------
// 4. bind: partial application, metadata, and rebinding rules
// ---------------------------------------------------------------------------
function multiply(this: any, a: number, b: number): number {
  return a * b * (this.factor || 1);
}
const bound = multiply.bind({ factor: 2 });
show("bound_call", bound(3, 4));
show("original_call", multiply.call({ factor: 1 }, 3, 4));
const doubled = multiply.bind({ factor: 1 }, 2);
show("partial", doubled(5));
const rebound = bound.bind({ factor: 10 });
show("rebind_keeps_first_receiver", rebound(3, 4));

function sum3(this: any, a: number, b: number, c: number): number {
  return this.base + a + b + c;
}
const partialSum = sum3.bind({ base: 10 }, 1, 2);
show("partial_sum", [partialSum(3), partialSum(-3), partialSum.length]);

// A call with another receiver cannot move a bound function.
show("bound_ignores_call_receiver", partialSum.call({ base: 999 }, 3));
show("bound_ignores_apply_receiver", partialSum.apply({ base: 999 }, [3] as any));

// Rebinding a bound function: the first receiver wins, arguments accumulate.
function show2(this: any, ...args: any[]): string { return this.name + ":" + args.join(","); }
const firstArgs = show2.bind({ name: "first" }, "a");
const secondArgs = firstArgs.bind({ name: "second" }, "b");
show("rebind_accumulates", secondArgs("c"));
show("rebind_with_call", secondArgs.call({ name: "third" }, "d"));
show("rebind_name", secondArgs.name);

// bind applied to itself: Function.prototype.bind.bind(f) is a binder.
function greetRecv(this: any, suffix: string): string {
  const self: any = this;
  return (self && self.tag ? self.tag : "no-this") + suffix;
}
const target: any = { tag: "target" };
const other: any = { tag: "other" };
const bindOfGreet: any = Function.prototype.bind.bind(greetRecv);
const boundViaBindBind = bindOfGreet(target);
show("bindbind_result", boundViaBindBind("-x"));
show("bindbind_name", boundViaBindBind.name);

const viaCall: any = Function.prototype.bind.call(greetRecv, target, "-fixed");
show("bind_via_call", viaCall());
show("bind_via_call_length", viaCall.length);

const binder: any = Function.prototype.bind.bind(Function.prototype.bind);
show("binder_factory", binder(greetRecv)(target, "-A")());

const firstBind = greetRecv.bind(target);
const secondBind = firstBind.bind(other);
show("rebind_keeps_this", secondBind("-y"));
show("zero_bound_this", greetRecv.bind(undefined)("-z"));
show("two_binds_differ", greetRecv.bind(target) !== greetRecv.bind(target));

(greetRecv as any).marker = "on-target";
show("bound_sees_marker", String((greetRecv.bind(target) as any).marker));
show("target_keeps_marker", (greetRecv as any).marker);

// A bound method used as a callback keeps its receiver.
class Reporter {
  tag: string;
  constructor(tag: string) { this.tag = tag; }
  describe(suffix: string): string { return this.tag + suffix; }
}
const inst = new Reporter("R");
const boundMethod = inst.describe.bind(inst);
show("bound_callback", ["-1", "-2"].map(boundMethod));
show("unbound_callback", tryOf(() => ["-3"].map(inst.describe)));

// ---------------------------------------------------------------------------
// 5. Uncurried this: Function.prototype.call.bind(f)
// ---------------------------------------------------------------------------
const hasOwn: any = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
const sample: any = { present: 1 };
show("hasOwn_true", hasOwn(sample, "present"));
show("hasOwn_false", hasOwn(sample, "missing"));
show("hasOwn_on_bare", hasOwn(Object.create(null), "anything"));
show("derived_name", hasOwn.name);
show("derived_length", hasOwn.length);
show("call_length", Function.prototype.call.length);
show("target_length", Object.prototype.hasOwnProperty.length);

const uncurry: any = Function.prototype.bind.bind(Function.prototype.call);
const toStringOf: any = uncurry(Object.prototype.toString);
show("tag_array", toStringOf([]));
show("tag_null", toStringOf(null));
show("tag_number", toStringOf(3));
show("tag_map", toStringOf(new Map()));

const slice: any = Function.prototype.call.bind(Array.prototype.slice);
const join: any = Function.prototype.call.bind(Array.prototype.join);
const arrayLike: any = { length: 3, 0: "a", 1: "b", 2: "c" };
show("borrowed_slice", slice(arrayLike, 1));
show("borrowed_join", join(arrayLike, "-"));
show("borrowed_on_string", join(Object("xyz"), "+"));

const push: any = Function.prototype.call.bind(Array.prototype.push);
const pseudoArray: any = { length: 0 };
push(pseudoArray, "one");
push(pseudoArray, "two");
show("borrowed_push", [pseudoArray.length, pseudoArray[0], pseudoArray[1], Array.isArray(pseudoArray)]);

const applyOf: any = Function.prototype.apply.bind(Math.max);
show("apply_bound_max", applyOf(null, [3, 9, 4]));
show("apply_bound_max_empty", applyOf(null, []));
const maxOf: any = Function.prototype.apply.bind(Math.max, null);
show("max_of_array", maxOf([1, 8, 2]));
show("min_of_array", Function.prototype.apply.bind(Math.min, null)([1, 8, 2]));
show("call_call", Function.prototype.call.call(Object.prototype.hasOwnProperty, sample, "present"));
show("apply_call", Function.prototype.apply.call(Math.max, null, [5, 6]));

const numberToFixed: any = Function.prototype.call.bind(Number.prototype.toFixed);
show("borrowed_toFixed", numberToFixed(1.2345, 2));
show("borrowed_wrong_receiver", tryOf(() => numberToFixed("1.2345", 2)));
show("distinct_helpers", Function.prototype.call.bind(Object.prototype.hasOwnProperty) !==
  Function.prototype.call.bind(Object.prototype.hasOwnProperty));

class Counter {
  n = 0;
  bump(by: number): number { this.n += by; return this.n; }
}
const bump: any = Function.prototype.call.bind(Counter.prototype.bump);
const counter = new Counter();
show("borrowed_class_method", [bump(counter, 2), bump(counter, 3)]);
show("borrowed_on_plain", bump({ n: 10 } as any, 5));

const getterSource: any = {
  _v: "inner",
  get value(): string { return "read:" + this._v; },
};
const getValue: any = Function.prototype.call.bind(
  (Object.getOwnPropertyDescriptor(getterSource, "value") as any).get,
);
show("borrowed_getter", getValue(getterSource));
show("borrowed_getter_foreign", getValue({ _v: "foreign" }));

// The idiom applied to bind itself produces a binder.
const binderOf: any = Function.prototype.call.bind(Function.prototype.bind);
function shout(this: any, word: string): string {
  const self: any = this;
  return (self && self.prefix ? self.prefix : "-") + word;
}
show("binder_of", binderOf(shout, { prefix: ">>" })("hi"));

show("call_on_bound_ignores_receiver", Function.prototype.call.call(
  shout.bind({ prefix: "**" }), { prefix: "??" }, "x"));
const asMethod: any = { hasOwn };
show("helper_as_method", asMethod.hasOwn(sample, "present"));

// A helper keeps working after its target gains a property.
(Object.prototype.hasOwnProperty as any).marker = "late";
show("helper_unaffected", [hasOwn(sample, "present"), (Object.prototype.hasOwnProperty as any).marker]);
