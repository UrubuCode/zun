// Math rounding family (floor, ceil, round, trunc), sign and abs.
// Focus: negative values, exact .5 ties, the -0 windows of round/ceil/trunc,
// the largest double below 0.5, toFixed as a decimal rounding, coercion of
// non-number arguments, and non-finite inputs. Every printed value is exact.

type Num = (x: number) => number;

function fmt(v: number): string {
  return String(v) + (Object.is(v, -0) ? "(-0)" : "");
}

// --- ordinary values on both sides of zero ---
console.log("floor_2.9=" + fmt(Math.floor(2.9)));
console.log("floor_-2.1=" + fmt(Math.floor(-2.1)));
console.log("floor_-2.9=" + fmt(Math.floor(-2.9)));
console.log("floor_0=" + fmt(Math.floor(0)));
console.log("floor_-0=" + fmt(Math.floor(-0)));
console.log("ceil_2.1=" + fmt(Math.ceil(2.1)));
console.log("ceil_-2.9=" + fmt(Math.ceil(-2.9)));
console.log("ceil_-2.1=" + fmt(Math.ceil(-2.1)));
console.log("ceil_-0.1=" + fmt(Math.ceil(-0.1)));
console.log("ceil_0=" + fmt(Math.ceil(0)));
console.log("trunc_5.9=" + fmt(Math.trunc(5.9)));
console.log("trunc_-5.9=" + fmt(Math.trunc(-5.9)));
console.log("trunc_0.1=" + fmt(Math.trunc(0.1)));
console.log("trunc_-0.1=" + fmt(Math.trunc(-0.1)));
console.log("trunc_NaN=" + fmt(Math.trunc(NaN)));
console.log("round_2.4=" + fmt(Math.round(2.4)));
console.log("round_2.6=" + fmt(Math.round(2.6)));
console.log("round_0=" + fmt(Math.round(0)));

// --- the rounding family side by side: halves, negative halves, zeros ---
function row(x: number): void {
  const r = Math.round(x);
  console.log(
    String(x) +
      " | round:" + fmt(r) +
      " | floor+0.5:" + fmt(Math.floor(x + 0.5)) +
      " | ceil-0.5:" + fmt(Math.ceil(x - 0.5)) +
      " | trunc:" + fmt(Math.trunc(x)) +
      " | floor:" + fmt(Math.floor(x)) +
      " | ceil:" + fmt(Math.ceil(x))
  );
}

// ties round toward +Infinity: 2.5 -> 3 but -2.5 -> -2, never away from zero
row(0.5);
row(1.5);
row(2.5);
row(3.5);
row(-0.5);
row(-1.5);
row(-2.5);
row(-3.5);

// either side of a tie
row(0.4);
row(0.6);
row(-0.4);
row(-0.6);
row(2.49999);
row(2.50001);

// the zeros: round and trunc keep the sign, floor(x+0.5) destroys it
row(0);
row(-0);
row(-0.2);
row(-0.1);

// the famous double just below a half: adding 0.5 rounds up in binary
row(0.49999999999999994);
row(-0.49999999999999994);
row(-0.5000000000000001);
row(0.5000000000000001);
row(1.4999999999999998);

// above 2^52 every double is an integer, so the variants agree
row(4503599627370496);
row(4503599627370495.5);
row(4503599627370497);
row(9007199254740992);
row(9007199254740993);
row(-9007199254740992);
row(1e21);
row(-1e21);

// non-finite and the extreme finite values
row(Infinity);
row(-Infinity);
row(NaN);
row(Number.MAX_VALUE);
row(Number.MIN_VALUE);
row(-Number.MIN_VALUE);

// --- Math.round's explicit sign rules ---
console.log("round(-0)_neg0=" + Object.is(Math.round(-0), -0));
console.log("round(-0.2)_neg0=" + Object.is(Math.round(-0.2), -0));
console.log("round(-0.5)_neg0=" + Object.is(Math.round(-0.5), -0));
console.log("round(-0.500001)=" + fmt(Math.round(-0.500001)));
console.log("round(0.49999999999999994)=" + fmt(Math.round(0.49999999999999994)));

// --- toFixed(0) is a DECIMAL rounding (ties away from zero) that returns a string ---
const fixed: number[] = [0.5, 1.5, 2.5, -0.5, -1.5, -2.5, 0.49999999999999994, 1.005, -0];
for (const x of fixed) {
  console.log(
    "toFixed0(" + String(x) + ")=" + x.toFixed(0) +
      " round=" + fmt(Math.round(x)) +
      " agree=" + (x.toFixed(0) === String(Math.round(x)))
  );
}

// --- coercion: every rounding method runs ToNumber first ---
const vals: [string, any][] = [
  ["undefined", undefined],
  ["null", null],
  ["true", true],
  ["empty_string", ""],
  ["hex_string", "0x10"],
  ["bad_string", "12abc"],
  ["empty_array", []],
  ["one_array", [9]],
  ["date_like", { valueOf: () => 7.5 }],
  ["string_only", { toString: () => "-3" }],
];

function coerceRow(name: string, fn: Num): void {
  const cells: string[] = [];
  for (const pair of vals) {
    cells.push(pair[0] + ":" + fmt(fn(pair[1] as any)));
  }
  console.log("coerce_" + name + " | " + cells.join(" "));
}

coerceRow("abs", Math.abs);
coerceRow("sign", Math.sign);
coerceRow("floor", Math.floor);
coerceRow("ceil", Math.ceil);
coerceRow("round", Math.round);
coerceRow("trunc", Math.trunc);

console.log("round_null_neg0=" + Object.is(Math.round(null as any), -0));
console.log("round_null=" + fmt(Math.round(null as any)));
console.log("round_no_args=" + fmt((Math.round as any)()));
console.log("round_length=" + Math.round.length);

// --- sign and abs: the signed-zero and non-finite rows ---
console.log("sign_5=" + fmt(Math.sign(5)));
console.log("sign_-5=" + fmt(Math.sign(-5)));
console.log("sign_-7=" + fmt(Math.sign(-7)));
console.log("sign_0=" + fmt(Math.sign(0)));
console.log("sign_-0=" + fmt(Math.sign(-0)));
console.log("sign_NaN=" + fmt(Math.sign(NaN)));
console.log("sign_Inf=" + fmt(Math.sign(Infinity)));
console.log("sign_-Inf=" + fmt(Math.sign(-Infinity)));
console.log("sign_abs=" + fmt(Math.sign(Math.abs(-5))));
console.log("abs_5=" + fmt(Math.abs(5)));
console.log("abs_-5=" + fmt(Math.abs(-5)));
console.log("abs_-12=" + fmt(Math.abs(-12)));
console.log("abs_0=" + fmt(Math.abs(0)));
console.log("abs_-0=" + fmt(Math.abs(-0)));
console.log("abs_-Inf=" + fmt(Math.abs(-Infinity)));
console.log("abs_NaN=" + fmt(Math.abs(NaN)));
console.log("abs_-3.14=" + fmt(Math.abs(-3.14)));
console.log("abs_str=" + fmt(Math.abs(Number("-10"))));
console.log("abs_-MIN_VALUE=" + fmt(Math.abs(-Number.MIN_VALUE)));
console.log("abs_-MAX_VALUE=" + fmt(Math.abs(-Number.MAX_VALUE)));

// --- extra arguments to a unary method are ignored and never coerced ---
const boom: any = {
  valueOf() {
    throw new RangeError("must not run");
  },
};
console.log("abs_extra_args=" + fmt((Math.abs as any)(-1, boom)));
console.log("sqrt_extra_args=" + fmt((Math.sqrt as any)(16, boom)));

// --- BigInt and Symbol cannot be converted by ToNumber at all ---
function attempt(label: string, fn: () => any): void {
  try {
    console.log(label + "=" + fmt(fn()));
  } catch (e) {
    console.log(label + "!" + (e as any).name);
  }
}
attempt("abs_bigint", () => Math.abs(1n as any));
attempt("floor_bigint", () => Math.floor(1n as any));
attempt("round_symbol", () => Math.round(Symbol("s") as any));
attempt("trunc_symbol", () => Math.trunc(Symbol("s") as any));

// --- the sign-preserving family over the five special inputs ---
const inputs: [string, number][] = [
  ["NaN", NaN],
  ["+0", 0],
  ["-0", -0],
  ["+Inf", Infinity],
  ["-Inf", -Infinity],
];

function grid(name: string, fn: Num): void {
  const cells: string[] = [];
  for (const pair of inputs) {
    cells.push(pair[0] + ":" + fmt(fn(pair[1])));
  }
  console.log("grid_" + name + " | " + cells.join(" | "));
}

grid("abs", Math.abs);
grid("sign", Math.sign);
grid("trunc", Math.trunc);
grid("floor", Math.floor);
grid("ceil", Math.ceil);
grid("round", Math.round);
