// Implicit numeric coercion across the operators: the + operator's string
// versus numeric decision, ToPrimitive with its hints (valueOf, toString,
// Symbol.toPrimitive), arithmetic on non-numbers, the full == matrix next to
// ===, Object.is and SameValueZero, the relational grid, BigInt comparisons,
// ++/-- and compound assignment (evaluation order through accessors), and the
// numeric property-key rules that reuse the same ToString.

function show(label: string, fn: () => string): void {
  try {
    console.log(label + "=" + fn());
  } catch (e) {
    console.log(label + "!" + (e as Error).name);
  }
}

// --- the + operator: string concatenation only when a string operand exists ---
console.log("plus_num_str=" + (1 + "2") + " plus_str_num=" + ("2" + 1) + " plus_nums=" + (1 + 2));
console.log("plus_bools=" + (true + true) + "," + (true + false) + "," + (false + false) + "," + (true + 1));
console.log("plus_bool_num=" + (false + 10) + "," + (true * 3) + "," + (false * 5) + "," + (true - false));
console.log("plus_bool_concat=" + ("" + true + false) + " product=" + ((true + true) * (true + true + true)));
console.log("plus_null_undef=" + (1 + null + undefined) + "," + (1 + null) + "," + (1 + undefined));
console.log("plus_null_null=" + (null + null) + " undef_plus_1=" + (undefined + 1) + " false_plus_null=" + (false + null));
console.log("plus_arrays=" + ([] + []) + "|" + ([] + {}) + "|" + ([1] + [2]) + "|" + ([1, 2] + [3, 4]));
console.log("plus_array_num=" + (1 + []) + "," + (1 + [2]) + "," + ([] + 1) + "," + ([3] + 1) + "," + ("x" + []) + "," + ("x" + {}));
console.log("plus_object_str=" + ({} + "") + "," + ({} + []));

// --- string operands in the other arithmetic operators are converted with ToNumber ---
console.log("arith_strings=" + ("10" - 5) + "," + ("10" * "2") + "," + ("10" / "4") + "," + ("6" % "4"));
console.log("arith_left_assoc=" + ("10" + 5 - 3) + "," + (10 + "5" - 3) + "," + ("3" * true));
console.log("arith_empty_and_bad=" + ("10" - "") + "," + ("abc" - 1) + "," + (" 12 " - 2));
console.log("arith_null_undef=" + ("3" - null) + "," + ("3" - undefined));
show("symbol_arith", () => String((Symbol("s") as any) * 2));

// --- ToPrimitive: valueOf for arithmetic, toString for template literals and keys ---
const both: any = { valueOf: () => 10, toString: () => "str" };
console.log("obj_plus_5=" + (both + 5) + " obj_times_2=" + (both * 2) + " concat=" + ("" + both));
console.log("obj_template=" + `${both}` + " obj_eq_10=" + (both == 10) + " obj_lt_11=" + (both < 11));
const onlyStr: any = { toString: () => "7" };
console.log("only_str_plus=" + (onlyStr + 1) + " times2=" + (onlyStr * 2));
const numObj: any = { valueOf: () => 3, toString: () => "9" };
console.log("num_obj_plus=" + (numObj + 1) + " template=" + `${numObj}` + " eq3=" + (numObj == 3));
const plain: any = { valueOf: () => 3 };
console.log("valueOf_only_lt4=" + (plain < 4) + " minus1=" + (plain - 1) + " times2=" + (plain * 2));
const hook: any = { toString: () => "hi" };
const bag: any = { hi: "found" };
console.log("key_from_toString=" + bag[hook]);

// --- Symbol.toPrimitive decides, and receives the hint ---
const hints: string[] = [];
const prim: any = {
  [Symbol.toPrimitive](hint: string) {
    hints.push(hint);
    return 7;
  },
};
console.log("prim_plus=" + (prim + 1) + " prim_times=" + (prim * 2) + " prim_template=" + `${prim}`);
console.log("prim_hints=" + hints.join(","));
const hintsNumber: string[] = [];
const primNumber: any = {
  [Symbol.toPrimitive](hint: string) {
    hintsNumber.push(hint);
    return hint === "string" ? "S" : 3;
  },
};
console.log("prim_gt=" + (primNumber > 2) + " prim_str=" + String(primNumber) + " prim_hints=" + hintsNumber.join(","));

// --- Date and Symbol-free numeric coercion ---
const epoch: any = new Date(0);
console.log("date_unary=" + (+epoch) + " date_lt=" + (epoch < new Date(5)) + " date_plus_type=" + typeof (epoch + 1));

// --- the == matrix over number-ish values, with the other relations beside it ---
const wrapper: any = new Number(0);
const valued: any = { valueOf: () => 1 };
const rows: [string, any][] = [
  ["0", 0], ["-0", -0], ["1", 1], ["NaN", NaN], ["Inf", Infinity], ["''", ""], ["' '", " "],
  ["'0'", "0"], ["'1'", "1"], ["'0.0'", "0.0"], ["'Inf'", "Infinity"], ["false", false],
  ["true", true], ["null", null], ["undef", undefined], ["[]", []], ["[0]", [0]], ["[1]", [1]],
  ["{}", {}], ["0n", 0n], ["1n", 1n], ["Num0", wrapper], ["valueOf1", valued],
];
console.log("legend=" + rows.map((r) => r[0]).join(" "));
for (const left of rows) {
  const cells: string[] = [];
  for (const right of rows) cells.push((left[1] as any) == (right[1] as any) ? "T" : ".");
  console.log("eq  " + left[0].padEnd(9, " ") + cells.join(""));
}
for (const left of rows) {
  const cells: string[] = [];
  for (const right of rows) cells.push((left[1] as any) === (right[1] as any) ? "T" : ".");
  console.log("id  " + left[0].padEnd(9, " ") + cells.join(""));
}
const looseOnly: string[] = [];
const objectIsDiffers: string[] = [];
const sameValueZeroDiffers: string[] = [];
for (const left of rows) {
  for (const right of rows) {
    const a: any = left[1];
    const b: any = right[1];
    if ((a == b) !== (a === b)) looseOnly.push(left[0] + "==" + right[0]);
    if (Object.is(a, b) !== (a === b)) objectIsDiffers.push(left[0] + "/" + right[0]);
    if ([a].includes(b) !== Object.is(a, b)) sameValueZeroDiffers.push(left[0] + "/" + right[0]);
  }
}
console.log("loose_only=" + looseOnly.join(" "));
console.log("object_is_differs=" + objectIsDiffers.join(" "));
console.log("samevaluezero_differs_from_object_is=" + sameValueZeroDiffers.join(" "));

// --- transitivity fails under == ---
console.log("transitivity_1=" + ("0" == false) + "," + (false == []) + "," + ("0" == ([] as any)));
console.log("transitivity_2=" + (0 == "") + "," + ("" == "0") + "," + (0 == "0"));
console.log("transitivity_3=" + (1 == "1") + "," + ("1" == true) + "," + (1 == true));
console.log("array_loose=" + ([] == false) + "," + ([] == 0) + "," + ([0] == false) + "," + ([0] == 0));
console.log("array_loose2=" + ([1] == 1) + "," + ([1] == true) + "," + (["1"] == 1) + "," + ([] == ""));
console.log("array_loose3=" + ([null] == 0) + "," + ([undefined] == 0) + "," + ([] == ![]) + "," + ([1, 2] == "1,2"));
console.log("string_vs_number=" + (0 == "") + "," + (0 == "0") + "," + ("" == "0") + "," + ("0" == false) + "," + (0 === "0"));
console.log("whitespace_eq=" + ("  " == 0) + "," + (" 1 " == 1) + "," + ("1e2" == 100) + "," + (1 === "1"));
console.log("nan_eq=" + (NaN == NaN) + "," + (NaN === NaN) + "," + (null == 0) + "," + (undefined == 0));
console.log("null_eq=" + (null == undefined) + "," + (null === undefined) + "," + (null == false) + "," + (0 == false));

// --- relational grid: <, >, <=, >= after ToPrimitive, with NaN and null special ---
const relRows: [string, any][] = [
  ["null", null], ["undef", undefined], ["0", 0], ["NaN", NaN], ["''", ""], ["[]", []],
];
for (const left of relRows) {
  const cells: string[] = [];
  for (const right of relRows) {
    const a: any = left[1];
    const b: any = right[1];
    const code = (a < b ? "<" : "") + (a > b ? ">" : "") + (a <= b ? "l" : "") + (a >= b ? "g" : "") + (a == b ? "=" : "");
    cells.push(code === "" ? "-" : code);
  }
  console.log("rel " + left[0].padEnd(6, " ") + cells.join(" "));
}
console.log("null_relations=" + (null >= 0) + "," + (null > 0) + "," + (null <= 0) + "," + ((null as any) == 0));
console.log("undef_relations=" + ((undefined as any) >= 0) + "," + ((undefined as any) <= 0) + "," + ((undefined as any) == null));
console.log("string_relations=" + ("10" < "2") + "," + ("10" < 2) + "," + ("5" > 4) + "," + ("5" > "40"));
console.log("object_relation=" + ((({ valueOf: () => 3 }) as any) < 4));

// --- BigInt against Number: mixed comparisons are exact, mixed arithmetic throws ---
console.log("bigint_lt=" + (2n < 3) + "," + (2n > 3) + "," + (2n <= 2) + "," + (3n >= 2));
console.log("bigint_loose=" + (2n == 2) + "," + (2n === 2) + "," + (2n == "2") + "," + (0n == false));
show("bigint_plus_number", () => String((1n as any) + 1));
show("bigint_times_number", () => String((2n as any) * 3));
console.log("bigint_typed_arith=" + (2n * 3n) + "," + (7n / 2n) + "," + (-7n % 3n));

// --- switch uses strict equality: no coercion between cases ---
function classify(x: any): string {
  switch (x) {
    case 1: return "num one";
    case "1": return "str one";
    case true: return "bool true";
    case null: return "null";
    case undefined: return "undefined";
    default: return "other";
  }
}
console.log("switch=" + [classify(1), classify("1"), classify(true), classify(1 === 1), classify(null), classify(undefined)].join(","));
console.log("switch_computed=" + [classify(1 + 0), classify("1" + ""), classify(2)].join(","));

// --- ++ and -- coerce with ToNumeric; postfix returns the coerced old value ---
function label(v: any): string {
  return typeof v + ":" + String(v);
}
let s: any = "5";
const sOld = s++;
console.log("str_post_old=" + label(sOld) + " str_post_new=" + label(s));
let s2: any = "5";
const s2New = ++s2;
console.log("str_pre_result=" + label(s2New) + " str_pre_var=" + label(s2));
const cases: [string, any][] = [
  ["empty_str", ""], ["space_str", "  "], ["hex_str", "0x10"], ["exp_str", "1e3"], ["bad_str", "abc"],
  ["true", true], ["false", false], ["null", null], ["undefined", undefined], ["emptyarr", []],
  ["arr7", [7]], ["arr_two", [1, 2]], ["obj", {}], ["negzero", -0], ["infinity", Infinity], ["nan", NaN],
];
for (const c of cases) {
  let inc: any = c[1];
  const incOld = inc++;
  let dec: any = c[1];
  const decOld = dec--;
  console.log(c[0] + " | ++old:" + label(incOld) + " | ++new:" + label(inc) + " | --old:" + label(decOld) + " | --new:" + label(dec));
}
const order: string[] = [];
let box: any = {
  valueOf() {
    order.push("valueOf");
    return 41;
  },
  toString() {
    order.push("toString");
    return "999";
  },
};
const boxOld = box++;
console.log("box_old=" + label(boxOld) + " box_new=" + label(box) + " box_order=" + order.join(","));
let z: any = 1;
z--;
console.log("one_dec_isNeg0=" + Object.is(z, -0) + " zero_dec=" + label((() => { let q: any = 0; q--; return q; })()));
let nz: any = -0;
const nzOld = nz++;
console.log("negzero_post_old_isNeg0=" + Object.is(nzOld, -0) + " negzero_post_new=" + label(nz));
let big: any = Number.MAX_SAFE_INTEGER;
big++;
const afterOne = big;
big++;
console.log("maxsafe_plus1=" + label(afterOne) + " maxsafe_plus2=" + label(big) + " isSafe=" + Number.isSafeInteger(big));
const reads: string[] = [];
const acc: any = {
  _v: 10,
  get v() {
    reads.push("get");
    return this._v;
  },
  set v(n: number) {
    reads.push("set:" + n);
    this._v = n;
  },
};
acc.v++;
console.log("accessor_log=" + reads.join(",") + " accessor_final=" + label(acc._v));

// --- compound assignment: reference evaluated once, read, right side, then write ---
const trace: string[] = [];
function makeCell(initial: number): any {
  let inner = initial;
  const cell: any = {};
  Object.defineProperty(cell, "v", {
    get() {
      trace.push("get:" + String(inner));
      return inner;
    },
    set(x: number) {
      trace.push("set:" + String(x));
      inner = x;
    },
    configurable: true,
  });
  return cell;
}
function rhs(name: string, value: number): number {
  trace.push("rhs:" + name);
  return value;
}
function report(label: string): void {
  console.log(label + " => " + trace.join(" "));
  trace.length = 0;
}
let c = makeCell(10);
c.v += rhs("plus", 5);
report("plus_equals");
console.log("plus_result=" + c.v);
c = makeCell(10);
c.v -= rhs("minus", 3);
report("minus_equals");
c = makeCell(10);
c.v *= rhs("times", 4);
report("times_equals");
c = makeCell(2);
c.v **= rhs("power", 10);
report("power_equals");
c = makeCell(10);
c.v /= rhs("div", 4);
report("div_equals");
c = makeCell(-8);
c.v >>>= rhs("ushift", 1);
report("ushift_equals");
c = makeCell(5);
c.v |= rhs("or", 8);
report("or_equals");
c = makeCell(5);
c.v ^= rhs("xor", 1);
report("xor_equals");
c = makeCell(5);
c.v ||= rhs("never", 99);
report("or_equals_truthy");
c = makeCell(0);
c.v ||= rhs("taken", 99);
report("or_equals_falsy");
c = makeCell(5);
c.v &&= rhs("taken2", 42);
report("and_equals_truthy");
c = makeCell(5);
c.v ??= rhs("never2", 1);
report("nullish_equals_present");
const keyLog: string[] = [];
function key(name: string): string {
  keyLog.push(name);
  return "n";
}
const keyed: any = { n: 1 };
keyed[key("k1")] += 10;
keyed[key("k2")] *= 2;
console.log("key_evaluations=" + keyLog.join(",") + " value=" + keyed.n);
const arr: number[] = [10, 20, 30];
let i = 0;
arr[i++] += 5;
console.log("index_once=" + arr.join(",") + " i=" + i);
let s3: any = "n";
s3 += 1;
s3 += 2;
console.log("string_plus=" + s3 + " typeof=" + typeof s3);
let m: any = 1;
m += "1";
console.log("number_then_string=" + m + " typeof=" + typeof m);
let t: any = 1;
t += undefined;
let u: any = 1;
u += null;
console.log("plus_undefined=" + String(t) + " plus_null=" + String(u));
const coerce: string[] = [];
let boxed: any = {
  valueOf() {
    coerce.push("valueOf");
    return 7;
  },
};
boxed += 1;
console.log("object_plus=" + String(boxed) + " typeof=" + typeof boxed + " calls=" + coerce.join(","));

// --- numeric property keys: canonical numeric strings become indices, the rest are plain keys ---
const a: any = ["a", "b", "c"];
console.log("key_num=" + a[1] + " key_str=" + a["1"] + " key_float=" + a[1.0] + " key_bigint=" + a[1n]);
console.log("key_computed=" + a[0 + 1] + " all_same=" + (a[1] === a["1"] && a["1"] === a[1n]));
a["01"] = "zero-one";
a["1.0"] = "one-point-zero";
a[" 1"] = "space-one";
a["+1"] = "plus-one";
a["1e0"] = "exp-one";
console.log("len_after_fakes=" + a.length + " fake_01=" + a["01"] + " fake_1p0=" + a["1.0"] + " real_1=" + a[1]);
console.log("fake_keys=" + Object.keys(a).join(","));
const z2: any = ["first"];
console.log("negzero_read=" + z2[-0]);
z2[-0] = "replaced";
console.log("negzero_write=" + z2[0] + " len=" + z2.length);
const o: any = {};
const numericKeys: [string, any][] = [
  ["int", 1], ["negint", -1], ["frac", 1.5], ["negzero", -0], ["nan", NaN], ["inf", Infinity],
  ["neginf", -Infinity], ["e21", 1e21], ["e_neg7", 1e-7], ["maxsafe", 9007199254740991], ["bigint", 10n],
];
for (const k of numericKeys) {
  o[k[1]] = k[0];
}
console.log("obj_keys=" + Object.keys(o).join("|"));
console.log("obj_read_negzero=" + o[0] + " obj_read_bigint=" + o["10"] + " obj_read_e21=" + o["1e+21"] + " obj_read_inf=" + o["Infinity"]);
const mixed: any = {};
mixed["b"] = 1;
mixed[2] = 2;
mixed["a"] = 3;
mixed[10] = 4;
mixed[1] = 5;
mixed["01"] = 6;
mixed[0] = 7;
console.log("mixed_keys=" + Object.keys(mixed).join(","));
console.log("mixed_json=" + JSON.stringify(mixed));
const d: any = [10, 20, 30];
delete d[1.0];
console.log("after_delete_len=" + d.length + " read=" + String(d[1]) + " has=" + (1 in d));
Object.defineProperty(d, 3n as any, { value: 40, enumerable: true, writable: true, configurable: true });
console.log("defineProperty_bigint_len=" + d.length + " read=" + d[3]);

// --- sparse writes through an object whose toString names a far slot leave holes ---
const far: any = { toString: () => "5" };
const holey: any[] = ["a", "b"];
holey[far] = "z";
console.log("holey_length=" + holey.length + " keys=" + Object.keys(holey).join(","));
console.log("filter_boolean=" + [0, "", null, undefined, NaN, false, 1, "x"].filter(Boolean).length);
