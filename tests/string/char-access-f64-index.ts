// Cross-runtime: typed string char-access with a JS `number` index. charCodeAt,
// charAt, and bracket indexing on a statically typed `string` must match Node
// byte-for-byte across ToIntegerOrInfinity coercion (fractional, NaN, +/-
// Infinity), negative indices, out-of-range indices, and UTF-16 code units
// including a surrogate pair. Every result goes through `show` so `undefined`
// and the empty string are visible.

function show(x: unknown): string {
  if (x === undefined) return "undefined";
  if (typeof x === "string") return JSON.stringify(x);
  return String(x);
}

const s: string = "abc";
// A surrogate pair: U+1F600, stored as two UTF-16 code units.
const astral: string = "x\u{1F600}y";

// charCodeAt: always a number; out of range -> NaN.
console.log(show(s.charCodeAt(0)));
console.log(show(s.charCodeAt(2)));
console.log(show(s.charCodeAt(3)));
console.log(show(s.charCodeAt(99)));
console.log(show(s.charCodeAt(-1)));
console.log(show(s.charCodeAt(1.9)));
console.log(show(s.charCodeAt(NaN)));
console.log(show(s.charCodeAt(Infinity)));
console.log(show(s.charCodeAt(-Infinity)));
console.log(show(astral.charCodeAt(1)));
console.log(show(astral.charCodeAt(2)));

// charAt: always a string; out of range -> "".
console.log(show(s.charAt(0)));
console.log(show(s.charAt(2)));
console.log(show(s.charAt(3)));
console.log(show(s.charAt(-1)));
console.log(show(s.charAt(1.9)));
console.log(show(s.charAt(NaN)));
console.log(show(s.charAt(Infinity)));

// Bracket indexing: a string for a canonical in-range integer, else undefined.
console.log(show(s[0]));
console.log(show(s[2]));
console.log(show(s[3]));
console.log(show(s[-1]));
console.log(show(s[1.5]));

// A dynamic (non-constant) numeric index exercises the same typed path.
let i = 1;
console.log(show(s.charCodeAt(i)));
console.log(show(s.charAt(i)));
console.log(show(s[i]));
