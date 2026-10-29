// Cross-runtime: the regexp-driven String methods -- match (null vs array with index,
// input, groups), search (index or -1), matchAll (a lazy iterator of full exec results,
// refusing a non-global regexp, cloning it so the caller's lastIndex is untouched) --
// plus the Symbol.match / Symbol.search / Symbol.matchAll dispatch that lets any object
// carrying those methods take over, and the null/undefined fallback rules.
// Output is ASCII-only.

function show(x: unknown): string {
  if (x === undefined) return "undefined";
  if (x === null) return "null";
  if (typeof x === "string") {
    return '"' + x.replace(/[^\x20-\x7e]/g, (c) => "\\u" + c.charCodeAt(0).toString(16).padStart(4, "0")) + '"';
  }
  if (Array.isArray(x)) return "[" + x.map((e) => show(e)).join(",") + "]";
  if (typeof x === "object") {
    // null-prototype objects (such as match groups) have no toString; print their keys
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

// --- match with a non-global regexp: an array with index, input and groups ---
const text = "The quick brown fox";
const m1: any = text.match(/quick/);
console.log("match-value=" + m1[0] + " index=" + m1.index + " input-len=" + m1.input.length);
console.log("match-length=" + m1.length + " groups=" + show(m1.groups));
const m2: any = "hello world".match(/world/);
console.log("match-found=" + (m2 !== null) + " value=" + show(m2[0]) + " index=" + m2.index + " input=" + show(m2.input));
console.log("match-nomatch=" + show(text.match(/xyz/)));
const named: any = "2024-06-01".match(/(?<y>\d{4})-(?<m>\d{2})/);
console.log("match-named=" + named.groups.y + "/" + named.groups.m + " keys=" + Object.keys(named.groups).join(","));
console.log("match-groups-proto=" + show(Object.getPrototypeOf(named.groups)));
const opt: any = "ab".match(/(a)(z)?(b)/);
console.log("match-unmatched-group=" + show(opt[2]) + " len=" + opt.length);

// --- match with a global regexp: every match, no index, null when none ---
console.log("match-global=" + show("The quick brown fox".match(/o/g)));
console.log("match-global-nomatch=" + show("abc".match(/z/g)));
console.log("match-global-empty-matches=" + show("ab".match(/x*/g)));
const globalRe = /o/g;
globalRe.lastIndex = 5;
console.log("match-global-lastIndex=" + show("foo".match(globalRe)) + " after=" + globalRe.lastIndex);

// --- match with a string argument builds a regexp from it (metacharacters are live) ---
console.log("match-string-arg=" + show("a.b".match("." as any)) + " " + show("a.b".match("\\." as any)));
console.log("match-no-arg=" + show("ab".match(undefined as any)));

// --- search: the index of the first match, or -1; the lastIndex is ignored ---
console.log("search-brown=" + text.search(/brown/) + " none=" + text.search(/xyz/) + " start=" + text.search(/^The/));
console.log("search-digit=" + "abc123def".search(/\d/) + " letter=" + "123abc".search(/[a-z]/));
console.log("search-case=" + "Hello".search(/hello/i) + " case-sensitive=" + "Hello".search(/hello/));
const sticky = /b/g;
sticky.lastIndex = 2;
console.log("search-ignores-lastIndex=" + "abab".search(sticky) + " lastIndex-after=" + sticky.lastIndex);
console.log("search-string-arg=" + "a.b".search("." as any) + " " + "a.b".search("\\." as any));

// --- matchAll: a lazy iterator of full exec results; each has index, input, groups ---
const rows = Array.from("aa-10 bb-20".matchAll(/(?<l>[a-z]+)-(\d+)/g)).map((m) =>
  m[0] + ":" + m[1] + ":" + m[2] + ":" + m.groups?.l + ":" + m.index,
);
console.log("matchAll-rows=" + rows.join("|"));
const entries: any[] = [..."a1b2".matchAll(/(?<l>[a-z])(\d)/g)];
console.log("matchAll-count=" + entries.length + " first=" + entries[0][0] + "|" + entries[0][1] + "|" + entries[0][2] + "|" + entries[0].index + "|" + entries[0].input);
console.log("matchAll-groups=" + show(entries[0].groups) + " isArray=" + Array.isArray(entries[0]) + " len=" + entries[0].length);
console.log("matchAll-second=" + entries[1].index + ":" + entries[1].groups.l);

// --- matchAll REQUIRES a global regexp; a string argument is wrapped in one ---
console.log("matchAll-nonglobal=" + attempt(() => [..."aa".matchAll(/a/ as any)].length));
console.log("matchAll-sticky-only=" + attempt(() => [..."aa".matchAll(/a/y as any)].length));
console.log("matchAll-g=" + [..."aa".matchAll(/a/g)].length + " gy=" + [..."aab".matchAll(/a/gy)].length + " gi=" + [..."aA".matchAll(/a/gi)].length);
console.log("matchAll-string-arg=" + [..."a.a".matchAll("a" as any)].length + " meta=" + [..."a.b".matchAll("." as any)].length);
console.log("matchAll-undefined-arg=" + [..."ab".matchAll(undefined as any)].length);

// --- the caller's regexp is READ once but never written; its lastIndex is preserved ---
const re = /a/g;
re.lastIndex = 1;
console.log("clone-count=" + [..."aaa".matchAll(re)].length + " clone-lastIndex=" + re.lastIndex);
const re2 = /a/g;
console.log("from-zero=" + [..."aaa".matchAll(re2)].length + ":" + re2.lastIndex);

// --- the iterator is lazy: one step does not run the rest ---
const lazy: any = "abcabc".matchAll(/[abc]/g);
const first = lazy.next();
console.log("lazy1=" + first.value[0] + ":" + first.value.index + ":" + first.done);
const second = lazy.next();
console.log("lazy2=" + second.value[0] + ":" + second.value.index);
console.log("lazy-rest=" + [...lazy].map((m: any) => m[0] + m.index).join(","));
const done = lazy.next();
console.log("lazy-exhausted=" + String(done.value) + ":" + done.done + " again=" + [...lazy].length);
console.log("matchAll-self-iterable=" + ((lazy as any)[Symbol.iterator]() === lazy) + " tag=" + Object.prototype.toString.call("ab".matchAll(/a/g)));

// --- zero-length matches advance one position at a time and terminate ---
console.log("zero-len=" + [..."ab".matchAll(/(?:)/g)].map((m: any) => m.index).join(","));
console.log("zero-len-u=" + [..."\u{1F600}b".matchAll(/(?:)/gu)].map((m: any) => m.index).join(","));
console.log("zero-len-no-u=" + [..."\u{1F600}b".matchAll(/(?:)/g)].map((m: any) => m.index).join(","));
console.log("astral-star=" + [..."\u{1F600}".matchAll(/x*/gu)].length);
console.log("nomatch=" + [..."abc".matchAll(/z/g)].length + " match-nomatch=" + show("abc".match(/z/g)));

// --- a shared global regexp reused across matchAll calls keeps its own lastIndex ---
const shared = /[ab]/g;
console.log("shared-run1=" + [..."ab".matchAll(shared)].length + " run2=" + [..."ab".matchAll(shared)].length + " lastIndex=" + shared.lastIndex);

// --- Symbol.match / Symbol.search / Symbol.matchAll take over the String methods ---
const mat: any = {};
mat[Symbol.match] = function (...args: any[]) { return ["M", args[0], "argc" + args.length]; };
console.log("protocol-match=" + ("abc".match(mat) as any).join("|"));
const sea: any = {};
sea[Symbol.search] = function (...args: any[]) { return args.length * 100 + args[0].length; };
console.log("protocol-search=" + "abcd".search(sea));
const mall: any = {};
mall[Symbol.matchAll] = function (str: string) { return [str, "x", "y"]; };
console.log("protocol-matchAll=" + [...("abc".matchAll(mall) as any)].join("|"));
console.log("protocol-match-null=" + attempt(() => {
  const nulled: any = { toString() { return "b"; } };
  nulled[Symbol.match] = null;
  return "abc".match(nulled);
}));
const undefd: any = { toString() { return "c"; } };
undefd[Symbol.search] = undefined;
console.log("protocol-search-undef=" + "abc".search(undefd));
const bad: any = {};
bad[Symbol.match] = 42;
console.log("protocol-bad=" + attempt(() => "abc".match(bad)));
const protoSearch: any = {};
protoSearch[Symbol.search] = function () { return "INHERITED"; };
console.log("protocol-inherited=" + "q".search(Object.create(protoSearch)));
console.log("protocol-regexp-direct=" + (/b/[Symbol.match]("abc") as any)[0] + " " + /c/[Symbol.search]("abc") + " " + typeof RegExp.prototype[Symbol.matchAll]);
