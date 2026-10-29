// Cross-runtime: every road from a value to a string -- the + operator, String(),
// template literals, ToPrimitive hints (toString vs valueOf vs Symbol.toPrimitive),
// concat's argument coercion and its left-to-right order, relational comparison by
// UTF-16 code unit, default sort, switch over strings, and a deterministic subset
// of localeCompare. Output is ASCII-only: non-ASCII is escaped by show().

function show(x: unknown): string {
  if (x === undefined) return "undefined";
  if (x === null) return "null";
  if (typeof x === "string") {
    return '"' + x.replace(/[^\x20-\x7e]/g, (c) => "\\u" + c.charCodeAt(0).toString(16).padStart(4, "0")) + '"';
  }
  if (Array.isArray(x)) return "[" + x.map((e) => show(e)).join(",") + "]";
  return String(x);
}

function attempt(f: () => unknown): string {
  try {
    return show(f());
  } catch (e: any) {
    return "!" + e.name;
  }
}

// --- the + operator: numeric addition until the first string, then concatenation ---
console.log("plus-num-then-str=" + show(1 + 2 + "x" + 3 + 4));
console.log("plus-str-first=" + show("a" + 1 + true));
console.log("plus-null-undef=" + show("n" + null + "u" + undefined));
console.log("plus-bools=" + show("b" + true + false));
console.log("plus-inf-nan=" + show("" + 1 / 0 + "," + -1 / 0 + "," + 0 / 0));
console.log("plus-neg-zero=" + show("" + -0));
console.log("plus-exp=" + show("" + 1e21 + " " + 1e-7 + " " + 123456789012345680000));
console.log("plus-array=" + show("" + [1, 2, 3]));
console.log("plus-nested-array=" + show("" + [[1], [2, [3]]]));
console.log("plus-array-holes=" + show("" + [null, undefined, 1]));
console.log("plus-object=" + show("" + {}));

// --- String(): explicit conversion; unlike +, it accepts a Symbol ---
console.log("String-num=" + show(String(42)));
console.log("String-bool=" + show(String(true)) + "/" + show(String(false)));
console.log("String-nullish=" + show(String(null)) + "/" + show(String(undefined)));
console.log("String-arr=" + show(String([1, 2, 3])));
console.log("String-empty-arr=" + show(String([])));
console.log("String-obj=" + show(String({ a: 1 })));
console.log("String-symbol=" + attempt(() => String(Symbol("tag"))));
console.log("String-symbol-plus=" + attempt(() => "" + Symbol("tag")));
console.log("String-symbol-template=" + attempt(() => `${Symbol("tag")}`));

// --- template literals use ToString (hint string), so toString wins ---
const obj = {
  toString() { return "T"; },
  valueOf() { return 42; },
};
console.log("tpl-obj=" + show(`val=${obj}`));
console.log("tpl-obj-and-plus=" + show(`${obj} and ${obj + 1}`));
console.log("tpl-arr=" + show(`arr=${[1, 2, 3]}`));
console.log("tpl-nested=" + show(`nested=${[[1, 2], [3, 4]]}`));
console.log("tpl-nullish=" + show(`null=${null} undef=${undefined}`));
const o2 = { toString() { return "only-str"; } };
console.log("tpl-only-toString=" + show(`o2=${o2}`));

// --- Symbol.toPrimitive receives the hint and decides the answer ---
const sym = { [Symbol.toPrimitive](h: string) { return h === "string" ? "S" : 99; } };
console.log("toPrimitive-tpl=" + show(`sym=${sym}`));
console.log("toPrimitive-plus=" + show("" + sym));
console.log("toPrimitive-minus=" + show((sym as any) - 1));
const badPrim = { [Symbol.toPrimitive]() { return {}; } };
console.log("toPrimitive-object-result=" + attempt(() => "" + badPrim));
const bothObjects = { toString() { return {}; }, valueOf() { return {}; } };
console.log("both-return-objects=" + attempt(() => "" + bothObjects));

// --- hints: default and number use valueOf first, string uses toString first ---
const both = {
  toString() { return "STR"; },
  valueOf() { return 7; },
};
console.log("hint-plus-empty=" + show("" + both));
console.log("hint-template=" + show(`${both}`));
console.log("hint-String=" + show(String(both)));
console.log("hint-plus-after=" + show(both + ""));
console.log("hint-add=" + show(both + 1));
console.log("hint-mul=" + show((both as any) * 2));
console.log("hint-compare=" + show((both as any) > 6));
console.log("hint-join=" + show([both].join("-")));
console.log("hint-array-tpl=" + show(`${[both, both]}`));
const onlyValue = { valueOf() { return 99; } };
console.log("only-valueOf-plus=" + show("" + onlyValue));
console.log("only-valueOf-tpl=" + show(`${onlyValue}`));
console.log("only-valueOf-String=" + show(String(onlyValue)));
console.log("only-valueOf-join=" + show([onlyValue].join("")));

// --- String.prototype.concat: arity from 0 to many, apply, spread, empties ---
const base = "x";
console.log("concat-zero=" + show(base.concat()));
console.log("concat-one=" + show(base.concat("a")));
console.log("concat-five=" + show(base.concat("a", "b", "c", "d", "e")));
console.log("concat-ten=" + show("".concat("0", "1", "2", "3", "4", "5", "6", "7", "8", "9")));
const many: string[] = [];
for (let i = 0; i < 64; i++) many.push(String(i % 10));
const big = String.prototype.concat.apply("", many);
console.log("concat-apply64-len=" + big.length);
console.log("concat-apply64=" + show(big.slice(0, 12) + "..." + big.slice(-4)));
console.log("concat-spread=" + show("".concat(...["p", "q", "r"])));
console.log("concat-spread-empty=" + "".concat(...([] as string[])).length);
console.log("concat-empties=" + show("".concat("", "", "")) + "/" + show("a".concat("", "b", "", "c")));

// --- concat coerces each argument with ToString, exactly like + does for objects ---
console.log("concat-int=" + show("n=".concat(42 as any)));
console.log("concat-negzero=" + show("n=".concat(-0 as any)));
console.log("concat-float=" + show("n=".concat(1.5 as any)));
console.log("concat-exp=" + show("n=".concat(1e21 as any)));
console.log("concat-tiny=" + show("n=".concat(1e-7 as any)));
console.log("concat-nan-inf=" + show("n=".concat(NaN as any, ",", -Infinity as any)));
console.log("concat-bools=" + show("b=".concat(true as any, ",", false as any)));
console.log("concat-null-undef=" + show("".concat(null as any, "|", undefined as any)));
console.log("concat-array=" + show("a=".concat([1, 2, 3] as any)));
console.log("concat-array-nested=" + show("".concat([[1], [2, 3]] as any)));
console.log("concat-array-holes=" + show("".concat([null, undefined, 1] as any)));
console.log("concat-obj=" + show("".concat({} as any)));
const custom: any = { toString: () => "CUSTOM" };
console.log("concat-custom=" + show("".concat(custom)));
const bothConcat: any = { toString: () => "STR", valueOf: () => 99 };
console.log("concat-prefers-toString=" + show("".concat(bothConcat)));
console.log("concat-mixed=" + show("".concat("a", 1 as any, true as any, null as any, undefined as any, [2] as any)));
console.log("concat-symbol=" + attempt(() => "".concat(Symbol("s") as any)));

// --- coercion order is left to right, one ToString per argument ---
const order: string[] = [];
const oa: any = { toString() { order.push("a"); return "A"; } };
const ob: any = { toString() { order.push("b"); return "B"; } };
console.log("concat-order-result=" + show("x".concat(oa, 2 as any, ob)));
console.log("concat-order=" + order.join(","));
order.length = 0;
console.log("plus-order-result=" + show(oa + "" + ob));
console.log("plus-order=" + order.join(","));

// --- concat never mutates its receiver; it equals the + chain ---
const src = "keep";
src.concat("more");
console.log("concat-no-mutate=" + show(src) + "/" + src.length);
console.log("concat-eq-plus=" + ("a".concat("b", "c") === "a" + "b" + "c"));
console.log("concat-call-num=" + show(String.prototype.concat.call(5 as any, "!")));
console.log("concat-call-bool=" + show(String.prototype.concat.call(true as any, "!")));
const hi = String.fromCharCode(0xd83d);
const lo = String.fromCharCode(0xde00);
console.log("concat-halves-len=" + hi.concat(lo).length);
console.log("concat-halves-cp=" + hi.concat(lo).codePointAt(0)!.toString(16));

// --- relational comparison is by UTF-16 code unit; default sort uses it too ---
console.log("rel-a-b=" + ("a" < "b") + " " + ("b" < "a"));
console.log("rel-apple-banana=" + ("apple" < "banana"));
console.log("rel-Z-a=" + ("Z" < "a"));
console.log("rel-10-9=" + ("10" < "9"));
console.log("rel-prefix=" + ("abc" < "abd") + " " + ("abc" < "ab") + " " + ("ab" < "abc"));
console.log("rel-empty=" + ("" < "a"));
console.log("rel-accent=" + ("caf\u00e9" < "cafz"));
console.log("rel-astral-vs-a=" + ("\u{1F600}" < "a"));
console.log("rel-z-vs-astral=" + ("z" < "\u{1F600}"));
console.log("rel-fullwidth-vs-astral=" + ("\uff3a" < "\u{1F600}"));
console.log("rel-A-vs-0=" + ("A" > "0"));
console.log("rel-Apple-apple=" + ("Apple" < "apple"));
console.log("rel-le-equal=" + ("apple" <= "apple"));
console.log("sort-default=" + show(["banana", "apple", "Cherry", "10", "9"].sort()));

// --- localeCompare: only the sign of results for ASCII letters, identical on both ---
console.log("lc-a-b=" + Math.sign("a".localeCompare("b")));
console.log("lc-b-a=" + Math.sign("b".localeCompare("a")));
console.log("lc-a-a=" + "a".localeCompare("a"));
console.log("lc-apple-banana=" + Math.sign("apple".localeCompare("banana")));
console.log("lc-abc-abd=" + Math.sign("abc".localeCompare("abd")));
console.log("lc-empty=" + Math.sign("".localeCompare("a")));

// --- switch uses strict equality, so it is case-sensitive and type-sensitive ---
function color(s: string): number {
  switch (s) {
    case "red": return 1;
    case "green": return 2;
    case "blue": return 3;
    default: return 0;
  }
}
function priority(s: string): string {
  switch (s) {
    case "high":
    case "urgent":
      return "important";
    case "low":
      return "minor";
    default:
      return "normal";
  }
}
function kind(x: any): string {
  switch (x) {
    case 1: return "num";
    case "1": return "str";
    default: return "other";
  }
}
console.log("switch-color=" + color("red") + "/" + color("green") + "/" + color("yellow"));
console.log("switch-priority=" + priority("high") + "/" + priority("urgent") + "/" + priority("low") + "/" + priority("med"));
console.log("switch-kind-num=" + kind(1));
console.log("switch-kind-str=" + kind("1"));
console.log("switch-kind-boxed=" + kind(new String("1")));
console.log("switch-case-sensitive=" + color("Red"));
