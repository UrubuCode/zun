// Cross-runtime: the String methods that take a RegExp, and the result shapes
// they produce. Covers replacement patterns ($1 $& $` $' $$ $<name>, out-of-range
// and non-participating groups), replacer callbacks and their argument list,
// split with captures / limit / zero-width separators, match versus matchAll,
// the exec result object (index, input, groups, indices with the d flag), named
// groups that did not participate, and the Symbol methods called directly.

function ex(re: RegExp, s: string): string {
  const r: any = re.exec(s);
  if (r === null) return "null";
  const parts: string[] = [];
  for (let i = 0; i < r.length; i++) parts.push(i + ":" + String(r[i]));
  return "@" + r.index + " " + parts.join(" ");
}

function j(a: any[]): string {
  return "[" + a.map((x) => (x === undefined ? "u" : JSON.stringify(x))).join(",") + "]";
}

function own(m: any): string {
  return Object.getOwnPropertyNames(m).join(",");
}

// --- replacement patterns on a string replacement ---
console.log("swap-groups=" + "Doe, Jane; Smith, John".replace(/(\w+), (\w+)/g, "$2 $1"));
console.log("order=" + "abc".replace(/(a)(b)(c)/, "$3$2$1-$4"));
console.log("amp=" + "abc".replace(/b/, "[$&]"));
console.log("pre-post=" + "abcde".replace(/c/, "<$`|$'>"));
console.log("dollar-escape=" + "abc".replace(/b/, "$$"));
console.log("dollar-literal=" + "abc".replace(/b/, "$x$"));
console.log("group0-literal=" + "abc".replace(/(b)/, "[$0]"));
console.log("oob-literal=" + "a".replace(/(a)/, "[$2]"));
console.log("two-digit=" + "a".replace(/(a)/, "[$12]"));
console.log("two-digit-real=" + "abcdefghijkl".replace(/(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)(k)/, "$11"));
console.log("nonpart-group=" + "b".replace(/(?:(a)|b)/, "[$1]"));
console.log("replace-all-amp=" + "aXa".replaceAll(/a/g, "<$&>"));
console.log("replace-all-string=" + "a-b-c".replaceAll("-", "/"));
console.log("replace-all-pattern=" + "a.b.c".replaceAll(/\./g, "$$"));
console.log("replace-nomatch=" + "abc".replace(/z/, "X"));
console.log("replace-global-nomatch=" + "abc".replace(/z/g, "X"));

// --- replace with a function: arguments are match, captures, offset, input, groups ---
const seen: string[] = [];
const out = "a1 b22".replace(/([a-z])(\d+)/g, (match, letter, digits, offset, input) => {
  seen.push([match, letter, digits, offset, input.length].join(":"));
  return letter.toUpperCase() + Number(digits) * 2;
});
console.log("fn-out=" + out);
console.log("fn-args=" + seen.join("|"));
const named = "x=12;y=3".replace(/(?<key>[a-z])=(?<value>\d+)/g, (...args: any[]) => {
  const groups = args[args.length - 1];
  const offset = args[args.length - 3];
  return groups.key.toUpperCase() + ":" + groups.value + "@" + offset;
});
console.log("fn-named=" + named);
const red = "red:10 blue:20".replace(/(?<name>[a-z]+):(?<value>\d+)/g, (...args: any[]) => {
  const groups = args[args.length - 1];
  const offset = args[args.length - 3];
  return groups.name.toUpperCase() + "@" + offset + "=" + (Number(groups.value) + 1);
});
console.log("fn-named-math=" + red);
console.log("lookahead-capture-repl=" + "aba".replace(/(?=(a))/g, "[$1]"));
console.log("replacer-arity=" + "ab".replace(/a(?=(b))/, function () {
  const a = Array.prototype.slice.call(arguments);
  return "<" + a.length + ":" + String(a[1]) + ":" + a[2] + ">";
}));
console.log("named-repl=" + "2026-05-22".replace(/(?<y>\d{4})-(?<m>\d{2})-(?<d>\d{2})/, "$<d>/$<m>/$<y>"));
console.log("named-repl-nonpart=" + "b".replace(/(?<p>a)|(?<q>b)/, "[$<p>][$<q>]"));
console.log("named-repl-unknown=" + "a".replace(/(?<p>a)|(?<q>b)/, "[$<zz>]"));
console.log("named-literal-no-groups=" + "a".replace(/a/, "[$<p>]"));
console.log("named-literal-cap=" + "a".replace(/(a)/, "[$<p>]"));

// --- zero-length replacements step forward through the input ---
const hits: string[] = [];
const zl = "ab".replace(/(?=.)/g, (m, offset) => {
  hits.push(offset + ":" + m.length);
  return "|";
});
console.log("zl-fn=" + zl + " " + hits.join(","));
console.log("zl-empty=" + "abc".replace(/(?:)/g, "-"));
console.log("zl-anchors=" + "ab".replace(/^|$/g, "|"));
console.log("zl-lookahead=" + "aaa".replace(/(?=a)/g, "X"));
console.log("zl-lookbehind=" + "abcabc".replace(/(?<=b)/g, "-"));

// --- split: captures are spliced in, and the limit counts them ---
const input = "a1b22c";
console.log("split-caps=" + JSON.stringify(input.split(/(\d+)/)));
console.log("split-caps-limit=" + JSON.stringify(input.split(/(\d+)/, 4)));
console.log("split-digits=" + "a1b2c3".split(/\d/).join("|"));
console.log("split-spaces=" + "hello  world   foo".split(/\s+/).join("|"));
console.log("split-classes=" + "a,b;c,d".split(/[,;]/).join("|"));
console.log("split-limit=" + "a1b2c3".split(/\d/, 2).join("|"));
console.log("split-groups=" + "a1b2c3".split(/(\d)/).join("|"));
console.log("split-cap-limit-1=" + j("a1b".split(/(\d)/, 1)));
console.log("split-cap-limit-2=" + j("a1b".split(/(\d)/, 2)));
console.log("split-cap-limit-3=" + j("a1b".split(/(\d)/, 3)));
console.log("split-optional-cap=" + j("a1b".split(/(\d)(x)?/, 3)));
console.log("split-limit-0=" + j("a1b2c".split(/\d/, 0)));
console.log("split-limit-big=" + j("a1b2c".split(/\d/, 99)));
console.log("split-limit-frac=" + j("a1b2c".split(/\d/, 2.9)));
console.log("split-limit-neg=" + j("a1b2c".split(/\d/, -1)));
console.log("split-limit-nan=" + j("a1b2c".split(/\d/, NaN as any)));
console.log("split-limit-2p32=" + "a1b2c".split(/\d/, 4294967296 as any).length);
console.log("split-empty-subject=" + j("".split(/x/)) + " " + j("".split(/(?:)/)));
console.log("split-whole=" + j("ab".split(/ab/)));
console.log("split-edges=" + j(",a,".split(/,/)));
console.log("split-adjacent=" + j("a,,b".split(/,/)));
console.log("split-zero-width-units=" + j("a\u{1F600}b".split(/(?:)/)).length + " " + "a\u{1F600}b".split(/(?:)/).length);
console.log("split-zero-width-u=" + "a\u{1F600}b".split(/(?:)/u).length);
console.log("split-direct=" + j(/,/[Symbol.split]("a,b,c")) + " " + j(/,/[Symbol.split]("a,b,c", 2)));
console.log("split-sticky-flag=" + /,/y[Symbol.split]("a,b,c").join("|"));
console.log("split-nonstring-receiver=" + JSON.stringify(String.prototype.split.call(["a", "b"] as any, /,/)));

// --- match: /g returns strings only; no /g returns the exec object ---
const mg: any = "a1 b22 c333".match(/[a-z](\d+)/g);
const ms: any = "a1 b22 c333".match(/[a-z](\d+)/);
console.log("match-g=" + JSON.stringify(mg) + " own=" + own(mg));
console.log("match-one=" + ms[0] + " " + ms[1] + " " + ms.index + " own=" + own(ms));
console.log("match-one-input=" + ms.input);
console.log("match-g-miss=" + String("abc".match(/z/g)));
console.log("match-miss=" + String("abc".match(/z/)));
const mm: any = "xa".match(/a/);
console.log("match-json=" + JSON.stringify(mm));

// --- the exec result is a real Array with index, input and groups as extras ---
const m: any = /(a)(z)?(b)/.exec("xxabyy");
console.log("isArray=" + Array.isArray(m) + " len=" + m.length);
console.log("slots=" + [m[0], m[1], String(m[2]), m[3]].join("|"));
console.log("index=" + m.index + " input=" + m.input + " groups=" + String(m.groups));
console.log("own=" + own(m));
console.log("json=" + JSON.stringify(m));
console.log("nonpart-in=" + (2 in m));
const extra: any = Object.getOwnPropertyDescriptor(m, "index");
console.log("extra-desc=" + extra.writable + "/" + extra.enumerable + "/" + extra.configurable);
console.log("keys=" + Object.keys(m).join(","));
console.log("index-whole=" + (/(b)/.exec("aab") as any).index + " " + (/(?=b)/.exec("ab") as any).index);
console.log("input-coerced=" + (/2/.exec(123 as any) as any).input + " " + (/,/.exec([1, 2] as any) as any).input);
console.log("boxed-input=" + typeof (/b/.exec(new String("abc") as any) as any).input);
const gexec: any = /(\d)/g.exec("a1b2");
console.log("g-exec-shape=" + gexec[0] + "/" + gexec[1] + "/" + gexec.index);
console.log("exec-null=" + String(/z/.exec("abc")) + " " + String("abc".match(/z/g)));
const fresh1 = /a/.exec("a");
const fresh2 = /a/.exec("a");
console.log("fresh=" + (fresh1 === fresh2) + ":" + ((fresh1 as any)[0] === (fresh2 as any)[0]));

// --- named groups: a key exists whichever branch won, valued undefined when it lost ---
const alt = /(?<p>a)|(?<q>b)/;
function dumpGroups(r: any): string {
  if (!r) return "null";
  const g = r.groups;
  const keys = Object.keys(g);
  return keys.map((k) => k + "=" + String(g[k])).join(",") + " #" + keys.length;
}
console.log("hit-p=" + dumpGroups(alt.exec("a")));
console.log("hit-q=" + dumpGroups(alt.exec("b")));
console.log("in-p=" + ("p" in (alt.exec("b") as any).groups));
console.log("has-own-q=" + Object.prototype.hasOwnProperty.call((alt.exec("a") as any).groups, "q"));
const mb: any = alt.exec("b");
console.log("numbered=" + mb.length + "|" + String(mb[1]) + "|" + String(mb[2]));
console.log("groups-proto=" + String(Object.getPrototypeOf(mb.groups)));
console.log("groups-tostring=" + typeof mb.groups.toString);
console.log("groups-tag=" + Object.prototype.toString.call(mb.groups));
console.log("groups-absent=" + String((/(a)/.exec("a") as any).groups) + " " + ("groups" in (/(a)/.exec("a") as any)));
console.log("opt-hit=" + dumpGroups(/x(?<n>\d+)?y/.exec("x12y")));
console.log("opt-miss=" + dumpGroups(/x(?<n>\d+)?y/.exec("xy")));
console.log("nla-group=" + dumpGroups(/a(?!(?<z>b))/.exec("ac")));
console.log("reset-groups=" + dumpGroups(/(?:(?<u>a)x)?(?<v>a)/.exec("a")) + " / " + dumpGroups(/(?:(?<u>a)x)?(?<v>a)/.exec("ax")));
console.log("matchall-groups=" + [..."ab".matchAll(/(?<p>a)|(?<q>b)/g)].map((x: any) => String(x.groups.p) + ">" + String(x.groups.q)).join(" "));
console.log("fn-groups=" + "b".replace(alt, (...args: any[]) => {
  const gr = args[args.length - 1];
  return String(gr.p) + "/" + String(gr.q) + "/" + Object.keys(gr).join("+");
}));

// --- d flag: indices for every group, and undefined for groups that did not take part ---
const dRe = /(?<year>\d{4})-(?<month>\d{2})-(?<day>\d{2})/d;
const dm: any = dRe.exec("date: 2026-05-22 end");
console.log("d-full=" + dm[0] + "@" + dm.index + " " + dm.indices[0].join(","));
console.log("d-groups=" + dm.indices.slice(1).map((p: any) => p.join(",")).join(" "));
console.log("d-named=" + dm.indices.groups.year.join(",") + " " + dm.indices.groups.month.join(",") + " " + dm.indices.groups.day.join(","));
console.log("d-slice=" + "date: 2026-05-22 end".slice(dm.indices[1][0], dm.indices[1][1]));
const dWords: any[] = [];
for (const wm of "foo bar baz".matchAll(/\b(\w+)\b/dg)) dWords.push(wm[1] + "@" + wm.indices![1].join("-"));
console.log("d-matchall=" + dWords.join("|"));
const dOpt: any = /(\d+)(?:\.(\d+))?/d.exec("42");
console.log("d-opt=" + dOpt[1] + " " + String(dOpt[2]) + " " + dOpt.indices[1].join(",") + " " + String(dOpt.indices[2]));
const dNamed: any = /(?<a>a)(?<b>b)?/d.exec("a");
console.log("d-named-opt=" + JSON.stringify(dNamed.indices.groups) + " " + String(dNamed.indices.groups.b));
console.log("d-own=" + own(dOpt));
console.log("d-indices-array=" + Array.isArray(dOpt.indices) + " " + dOpt.indices.length);
const dNamedAlt: any = /(?<p>a)|(?<q>b)/d.exec("b");
console.log("d-groups-keys=" + Object.keys(dNamedAlt.indices.groups).join(","));
console.log("d-groups-p=" + String(dNamedAlt.indices.groups.p) + " q=" + JSON.stringify(dNamedAlt.indices.groups.q));
console.log("d-unnamed-groups=" + String((/(a)/d.exec("a") as any).indices.groups));

// --- String methods coerce a non-string receiver before matching ---
console.log("match-number=" + JSON.stringify(String.prototype.match.call(123, /\d/)));
console.log("replace-number=" + String.prototype.replace.call(123, /2/, "-"));
console.log("search-bool=" + String.prototype.search.call(true, /r/));
console.log("match-null-this=" + attempt(() => String.prototype.match.call(null, /a/)));

function attempt(f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "!" + e.constructor.name;
  }
}

// --- Symbol methods called directly are what the String methods run underneath ---
console.log("sym-rep=" + /b/[Symbol.replace]("abc", "X"));
console.log("sym-rep-amp=" + /b/[Symbol.replace]("abc", "[$&]"));
console.log("sym-rep-group=" + /(b)(c)/[Symbol.replace]("abc", "$2$1"));
console.log("sym-rep-named=" + /(?<x>b)/[Symbol.replace]("abc", "[$<x>]"));
console.log("sym-rep-fn=" + /(b)/[Symbol.replace]("abc", (...a: any[]) => "<" + a.length + ":" + a[1] + ":" + a[2] + ">"));
console.log("sym-rep-number=" + /2/[Symbol.replace](123 as any, "-"));
console.log("sym-rep-array=" + /,/g[Symbol.replace]([1, 2, 3] as any, ";"));
console.log("sym-rep-undef=" + /n/[Symbol.replace](undefined as any, "-"));
console.log("sym-search=" + /c/[Symbol.search]("abc") + " miss=" + /z/[Symbol.search]("abc") + " num=" + /2/[Symbol.search](123 as any));
const symMatch: any = /b/[Symbol.match]("abc");
console.log("sym-match-one=" + symMatch[0] + ":" + symMatch.index + ":" + symMatch.input);
console.log("sym-match-g=" + (/[ab]/g[Symbol.match]("abc") as any).join(",") + " miss=" + String(/z/g[Symbol.match]("abc")));
const it: any = /b/g[Symbol.matchAll]("abcb");
console.log("sym-matchall-type=" + typeof it.next + " count=" + [...it].length);
try {
  console.log("sym-matchall-nong=" + [.../b/[Symbol.matchAll]("ab") as any].length);
} catch (e: any) {
  console.log("sym-matchall-nong!" + e.constructor.name);
}
console.log("sym-on-proto=" + Object.prototype.hasOwnProperty.call(RegExp.prototype, Symbol.replace) + " not-own=" + !Object.prototype.hasOwnProperty.call(/a/, Symbol.replace));
console.log("sym-shared=" + (/a/[Symbol.replace] === /b/[Symbol.replace]));
