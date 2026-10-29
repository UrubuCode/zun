// Text to number, in both directions of the grammar. Numeric literal forms
// (radix prefixes, separators, exponents, leading and trailing dots, BigInt
// literals), then the three string-to-number routes (Number, parseInt,
// parseFloat, unary plus) side by side, the whitespace grammar code point by
// code point, radix handling of parseInt, and the non-string inputs where the
// routes disagree because parseInt and parseFloat stringify first.

function attempt(label: string, fn: () => string): void {
  try {
    console.log(label + "=" + fn());
  } catch (e) {
    console.log(label + "!" + (e as Error).name);
  }
}

// --- radix-prefixed and decimal literals: all ordinary doubles ---
console.log("bin=" + 0b1010 + "," + 0B1010 + "," + 0b11111111);
console.log("oct=" + 0o17 + "," + 0O17 + "," + 0o777);
console.log("hex=" + 0x1f + "," + 0X1F + "," + 0xff + "," + 0xFF + "," + 0xdeadbeef);
console.log("hex_big=" + 0xffffffff + "," + 0x20000000000000);
console.log("prefix_equalities=" + (0b1111 === 15) + "," + (0o17 === 15) + "," + (0xf === 15));
console.log("prefix_zero=" + 0b0 + "," + 0o0 + "," + 0x0 + "," + Object.is(0x0, -0));

// --- separators inside every form, never at an edge ---
console.log("sep_dec=" + 1_000_000 + "," + 1_0 + "," + 10_00.00_1);
console.log("sep_bin=" + 0b1010_1010 + "," + 0o1_7 + "," + 0xFF_FF);
console.log("sep_exp=" + 1_0e1_0);
console.log("sep_invisible=" + (1_000 === 1000) + "," + (0xFF_FF === 65535));

// --- exponent forms and the extremes they reach ---
console.log("exp=" + 1e3 + "," + 1E3 + "," + 1e+3 + "," + 1e-3 + "," + 2.5e2 + "," + 2.5E-2);
console.log("exp_zero=" + 5e0 + "," + 0e0 + "," + 1e0);
console.log("exp_extremes=" + 1e308 + "," + 1e309 + "," + 1e-323 + "," + 1e-324);
console.log("literal_overflow=" + String(1e400) + "," + String(-1e400));
console.log("literal_underflow=" + String(1e-400) + " neg_is_neg_zero=" + Object.is(-1e-400, -0));

// --- leading and trailing dots ---
console.log("dots=" + .5 + "," + 5. + "," + 5.0 + "," + .25e1);
console.log("dot_equalities=" + (.5 === 0.5) + "," + (5. === 5) + "," + (5.0 === 5));

// --- property access on a literal needs a second dot, a space, or parentheses ---
console.log("double_dot=" + 5..toString());
console.log("spaced_dot=" + 5 .toString(2));
console.log("parenthesised=" + (5).toString(16));
console.log("decimal_dot=" + 5.0.toString(2));
console.log("fraction_dot=" + 0.5.toString(2));
console.log("exp_dot=" + 5e1.toString(8));
console.log("hex_dot=" + 0xff.toString(36));

// --- every numeric literal is a double; there is no integer type ---
console.log("typeof=" + typeof 1 + "," + typeof 0x1f + "," + typeof 1_000 + "," + typeof .5);
console.log("is_integer=" + Number.isInteger(0xff) + "," + Number.isInteger(5.) + "," + Number.isInteger(.5));
console.log("hex_arithmetic=" + (0xff / 2) + "," + (0b1 / 4));
console.log("literal_precision=" + 9007199254740993 + "," + 900719925474099_3);
console.log("long_decimal_literal=" + 0.30000000000000004 + "," + (0.1 + 0.2 === 0.30000000000000004));

// --- BigInt literals: same prefixes and separators, no dot; read through String() ---
console.log("bigint_forms=" + String(1_000n) + "," + String(0xffn) + "," + String(0b1010n) + "," + String(0o17n) + "," + String(0n));
console.log("bigint_typeof=" + typeof 1n + "," + typeof 0xffn);
console.log("bigint_equalities=" + (0xffn === 255n) + "," + (1_000n === 1000n));

// --- unary minus is an operator, not part of the literal ---
console.log("neg_literal=" + -0 + "," + Object.is(-0, -0) + "," + Object.is(-(0), -0));
console.log("neg_hex=" + -0xff + "," + (-0xff === -255));
console.log("neg_exp_precedence=" + -1e3 + "," + (0 - 1e3));

// --- the same digits through Number, parseInt and parseFloat ---
const texts: string[] = [
  "1_000", "0b1010", "0o17", "0x1f", "0X1F", "+0x1f", "-0x1f", "1e3", "1E3",
  ".5", "5.", "0b1_0", "0xff", "0o8", "0b2", "1_0e1",
];
for (const s of texts) {
  console.log(
    "text " + s +
      " Number=" + String(Number(s)) +
      " parseInt=" + String(parseInt(s)) +
      " parseFloat=" + String(parseFloat(s))
  );
}

// --- Number: whole-string ToNumber, with the familiar edge strings ---
const numberCases: string[] = [
  "", "   ", "0x1F", "0b101", "0o17", "  42  ", "1e3", "Infinity", "-Infinity",
  "123abc", "08", ".5", "5.", "\t\n10\n", "+", "-", "+5", "-5", "-0", "+0x10",
  "-0x10", "1e", "1e+", "e3", "0x", "0b", "0o", "00012", "1.2.3", "Infinity1",
  "infinity", "-0b1", "0.0000001",
];
for (const s of numberCases) {
  console.log("Number[" + JSON.stringify(s) + "]=" + String(Number(s)) + " neg0=" + Object.is(Number(s), -0));
}

// --- whitespace grammar, code point by code point: trim set versus rejects ---
const wsCodes: number[] = [
  0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003,
  0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f,
  0x205f, 0x3000, 0xfeff,
];
const notWhitespace: number[] = [0x200b, 0x200c, 0x200d, 0x180e, 0x0085, 0x00a1, 0x001c, 0x001f, 0x0000];
for (const c of wsCodes) {
  const ch = String.fromCharCode(c);
  console.log("ws U+" + c.toString(16).padStart(4, "0") + " around=" + String(Number(ch + "1" + ch)) + " inner=" + String(Number("1" + ch + "2")));
}
for (const c of notWhitespace) {
  const ch = String.fromCharCode(c);
  console.log("not_ws U+" + c.toString(16).padStart(4, "0") + " around=" + String(Number(ch + "1" + ch)));
}
console.log("ws_only_is_zero=" + Number("  \t\n  ") + " empty_is_zero=" + Number(""));
console.log("ws_sign=" + Number(" +5 ") + " " + Number("\t-3.14\t") + " " + Number(" + 5"));
console.log("ws_infinity=" + Number("  Infinity  ") + " " + Number("  -Infinity  ") + " infinity_lower=" + Number("infinity"));
console.log("ws_hex=" + Number("  0x1F  ") + " " + Number("\t0b101\t") + " " + Number("\n0o17\n") + " " + Number("0x 1F"));
console.log("ws_inner=" + Number("1 2") + " " + Number("- 1") + " " + Number("1e 3") + " " + Number("1 .5"));

// --- parseInt: prefix reader; the radix goes through ToInt32 and then 2..36 ---
console.log("pi_prefix=" + parseInt("42abc") + "," + parseInt("abc42") + "," + parseInt("  42px") + "," + parseInt("4.9"));
console.log("pi_0x=" + parseInt("0x1F") + "," + parseInt("0x1F", 16) + "," + parseInt("1F", 16) + "," + parseInt("-0xff"));
console.log("pi_octal_free=" + parseInt("08") + "," + parseInt("0o17") + "," + parseInt("0b101"));
console.log("pi_radix_digits=" + parseInt("z", 36) + "," + parseInt("zz", 36) + "," + parseInt("z", 35) + "," + parseInt("10", 2));
console.log("pi_radix0=" + parseInt("123", 0) + " radix_omitted=" + parseInt("123") + " radix_10=" + parseInt("123abc", 10));
console.log("pi_radix_frac=" + parseInt("11", 2.9) + "," + parseInt("11", 2.1) + "," + parseInt("11", "2" as any) + "," + parseInt("11", "2.9" as any));
console.log("pi_radix_valueOf=" + parseInt("11", { valueOf: () => 16 } as any) + " radix_array=" + parseInt("11", [16] as any));
console.log("pi_radix_wrap=" + parseInt("11", 4294967298) + "," + parseInt("11", 4294967312) + "," + parseInt("11", 4294967296));
console.log("pi_radix_neg_wrap=" + parseInt("11", -4294967294));
console.log("pi_radix_empties=" + parseInt("11", undefined) + "," + parseInt("11", null as any) + "," + parseInt("11", NaN) + "," + parseInt("11", Infinity));
console.log("pi_radix_no_radix=" + parseInt("11", -Infinity) + "," + parseInt("11", false as any) + "," + parseInt("11", "" as any) + "," + parseInt("11", {} as any));
console.log("pi_radix_1=" + parseInt("11", 1) + " radix_true=" + parseInt("11", true as any) + " radix_37=" + parseInt("11", 37));
console.log("pi_radix_36=" + parseInt("11", 36) + " radix_neg2=" + parseInt("11", -2) + " radix_neg16=" + parseInt("ff", -16));
console.log("pi_radix_digit_too_big=" + parseInt("2", 2) + "," + parseInt("8", 8) + "," + parseInt("10201", 2) + "," + parseInt("1789", 8) + "," + parseInt("1fg2", 16));
console.log("pi_hex_prefix_radix=" + parseInt("0x11", 0) + "," + parseInt("0x11", 16) + "," + parseInt("0X11", 16) + "," + parseInt("0x11", 10));
console.log("pi_hex_prefix_other=" + parseInt("0x11", 8) + "," + parseInt("0x11", 36) + "," + parseInt("0x11", 34));
console.log("pi_hex_signs=" + parseInt("-0x1f", 16) + "," + parseInt("-0x1f", 0) + "," + parseInt("+0x1f", 16));
console.log("pi_bare_x=" + parseInt("0x", 16) + "," + parseInt("0x", 0) + " g2_r16=" + parseInt("g2", 16));
console.log("pi_neg_zero=" + Object.is(parseInt("-0"), -0) + "," + Object.is(parseInt("0"), -0) + "," + Object.is(parseInt("-0", 16), -0));
console.log("pf_neg_zero=" + Object.is(parseFloat("-0"), -0));
console.log("pi_inf_nan=" + parseInt("Infinity") + " pi_signed_gap=" + parseInt("   -  7"));

// --- parseFloat: decimal and exponent prefix, Infinity, stops at junk ---
console.log("pf_prefix=" + parseFloat("3.14abc") + "," + parseFloat("  .5") + "," + parseFloat("3.14.15") + "," + parseFloat("  12  34"));
console.log("pf_exp=" + parseFloat("1e3") + "," + parseFloat("1.2e-3") + "," + parseFloat("1e") + "," + parseFloat("+7.5"));
console.log("pf_inf=" + parseFloat("Infinity") + "," + parseFloat("-Infinity") + "," + parseFloat("5.") + "," + parseFloat("0x10"));
console.log("pf_nan=" + parseFloat(".") + "," + parseFloat("abc") + "," + parseFloat("12abc") + "," + parseFloat("  -7.5e2xyz"));
console.log("pf_small=" + parseFloat("0.0000001") + "," + parseFloat("1_000") + "," + parseFloat(" 0x1F "));

// --- unary plus and minus follow Number() on strings ---
const unaryInputs: string[] = [
  "42", "  7  ", "", "3.14", "0x1F", "1e3", "   ", "12px", "Infinity", "-0", "-5", "--5", "0b101",
];
for (const s of unaryInputs) {
  console.log("unary " + JSON.stringify(s) + " plus=" + String(+s) + " minus=" + String(-s));
}
console.log("unary_booleans=" + (+true) + "," + (+false) + "," + (-true) + "," + (-false));
console.log("unary_null_undefined=" + (+null) + "," + (+undefined) + "," + (-null) + "," + (-undefined));
console.log("unary_arrays=" + (+[]) + "," + (+[5]) + "," + (+[1, 2]) + "," + (-[]) + "," + (-[5]) + "," + (-[1, 2]));
console.log("unary_double_minus=" + (- -3) + " minus_hex_string=" + (-"0x10"));
attempt("unary_symbol_plus", () => String(+(Symbol("s") as any)));
attempt("unary_symbol_minus", () => String(-(Symbol("s") as any)));

// --- the three conversion routes on the SAME inputs; their disagreement is the point ---
const routeInputs: any[] = [
  "42abc", "  42  ", "3.99", "1_000", "0.0000001", "Infinity", "null", true, false,
  null, undefined, [], [7], [1, 2], Number.MAX_VALUE,
];
for (const v of routeInputs) {
  const label = typeof v === "string" ? JSON.stringify(v) : typeof v + ":" + String(v);
  console.log(
    "route " + label +
      " unary=" + String(+v) +
      " Number=" + String(Number(v)) +
      " parseInt=" + String(parseInt(v)) +
      " parseFloat=" + String(parseFloat(v))
  );
}

// --- BigInt: Number() and the parsers accept it, unary plus refuses it ---
console.log("bigint_Number=" + Number(10n) + "," + Number(-10n) + " parseInt=" + parseInt(10n as any));
attempt("bigint_unary_plus", () => String(+(10n as any)));
attempt("bigint_unary_minus_ok", () => String(-(10n as any)));

// --- parseInt and parseFloat stringify first: small and large numbers read back as text ---
console.log("stringify_5e-7_pi=" + parseInt(0.0000005 as any) + " Number=" + Number(0.0000005));
console.log("stringify_1e-6_pi=" + parseInt(0.000001 as any) + " pf=" + parseFloat(0.000001 as any));
console.log("stringify_1e21_pi=" + parseInt(1e21 as any) + " Number=" + Number(1e21));
console.log("stringify_1e20_pi=" + parseInt(1e20 as any));
console.log("stringify_neg255_9=" + parseInt(-255.9 as any) + " " + Number(-255.9));
console.log("stringify_max_pf=" + parseFloat(Number.MAX_VALUE as any));
console.log("stringify_min_pf=" + parseFloat(Number.MIN_VALUE as any));
console.log("stringify_maxsafe_pi=" + parseInt(Number.MAX_SAFE_INTEGER as any));
console.log("stringify_neg_zero_pi=" + Object.is(parseInt(-0 as any), -0) + " " + String(parseInt(-0 as any)));
console.log("stringify_nan_inf_pi=" + parseInt(NaN as any) + " " + parseInt(Infinity as any) + " " + parseInt(-Infinity as any));
console.log("stringify_bigint_pi=" + parseInt(10n as any) + " " + parseInt(-10n as any) + " " + parseInt((2n ** 70n) as any));
console.log("stringify_wrappers=" + parseInt(new Number(3.7) as any) + " " + parseFloat(new String("4.2xyz") as any) + " " + Number(new Boolean(true)));
console.log("stringify_arrays=" + parseInt([15] as any) + "," + parseInt([1, 2] as any) + "," + parseInt([[7]] as any) + "," + parseInt([] as any));
console.log("stringify_nullish=" + parseInt(null as any) + "," + parseInt(undefined as any) + "," + parseInt(true as any));
console.log("stringify_objects=" + parseInt({} as any) + "," + parseInt({ valueOf: () => 8 } as any) + "," + parseInt({ toString: () => "9.5x" } as any));

// --- coercion hooks: Number prefers valueOf, the parsers prefer toString ---
const both: any = { valueOf: () => 1, toString: () => "2" };
console.log("hooks_Number=" + Number(both) + " parseInt=" + parseInt(both) + " unary=" + (+both));
const valueOfObject: any = { valueOf: () => ({}), toString: () => "3.25" };
console.log("hooks_valueOf_object=" + Number(valueOfObject) + " parseFloat=" + parseFloat(valueOfObject));
attempt("symbol_Number", () => String(Number(Symbol("s") as any)));
attempt("symbol_parseInt", () => String(parseInt(Symbol("s") as any)));
attempt("symbol_parseFloat", () => String(parseFloat(Symbol("s") as any)));
console.log("symbol_String_ok=" + String(Symbol("s")));

// --- Number() with no argument is +0; with undefined it is NaN ---
console.log("number_noargs=" + Number() + " pos_zero=" + Object.is(Number(), 0));
console.log("number_undefined=" + Number(undefined) + " number_null=" + Number(null));
