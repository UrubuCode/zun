#!/usr/bin/env node
// Operators, precedence, coercion, numeric and BigInt literals.
// One label per behaviour; a changed line points straight at the rule that moved.

// --- truthiness and value-selecting short-circuit ---
console.log("or_empty_str=" + ("" || "fb"));
console.log("or_str=" + ("a" || "fb"));
console.log("and_empty_str=" + ("" && "x"));
console.log("and_str=" + ("a" && "x"));
console.log("ternary_empty=" + ("" ? "y" : "n"));
console.log("ternary_space=" + ("   " ? "y" : "n"));
console.log("ternary_zero=" + (0 ? "y" : "n"));
console.log("ternary_one=" + (1 ? "y" : "n"));
console.log("ternary_null=" + (null ? "y" : "n"));
console.log("or_chain=" + ("" || "" || "fb" || "x"));
console.log("and_or_prec_1=" + ("a" || "b" && "c"));
console.log("and_or_prec_2=" + ("" || "x" && "y"));
console.log("mixed_values=" + String(0 || "a" && "b"));
console.log("or_falsy_run=" + String(0 || NaN || -0 || ""));
console.log("and_before_or=" + (false && false || true));
console.log("or_parens=" + (false && (false || true)));

// --- loose versus strict equality ---
console.log("null_eq_undef=" + (null == undefined));
console.log("undef_eq_null=" + (undefined == null));
console.log("null_eq_null=" + (null == null));
console.log("undef_eq_undef=" + (undefined == undefined));
console.log("null_strict_undef=" + ((null as any) === (undefined as any)));
console.log("zero_eq_false=" + ((0 as any) == (false as any)));
console.log("one_eq_true=" + ((1 as any) == (true as any)));
console.log("zero_strict_false=" + ((0 as any) === (false as any)));
console.log("one_eq_str=" + ((1 as any) == ("1" as any)));
console.log("one_strict_str=" + ((1 as any) === ("1" as any)));
console.log("nan_eq_nan=" + (NaN == NaN));
console.log("nan_strict_nan=" + (NaN === NaN));
console.log("neg_zero_strict=" + (-0 === 0));
console.log("object_is_neg_zero=" + Object.is(-0, 0));
console.log("object_is_nan=" + Object.is(NaN, NaN));
console.log("empty_arr_eq_zero=" + (([] as any) == 0));
console.log("empty_arr_eq_empty_str=" + (([] as any) == ""));
console.log("null_eq_zero=" + ((null as any) == 0));
console.log("null_ge_zero=" + ((null as any) >= 0));

// --- relational operators chain through a boolean ---
console.log("chain_1_2_3=" + (1 < 2 < 3));
console.log("chain_3_2_1=" + (3 > 2 > 1));
console.log("chain_5_4_3=" + (5 > 4 > 3));
console.log("true_gt_zero=" + (true > 0));
console.log("false_lt_one=" + (false < 1));
console.log("eq_after_rel=" + (1 == 1 < 2));
console.log("strict_after_rel=" + (true === 1 < 2));
console.log("string_relational=" + ("apple" < "banana"));
console.log("string_chain=" + ("apple" < "banana" < "cherry"));
console.log("string_vs_number=" + ("10" < "9") + "/" + (10 < 9));

// --- where binary operators and `in` / `instanceof` bind ---
const keyed: any = { ab: "yes", a: "no", true: "T", false: "F" };
console.log("plus_before_in=" + ("a" + "b" in keyed));
console.log("parenthesised_in=" + ("a" + ("b" in keyed)));
console.log("plus_before_instanceof=" + (1 + 2 instanceof Object));
console.log("not_before_in=" + (!0 in keyed));
console.log("not_of_in=" + !(0 in keyed));
console.log("typeof_before_eq=" + (typeof 1 === "number"));
console.log("typeof_of_comparison=" + typeof (1 === 1));
console.log("void_precedence=" + (void 0 === undefined));
console.log("void_of_comparison=" + String(void (0 === undefined)));
console.log("eq_tighter_than_bitand=" + (1 & 3 == 1));
console.log("eq_tighter_than_bitor=" + (0 | 1 == 1));
console.log("xor_and_precedence=" + (1 ^ 2 & 3));
console.log("shift_after_plus=" + (1 + 2 << 3));
console.log("shift_parens=" + (1 + (2 << 3)));
console.log("shift_right_plus=" + (16 >> 1 + 1));
console.log("unsigned_shift=" + (-1 >>> 28));
console.log("ternary_test=" + (false || true ? "taken" : "not"));
console.log("ternary_grouped=" + (false || (true ? "taken" : "not")));
console.log("left_to_right=" + (1 + 2 + "3") + "|" + ("1" + 2 + 3));
console.log("unary_plus=" + (+"42") + "|" + (+"") + "|" + (+"x"));
console.log("typeof_null=" + typeof null + "|" + typeof NaN);
console.log("not_member=" + !keyed.ab + "|" + (!keyed.ab === false));

let assigned: any = 0;
assigned = 1 < 2;
console.log("assign_after_compare=" + assigned);

// `in` against arrays, objects, inherited members and private names
const inArr = [10, 20, 30];
console.log("in_index=" + (0 in inArr) + "|" + (3 in inArr) + "|" + ("length" in inArr));
const inObj = { a: 1, b: undefined, c: null };
console.log("in_undefined_value=" + ("b" in inObj) + "|" + ("d" in inObj));
class Foo { x = 1; method(): void {} }
const foo = new Foo();
console.log("in_members=" + ("x" in foo) + "|" + ("method" in foo) + "|" + ("toString" in foo));
class HasPrivate { #value = 42; static check(o: any): boolean { return #value in o; } }
console.log("private_in=" + HasPrivate.check(new HasPrivate()) + "|" + HasPrivate.check({}));

// `in` and `instanceof` share a level and associate left to right
class Box {}
const box = new Box();
console.log("instanceof_then_in=" + (box instanceof Box in keyed));
let instThrow = "none";
try {
  instThrow = String(({} as any) instanceof ({} as any));
} catch (e) {
  instThrow = (e as any).constructor.name;
}
console.log("instanceof_non_callable=" + instThrow);

// --- exponentiation ---
console.log("pow_right_assoc=" + (2 ** 3 ** 2) + "|" + ((2 ** 3) ** 2) + "|" + (2 ** 2 ** 3));
console.log("neg_pow=" + (-(2 ** 2)) + "|" + ((-2) ** 2));
console.log("pow_fraction=" + (4 ** 0.5) + "|" + (2 ** -2));
console.log("pow_zero_nan=" + (0 ** 0) + "|" + (NaN ** 0) + "|" + ((-8) ** (1 / 3)));
let powAssign = 2;
powAssign **= 3;
console.log("pow_assign=" + powAssign);
let powAssignRhs = 3;
powAssignRhs **= 2 ** 1;
console.log("pow_assign_rhs=" + powAssignRhs);
console.log("bigint_pow=" + (2n ** 8n) + "|" + ((-2n) ** 3n));

// --- increment and decrement values ---
let i = 5;
console.log("post_inc=" + (i++) + "|" + i);
console.log("pre_inc=" + (++i) + "|" + i);
let a = 1;
const mixedIncr = a++ + ++a;
console.log("mixed_incr=" + mixedIncr + "|" + a);
let b = 10;
const incrArr = [b++, b++, ++b];
console.log("incr_arr=" + incrArr.join(",") + "|" + b);
let x = 3;
const decr = x-- - --x;
console.log("decr=" + decr + "|" + x);
let n = 0;
n = n++;
console.log("self_postfix=" + n);
let p = 3;
const q1 = p++;
const q2 = ++p;
const q3 = p--;
const q4 = --p;
console.log("incr_sequence=" + [q1, q2, q3, q4].join(",") + "|" + p);

// --- comma operator: every operand runs, the last one is the value ---
let cx = (1, 2, 3);
console.log("comma_value=" + cx);
const commaLog: number[] = [];
function s(v: number): number { commaLog.push(v); return v; }
const cy = (s(10), s(20), s(30));
console.log("comma_last=" + cy + "|" + commaLog.join(","));
const cr = (s(1), s(2)) + (s(3), s(4));
console.log("comma_sum=" + cr + "|" + commaLog.join(","));
let cz = 0;
cz = (cz += 1, cz += 2, cz += 3);
console.log("comma_assign=" + cz);
console.log("comma_in_call=" + String((1, 2, 3)));
let ci = 0;
let cj = 0;
for (ci = 0, cj = 10; ci < 3; ci++, cj -= 3) { /* header carries the work */ }
console.log("comma_for=" + ci + "|" + cj);

// --- void always yields undefined, but still evaluates its operand ---
console.log("void_zero=" + String(void 0));
console.log("void_expr=" + String(void (1 + 2)));
console.log("void_string=" + String(void "anything"));
const voidLog: string[] = [];
function voidCall(): number { voidLog.push("called"); return 99; }
const voidResult = void voidCall();
console.log("void_call=" + String(voidResult) + "|" + voidLog.join(","));
let vx = 5;
console.log("void_postfix=" + String(void vx++) + "|" + vx);
console.log("void_is_undef=" + (void 0 === undefined) + "|typeof=" + typeof void 0);

// --- delete on properties, indices and nested members ---
const del: any = { a: 1, b: 2, c: 3 };
console.log("delete_prop=" + (delete del.b) + "|" + Object.keys(del).join(","));
console.log("delete_missing=" + (delete del.nope));
console.log("delete_computed=" + (delete del["a"]) + "|" + ("a" in del));
const delArr: any[] = [10, 20, 30];
console.log("delete_index=" + (delete delArr[1]) + "|" + delArr.length + "|" + (1 in delArr));
const delNested: any = { p: { q: 7 } };
console.log("delete_nested=" + (delete delNested.p.q) + "|" + ("q" in delNested.p));

// --- numeric literals with separators and BigInt ---
console.log("sep_decimal=" + 1_000_000);
console.log("sep_binary=" + 0b1010_0001_1000_0101);
console.log("sep_binary_small=" + 0b1010_0101);
console.log("sep_hex=" + 0xFF_EC_DE_5E);
console.log("sep_hex_small=" + 0xFF_FF);
console.log("sep_octal=" + 0o12_34);
console.log("sep_float=" + 3.141_592_653);
console.log("sep_bigint=" + 1_234_567_890n);
console.log("bigint_type=" + typeof 1n + "|" + (1n === 1n) + "|" + (1n == 1));

// --- nested conditional chains: right-associative, as minified code uses them ---
function grade(n: number): string { return n >= 90 ? "A" : n >= 80 ? "B" : n >= 70 ? "C" : "F"; }
console.log("ternary_chain=" + [95, 85, 72, 50].map(grade).join(","));
const ta = 1, tb = 0, tc = 1;
console.log("ternary_nested=" + (ta ? tb ? "ab" : tc ? "ac" : "a" : "none"));
console.log("ternary_right_assoc=" + (true ? false ? 1 : 2 : 3) + "|" + (false ? 1 : true ? 2 : 3));
function minifiedClassify(n: number): string {
  return n < 0 ? "neg" : n === 0 ? "zero" : n < 10 ? "small" : n < 100 ? "medium" : "large";
}
console.log("ternary_minified=" + [-5, 0, 7, 42, 999].map(minifiedClassify).join(","));
