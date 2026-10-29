// Symbol coercion and the conversion hooks around it: implicit string and number
// conversion of a symbol always throws; Symbol.toPrimitive hints and fallbacks;
// the ordinary valueOf/toString order; Date's default hint; Symbol.hasInstance;
// and how Object.prototype.toString reads Symbol.toStringTag.

function bad(label: string, fn: () => any): void {
  try { fn(); console.log(label + "=no_throw"); }
  catch (e: any) { console.log(label + "=" + e.constructor.name); }
}
function show(label: string, fn: () => any): void {
  try { const v = fn(); console.log(label + "=" + typeof v + ":" + String(v)); }
  catch (e: any) { console.log(label + "=" + e.constructor.name); }
}

// --- implicit conversion of a symbol to a string or a number always throws ---
const named: any = Symbol("hello");
bad("implicit_concat", () => named + "");
bad("implicit_concat_left", () => "" + named);
bad("implicit_template", () => `${named}`);
bad("implicit_unary_plus", () => +named);
bad("implicit_number_ctor", () => Number(named));
bad("implicit_minus", () => named - 1);
bad("implicit_multiply", () => named * 2);
bad("implicit_bitnot", () => ~named);
bad("implicit_relational", () => named < "z");
bad("implicit_parseInt", () => parseInt(named));
bad("implicit_array_join", () => [named].join(""));
bad("implicit_compound_assign", () => { let x: any = named; x += 1; return x; });

// --- what does NOT throw: boolean context, identity, and loose equality with a string ---
console.log("loose_eq_string=" + ((named == "Symbol(hello)") as boolean));
console.log("truthy=" + Boolean(named) + ":" + (named ? "y" : "n") + ":not=" + !named);
console.log("strict_eq=" + (named === named) + ":" + (named === Symbol("hello")));
console.log("loose_eq_self=" + (named == named) + ":null=" + (named == null) + ":undefined=" + (named == undefined));
console.log("typeof_named=" + typeof named);

// --- a wrapper object converts explicitly through toString, but not implicitly ---
const wrapper: any = Object(named);
console.log("wrapper_toString=" + wrapper.toString());
console.log("wrapper_valueOf_same=" + (wrapper.valueOf() === named));
bad("wrapper_implicit_concat", () => wrapper + "");
bad("wrapper_String_call", () => String(wrapper));

// --- Symbol.toPrimitive: each coercion site asks for its own hint ---
const seen: string[] = [];
const hinted: any = {
  [Symbol.toPrimitive](hint: string) {
    seen.push(hint);
    return hint === "number" ? 4 : hint === "string" ? "S" : "D";
  },
};
console.log("hint_unary_plus=" + (+hinted) + ":" + seen.join(","));
seen.length = 0;
console.log("hint_String_call=" + String(hinted) + ":" + seen.join(","));
seen.length = 0;
console.log("hint_addition=" + (hinted + "!") + ":" + seen.join(","));
seen.length = 0;
console.log("hint_template=" + `${hinted}` + ":" + seen.join(","));
seen.length = 0;
console.log("hint_loose_equality=" + (hinted == "D") + ":" + seen.join(","));
seen.length = 0;
console.log("hint_relational=" + (hinted < 5) + ":" + seen.join(","));
seen.length = 0;
console.log("hint_subtraction=" + (hinted - 1) + ":" + seen.join(","));
seen.length = 0;
console.log("hint_property_key=" + Object.keys({ [hinted]: 1 }).join("") + ":" + seen.join(","));

// --- Symbol.toPrimitive returning a Money-like value through the same hook ---
class Money {
  amount: number;
  currency: string;
  constructor(amount: number, currency: string) {
    this.amount = amount;
    this.currency = currency;
  }
  [Symbol.toPrimitive](hint: string) {
    if (hint === "number") return this.amount;
    if (hint === "string") return this.amount + " " + this.currency;
    return this.amount;
  }
}
const price = new Money(9.99, "USD");
console.log("money_number=" + (+price));
console.log("money_template=" + `${price}`);
console.log("money_default=" + ((price as any) + 0.01));

// --- a Symbol.toPrimitive that counts its calls, and a value with no hook at all ---
const counter: any = {
  n: 0,
  [Symbol.toPrimitive](hint: string) {
    this.n++;
    return hint === "string" ? "count:" + this.n : this.n;
  },
};
console.log("counter_first=" + (+counter));
console.log("counter_second=" + (+counter));
console.log("counter_template=" + `${counter}` + ":n=" + counter.n);

// --- a hook that returns an object, an array, or a symbol ---
const objHook: any = { [Symbol.toPrimitive]() { return { boxed: true }; } };
bad("hook_object_default", () => objHook + "");
bad("hook_object_String", () => String(objHook));
bad("hook_object_unary_plus", () => +objHook);
bad("hook_array_return", () => ({ [Symbol.toPrimitive]() { return []; } } as any) + "");
const symHook: any = { [Symbol.toPrimitive]() { return Symbol("prim"); } };
console.log("hook_symbol_return_type=" + typeof symHook[Symbol.toPrimitive]("default"));
bad("hook_symbol_return_then_concat", () => symHook + "");

// --- a non-callable, non-nullish hook is refused before any fallback ---
const numHook: any = { [Symbol.toPrimitive]: 42, valueOf() { return 1; } };
bad("hook_non_callable_number", () => numHook + "");
const strHook: any = { [Symbol.toPrimitive]: "nope", toString() { return "t"; } };
bad("hook_non_callable_string", () => String(strHook));

// --- null and undefined hooks fall back to OrdinaryToPrimitive ---
const nullHook: any = { [Symbol.toPrimitive]: null, valueOf() { return 5; }, toString() { return "five"; } };
console.log("null_hook_number=" + (nullHook * 2));
console.log("null_hook_String=" + String(nullHook));
console.log("null_hook_default=" + (nullHook + ""));
const undefHook: any = { [Symbol.toPrimitive]: undefined, valueOf() { return 7; }, toString() { return "seven"; } };
console.log("undef_hook_number=" + (undefHook - 1));
console.log("undef_hook_template=" + `${undefHook}`);

// --- with no hook the order is valueOf-then-toString for number and default,
//     toString-then-valueOf for string ---
const order: string[] = [];
const spy: any = {
  valueOf() { order.push("valueOf"); return 3; },
  toString() { order.push("toString"); return "three"; },
};
const numberResult = spy * 1;
console.log("order_number=" + order.join(",") + "=>" + numberResult);
order.length = 0;
const stringResult = String(spy);
console.log("order_string=" + order.join(",") + "=>" + stringResult);
order.length = 0;
const defaultResult = spy + "";
console.log("order_default=" + order.join(",") + "=>" + defaultResult);

// --- when valueOf returns an object, toString is tried next; when both do, TypeError ---
const objValueOf: any = { valueOf() { return {}; }, toString() { return "fromToString"; } };
console.log("valueOf_object_falls_to_toString=" + (objValueOf + ""));
const bothObjects: any = { valueOf() { return {}; }, toString() { return {}; } };
bad("both_objects_default", () => bothObjects + "");
bad("both_objects_number", () => bothObjects * 1);

// --- the hook is called with exactly one argument: the hint, as a string ---
const hints: string[] = [];
const hintSpy: any = {
  [Symbol.toPrimitive](h: any) { hints.push(String(h) + "/" + typeof h + "/" + arguments.length); return 1; },
};
const hintA = hintSpy + 1;
const hintB = `${hintSpy}`;
const hintC = +hintSpy;
const hintD = hintSpy == 1;
const hintE = hintSpy < 2;
console.log("hint_arguments=" + hints.join(",") + ":results=" + hintA + "," + hintB + "," + hintC + "," + hintD + "," + hintE);

// --- the coercion order of an addition and of a relational comparison ---
const seqAdd: string[] = [];
const left: any = { [Symbol.toPrimitive]() { seqAdd.push("left"); return "L"; } };
const right: any = { [Symbol.toPrimitive]() { seqAdd.push("right"); return 2; } };
console.log("add_result=" + (left + right) + ":order=" + seqAdd.join(","));
const seqRel: string[] = [];
const ra: any = { [Symbol.toPrimitive]() { seqRel.push("a"); return 1; } };
const rb: any = { [Symbol.toPrimitive]() { seqRel.push("b"); return 2; } };
console.log("rel_lt=" + (ra < rb) + ":order=" + seqRel.join(","));
seqRel.length = 0;
console.log("rel_gt=" + (ra > rb) + ":order=" + seqRel.join(","));

// --- abstract equality coerces an object against a primitive, never two objects ---
let valueOfCalls = 0;
const valueObj: any = { valueOf() { valueOfCalls++; return 5; } };
console.log("eq_number=" + (valueObj == 5) + ":eq_string=" + (valueObj == "5") + ":eq_true=" + (valueObj == true));
console.log("eq_calls=" + valueOfCalls);
console.log("eq_two_objects=" + (valueObj == { valueOf() { return 5; } }));

// --- Date: default hint behaves like string, unary plus like number ---
const dt = new Date(Date.UTC(2020, 0, 2, 3, 4, 5));
console.log("date_default_is_string=" + ((dt as any) + "" === dt.toString()));
console.log("date_number_is_time=" + (+dt === dt.getTime()));
console.log("date_template_is_string=" + (`${dt}` === dt.toString()));
console.log("date_hook_name_length=" + (Date.prototype as any)[Symbol.toPrimitive].name + ":" + (Date.prototype as any)[Symbol.toPrimitive].length);
bad("date_hook_bad_hint", () => (Date.prototype as any)[Symbol.toPrimitive].call(dt, "weird"));
bad("date_hook_missing_hint", () => (Date.prototype as any)[Symbol.toPrimitive].call(dt));
bad("date_hook_plain_receiver", () => (Date.prototype as any)[Symbol.toPrimitive].call({}, "string"));
console.log("date_relational=" + (dt > new Date(0)));

// --- Symbol.hasInstance: a static method on a class, and on a plain object ---
class Even {
  static [Symbol.hasInstance](v: any): boolean {
    return typeof v === "number" && v % 2 === 0;
  }
}
console.log("hasInstance_class=" + ((2 as any) instanceof Even) + ":" + ((3 as any) instanceof Even) +
  ":" + (("4" as any) instanceof Even) + ":" + ((new Number(2) as any) instanceof Even) + ":" + (({} as any) instanceof Even));
const evens: any = { [Symbol.hasInstance](v: any) { return typeof v === "number" && v % 2 === 0; } };
console.log("hasInstance_plain_object=" + ((4 as any) instanceof evens) + ":" + ((5 as any) instanceof evens) + ":" + ((null as any) instanceof evens));
console.log("hasInstance_rhs_typeof=" + typeof evens);

// --- the return value is coerced with ToBoolean ---
const truthyRet: any = { [Symbol.hasInstance]() { return "yes"; } };
const zeroRet: any = { [Symbol.hasInstance]() { return 0; } };
const undefRet: any = { [Symbol.hasInstance]() { return undefined; } };
const nanRet: any = { [Symbol.hasInstance]() { return NaN; } };
const objRet: any = { [Symbol.hasInstance]() { return {}; } };
console.log("coerce_string=" + ((1 as any) instanceof truthyRet));
console.log("coerce_zero=" + ((1 as any) instanceof zeroRet));
console.log("coerce_undefined=" + ((1 as any) instanceof undefRet));
console.log("coerce_nan=" + ((1 as any) instanceof nanRet));
console.log("coerce_object=" + ((1 as any) instanceof objRet));
console.log("coerce_result_type=" + typeof ((1 as any) instanceof truthyRet));

// --- the hook receives the LEFT operand as its argument and the right-hand side as this ---
let seenArg = "";
let seenThis = "";
const spyRhs: any = {
  mark: "rhs",
  [Symbol.hasInstance](v: any) {
    seenArg = typeof v + ":" + String(v);
    seenThis = this.mark;
    return true;
  },
};
console.log("hook_result=" + (({ k: 1 } as any) instanceof spyRhs));
console.log("hook_arg=" + seenArg + ":this=" + seenThis);
const base: any = { [Symbol.hasInstance]() { return true; } };
const derived: any = Object.create(base);
console.log("hasInstance_inherited=" + ((1 as any) instanceof derived));

// --- a constructor's own @@hasInstance replaces the prototype walk until it is removed ---
function Ordinary() { /* plain constructor */ }
const inst: any = new (Ordinary as any)();
console.log("default_walk=" + (inst instanceof (Ordinary as any)));
Object.defineProperty(Ordinary, Symbol.hasInstance, { value: () => false, configurable: true });
console.log("overridden_false=" + (inst instanceof (Ordinary as any)));
delete (Ordinary as any)[Symbol.hasInstance];
console.log("restored_walk=" + (inst instanceof (Ordinary as any)));
const nulled: any = function NulledCtor() { /* ctor */ };
Object.defineProperty(nulled, Symbol.hasInstance, { value: undefined, configurable: true });
const ni: any = new nulled();
console.log("hook_undefined_falls_back=" + (ni instanceof nulled));
Object.defineProperty(nulled, Symbol.hasInstance, { value: null, configurable: true });
console.log("hook_null_falls_back=" + (ni instanceof nulled));

// --- the default hook on Function.prototype is non-writable and non-configurable ---
const fd: any = Object.getOwnPropertyDescriptor(Function.prototype, Symbol.hasInstance);
console.log("default_hook_type=" + typeof fd.value + ":" + fd.value.name + ":len=" + fd.value.length);
console.log("default_hook_flags=" + fd.writable + ":" + fd.enumerable + ":" + fd.configurable);
console.log("default_hook_direct=" + fd.value.call(Ordinary, inst) + ":" + fd.value.call(Ordinary, {}));

// --- refusals: a right-hand side that is not callable and has no hook, or an arrow ---
bad("rhs_plain_object", () => (1 as any) instanceof ({} as any));
bad("rhs_number", () => (1 as any) instanceof (5 as any));
bad("rhs_string", () => (1 as any) instanceof ("s" as any));
bad("rhs_null", () => (1 as any) instanceof (null as any));
bad("rhs_undefined", () => (1 as any) instanceof (undefined as any));
bad("hook_not_callable", () => (1 as any) instanceof ({ [Symbol.hasInstance]: 42 } as any));
bad("hook_throws", () => (1 as any) instanceof ({ [Symbol.hasInstance]() { throw new RangeError("no"); } } as any));
const arrow: any = () => 1;
bad("arrow_rhs_no_prototype", () => ({} as any) instanceof arrow);

// --- Symbol.toStringTag read by Object.prototype.toString: a plain Get, so getters run ---
let tagGets = 0;
const dyn: any = {};
Object.defineProperty(dyn, Symbol.toStringTag, { get() { tagGets++; return "Dyn" + tagGets; }, configurable: true });
console.log("tag_getter_first=" + Object.prototype.toString.call(dyn));
console.log("tag_getter_second=" + Object.prototype.toString.call(dyn) + ":gets=" + tagGets);
const boom: any = {};
Object.defineProperty(boom, Symbol.toStringTag, { get() { throw new RangeError("tag"); }, configurable: true });
bad("tag_getter_throws", () => Object.prototype.toString.call(boom));
console.log("tag_getter_throw_object_intact=" + typeof boom);

// --- a non-string tag is ignored in favour of the built-in tag; a string tag wins ---
function tagged(v: any): string {
  const o: any = {};
  Object.defineProperty(o, Symbol.toStringTag, { value: v, configurable: true });
  return Object.prototype.toString.call(o);
}
const tagCases: [string, any][] = [
  ["number", 42], ["null", null], ["undefined", undefined], ["boolean", true],
  ["object", {}], ["symbol", Symbol("s")], ["empty_string", ""], ["string", "Ok"],
];
for (const [label, v] of tagCases) console.log("tag_" + label + "=" + tagged(v));

// --- the built-in tag comes from the internal slot, not from the prototype ---
const builtinCases: [string, any][] = [
  ["array", []], ["function", function () { /* fn */ }], ["arrow", () => 1],
  ["error", new Error("x")], ["type_error", new TypeError("x")], ["date", new Date(0)],
  ["regexp", /x/], ["boolean_box", new Boolean(true)], ["number_box", new Number(1)],
  ["string_box", new String("s")], ["null", null], ["undefined", undefined],
  ["number", 1], ["string", "s"], ["null_prototype", Object.create(null)],
];
for (const [label, v] of builtinCases) console.log("builtin_" + label + "=" + Object.prototype.toString.call(v));
console.log("builtin_arguments=" + (function () { return Object.prototype.toString.call(arguments); })());

// --- an array with a string tag: the tag changes toString only, not Array.isArray or join ---
const taggedArray: any = [1, 2];
Object.defineProperty(taggedArray, Symbol.toStringTag, { value: "NotArray", configurable: true });
console.log("array_tagged=" + Object.prototype.toString.call(taggedArray));
console.log("array_isArray=" + Array.isArray(taggedArray) + ":join=" + taggedArray.join(","));

// --- the tag is inherited through the prototype chain, and a class may define it as a getter ---
const tagProto: any = {};
Object.defineProperty(tagProto, Symbol.toStringTag, { value: "Inherited", configurable: true });
const child: any = Object.create(tagProto);
console.log("inherited_tag=" + Object.prototype.toString.call(child) + ":own_symbols=" + Object.getOwnPropertySymbols(child).length);
class TaggedClass {
  get [Symbol.toStringTag]() { return "TaggedClass"; }
}
console.log("class_getter_tag=" + Object.prototype.toString.call(new TaggedClass()));
console.log("class_getter_enumerable=" + (Object.getOwnPropertyDescriptor(TaggedClass.prototype, Symbol.toStringTag) as any).enumerable);
console.log("toString_length_name=" + Object.prototype.toString.length + ":" + Object.prototype.toString.name);
