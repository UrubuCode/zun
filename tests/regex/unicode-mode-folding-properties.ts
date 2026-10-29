// Cross-runtime: the u flag, seen from four angles. Code-point stepping for `.`,
// sticky cursors and zero-length matches (the same regex advances by a code unit
// without u and by a code point with it); the two meanings of `i` (ES5 uppercase
// canonicalization versus full simple case folding under u); the \p{...} property
// escapes in their long, short and gc=/sc=/scx= spellings, plus the list of forms
// that are refused. \p is never combined with i, and no range is folded under iu,
// because the reference runtimes disagree on those.

function attempt(f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "!" + e.constructor.name;
  }
}

function syn(src: string, flags: string): string {
  try {
    new RegExp(src, flags);
    return "ok";
  } catch (e: any) {
    return "!" + e.constructor.name;
  }
}

function t(src: string, subject: string, flags: string = "u"): string {
  try {
    return String(new RegExp(src, flags).test(subject));
  } catch (e: any) {
    return "!" + e.constructor.name;
  }
}

function idxs(s: string, re: RegExp): string {
  return [...s.matchAll(re)].map((x: any) => x.index).join(",");
}

const GRIN = "\u{1F600}";
const ALPHA = "α";
const CYR = "д";
const HIRA = "あ";
const KATA_DOT = "・";

// --- `.` consumes an astral code point as one match only under u ---
console.log("dot-units=" + ("A" + GRIN + "B").match(/./g)!.length);
console.log("dot-points=" + ("A" + GRIN + "B").match(/./gu)!.length);
console.log("dot-points-json=" + JSON.stringify(("A" + GRIN + "B").match(/./gu)));
console.log("astral-class=" + t("^[\\u{1F600}]$", GRIN) + "/" + t("^[\\u{1F600}-\\u{1F64F}]$", "\u{1F607}"));

// --- sticky with u: the cursor must sit on a code point boundary to match ---
const sticky = /\u{1F642}/uy;
const sMoji = "a\u{1F642}\u{1F642}";
sticky.lastIndex = 1;
console.log("sticky-u-1=" + sticky.test(sMoji) + ":" + sticky.lastIndex);
console.log("sticky-u-2=" + sticky.test(sMoji) + ":" + sticky.lastIndex);
console.log("sticky-u-3=" + sticky.test(sMoji) + ":" + sticky.lastIndex);
const word = /(?<w>\w+)/y;
word.lastIndex = 2;
const wm: any = word.exec("--abc");
console.log("sticky-named=" + wm.groups.w + ":" + word.lastIndex);
const dotSticky = /./uy;
for (const index of [0, 1, 3, 4]) {
  dotSticky.lastIndex = index;
  const match: any = dotSticky.exec("A" + GRIN + "B");
  console.log("sticky-dot-" + index + "=" + (match ? match[0] : "null") + ":" + (match ? match.index : "-") + ":" + dotSticky.lastIndex);
}

// --- zero-length matches advance one code UNIT without u, one code POINT with it ---
console.log("zl-ascii=" + idxs("ab", /(?:)/g));
console.log("zl-ascii-count=" + [..."abc".matchAll(/(?:)/g)].length);
console.log("zl-empty-subject=" + idxs("", /(?:)/g));
console.log("zl-astral-nou=" + idxs(GRIN + "b", /(?:)/g));
console.log("zl-astral-u=" + idxs(GRIN + "b", /(?:)/gu));
console.log("zl-astral-nou-count=" + [...(GRIN + "b").matchAll(/(?:)/g)].length);
console.log("zl-astral-u-count=" + [...(GRIN + "b").matchAll(/(?:)/gu)].length);
console.log("zl-subject-len=" + (GRIN + "b").length);
console.log("zl-rep-ascii=" + "ab".replace(/(?:)/g, "-"));
console.log("zl-rep-astral-u=" + (GRIN + "b").replace(/(?:)/gu, "-"));
console.log("zl-rep-astral-nou-len=" + (GRIN + "b").replace(/(?:)/g, "-").length);
console.log("zl-rep-astral-u-len=" + (GRIN + "b").replace(/(?:)/gu, "-").length);
console.log("zl-star=" + "bac".replace(/a*/g, "-"));
console.log("zl-star-astral-u=" + GRIN.replace(/x*/gu, "-"));
console.log("zl-star-astral-nou-len=" + GRIN.replace(/x*/g, "-").length);
console.log("zl-star-idx=" + idxs("baac", /a*/g));
console.log("zl-star-lens=" + [..."baac".matchAll(/a*/g)].map((x: any) => x[0].length).join(","));
const loop = /a*/g;
const steps: string[] = [];
let guard = 0;
let lm: any;
while ((lm = loop.exec("baac")) !== null && guard++ < 8) {
  steps.push(lm.index + ":" + lm[0].length + ":" + loop.lastIndex);
  if (lm[0] === "") loop.lastIndex++;
}
console.log("zl-exec-loop=" + steps.join(" "));
console.log("zl-split-ascii=" + "abc".split(/(?:)/).join("|"));
console.log("zl-split-astral-nou=" + (GRIN + "b").split(/(?:)/).length);
console.log("zl-split-astral-u=" + (GRIN + "b").split(/(?:)/u).length);
console.log("zl-lookahead=" + idxs("aXbX", /(?=X)/g) + " " + "aXbX".replace(/(?=X)/g, "-"));
console.log("zl-lookbehind=" + "aXbX".replace(/(?<=X)/g, "-"));
console.log("zl-alt=" + "aab".replace(/a?/g, "[$&]"));
console.log("zl-alt-u=" + (GRIN + "a").replace(/a?/gu, "[$&]"));
console.log("zl-lone-lead=" + idxs("\uD83Db", /(?:)/gu));
console.log("zl-lone-trail=" + idxs("\uDE00b", /(?:)/gu));
console.log("zl-pair-split=" + idxs("\uDE00\uD83D", /(?:)/gu));

// --- i without u: ES5 Canonicalize (uppercase), with the ASCII guard ---
function pair(src: string, subject: string): string {
  const nou = new RegExp(src, "i").test(subject);
  const uu = new RegExp(src, "iu").test(subject);
  return "i=" + nou + " iu=" + uu;
}

const KELVIN = "K";
const ANGSTROM = "Å";
const LONG_S = "ſ";
const DOTLESS_I = "ı";
const DOTTED_I = "İ";
const MICRO = "µ";
const MU = "μ";

console.log("kelvin-vs-k=" + pair("^k$", KELVIN));
console.log("kelvin-vs-K=" + pair("^K$", KELVIN));
console.log("k-vs-kelvin=" + pair("^" + KELVIN + "$", "k"));
console.log("longs-vs-s=" + pair("^s$", LONG_S));
console.log("longs-vs-S=" + pair("^S$", LONG_S));
console.log("s-vs-longs=" + pair("^" + LONG_S + "$", "s"));
console.log("angstrom-vs-aring=" + pair("^å$", ANGSTROM));
console.log("angstrom-vs-Aring=" + pair("^Å$", ANGSTROM));
console.log("micro-vs-mu=" + pair("^" + MU + "$", MICRO));
console.log("micro-vs-Mu=" + pair("^Μ$", MICRO));
console.log("final-vs-medial=" + pair("^σ$", "ς"));
console.log("final-vs-capital=" + pair("^Σ$", "ς"));
console.log("medial-vs-capital=" + pair("^Σ$", "σ"));
console.log("dotless-vs-i=" + pair("^i$", DOTLESS_I));
console.log("dotless-vs-I=" + pair("^I$", DOTLESS_I));
console.log("dotted-vs-i=" + pair("^i$", DOTTED_I));
console.log("dotted-vs-I=" + pair("^I$", DOTTED_I));
console.log("sharp-vs-ss=" + pair("^ss$", "ß"));
console.log("sharp-vs-SS=" + pair("^SS$", "ß"));
console.log("sharp-vs-capital=" + pair("^ẞ$", "ß"));
console.log("ligature-fi=" + pair("^fi$", "ﬁ"));
console.log("ligature-vs-self=" + pair("^ﬁ$", "ﬁ"));
console.log("class-kelvin=" + pair("^[k]$", KELVIN));
console.log("class-longs=" + pair("^[s]$", LONG_S));
console.log("class-two=" + pair("^[xk]$", KELVIN));
console.log("range-upper=" + pair("^[A-Z]$", "a"));
console.log("range-ascii=" + pair("^[a-z]$", "K"));
console.log("negclass-kelvin=" + pair("^[^k]$", KELVIN));
console.log("w-kelvin=" + pair("^\\w$", KELVIN));
console.log("w-longs=" + pair("^\\w$", LONG_S));
console.log("W-kelvin=" + pair("^\\W$", KELVIN));
console.log("b-kelvin=" + pair("\\bk", "k" + KELVIN));
console.log("backref-fold=" + pair("^(k)" + KELVIN + "\\1$", "k" + KELVIN + KELVIN));
console.log("backref-ascii=" + pair("^(a)\\1$", "aA"));
console.log("deseret-u=" + new RegExp("^\u{10428}$", "iu").test("\u{10400}"));
console.log("deseret-nou=" + new RegExp("^\u{10428}$", "i").test("\u{10400}"));
console.log("deseret-plain=" + new RegExp("^\u{10428}$", "u").test("\u{10400}"));
console.log("ascii=" + pair("^abc$", "ABC"));
console.log("ascii-digit=" + pair("^\\d$", "7"));
console.log("dot-folding=" + pair("^.$", KELVIN));

// --- \p{...}: the general category in every spelling that must resolve the same ---
console.log("Lu-bare=" + t("^\\p{Lu}$", "A"));
console.log("Lu-long=" + t("^\\p{Uppercase_Letter}$", "A"));
console.log("Lu-gc=" + t("^\\p{gc=Lu}$", "A"));
console.log("Lu-gc-long=" + t("^\\p{General_Category=Uppercase_Letter}$", "A"));
console.log("Lu-on-lower=" + t("^\\p{Lu}$", "a"));
console.log("Ll=" + t("^\\p{Ll}$", "a") + "/" + t("^\\p{Lowercase_Letter}$", "a"));
console.log("Nd=" + t("^\\p{Nd}$", "7") + "/" + t("^\\p{Decimal_Number}$", "7"));
console.log("L-supercat=" + t("^\\p{L}$", ALPHA) + "/" + t("^\\p{Letter}$", ALPHA));
console.log("P-punct=" + t("^\\p{P}$", ",") + "/" + t("^\\p{Punctuation}$", ","));
console.log("Zs-space=" + t("^\\p{Zs}$", " ") + "/" + t("^\\p{Space_Separator}$", " "));
console.log("sc-greek=" + t("^\\p{Script=Greek}$", ALPHA));
console.log("sc-short=" + t("^\\p{sc=Greek}$", ALPHA));
console.log("sc-code=" + t("^\\p{sc=Grek}$", ALPHA));
console.log("scx-greek=" + t("^\\p{Script_Extensions=Greek}$", ALPHA));
console.log("scx-short=" + t("^\\p{scx=Grek}$", ALPHA));
console.log("sc-cyrillic=" + t("^\\p{Script=Cyrillic}$", CYR));
console.log("sc-greek-on-cyr=" + t("^\\p{Script=Greek}$", CYR));
console.log("sc-hiragana=" + t("^\\p{Script=Hiragana}$", HIRA));
console.log("dot-sc-common=" + t("^\\p{Script=Common}$", KATA_DOT));
console.log("dot-sc-kana=" + t("^\\p{Script=Katakana}$", KATA_DOT));
console.log("dot-scx-kana=" + t("^\\p{Script_Extensions=Katakana}$", KATA_DOT));
console.log("dot-scx-hira=" + t("^\\p{Script_Extensions=Hiragana}$", KATA_DOT));
console.log("Alphabetic=" + t("^\\p{Alphabetic}$", ALPHA));
console.log("Alpha-alias=" + t("^\\p{Alpha}$", ALPHA));
console.log("ASCII=" + t("^\\p{ASCII}$", "a") + "/" + t("^\\p{ASCII}$", ALPHA));
console.log("Any=" + t("^\\p{Any}$", "\u{10000}"));
console.log("Assigned=" + t("^\\p{Assigned}$", "a"));
console.log("White_Space=" + t("^\\p{White_Space}$", "\t"));
console.log("Emoji-presentation=" + t("^\\p{Emoji_Presentation}$", GRIN));
console.log("ID_Start=" + t("^\\p{ID_Start}$", "a") + "/" + t("^\\p{ID_Start}$", "7"));
console.log("P-neg=" + t("^\\P{Lu}$", "a") + "/" + t("^\\P{Lu}$", "A"));
console.log("P-neg-script=" + t("^\\P{Script=Greek}$", CYR));
console.log("neg-in-class=" + t("^[^\\p{L}]$", "7"));
console.log("union-in-class=" + t("^[\\p{Nd}\\p{Lu}]$", "A") + "/" + t("^[\\p{Nd}\\p{Lu}]$", "7"));
console.log("letters-matchall=" + JSON.stringify([..."AΩЖ1_\u{1F642}".matchAll(/\p{Letter}/gu)].map((x) => x[0])));
console.log("non-ascii-matchall=" + JSON.stringify([..."AΩЖ1_\u{1F642}".matchAll(/\P{ASCII}/gu)].map((x) => x[0])));
console.log("letters-vs-numbers=" + JSON.stringify("Aé中-12-\u{1F600}".match(/\p{Letter}+/gu)));
console.log("numbers=" + JSON.stringify("Aé中-12-\u{1F600}".match(/\p{Number}+/gu)));
console.log("non-ascii-run=" + JSON.stringify("Aé中-12-\u{1F600}".match(/\P{ASCII}+/gu)));
console.log("word-chars-u=" + JSON.stringify("café λ \u{1F600}".match(/\p{L}+/gu)));

// --- what is REFUSED: \p needs u, and names must match the table exactly ---
console.log("no-u-flag=" + attempt(() => new RegExp("\\p{L}").test("p")));
console.log("unknown-name=" + syn("\\p{NotARealProperty}", "u"));
console.log("unknown-value=" + syn("\\p{Script=Klingon}", "u"));
console.log("binary-with-value=" + syn("\\p{Alphabetic=Yes}", "u"));
console.log("gc-as-binary-loose=" + syn("\\p{general_category=Lu}", "u"));
console.log("lowercase-lu=" + syn("\\p{lu}", "u"));
console.log("script-lowercase=" + syn("\\p{Script=greek}", "u"));
console.log("empty-braces=" + syn("\\p{}", "u"));
console.log("no-braces=" + syn("\\pL", "u"));
console.log("space-in-name=" + syn("\\p{Uppercase Letter}", "u"));
console.log("p-literal-no-u=" + /^p\{L\}$/.test("p{L}"));
console.log("p-literal-u=" + syn("\\p", "u"));
