// Boolean coercion across every value kind: Boolean() as function vs
// constructor, ToBoolean's closed falsy list (eight primitives, nothing else),
// the proof that ToBoolean never calls user code, the == / === coercion
// tables, comparison and negation results, and the wrapper objects in
// coercing positions. One labelled case per line.

function out(label: string, value: unknown): void {
  console.log(label + "=" + String(value));
}

function attempt(label: string, fn: () => unknown): void {
  try {
    out(label, fn());
  } catch (e) {
    console.log(label + "!" + (e as any).constructor.name);
  }
}

// ---------------------------------------------------------------------------
// 1. Boolean as a function versus Boolean as a constructor
// ---------------------------------------------------------------------------
console.log("--- constructor and function ---");
out("ctor_typeof", typeof Boolean);
out("ctor_name", Boolean.name);
out("ctor_length", Boolean.length);
out("ctor_proto_is_Function", Object.getPrototypeOf(Boolean) === Function.prototype);
out("ctor_own_keys", Object.keys(Boolean).length);
out("call_no_args", Boolean());
out("call_typeof_zero", typeof Boolean(0));
out("call_tag", Object.prototype.toString.call(Boolean(false)));
out("new_typeof", typeof new Boolean(false));
out("new_tag", Object.prototype.toString.call(new Boolean(false)));
out("new_no_args_valueOf", new Boolean().valueOf());
out("new_no_args_truthy", new Boolean() ? "truthy" : "falsy");
out("new_false_valueOf", new Boolean(false).valueOf());
out("new_false_toString", new Boolean(false).toString());
out("new_false_truthy", new Boolean(false) ? "truthy" : "falsy");
out("new_false_eq_false", new Boolean(false) == false);
out("new_false_strict_false", (new Boolean(false) as any) === false);
out("new_true_eq_new_true", new Boolean(true) == new Boolean(true));
out("new_instanceof_Boolean", new Boolean(true) instanceof Boolean);
out("primitive_instanceof_Boolean", (true as any) instanceof Boolean);
out("reflect_construct_valueOf", Reflect.construct(Boolean, [1]).valueOf());

const protoDesc = Object.getOwnPropertyDescriptor(Boolean, "prototype") as PropertyDescriptor;
out("prototype_flags_wec", [protoDesc.writable, protoDesc.enumerable, protoDesc.configurable].join(","));
out("prototype_constructor_identity", Boolean.prototype.constructor === Boolean);
out("prototype_typeof", typeof Boolean.prototype);
out("prototype_truthy", Boolean.prototype ? "truthy" : "falsy");
out("prototype_tag", Object.prototype.toString.call(Boolean.prototype));
out("prototype_valueOf", Boolean.prototype.valueOf());
out("prototype_toString", Boolean.prototype.toString());
out("prototype_parent_is_Object", Object.getPrototypeOf(Boolean.prototype) === Object.prototype);
out("valueOf_shape", (Boolean.prototype.valueOf as any).length + ":" + (Boolean.prototype.valueOf as any).name);
out("toString_shape", (Boolean.prototype.toString as any).length + ":" + (Boolean.prototype.toString as any).name);

// Brand checks: only a genuine Boolean receiver (primitive or wrapper) passes.
attempt("brand_valueOf_prim_true", () => (Boolean.prototype.valueOf as any).call(true));
attempt("brand_valueOf_wrapper_false", () => (Boolean.prototype.valueOf as any).call(new Boolean(false)));
attempt("brand_valueOf_number", () => (Boolean.prototype.valueOf as any).call(1));
attempt("brand_valueOf_number_wrapper", () => (Boolean.prototype.valueOf as any).call(new Number(1)));
attempt("brand_valueOf_string_wrapper", () => (Boolean.prototype.valueOf as any).call(new String("true")));
attempt("brand_valueOf_object", () => (Boolean.prototype.valueOf as any).call({}));
attempt("brand_valueOf_null", () => (Boolean.prototype.valueOf as any).call(null));
attempt("brand_valueOf_undefined", () => (Boolean.prototype.valueOf as any).call(undefined));
attempt("brand_valueOf_faked_slot", () => (Boolean.prototype.valueOf as any).call({ valueOf: () => true }));
attempt("brand_toString_object", () => (Boolean.prototype.toString as any).call({}));
attempt("brand_toString_prim_false", () => (Boolean.prototype.toString as any).call(false));
const detachedToString: any = Boolean.prototype.toString;
out("detached_apply_true", detachedToString.apply(true, []));
out("detached_call_false", detachedToString.call(false));
out("reflect_apply_wrapper", Reflect.apply(Boolean.prototype.valueOf, new Boolean(true), []));

// Subclassing keeps the internal slot and the prototype chain.
class Flag extends Boolean {
  describe(): string {
    return this.valueOf() ? "on" : "off";
  }
}
const flagOn = new Flag(true);
const flagOff = new Flag(0 as any);
out("subclass_values", flagOn.valueOf() + "," + flagOff.valueOf());
out("subclass_describe", flagOn.describe() + "," + flagOff.describe());
out("subclass_tag", Object.prototype.toString.call(flagOn));
out("subclass_instanceof", (flagOn instanceof Flag) + "," + (flagOn instanceof Boolean));
out("subclass_truthy_off", flagOff ? "truthy" : "falsy");
out("subclass_toString", flagOn.toString() + "," + flagOff.toString());

// Boolean() reads the argument's slot, never its valueOf.
const trap: any = { valueOf: () => false };
out("Boolean_of_valueOf_trap", Boolean(trap));
out("Boolean_of_wrapper_false", Boolean(new Boolean(false)));
out("Boolean_of_boxed_zero", Boolean(new Number(0)));
out("Boolean_extra_argument_ignored", Boolean(false, true as any));

// ---------------------------------------------------------------------------
// 2. Wrapper objects in coercing positions: valueOf answers, so a false
//    wrapper can still act as false in arithmetic and loose equality.
// ---------------------------------------------------------------------------
console.log("--- wrapper in coercing positions ---");
const falseBox: any = new Boolean(false);
out("box_plus_zero", falseBox + 0);
out("box_template", `${falseBox}`);
out("box_loose_zero", falseBox == 0);
out("box_loose_empty", falseBox == "");
out("box_negated", !falseBox);
out("box_and_right", falseBox && "reached");
out("box_nullish_keeps_object", falseBox ?? "never");
out("box_json_property", JSON.stringify({ b: falseBox }));
out("box_json_alone", JSON.stringify(falseBox));
out("box_own_keys", Object.keys(falseBox).length);
falseBox.extra = 3;
out("box_extra_keys", Object.keys(falseBox).join(","));

// ---------------------------------------------------------------------------
// 3. ToBoolean over every value kind: five spellings must all agree, and the
//    Proxy spy records zero trap calls, proving ToBoolean never coerces.
// ---------------------------------------------------------------------------
console.log("--- truthiness table ---");
const calls: string[] = [];

const spyHandler: ProxyHandler<any> = {
  get(t, k) {
    calls.push("get:" + String(k));
    return Reflect.get(t, k);
  },
  has(t, k) {
    calls.push("has:" + String(k));
    return Reflect.has(t, k);
  },
  ownKeys(t) {
    calls.push("ownKeys");
    return Reflect.ownKeys(t);
  },
  getOwnPropertyDescriptor(t, k) {
    calls.push("gopd:" + String(k));
    return Reflect.getOwnPropertyDescriptor(t, k);
  },
  getPrototypeOf(t) {
    calls.push("getPrototypeOf");
    return Reflect.getPrototypeOf(t);
  },
};

const lying: any = {
  valueOf() {
    calls.push("valueOf");
    return false;
  },
  toString() {
    calls.push("toString");
    return "";
  },
};
const primitiveHook: any = {
  [Symbol.toPrimitive](hint: string) {
    calls.push("toPrimitive:" + hint);
    return false;
  },
};
const spy: any = new Proxy({ valueOf: () => false }, spyHandler);
const revocable = Proxy.revocable({}, {});
revocable.revoke();

const truthyKinds: [string, unknown][] = [
  ["false", false],
  ["true", true],
  ["zero", 0],
  ["neg_zero", -0],
  ["zero_bigint", 0n],
  ["empty_string", ""],
  ["nan", NaN],
  ["null", null],
  ["undefined", undefined],
  ["space", " "],
  ["zero_string", "0"],
  ["false_string", "false"],
  ["neg_one", -1],
  ["min_value", Number.MIN_VALUE],
  ["neg_bigint", -1n],
  ["neg_infinity", -Infinity],
  ["empty_array", []],
  ["array_zero", [0]],
  ["empty_object", {}],
  ["null_proto_object", Object.create(null)],
  ["function", function () {}],
  ["arrow", () => 0],
  ["class", class {}],
  ["boolean_false_wrapper", new Boolean(false)],
  ["number_zero_wrapper", new Number(0)],
  ["number_nan_wrapper", new Number(NaN)],
  ["string_empty_wrapper", new String("")],
  ["bigint_zero_wrapper", Object(0n)],
  ["symbol", Symbol("s")],
  ["empty_map", new Map()],
  ["empty_set", new Set()],
  ["regexp", /(?:)/],
  ["error", new Error("x")],
  ["empty_typed_array", new Uint8Array(0)],
  ["array_buffer", new ArrayBuffer(0)],
  ["date_epoch", new Date(0)],
  ["resolved_promise_false", Promise.resolve(false)],
  ["valueOf_false_object", lying],
  ["toPrimitive_false_object", primitiveHook],
  ["proxy_spy", spy],
  ["revoked_proxy", revocable.proxy],
];

const falsyLabels: string[] = [];
for (const [label, v] of truthyKinds) {
  calls.length = 0;
  const viaTernary = v ? "T" : "F";
  const viaNot = !v;
  const viaBoolean = Boolean(v);
  const viaDouble = !!v;
  const agree = viaBoolean === viaDouble && viaDouble === (v ? true : false) && viaNot === !viaBoolean;
  if (!viaBoolean) falsyLabels.push(label);
  console.log(
    label +
      " | ternary:" + viaTernary +
      " | not:" + viaNot +
      " | Boolean:" + viaBoolean +
      " | double_not:" + viaDouble +
      " | agree:" + agree +
      " | calls:[" + calls.join(",") + "]"
  );
}
out("falsy_labels", falsyLabels.join(","));
out("falsy_count", falsyLabels.length);

// The same object in contexts that DO coerce, for contrast.
calls.length = 0;
out("plus_one_lying", lying + 1);
console.log("plus_one_calls=[" + calls.join(",") + "]");
calls.length = 0;
out("loose_false_lying", lying == false);
console.log("loose_false_calls=[" + calls.join(",") + "]");
calls.length = 0;
out("less_than_one_lying", lying < 1);
console.log("less_than_calls=[" + calls.join(",") + "]");
calls.length = 0;
out("template_lying", `${lying}`);
console.log("template_calls=[" + calls.join(",") + "]");
calls.length = 0;
out("plus_one_hook", primitiveHook + 1);
console.log("hook_plus_calls=[" + calls.join(",") + "]");
calls.length = 0;
out("spy_property_read", spy.valueOf());
console.log("spy_read_calls=[" + calls.join(",") + "]");
calls.length = 0;
out("filter_Boolean_objects", [lying, primitiveHook, 0, "", false, {}].filter(Boolean).length);
console.log("filter_Boolean_calls=[" + calls.join(",") + "]");

// ---------------------------------------------------------------------------
// 4. Loose and strict equality tables. Object.create(null) cannot be
//    converted to a primitive, so its == comparisons throw TypeError.
// ---------------------------------------------------------------------------
console.log("--- equality tables ---");
function probe(f: () => boolean): string {
  try {
    return String(f());
  } catch (e) {
    return "!" + (e as any).constructor.name;
  }
}

const equalityRows: [string, any][] = [
  ["false", false],
  ["true", true],
  ["zero", 0],
  ["neg_zero", -0],
  ["one", 1],
  ["neg_one", -1],
  ["tenth", 0.1],
  ["nan", NaN],
  ["infinity", Infinity],
  ["zero_bigint", 0n],
  ["one_bigint", 1n],
  ["empty_string", ""],
  ["space_string", " "],
  ["zero_string", "0"],
  ["one_string", "1"],
  ["two_string", "2"],
  ["false_string", "false"],
  ["null", null],
  ["undefined", undefined],
  ["empty_array", []],
  ["array_zero", [0]],
  ["array_one", [1]],
  ["array_two_elems", [1, 2]],
  ["array_nested_empty", [[]]],
  ["array_empty_string", [""]],
  ["empty_object", {}],
  ["boolean_false_wrapper", new Boolean(false)],
  ["number_zero_wrapper", new Number(0)],
  ["string_empty_wrapper", new String("")],
  ["bigint_zero_wrapper", Object(0n)],
  ["symbol", Symbol("s")],
  ["date_epoch", new Date(0)],
  ["null_proto_object", Object.create(null)],
];

for (const [label, v] of equalityRows) {
  console.log(
    label +
      " | Boolean:" + Boolean(v) +
      " | ==true:" + probe(() => v == true) +
      " | ==false:" + probe(() => v == false) +
      " | ==1:" + probe(() => v == 1) +
      " | ==0:" + probe(() => v == 0) +
      " | ===true:" + (v === true) +
      " | ===0:" + (v === 0)
  );
}

// ---------------------------------------------------------------------------
// 5. Comparison and arithmetic facts that all produce booleans or numbers
// ---------------------------------------------------------------------------
console.log("--- comparisons ---");
out("nan_loose_nan", NaN == NaN);
out("nan_strict_nan", NaN === NaN);
out("nan_ne_nan", NaN != NaN);
out("object_is_nan", Object.is(NaN, NaN));
out("neg_zero_strict_zero", -0 === 0);
out("object_is_neg_zero", Object.is(-0, 0));
out("nan_lt_one", NaN < 1);
out("nan_ge_one", NaN >= 1);
out("null_eq_undefined", null == undefined);
out("null_strict_undefined", null === undefined);
out("null_loose_zero", null == 0);
out("null_ge_zero", null >= 0);
out("null_gt_zero", null > 0);
out("undefined_loose_zero", undefined == 0);
out("undefined_ge_zero", undefined >= 0);
out("string_lt_string", "10" < "9");
out("string_lt_number", "10" < (9 as any));
out("numeric_string_eq", "1e1" == (10 as any));
out("empty_string_eq_zero", "" == (0 as any));
out("whitespace_eq_zero", " \t\n" == (0 as any));
out("bigint_loose_number", 1n == (1 as any));
out("bigint_strict_number", (1n as any) === 1);
out("bigint_gt_fraction", 2n > (1.5 as any));
out("bool_plus_bool", (true as any) + (true as any));
out("bool_plus_string", (true as any) + "1");
out("bool_minus_one", (true as any) - 1);
out("false_lt_string_one", (false as any) < "1");
out("true_loose_string_one", (true as any) == "1");
out("true_loose_string_two", (true as any) == "2");
out("true_loose_one_array", (true as any) == [1] as any);
out("array_true_loose_true", [true] == (true as any));

// ---------------------------------------------------------------------------
// 6. Negation and double negation: ! always yields a real boolean
// ---------------------------------------------------------------------------
console.log("--- negation ---");
out("not_empty_array", ![]);
out("not_empty_object", !{});
out("double_not_neg_zero", !!-0);
out("double_not_nan", !!NaN);
out("triple_not_nan", !!!NaN);
out("double_not_space", !!" ");
out("double_not_string_zero", !!"0");
out("double_not_string_false", !!"false");
out("not_zero_bigint", !0n);
out("not_neg_bigint", !-1n);
out("not_null", !null);
out("not_undefined", !undefined);
out("double_not_symbol", !!Symbol("s"));
out("typeof_double_not", typeof !!"x");
out("de_morgan_and", !(true && false) === (!true || !false));
out("de_morgan_or", !(false || true) === (!false && !true));
out("not_not_identity_bool", !!true === true);

// ---------------------------------------------------------------------------
// 7. Ternary conditions and selected values
// ---------------------------------------------------------------------------
console.log("--- ternary ---");
out("ternary_zero", 0 ? "a" : "b");
out("ternary_empty_array", [] ? "a" : "b");
out("ternary_nested_falsy_chain", 0 ? "x" : "" ? "y" : "z");
out("ternary_right_assoc", true ? 1 : false ? 2 : 3);
out("ternary_mid_branch", false ? 1 : true ? 2 : 3);
out("ternary_result_type", typeof (1 ? "s" : 0));
out("ternary_wrapper_false", new Boolean(false) ? "truthy" : "falsy");
out("ternary_comparison_condition", 2 > 1 ? "yes" : "no");

// ---------------------------------------------------------------------------
// 8. Boolean as a predicate in array methods
// ---------------------------------------------------------------------------
console.log("--- array predicates ---");
const mixed: unknown[] = [0, "", null, "x", [], NaN, 1n, false, undefined];
out("filter_Boolean_count", mixed.filter(Boolean).length);
out("filter_Boolean_typeofs", mixed.filter(Boolean).map((v) => typeof v).join(","));
out("some_falsy", mixed.some((v) => !v));
out("every_truthy_empty", mixed.every(Boolean));
out("every_truthy_objects", [1, "a", {}, []].every(Boolean));
out("some_falsy_clean", [1, "a", 2].some((v) => !v));
