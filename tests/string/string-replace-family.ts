// Cross-runtime: String.prototype.replace and replaceAll in full. Covers the $-token
// table (for string and regexp patterns, with and without groups and named groups),
// the exact arguments a replacer function receives (match, groups..., offset, string,
// and the groups object only with named captures), how its return value is coerced,
// the empty pattern (replace inserts once; replaceAll between every code unit, which
// tears astral pairs), replaceAll's TypeError for a non-global regexp, lastIndex
// handling, and the Symbol.replace dispatch that lets any object take over.
// Output is ASCII-only.

function show(x: unknown): string {
  if (x === undefined) return "undefined";
  if (x === null) return "null";
  if (typeof x === "string") {
    return '"' + x.replace(/[^\x20-\x7e]/g, (c) => "\\u" + c.charCodeAt(0).toString(16).padStart(4, "0")) + '"';
  }
  if (Array.isArray(x)) return "[" + x.map((e) => show(e)).join(",") + "]";
  if (typeof x === "object") {
    // null-prototype objects (such as the replacer's groups argument) have no toString; print their keys
    const o = x as any;
    return "{" + Object.keys(o).map((k) => k + ":" + show(o[k])).join(",") + "}";
  }
  return String(x);
}

function attempt(f: () => unknown): string {
  try {
    return show(f());
  } catch (e: any) {
    return "!" + e.name;
  }
}

// --- replace with a string pattern replaces only the first occurrence ---
console.log("first=" + show("aXbXcX".replace("X", "-")));
console.log("first-word=" + show("cat cat cat".replace("cat", "dog")));
console.log("no-match=" + show("abc".replace("z", "!")));
console.log("whole=" + show("abc".replace("abc", "Z")) + " empty-repl=" + show("aXbX".replace("X", "")));

// --- dollar tokens with a string pattern: $& $` $' $$ are live; $1 $0 $<n> are literal ---
console.log("amp=" + show("hello".replace("ll", "[$&]")) + " twice=" + show("hello".replace("ll", "$&$&")));
console.log("prefix=" + show("abcdef".replace("cd", "[$`]")) + " prefix-start=" + show("abcdef".replace("ab", "[$`]")));
console.log("suffix=" + show("abcdef".replace("cd", "[$']")) + " suffix-end=" + show("abcdef".replace("ef", "[$']")));
console.log("dollar=" + show("price".replace("price", "$$")) + " amount=" + show("cost".replace("cost", "$$100")) + " then-amp=" + show("xy".replace("x", "$$$&")));
console.log("group-literal=" + show("abc".replace("b", "$1")) + " zero=" + show("abc".replace("b", "$0")) + " named=" + show("abc".replace("b", "$<n>")));
console.log("lone-dollar=" + show("abc".replace("b", "$")) + " dollar-x=" + show("abc".replace("b", "$x")) + " trailing=" + show("abc".replace("b", "z$")));
console.log("combo=" + show("abcdef".replace("cd", "<$`|$&|$'>")) + " combo2=" + show("12345".replace("3", "$`$&$'")));
console.log("num-pattern=" + show("a1b1".replace(1 as any, "-")) + " num-repl=" + show("abc".replace("b", 42 as any)));

// --- dollar tokens with a regexp, numbered and named ---
console.log("re-group=" + show("2023-01-02".replace(/(\d+)-(\d+)-(\d+)/, "$3/$2/$1")));
console.log("re-swap=" + show("John Smith".replace(/(\w+)\s(\w+)/, "$2 $1")));
console.log("re-amp=" + show("abc".replace(/b/, "[$&]")) + " re-escaped=" + show("price: 100".replace(/(\d+)/, "$$$1")));
console.log("re-named=" + show("test".replace(/(?<first>t)/, "[$<first>]")) + " re-unknown-named=" + show("a1".replace(/(?<l>[a-z])/g, "[$<zz>]")));
console.log("re-oob-group=" + show("ab".replace(/(a)/, "$2")) + " re-nomatch-groups=" + show("nomatch".replace(/(z)/, "$1$2")));
console.log("re-named-no-groups=" + show("abc".replace(/b/, "$<x>")) + " named-with-groups=" + show("abc".replace(/(?<y>b)/, "[$<x>]")));
console.log("re-unterminated=" + show("abc".replace(/(?<y>b)/, "[$<y]")) + " re-dollar-dollar=" + show("abc".replace(/b/, "$$")));
console.log("re-capture-swap=" + show("x1y2".replace(/(\w)(\d)/g, (_, a, b) => b + a)));

// --- the replacer function: exact argument list, and its return value ---
function dump(args: any[]): string {
  return args.length + "[" + args.map((a) => show(a)).join(",") + "]";
}
console.log("fn-str-pat=" + "abc".replace("b", (...a: any[]) => dump(a)));
console.log("fn-str-empty=" + "abc".replace("", (...a: any[]) => dump(a)));
console.log("fn-no-cap=" + "abc".replace(/b/, (...a: any[]) => dump(a)));
console.log("fn-one-cap=" + "abc".replace(/(b)/, (...a: any[]) => dump(a)));
console.log("fn-nonpart=" + "abc".replace(/(a)(z)?(b)/, (...a: any[]) => dump(a)));
console.log("fn-named=" + "abc".replace(/(?<x>b)/, (...a: any[]) => dump(a)));
console.log("fn-named-nonpart=" + "b".replace(/(?<p>a)|(?<q>b)/, (...a: any[]) => {
  const g = a[a.length - 1];
  return show(g.p) + "/" + show(g.q) + "/" + Object.keys(g).join("+");
}));
console.log("fn-groups-proto=" + "b".replace(/(?<q>b)/, (...a: any[]) => show(Object.getPrototypeOf(a[a.length - 1]))));
const offsets: string[] = [];
"aXbXXc".replace(/X+/g, (m: string, off: number, whole: string) => {
  offsets.push(off + ":" + m.length + ":" + whole.length);
  return "-";
});
console.log("fn-offsets=" + offsets.join(","));
const allOffsets: number[] = [];
"aXbXc".replaceAll("X", (...a: any[]) => {
  allOffsets.push(a[a.length - 2]);
  return "-";
});
console.log("fn-all-offsets=" + allOffsets.join(","));
console.log("fn-str-tokens-not-expanded=" + "abc".replace("b", () => "$&") + " re=" + "abc".replace(/(b)/, () => "$1"));
console.log("ret-num=" + "a".replace(/a/, (() => 42) as any) + " ret-undef=" + "a".replace(/a/, (() => undefined) as any) + " ret-null=" + "a".replace(/a/, (() => null) as any));
console.log("ret-bool=" + "a".replace(/a/, (() => true) as any) + " ret-arr=" + "a".replace(/a/, (() => [1, 2]) as any));
console.log("ret-obj=" + "a".replace(/a/, (() => ({ toString() { return "O"; } })) as any));
console.log("ret-symbol=" + attempt(() => "a".replace(/a/, (() => Symbol("s")) as any)));
const order: string[] = [];
"a1b2c3".replace(/\d/g, (m: string) => {
  order.push(m);
  return "";
});
console.log("fn-order=" + order.join(""));
const reMut = /\d/g;
let calls = 0;
const mutated = "1234".replace(reMut, (m: string) => {
  calls++;
  reMut.lastIndex = 0;
  return "<" + m + ">";
});
console.log("fn-mutating=" + mutated + ":" + calls + ":" + reMut.lastIndex);

// --- the empty pattern: replace inserts once at the front; replaceAll inserts everywhere ---
console.log("empty-replace=" + show("abc".replace("", "-")) + " on-empty=" + show("".replace("", "-")));
console.log("empty-replace-fn=" + show("abc".replace("", () => "-")) + " amp=" + show("abc".replace("", "[$&]")));
console.log("empty-backtick=" + show("abc".replace("", "[$`]")) + " quote=" + show("abc".replace("", "[$']")));
console.log("empty-index=" + "abc".replace("", function (m: string, i: number, s: string) { return "[" + show(m) + "," + i + "," + s + "]"; }));
console.log("replaceAll-empty=" + show("abc".replaceAll("", "-")) + " count=" + "abc".replaceAll("", "-").split("-").length);
console.log("replaceAll-empty-on-empty=" + show("".replaceAll("", "-")) + " one=" + show("a".replaceAll("", "-")));
const GRIN = String.fromCharCode(0xd83d) + String.fromCharCode(0xde00);
const torn = ("a" + GRIN + "b").replaceAll("", "-");
console.log("astral-torn-len=" + torn.length + " insertions=" + (torn.split("-").length - 1) + " wf=" + torn.isWellFormed());
console.log("regex-empty-g=" + show(("a" + GRIN + "b").replace(/(?:)/g, "-")));
console.log("regex-empty-gu-count=" + (("a" + GRIN + "b").replace(/(?:)/gu, "-").split("-").length - 1));
console.log("regex-empty-no-g=" + show("abc".replace(/(?:)/, "-")));
console.log("star=" + show("aXbXc".replace(/X*/g, "-")) + " opt=" + show("abc".replace(/x?/g, "-")));
console.log("boundary=" + show("ab cd".replace(/\b/g, "|")) + " lookahead=" + show("abc".replace(/(?=b)/g, "-")));
console.log("fn-empty-calls=" + (() => { const seen: string[] = []; "ab".replaceAll("", function (m: string, i: number) { seen.push(show(m) + "@" + i); return "-"; } as any); return seen.join(" "); })());

// --- replaceAll: a string pattern is literal, a global regexp runs everywhere ---
console.log("all-str=" + show("a-b-c".replaceAll("-", "/")) + " all-re=" + show("foo1foo2".replaceAll(/foo/g, "x")));
console.log("all-fn=" + show("a1b2".replaceAll(/\d/g, (m) => String(Number(m) * 3))));
console.log("all-literal-dot=" + show("a.b.c".replaceAll(".", "-")) + " star=" + show("a*b*".replaceAll("*", "+")));
console.log("all-literal-class=" + show("x[a]y[a]".replaceAll("[a]", "Z")) + " backslash=" + show("a\\db".replaceAll("\\d", "!")));
console.log("all-literal-pipe=" + show("a|b".replaceAll("|", "/")) + " paren=" + show("f(x)f(x)".replaceAll("(x)", "()")));
console.log("all-overlap=" + show("aaaa".replaceAll("aa", "X")) + " overlap3=" + show("aaaaa".replaceAll("aa", "X")));
console.log("all-tokens=" + show("a-b-c".replaceAll("-", "[$&]")) + " prefix=" + show("a-b".replaceAll("-", "[$`]")) + " suffix=" + show("a-b".replaceAll("-", "[$']")));
console.log("all-literal-group=" + show("a-b".replaceAll("-", "$1")) + " named=" + show("a-b".replaceAll("-", "$<n>")) + " lone=" + show("a-b".replaceAll("-", "$")));
console.log("all-dollar=" + show("a-b".replaceAll("-", "$$")));
console.log("all-re-group=" + show("a1b2".replaceAll(/([a-z])(\d)/g, "$2$1")) + " all-re-named=" + show("a1b2".replaceAll(/(?<l>[a-z])(?<d>\d)/g, "$<d>$<l>")));
console.log("all-re-amp=" + show("a1b2".replaceAll(/\d/g, "<$&>")) + " unknown=" + show("a1".replaceAll(/(?<l>[a-z])/g, "[$<zz>]")) + " oob=" + show("ab".replaceAll(/(a)/g, "$2")));
console.log("all-nonglobal=" + attempt(() => "aa".replaceAll(/a/ as any, "-")));
console.log("all-sticky-only=" + attempt(() => "aa".replaceAll(/a/y as any, "-")));
console.log("all-ignorecase-only=" + attempt(() => "aA".replaceAll(/a/i as any, "-")));
console.log("all-gy=" + attempt(() => "aa".replaceAll(/a/gy, "-")) + " gi=" + attempt(() => "aA".replaceAll(/a/gi, "-")));
console.log("all-string-looks-global=" + show("a/g/a".replaceAll("/g/", "-")));
console.log("replace-nonglobal=" + show("aa".replace(/a/, "-")));
const reLast = /a/g;
reLast.lastIndex = 2;
console.log("all-lastIndex=" + show("aaa".replaceAll(reLast, "-")) + ":" + reLast.lastIndex);
console.log("all-num-search=" + show("a1b".replaceAll(1 as any, "-")) + " num-repl=" + show("aXb".replaceAll("X", 9 as any)));
console.log("all-obj-search=" + show("aXb".replaceAll({ toString: () => "X" } as any, "-")));
console.log("all-undefined-search=" + show("aundefinedb".replaceAll(undefined as any, "-")) + " null=" + show("anullb".replaceAll(null as any, "-")));
console.log("all-undef-repl=" + show("a-b".replaceAll("-", undefined as any)) + " num-repl0=" + show("a-b".replaceAll("-", 0 as any)));

// --- Symbol.replace takes over replace and replaceAll on any object carrying it ---
const rep: any = { toString() { return "SHOULD-NOT-BE-USED"; } };
rep[Symbol.replace] = function (str: string, repl: any) {
  return "R(" + str + "|" + show(repl) + "|" + typeof repl + ")";
};
console.log("protocol-replace=" + "abc".replace(rep, "z"));
console.log("protocol-replace-fn=" + "abc".replace(rep, () => "z").slice(0, 3));
console.log("protocol-replaceAll=" + "abc".replaceAll(rep, "z"));
const inh: any = {};
inh[Symbol.replace] = function (s: string) { return "INHERITED:" + s; };
console.log("protocol-inherited=" + "q".replace(Object.create(inh), "-"));
const nulled: any = { toString() { return "b"; } };
nulled[Symbol.replace] = null;
console.log("protocol-null-falls-back=" + "abc".replace(nulled, "-"));
const bad: any = {};
bad[Symbol.replace] = 42;
console.log("protocol-bad=" + attempt(() => "abc".replace(bad, "-")));
const regexpLike: any = {};
regexpLike[Symbol.match] = true;
regexpLike[Symbol.replace] = function (s: string) { return "NEVER:" + s; };
console.log("protocol-replaceAll-regexplike=" + attempt(() => "abc".replaceAll(regexpLike, "-")));
regexpLike.flags = "g";
console.log("protocol-replaceAll-regexplike-g=" + "abc".replaceAll(regexpLike, "-"));
