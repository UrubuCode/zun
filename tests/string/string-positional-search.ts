// Cross-runtime: the literal (non-regex) search family -- indexOf, lastIndexOf,
// includes, startsWith, endsWith -- with their position arguments, the empty needle,
// and the IsRegExp refusal. startsWith/endsWith/includes throw a TypeError on a regexp
// argument, but the test is IsRegExp ("Symbol.match is truthy"), not instanceof, so a
// regexp whose Symbol.match is false is an ordinary string-like value. Output is ASCII-only.

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

// --- indexOf / lastIndexOf: first and last occurrence, with and without a start index ---
const hw = "hello world";
console.log("indexOf-world=" + hw.indexOf("world") + " missing=" + hw.indexOf("zzz"));
console.log("indexOf-o=" + hw.indexOf("o") + " from-5=" + hw.indexOf("o", 5) + " from-8=" + hw.indexOf("o", 8));
console.log("lastIndexOf-o=" + hw.lastIndexOf("o") + " from-5=" + hw.lastIndexOf("o", 5) + " from-0=" + hw.lastIndexOf("o", 0));
const repeated = "hello world hello";
console.log("lastIndexOf-hello=" + repeated.lastIndexOf("hello") + " from-10=" + repeated.lastIndexOf("hello", 10) + " from-5=" + repeated.lastIndexOf("hello", 5) + " from-0=" + repeated.lastIndexOf("hello", 0));
console.log("indexOf-overlap=" + "aaaa".indexOf("aa", 1) + " lastIndexOf-overlap=" + "aaaa".lastIndexOf("aa"));

// --- the empty needle matches everywhere, positions clamp into [0, length] ---
const hello = "hello";
console.log("empty-0=" + hello.indexOf("") + " empty-2=" + hello.indexOf("", 2) + " empty-10=" + hello.indexOf("", 10));
console.log("empty-last=" + hello.lastIndexOf("") + " last-2=" + hello.lastIndexOf("", 2));
console.log("empty-clamp=" + [-5, 0, 2, 9].map((p) => "abc".indexOf("", p)).join(",") + " / " + [-5, 0, 2, 9].map((p) => "abc".lastIndexOf("", p)).join(","));
console.log("empty-in-empty=" + "".indexOf("") + " x-in-empty=" + "".indexOf("x"));

// --- position arguments are coerced like ToIntegerOrInfinity ---
console.log("indexOf-frac=" + hw.indexOf("o", 4.9) + " indexOf-nan=" + hw.indexOf("o", NaN) + " indexOf-inf=" + hw.indexOf("o", Infinity));
console.log("indexOf-str-pos=" + hw.indexOf("o", "5" as any) + " indexOf-undef-pos=" + hw.indexOf("o", undefined));
console.log("lastIndexOf-nan=" + hw.lastIndexOf("o", NaN) + " lastIndexOf-neg=" + hw.lastIndexOf("h", -5));

// --- includes: a position, coerced and clamped ---
const banana = "bananana";
console.log("includes=" + banana.includes("ana") + " from-2=" + banana.includes("ana", 2) + " from-3=" + banana.includes("ana", 3));
console.log("includes-undef=" + banana.includes("ana", undefined) + " neg=" + banana.includes("ana", -2) + " frac=" + banana.includes("ana", 2.8));
console.log("includes-big=" + banana.includes("ana", 99) + " empty-inf=" + banana.includes("", Infinity) + " b-nan=" + banana.includes("b", NaN));
console.log("includes-hw-6=" + hw.includes("hello", 6) + " o-5=" + hw.includes("o", 5) + " o-8=" + hw.includes("o", 8));

// --- startsWith / endsWith, with the position acting as a start or an end ---
const foo = "foobarbaz";
console.log("starts=" + foo.startsWith("foo") + " bar-3=" + foo.startsWith("bar", 3) + " baz-neg1=" + foo.startsWith("baz", -1));
console.log("ends=" + foo.endsWith("baz") + " bar-end6=" + foo.endsWith("bar", 6) + " foo-99=" + foo.endsWith("foo", 99));
console.log("starts-nan=" + foo.startsWith("foo", NaN) + " ends-nan=" + foo.endsWith("foo", NaN));
console.log("empty-starts=" + "".startsWith("") + " empty-ends=" + "".endsWith("") + " abc-starts-empty=" + "abc".startsWith(""));
console.log("starts-end=" + "abc".startsWith("c", 3) + " ends-zero=" + "abc".endsWith("a", 0) + " ends-one=" + "abc".endsWith("a", 1));
console.log("hello-world-ends-hello=" + "hello world".endsWith("hello", 5) + " starts-world-6=" + "hello world".startsWith("world", 6));
console.log("ends-cross-runtime=" + "hello".endsWith("ld") + " " + "hello world".startsWith("He") + " " + "hello".includes("ell"));

// --- a regexp argument: includes/startsWith/endsWith refuse it, the others accept it ---
console.log("sw-regexp=" + attempt(() => "abc".startsWith(/a/ as any)));
console.log("ew-regexp=" + attempt(() => "abc".endsWith(/c/ as any)));
console.log("inc-regexp=" + attempt(() => "abc".includes(/b/ as any)));
console.log("indexOf-regexp=" + "a/b/c".indexOf(/b/ as any) + " lastIndexOf-regexp=" + "x/b/".lastIndexOf(/b/ as any));

// --- a regexp with Symbol.match set to false is no longer "a regexp" ---
const tame: any = /b/;
tame[Symbol.match] = false;
console.log("tame-source=" + show(tame.source) + " tostring=" + show(String(tame)));
console.log("tame-starts=" + "/b/xyz".startsWith(tame) + " includes=" + "xx/b/yy".includes(tame));
console.log("tame-ends=" + "xx/b/".endsWith(tame) + " includes-miss=" + "abc".includes(tame));

// --- a plain object with a truthy Symbol.match IS a regexp and is refused ---
const impostor: any = { toString() { return "ab"; } };
impostor[Symbol.match] = true;
console.log("impostor-starts=" + attempt(() => "abc".startsWith(impostor)));
console.log("impostor-includes=" + attempt(() => "abc".includes(impostor)));
console.log("impostor-indexOf=" + "xab".indexOf(impostor as any));

// --- a falsy Symbol.match leaves the object an ordinary value ---
const meek: any = { toString() { return "ab"; } };
meek[Symbol.match] = 0;
console.log("meek-starts=" + "abc".startsWith(meek));
const absent: any = { toString() { return "bc"; } };
console.log("absent-ends=" + "abc".endsWith(absent) + " includes=" + "abc".includes(absent));

// --- Symbol.match is read exactly once per call ---
let reads = 0;
const probed: any = { toString() { return "a"; } };
Object.defineProperty(probed, Symbol.match, {
  get() { reads++; return undefined; },
});
console.log("probed=" + "abc".startsWith(probed) + " reads=" + reads);

// --- other argument types coerce through ToString ---
console.log("num-arg=" + "12ab".startsWith(1 as any) + " undef-arg=" + "undefinedx".startsWith(undefined as any) + " null-arg=" + "xnull".endsWith(null as any));
console.log("sym-arg=" + attempt(() => "abc".includes(Symbol("s") as any)));
console.log("this-coerced=" + String.prototype.includes.call(12321 as any, "32") + " " + String.prototype.startsWith.call(true as any, "tr"));
console.log("this-null=" + attempt(() => String.prototype.includes.call(null as any, "a")));
