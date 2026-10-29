// Consolidated BigInt fixture: the BigInt() conversion rules, toString radix,
// the Number-mixing TypeErrors, loose vs strict equality, switch and collection
// identity, property keys, wrapper objects, and the constructor's shape.
// Deterministic output only; deliberate throws print e.name.

function op(label: string, fn: () => any): void {
  try {
    console.log(label + "=" + String(fn()));
  } catch (e) {
    console.log(label + "!" + (e as any).name);
  }
}

// --- BigInt(): string forms (StringToBigInt) ---
function big(label: string, v: any): void {
  try {
    console.log(label + "=" + String(BigInt(v)) + "n");
  } catch (e) {
    console.log(label + "!" + (e as any).name);
  }
}
big("plain", "10");
big("padded", "  10  ");
big("tabbed", "\t10\n");
big("empty", "");
big("blank", "   ");
big("bom_wrapped", "﻿10﻿");
big("plus", "+10");
big("minus", "-10");
big("minus_zero", "-0");
big("leading_zeros", "007");
big("inner_space", "1 0");
big("trailing_junk", "10x");
big("long", "123456789012345678901234567890");
big("hex_lower", "0x1f");
big("hex_upper", "0X1F");
big("binary", "0b101");
big("octal", "0o17");
big("hex_signed", "-0x1f");
big("hex_plus", "+0x1f");
big("bare_0x", "0x");
big("legacy_octal", "017");
big("fraction", "1.5");
big("trailing_dot", "1.");
big("exponent", "1e3");
big("separator", "1_0");
big("bigint_suffix", "1n");
big("infinity", "Infinity");
big("nan_string", "NaN");

// --- BigInt(): Number arguments must be integral and finite ---
big("num_int", 10);
big("num_neg_zero", -0);
big("num_fraction", 1.5);
big("num_tiny_fraction", 1.0000000000000002);
big("num_nan", NaN);
big("num_infinity", Infinity);
big("num_2pow53", 9007199254740992);
big("num_1e21", 1e21);
big("num_max_value", Number.MAX_VALUE);
big("num_min_value", Number.MIN_VALUE);

// --- BigInt(): other primitives and ToPrimitive(hint number) objects ---
big("true", true);
big("null", null);
big("undefined", undefined);
big("symbol", Symbol("s"));
big("bigint_itself", 7n);
big("empty_array", []);
big("array_one", [15]);
big("array_two", [1, 2]);
big("plain_object", {});
big("valueof_num", { valueOf: function () { return 7; } });
big("valueof_frac", { valueOf: function () { return 7.5; } });
big("valueof_string", { valueOf: function () { return "0x10"; } });
big("tostring_only", { toString: function () { return "42"; } });
big("number_wrapper", new Number(9));
big("string_wrapper", new String("9"));
big("bigint_wrapper", Object(9n));

// --- toString(radix): arbitrary precision and radix validation ---
const n255 = 255n;
console.log("r2=" + n255.toString(2) + " r8=" + n255.toString(8) + " r16=" + n255.toString(16) + " r36=" + n255.toString(36));
console.log("neg_hex=" + (-255n).toString(16) + " zero_hex=" + (0n).toString(16));
console.log("default_radix=" + n255.toString() + " explicit10=" + n255.toString(10) + " undef_radix=" + n255.toString(undefined));
console.log("float_radix=" + n255.toString(16.9) + " string_radix=" + n255.toString("16" as any));
op("radix_1", () => n255.toString(1));
op("radix_37", () => n255.toString(37));
op("radix_0", () => n255.toString(0));
const big128 = 2n ** 128n;
console.log("big_dec=" + big128.toString() + " big_hex=" + big128.toString(16));
console.log("big_bin_len=" + big128.toString(2).length + " big_minus_one_hex=" + (big128 - 1n).toString(16));
console.log("factorial25=" + [...Array(25)].reduce((a: bigint, _v, i) => a * BigInt(i + 1), 1n).toString());

// --- string contexts all use the decimal spelling ---
console.log("String=" + String(1234567890123456789012345n) + " template=" + `${-42n}` + " concat=" + ("v=" + 42n));
console.log("join=" + [1n, 2n, 3n].join("-") + " array_toString=" + String([1n, 2n]));
console.log("valueOf=" + (1n).valueOf() + " typeof_valueOf=" + typeof (1n).valueOf());
op("wrong_brand_number", () => (BigInt.prototype.toString as any).call(1));
console.log("wrapper_brand=" + BigInt.prototype.toString.call(Object(7n)));

// --- Number <-> BigInt: arithmetic refuses the mix, relations accept it ---
const b: any = 2n;
const n: any = 3;
op("bigint_plus_number", () => b + n);
op("number_plus_bigint", () => n + b);
op("bigint_minus_number", () => b - n);
op("bigint_times_number", () => b * n);
op("bigint_div_number", () => b / n);
op("bigint_mod_number", () => b % n);
op("bigint_pow_number", () => b ** n);
op("number_pow_bigint", () => n ** b);
op("bigint_plus_bool", () => b + (true as any));
op("bigint_plus_null", () => b + (null as any));
op("bigint_plus_undefined", () => b + (undefined as any));
op("bigint_plus_nan", () => b + (NaN as any));
op("and_mixed", () => b & n);
op("or_mixed", () => b | n);
op("xor_mixed", () => b ^ n);
op("shl_mixed", () => b << n);
op("shr_mixed", () => b >> n);
op("ushr_mixed", () => b >>> 1);
op("unary_plus", () => +b);
op("unary_tilde", () => ~b);
op("bigint_plus_string", () => b + "x");
op("string_plus_bigint", () => "x" + b);
op("bigint_plus_numeric_string", () => b + "3");
op("bigint_minus_string", () => b - ("3" as any));
op("bigint_plus_array", () => b + ([] as any));
op("bigint_plus_object", () => b + ({} as any));
op("lt", () => 1n < 2);
op("gt", () => 2n > 1);
op("le", () => 1n <= 1);
op("ge", () => 2n >= 3);
op("lt_float", () => 1n < 1.5);
op("gt_float", () => 2n > 1.5);
op("lt_nan", () => 1n < NaN);
op("gt_nan", () => 1n > NaN);
op("lt_infinity", () => (2n ** 1024n) < Infinity);
op("gt_neg_infinity", () => (-(2n ** 1024n)) > -Infinity);
op("lt_string", () => 1n < "2");
op("huge_vs_double", () => (2n ** 53n + 1n) > 9007199254740992);
op("math_abs", () => Math.abs(b));
op("math_sqrt", () => Math.sqrt(4n as any));
op("math_sign", () => Math.sign(b));
op("is_nan_of_bigint", () => isNaN(b));
op("number_is_integer", () => Number.isInteger(b));
op("parse_int_of_bigint", () => parseInt(b));
op("explicit_number", () => Number(2n));
op("explicit_boolean", () => Boolean(0n));
op("explicit_from_number", () => BigInt(2));

// --- loose equality crosses the type line; strict equality does not ---
op("loose_eq", () => 1n == 1);
op("loose_eq_float", () => 1n == 1.5);
op("strict_eq", () => 1n === (1 as any));
op("loose_eq_string", () => 1n == "1");
op("loose_eq_hex_string", () => 1n == "0x1");
op("loose_eq_bad_string", () => 1n == "1.5");
op("loose_eq_empty_string", () => 0n == "");
op("loose_eq_bool", () => 0n == false);
op("loose_eq_null", () => 0n == null);
op("loose_eq_undefined", () => 0n == undefined);
op("loose_eq_nan", () => 0n == NaN);
op("loose_eq_wrapper", () => 1n == (Object(1n) as any));

// --- the equality relations side by side ---
const pairs: [string, any, any][] = [
  ["1n_vs_1", 1n, 1],
  ["1n_vs_1n", 1n, 1n],
  ["1n_vs_'1'", 1n, "1"],
  ["1n_vs_true", 1n, true],
  ["0n_vs_false", 0n, false],
  ["0n_vs_''", 0n, ""],
  ["1n_vs_1.5", 1n, 1.5],
  ["2n_vs_2.0", 2n, 2.0],
  ["1n_vs_NaN", 1n, NaN],
  ["1n_vs_null", 1n, null],
  ["1n_vs_[1]", 1n, [1]],
  ["1n_vs_wrapper", 1n, Object(1n)],
];
for (const p of pairs) {
  console.log(
    p[0] +
      " | loose:" + ((p[1] as any) == (p[2] as any)) +
      " | strict:" + ((p[1] as any) === (p[2] as any)) +
      " | Object.is:" + Object.is(p[1], p[2]) +
      " | includes:" + [p[1]].includes(p[2] as any)
  );
}

// --- switch matches with strict equality: a numeric case never catches 1n ---
function classify(x: any): string {
  switch (x) {
    case 1:
      return "number_one";
    case 1n:
      return "bigint_one";
    case "1":
      return "string_one";
    case true:
      return "true";
    case 0:
      return "number_zero";
    case 0n:
      return "bigint_zero";
    default:
      return "default";
  }
}
const probes: [string, any][] = [
  ["1n", 1n],
  ["1", 1],
  ["'1'", "1"],
  ["true", true],
  ["0n", 0n],
  ["-0", -0],
  ["-0n", -0n],
  ["computed_1n", 0n + 1n],
  ["BigInt(1)", BigInt(1)],
  ["Number(1n)", Number(1n)],
  ["wrapper_1n", Object(1n)],
  ["2n", 2n],
];
for (const p of probes) console.log("switch(" + p[0] + ")=" + classify(p[1]));

function collect(x: any): string {
  const hits: string[] = [];
  switch (x) {
    case 1n:
      hits.push("one");
    case 2n:
      hits.push("two");
      break;
    case 3n:
      hits.push("three");
      break;
    default:
      hits.push("none");
  }
  return hits.join(">");
}
console.log("fallthrough_1n=" + collect(1n) + " fallthrough_2n=" + collect(2n) + " fallthrough_3n=" + collect(3n) + " fallthrough_1=" + collect(1));

function bucket(x: bigint): string {
  switch (true) {
    case x < 0n:
      return "negative";
    case x === 0n:
      return "zero";
    case x < 10:
      return "small";
    case x > 1e10:
      return "huge";
    default:
      return "medium";
  }
}
console.log("bucket=" + [-5n, 0n, 3n, 100n, 99999999999n].map((v) => bucket(v)).join(","));

const order: string[] = [];
function caseValue(name: string, v: any): any {
  order.push(name);
  return v;
}
switch (2n) {
  case caseValue("a", 1n):
    break;
  case caseValue("b", 2n):
    break;
  case caseValue("c", 3n):
    break;
}
console.log("case_evaluation_order=" + order.join(","));

// --- Map, Set and includes use SameValueZero: 1n and 1 stay distinct ---
const m = new Map<any, string>();
m.set(1n, "bigint");
m.set(1, "number");
m.set("1", "string");
console.log("map_size=" + m.size + " get_bigint=" + m.get(1n) + " get_number=" + m.get(1) + " get_string=" + m.get("1"));
console.log("map_get_other_bigint_instance=" + m.get(BigInt("1")));
console.log("set_dedup=" + new Set([1n, 1n, 1, "1", -0n, 0n]).size + " set_has_bigint=" + new Set([1n]).has(1n) + " set_has_number=" + new Set([1n]).has(1 as any));
console.log("includes_bigint=" + [1n, 2n].includes(1n) + " includes_number=" + ([1n, 2n] as any).includes(1) + " indexOf_bigint=" + [1n, 2n].indexOf(2n));
const mixed: any[] = [1, 1n, 2, 2n, "1", true];
console.log("indexOf_1=" + mixed.indexOf(1) + " indexOf_1n=" + mixed.indexOf(1n) + " lastIndexOf_1n=" + mixed.lastIndexOf(1n));
console.log("includes_2n=" + mixed.includes(2n) + " includes_2=" + mixed.includes(2));
console.log("filter_bigint=" + mixed.filter((v) => typeof v === "bigint").map(String).join(","));
console.log("count_loose_one=" + mixed.filter((v) => v == 1).length + " count_strict_one=" + mixed.filter((v) => v === 1).length);
console.log("object_is_cross_type=" + Object.is(1n, 1) + " object_is_neg_zero=" + Object.is(-0n, 0n) + " object_is_same=" + Object.is(1n, 1n));

// --- property keys: ToPropertyKey collapses 1n, 1 and "1" ---
const o: any = {};
o[1n] = "via-bigint";
console.log("read_by_number=" + o[1] + " read_by_string=" + o["1"]);
o[1] = "via-number";
console.log("read_by_bigint_after=" + o[1n] + " keys=" + Object.keys(o).join(",") + " key_typeof=" + typeof Object.keys(o)[0]);
o[2n ** 70n as any] = "huge";
console.log("huge_key=" + Object.keys(o).join("|"));
const arr: any = ["a", "b", "c"];
console.log("array_by_bigint=" + arr[1n]);
arr[3n] = "d";
console.log("array_len_after_bigint=" + arr.length + " array_join=" + arr.join(","));

// --- typeof, wrappers and their equality ---
const one: any = 1n;
console.log("typeof_prim=" + typeof one + " typeof_wrapper=" + typeof Object(one));
console.log("wrapper_tag=" + Object.prototype.toString.call(Object(one)) + " prim_tag=" + Object.prototype.toString.call(one));
console.log("wrapper_valueOf_is_prim=" + (Object(one).valueOf() === 1n) + " wrapper_loose_eq=" + (Object(one) == one) + " wrapper_strict_eq=" + ((Object(one) as any) === one));
console.log("two_wrappers_eq=" + (Object(1n) === Object(1n)) + " wrapper_instanceof=" + (Object(one) instanceof BigInt) + " prim_instanceof=" + ((one as any) instanceof BigInt));
console.log("proto_of_wrapper=" + (Object.getPrototypeOf(Object(one)) === BigInt.prototype));
op("valueOf_prim", () => BigInt.prototype.valueOf.call(1n));
op("valueOf_wrapper", () => BigInt.prototype.valueOf.call(Object(1n)));
op("valueOf_number", () => BigInt.prototype.valueOf.call(1 as any));
op("valueOf_string", () => BigInt.prototype.valueOf.call("1" as any));
op("toString_wrapper_radix", () => BigInt.prototype.toString.call(Object(255n), 16));
op("toString_prim_radix", () => BigInt.prototype.toString.call(255n, 16));
op("toString_number", () => BigInt.prototype.toString.call(255 as any, 16));
const w1: any = Object(7n);
const w2: any = Object(7n);
console.log("wrapper_strict=" + (w1 === w2) + " wrapper_loose=" + (w1 == w2) + " wrapper_self=" + (w1 === w1) + " wrapper_vs_primitive=" + (w1 == 7n));
console.log("wrapper_is_truthy=" + (Object(0n) ? "yes" : "no") + " truthy=" + (0n ? "t" : "f") + "," + (1n ? "t" : "f"));
w1.tagged = "extra";
console.log("wrapper_takes_properties=" + w1.tagged + " keys=" + Object.keys(w1).join(","));
op("reflect_set_on_primitive", () => Reflect.set(255n as any, "x", 1));
op("reflect_set_on_wrapper", () => Reflect.set(Object(255n), "x", 1));

// --- the constructor object and its prototype ---
console.log("ctor_typeof=" + typeof BigInt + " name=" + BigInt.name + " length=" + BigInt.length);
console.log("ctor_keys=[" + Object.keys(BigInt).join(",") + "] proto_is_Function=" + (Object.getPrototypeOf(BigInt) === Function.prototype));
const protoDesc = Object.getOwnPropertyDescriptor(BigInt, "prototype") as any;
console.log("prototype_flags=" + [protoDesc.writable, protoDesc.enumerable, protoDesc.configurable].join(","));
console.log("call_ok=" + String(BigInt(5)));
op("new_BigInt", () => new (BigInt as any)(5));
op("reflect_construct", () => Reflect.construct(BigInt as any, [5]));
op("subclass_construct", () => {
  class Big extends (BigInt as any) {}
  return new Big(5);
});
console.log("asIntN_arity=" + BigInt.asIntN.length + " asUintN_arity=" + BigInt.asUintN.length + " asIntN_name=" + BigInt.asIntN.name);
console.log("proto_typeof=" + typeof BigInt.prototype + " proto_tag=" + Object.prototype.toString.call(BigInt.prototype));
console.log("proto_toStringTag=" + String((BigInt.prototype as any)[Symbol.toStringTag]) + " constructor_identity=" + (BigInt.prototype.constructor === BigInt));
console.log("proto_of_proto=" + (Object.getPrototypeOf(BigInt.prototype) === Object.prototype));
op("valueOf_on_prototype", () => BigInt.prototype.valueOf.call(BigInt.prototype));
op("toString_on_prototype", () => BigInt.prototype.toString.call(BigInt.prototype));
op("valueOf_null", () => BigInt.prototype.valueOf.call(null));
op("valueOf_plain_object", () => BigInt.prototype.valueOf.call({}));
op("valueOf_fake_slot", () => BigInt.prototype.valueOf.call({ valueOf: () => 7n }));
op("toString_bad_radix", () => BigInt.prototype.toString.call(255n, 1));

// --- loose ToNumber also refuses BigInt inside ordinary comparisons ---
op("sort_comparator_bigint", () => [3n, 1n].sort((a: any, b: any) => (a - b) as any).join(","));
