// Number namespace and the Number wrapper: constants and their descriptors, the
// predicates (no coercion) versus the globals (coercing), the shape of the
// constructor and prototype, wrapper objects, subclassing, brand checks, and
// method dispatch on a missing member.

function attempt(label: string, fn: () => unknown): void {
  try {
    console.log(label + "=" + String(fn()));
  } catch (e) {
    console.log(label + "!" + (e as Error).name);
  }
}

// --- constants ---
console.log("MAX_SAFE_INTEGER=" + Number.MAX_SAFE_INTEGER);
console.log("MIN_SAFE_INTEGER=" + Number.MIN_SAFE_INTEGER);
console.log("MAX_VALUE=" + Number.MAX_VALUE);
console.log("MIN_VALUE=" + Number.MIN_VALUE);
console.log("EPSILON=" + Number.EPSILON);
console.log("POSITIVE_INFINITY=" + Number.POSITIVE_INFINITY);
console.log("NEGATIVE_INFINITY=" + Number.NEGATIVE_INFINITY);
console.log("NaN=" + Number.NaN);
console.log("EPSILON_is_2_pow_neg52=" + (Number.EPSILON === Math.pow(2, -52)));
console.log("MIN_SAFE_is_neg_MAX_SAFE=" + (Number.MIN_SAFE_INTEGER === -Number.MAX_SAFE_INTEGER));
console.log("POSITIVE_INFINITY_is_Infinity=" + (Number.POSITIVE_INFINITY === Infinity));
console.log("NaN_is_not_self_equal=" + (Number.NaN !== Number.NaN));
console.log("MIN_VALUE_is_subnormal=" + (Number.MIN_VALUE / 2 === 0));

// --- constant descriptors: non-writable, non-enumerable, non-configurable data ---
const constNames = [
  "MAX_SAFE_INTEGER", "MIN_SAFE_INTEGER", "MAX_VALUE", "MIN_VALUE",
  "EPSILON", "POSITIVE_INFINITY", "NEGATIVE_INFINITY", "NaN",
];
for (const name of constNames) {
  const d = Object.getOwnPropertyDescriptor(Number, name) as PropertyDescriptor;
  console.log("descriptor " + name + " flags=" + [d.writable, d.enumerable, d.configurable].join(",") + " isData=" + ("value" in d));
}
console.log("reflect_set_MAX_VALUE=" + Reflect.set(Number, "MAX_VALUE", 1));
console.log("MAX_VALUE_unchanged=" + (Number.MAX_VALUE === 1.7976931348623157e308));
console.log("reflect_delete_EPSILON=" + Reflect.deleteProperty(Number, "EPSILON"));
console.log("EPSILON_still_present=" + typeof Number.EPSILON);

// --- the constructor object ---
console.log("typeof_Number=" + typeof Number);
console.log("ctor_name=" + Number.name);
console.log("ctor_length=" + Number.length);
console.log("ctor_own_keys_empty=" + (Object.keys(Number).length === 0));
console.log("ctor_proto_is_Function_prototype=" + (Object.getPrototypeOf(Number) === Function.prototype));
const protoDesc = Object.getOwnPropertyDescriptor(Number, "prototype") as PropertyDescriptor;
console.log("prototype_flags=" + [protoDesc.writable, protoDesc.enumerable, protoDesc.configurable].join(","));

// --- statics: parsers are the very same function objects as the globals ---
console.log("parseInt_is_global=" + (Number.parseInt === parseInt));
console.log("parseFloat_is_global=" + (Number.parseFloat === parseFloat));
console.log("isNaN_is_not_global=" + (Number.isNaN !== (globalThis as any).isNaN));
console.log("isFinite_is_not_global=" + (Number.isFinite !== (globalThis as any).isFinite));
const statics = ["isFinite", "isInteger", "isNaN", "isSafeInteger", "parseFloat", "parseInt"];
for (const s of statics) {
  const fn = (Number as any)[s];
  console.log("static " + s + " typeof=" + typeof fn + " length=" + fn.length + " name=" + fn.name);
}

// --- prototype members: shape and arity ---
const protoMethods = ["toExponential", "toFixed", "toPrecision", "toString", "valueOf"];
for (const p of protoMethods) {
  const fn = (Number.prototype as any)[p];
  console.log("proto " + p + " typeof=" + typeof fn + " length=" + fn.length + " name=" + fn.name);
}
console.log("proto_own_keys_empty=" + (Object.keys(Number.prototype).length === 0));
console.log("constructor_identity=" + (Number.prototype.constructor === Number));
console.log("proto_tag=" + Object.prototype.toString.call(Number.prototype));
console.log("proto_value=" + String(Number.prototype.valueOf()) + " is_pos_zero=" + Object.is(Number.prototype.valueOf(), 0));
console.log("proto_proto_is_Object_prototype=" + (Object.getPrototypeOf(Number.prototype) === Object.prototype));
console.log("proto_toFixed_on_prototype=" + Number.prototype.toFixed(2));

// --- predicates do not coerce; globals do ---
const samples: [string, any][] = [
  ["nan", NaN], ["string_NaN", "NaN"], ["infinity", Infinity], ["five", 5],
  ["string_five", "5"], ["null", null], ["undefined", undefined], ["half", 0.5],
  ["whole_float", 3.0], ["bool_true", true], ["empty_string", ""],
];
for (const [label, v] of samples) {
  console.log(
    "pred " + label +
      " isNaN=" + Number.isNaN(v) +
      " isFinite=" + Number.isFinite(v) +
      " isInteger=" + Number.isInteger(v) +
      " isSafeInteger=" + Number.isSafeInteger(v) +
      " globalIsNaN=" + isNaN(v) +
      " globalIsFinite=" + isFinite(v)
  );
}

// --- safe-integer boundaries ---
const MS = Number.MAX_SAFE_INTEGER;
const edges: [string, number][] = [
  ["max_minus_1", MS - 1], ["max", MS], ["max_plus_1", MS + 1],
  ["min", Number.MIN_SAFE_INTEGER], ["min_minus_1", Number.MIN_SAFE_INTEGER - 1],
  ["two_pow_53", 2 ** 53], ["neg_zero", -0], ["neg_one", -1],
  ["five_and_half", 5.5], ["one_e21", 1e21], ["nan", NaN],
];
for (const [label, v] of edges) {
  console.log("safe " + label + "=" + Number.isSafeInteger(v) + " integer=" + Number.isInteger(v));
}
console.log("max_plus_1_eq_max_plus_2=" + (MS + 1 === MS + 2));

// --- call versus construct; wrappers unwrap to primitives under operators ---
console.log("call_typeof=" + typeof Number(5));
console.log("new_typeof=" + typeof new Number(5));
console.log("new_tag=" + Object.prototype.toString.call(new Number(5)));
console.log("new_loose_eq_5=" + (new Number(5) == 5));
console.log("new_strict_eq_5=" + ((new Number(5) as any) === 5));
console.log("new_instanceof=" + (new Number(5) instanceof Number) + " primitive_instanceof=" + ((5 as any) instanceof Number));
console.log("new_no_args_value=" + String(new Number().valueOf()));
console.log("call_no_args=" + String(Number()) + " is_pos_zero=" + Object.is(Number(), 0));
console.log("call_undefined=" + String(Number(undefined)));
console.log("call_null=" + String(Number(null)));
console.log("call_neg_zero_kept=" + Object.is(Number(-0), -0));
console.log("wrapper_plus_zero=" + (new Number(12.5) + 0));
console.log("wrapper_add_wrapper=" + (((new Number(1)) as any) + new Number(2)));

// --- subclassing keeps the internal slot ---
class Counted extends Number {
  extra(): string {
    return "x" + this.valueOf();
  }
}
const sub = new Counted(7);
console.log("subclass_valueOf=" + String(sub.valueOf()));
console.log("subclass_extra=" + sub.extra());
console.log("subclass_tag=" + Object.prototype.toString.call(sub));
console.log("subclass_plus_one=" + String((sub as any) + 1));
console.log("subclass_instanceof=" + (sub instanceof Counted) + "," + (sub instanceof Number));

// --- prototype methods brand-check their receiver ---
attempt("valueOf_on_object", () => Number.prototype.valueOf.call({}));
attempt("valueOf_on_null", () => Number.prototype.valueOf.call(null));
attempt("toString_on_string", () => Number.prototype.toString.call("5"));
attempt("toFixed_on_wrapper", () => Number.prototype.toFixed.call(new Number(1.234), 2));
attempt("toString_on_primitive_radix", () => Number.prototype.toString.call(255, 16));
attempt("valueOf_on_bigint", () => Number.prototype.valueOf.call(1n));
attempt("toFixed_on_boolean", () => Number.prototype.toFixed.call(true, 1));
attempt("toString_on_symbol", () => Number.prototype.toString.call(Symbol("s")));
attempt("toFixed_on_inherited", () => Number.prototype.toFixed.call(Object.create(Number.prototype), 2));
attempt("toFixed_on_subclass", () => Number.prototype.toFixed.call(sub, 1));

// --- the brand check runs before the argument is coerced ---
const spyLog: string[] = [];
const spy: any = {
  valueOf() {
    spyLog.push("coerced");
    return 2;
  },
};
attempt("bad_receiver_spy", () => Number.prototype.toFixed.call("nope", spy));
console.log("spy_after_bad_receiver=[" + spyLog.join(",") + "]");
attempt("good_receiver_spy", () => Number.prototype.toFixed.call(3.14159, spy));
console.log("spy_after_good_receiver=[" + spyLog.join(",") + "]");

// --- an object that merely inherits from Number.prototype is not a Number ---
const fake = Object.create(Number.prototype);
attempt("fake_valueOf", () => Number.prototype.valueOf.call(fake));
attempt("fake_toFixed", () => Number.prototype.toFixed.call(fake, 2));

// --- a missing member is a TypeError at the call site, not a compile failure ---
let missing = "NO-THROW";
try {
  (5 as any).missingMethod();
} catch (e) {
  missing = (e as Error).name;
}
console.log("missing_method=" + missing);
let missingWithArgs = "NO-THROW";
try {
  (7 as any).otherMissing(1, 2);
} catch (e) {
  missingWithArgs = (e as Error).name;
}
console.log("missing_method_with_args=" + missingWithArgs);
console.log("real_surface_toFixed=" + (3.14159).toFixed(2));
console.log("real_surface_toString16=" + (255).toString(16));
console.log("real_surface_toLocaleString_is_function=" + (typeof (1).toLocaleString === "function"));
