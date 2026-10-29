// Cross-runtime: the three substring extractors side by side. slice treats a negative
// index as an offset from the end and returns "" when start >= end; substring swaps
// reversed bounds and clamps negatives to 0; substr takes (start, length) and treats
// a negative start as an offset from the end. The same numbers give different answers,
// and every argument goes through ToIntegerOrInfinity (truncation, NaN to 0, infinities
// clamped). Output is ASCII-only.

function show(x: unknown): string {
  if (x === undefined) return "undefined";
  if (x === null) return "null";
  if (typeof x === "string") {
    return '"' + x.replace(/[^\x20-\x7e]/g, (c) => "\\u" + c.charCodeAt(0).toString(16).padStart(4, "0")) + '"';
  }
  if (Array.isArray(x)) return "[" + x.map((e) => show(e)).join(",") + "]";
  return String(x);
}

const s = "abcdef"; // indices 0..5

// slice(a, b) and substring(a, b) for the same arguments, on one line
function pair(a: any, b?: any): string {
  return "[" + s.substring(a, b) + "][" + s.slice(a, b) + "]";
}

// --- ordered ranges: identical answers ---
console.log("ordered-1-4=" + pair(1, 4));
console.log("ordered-0-6=" + pair(0, 6));
console.log("ordered-2-2=" + pair(2, 2));

// --- reversed ranges: substring swaps the bounds, slice returns empty ---
console.log("reversed-4-1=" + pair(4, 1));
console.log("reversed-6-0=" + pair(6, 0));
console.log("reversed-3-2=" + pair(3, 2));
console.log("swap-length=" + s.substring(5, 1).length + ":" + s.slice(5, 1).length);

// --- negative start: substring clamps to 0, slice counts from the end ---
console.log("neg-start-3-2=" + pair(-3, 2));
console.log("neg-start-3-undef=" + pair(-3, undefined));
console.log("neg-start-1-undef=" + pair(-1, undefined));
console.log("neg-start-99-undef=" + pair(-99, undefined));

// --- negative end: substring clamps to 0, slice counts from the end ---
console.log("neg-end-1-neg1=" + pair(1, -1));
console.log("neg-end-0-neg99=" + pair(0, -99));
console.log("neg-end-neg4-neg2=" + pair(-4, -2));

// --- beyond the end, and both bounds past the end ---
console.log("past-end-2-99=" + pair(2, 99));
console.log("past-end-99-2=" + pair(99, 2));
console.log("past-end-99-undef=" + "[" + s.substring(99) + "][" + s.slice(99) + "]");
console.log("past-end-both=" + pair(99, 100));

// --- NaN becomes 0 for both; undefined end means "to the end" ---
console.log("nan-3=" + pair(NaN, 3));
console.log("3-nan=" + pair(3, NaN));
console.log("undef-3=" + pair(undefined, 3));
console.log("2-undef=" + pair(2, undefined));
console.log("no-args=" + "[" + s.substring() + "][" + s.slice() + "]");

// --- infinities: +Infinity clamps to the length, -Infinity to 0 (or the end) ---
console.log("0-inf=" + pair(0, Infinity));
console.log("inf-2=" + pair(Infinity, 2));
console.log("neginf-2=" + pair(-Infinity, 2));
console.log("0-neginf=" + pair(0, -Infinity));

// --- fractions truncate toward zero; -0 behaves as 0 ---
console.log("frac=" + pair(1.9, 4.9));
console.log("frac-neg=" + pair(-1.9, undefined));
console.log("negzero=" + "[" + s.substring(-0) + "][" + s.slice(-0) + "]");
console.log("negzero-end=" + pair(0, -0));

// --- non-number arguments go through ToNumber first ---
console.log("str-args=" + pair("2" as any, "4" as any));
console.log("bool-args=" + pair(true as any, 3 as any));
const boxed: any = { valueOf() { return 1; } };
console.log("obj-args=" + pair(boxed, 3));
console.log("null-args=" + pair(null as any, 3));
console.log("str-bad-args=" + pair("x" as any, 3));

// --- the empty string never produces anything ---
console.log("empty=" + "[" + "".substring(-5, 5) + "][" + "".slice(-5, 5) + "]");

// --- substr(start, length): negative start is from the end, negative length is empty ---
const t = "hello world";
console.log("substr-0-5=" + show(t.substr(0, 5)));
console.log("substr-6=" + show(t.substr(6)));
console.log("substr-6-5=" + show(t.substr(6, 5)));
console.log("substr-neg5=" + show(t.substr(-5)));
console.log("substr-neg5-2=" + show(t.substr(-5, 2)));
console.log("substr-neg-length=" + show(t.substr(2, -1)));
console.log("substr-neg-start-big-length=" + show(t.substr(-5, 99)));
console.log("substr-neg-start-past=" + show(t.substr(-99, 5)));
console.log("substr-neg-start-zero-len=" + show(t.substr(-5, 0)));
console.log("substr-fraction=" + show(t.substr(1.9, 2.9)));
console.log("substr-nan-len=" + show(t.substr(0, NaN)));
console.log("substr-undef-len=" + show(t.substr(6, undefined)));
console.log("substr-cross-runtime=" + show("cross-runtime".substr(-7)) + "/" + show("cross-runtime".substr(-7, 3)));

// --- the three methods on the same bounds, in one place ---
console.log("substring-1-3=" + show(t.substring(1, 3)));
console.log("slice-1-3=" + show(t.slice(1, 3)));
console.log("substring-5-2=" + show(t.substring(5, 2)));
console.log("slice-5-2=" + show(t.slice(5, 2)));
console.log("substring-rest=" + show(t.substring(6)) + " slice-rest=" + show(t.slice(6)));
console.log("slice-6-neg1=" + show(t.slice(6, -1)));
console.log("slice-0-neg6=" + show(t.slice(0, -6)));
console.log("slice-neg-11=" + show(t.slice(-11)) + " past=" + show(t.slice(-100)));
console.log("substr-vs-slice-same=" + (t.substr(0, 5) === t.slice(0, 5)) + " " + (t.substr(-5) === t.slice(-5)));
