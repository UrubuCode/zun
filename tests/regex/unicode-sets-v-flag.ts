// Cross-runtime: the v flag (unicodeSets, ES2024). It turns a character class
// into a set expression with nesting, difference (--), intersection (&&) and
// multi-character string literals \q{...}, adds properties of strings such as
// RGI_Emoji_Flag_Sequence, and is STRICTER than u about punctuation: a table of
// characters must be escaped inside a class, and every doubled punctuator is
// reserved. Both halves are pinned here: what v can express and what it refuses.

function t(src: string, flags: string, subject: string): string {
  try {
    return String(new RegExp(src, flags).test(subject));
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

function both(src: string): string {
  return "u=" + syn(src, "u") + " v=" + syn(src, "v");
}

function attempt(fn: () => any): string {
  try {
    return String(fn());
  } catch (e: any) {
    return "!" + e.constructor.name;
  }
}

// --- the flag itself ---
console.log("flags=" + /a/v.flags);
console.log("unicodeSets=" + /a/v.unicodeSets + " unicode=" + /a/v.unicode);
console.log("u-unicodeSets=" + /a/u.unicodeSets);
console.log("uv=" + attempt(() => new RegExp("a", "uv").flags));
console.log("vv=" + syn("a", "vv"));
console.log("source=" + /[\p{L}--[a-z]]/v.source);
console.log("flags-order=" + new RegExp("a", "yvsimgd").flags);

// --- difference: letters that are not lowercase ASCII ---
const diff = /^[\p{L}--[a-z]]$/v;
console.log("diff-upper=" + diff.test("A"));
console.log("diff-lower=" + diff.test("a"));
console.log("diff-accented=" + diff.test("é"));
console.log("diff-digit=" + diff.test("1"));
console.log("diff-replace=" + "aAbB".replace(/[\p{L}--[a-z]]/gv, "-"));
console.log("diff-of-union=" + t("^[[a-z]--[aeiou]]$", "v", "e") + "," + t("^[[a-z]--[aeiou]]$", "v", "b"));
console.log("vi-diff=" + t("^[\\p{L}--[a-z]]$", "vi", "A"));

// --- intersection: ASCII letters only, and chains ---
const inter = /^[\p{ASCII}&&\p{L}]$/v;
console.log("inter-ascii-letter=" + inter.test("z"));
console.log("inter-ascii-digit=" + inter.test("7"));
console.log("inter-nonascii-letter=" + inter.test("é"));
console.log("inter-chain=" + t("^[\\p{L}&&\\p{ASCII}&&[a-m]]$", "v", "c"));
console.log("inter-chain-miss=" + t("^[\\p{L}&&\\p{ASCII}&&[a-m]]$", "v", "z"));
console.log("inter-of-union=" + t("^[[a-z]&&[aeiou]]$", "v", "e") + "," + t("^[[a-z]&&[aeiou]]$", "v", "b"));
console.log("amp-intersect=" + t("^[a&&b]$", "v", "a"));
console.log("amp-intersect-real=" + t("^[[ab]&&[bc]]$", "v", "b"));
console.log("single-amp=" + t("^[&]$", "v", "&"));
console.log("caretcaret-source=" + syn("[^^]", "v"));
console.log("caretcaret-match=" + t("[^^]", "v", "a") + "/" + t("[^^]", "v", "^"));

// --- nesting is a union inside v; under u the inner bracket just ends the class ---
console.log("nested=" + t("^[[a-c][0-2]]$", "v", "1"));
console.log("nested-miss=" + t("^[[a-c][0-2]]$", "v", "9"));
console.log("nested-deep=" + t("^[[[a]]]$", "v", "a"));
console.log("nested-negated=" + t("^[^[a-c]]$", "v", "d"));
console.log("nested-v=" + t("^[[a-z][0-9]]$", "v", "5"));
console.log("nested-u=" + syn("[[a-z][0-9]]", "u"));

// --- \q{...} matches whole strings, longest alternative first ---
const q = /^[\q{abc|d|}]$/v;
console.log("q-multi=" + q.test("abc"));
console.log("q-single=" + q.test("d"));
console.log("q-partial=" + q.test("a"));
console.log("q-empty=" + q.test(""));
console.log("q-replace=" + "xabcy".replace(/[\q{abc}]/v, "-"));
console.log("q-longest-first=" + "ab".replace(/[\q{ab|a}]/v, "-"));
console.log("q-union=" + t("^[\\q{ab}[c]]$", "v", "c"));
console.log("q-ci=" + t("^[\\q{ab}]$", "vi", "AB"));
console.log("neg-strings=" + syn("[^\\q{ab}]", "v"));
console.log("pos-strings=" + t("^[\\q{ab}]$", "v", "ab"));

// --- properties of strings exist only under v ---
console.log("rgi-emoji=" + t("^\\p{RGI_Emoji}$", "v", "\u{1F1E7}\u{1F1F7}"));
console.log("basic-emoji=" + t("^\\p{Basic_Emoji}$", "v", "\u{1F600}"));
console.log("rgi-under-u=" + attempt(() => new RegExp("\\p{RGI_Emoji}", "u").test("a")));
const flagSeq = new RegExp("\\p{RGI_Emoji_Flag_Sequence}", "gv");
const flagInput = "A\u{1F1E7}\u{1F1F7}B\u{1F1FA}\u{1F1F8}C";
console.log("flag-seq=" + JSON.stringify(flagInput.match(flagSeq)));
console.log("flag-seq-index=" + [...flagInput.matchAll(flagSeq)].map((x) => x.index).join(","));
const consonants = new RegExp("[\\p{ASCII}&&\\p{Letter}&&[^AEIOUaeiou]]+", "gv");
console.log("consonants=" + JSON.stringify("abc XYZ élan 123".match(consonants)));
console.log("consonants-flags=" + consonants.unicodeSets + "/" + consonants.flags);
console.log("astral-dot-v=" + ("\u{1F600}".match(/./gv) as any).length);
console.log("class-astral-v=" + t("^[\\u{1F600}]$", "v", "\u{1F600}"));

// --- v is stricter: these characters are members under u but must be escaped under v ---
console.log("lparen=" + both("[(]"));
console.log("rparen=" + both("[)]"));
console.log("lbrace=" + both("[{]"));
console.log("rbrace=" + both("[}]"));
console.log("lbracket=" + both("[[]"));
console.log("slash=" + both("[/]"));
console.log("pipe=" + both("[|]"));
console.log("minus=" + both("[-]"));
console.log("paren-v=" + attempt(() => new RegExp("[(]", "v").test("(")));
console.log("paren-u=" + attempt(() => new RegExp("[(]", "u").test("(")));
console.log("bracket-v=" + attempt(() => new RegExp("[\\]]", "v").test("]")));
console.log("doubled-amp-u=" + attempt(() => new RegExp("[a&&b]", "u").test("a")));
console.log("doubled-amp-v=" + attempt(() => new RegExp("[a&&b]", "v").test("a")));
console.log("single-amp-v=" + attempt(() => new RegExp("[a&b]", "v").test("&")));

// --- and each is fine escaped under v ---
console.log("esc-lparen=" + t("^[\\(]$", "v", "("));
console.log("esc-lbrace=" + t("^[\\{]$", "v", "{"));
console.log("esc-slash=" + t("^[\\/]$", "v", "/"));
console.log("esc-pipe=" + t("^[\\|]$", "v", "|"));
console.log("esc-minus=" + t("^[\\-]$", "v", "-"));
console.log("esc-lbracket=" + t("^[\\[]$", "v", "["));
console.log("range=" + t("^[a-c]$", "v", "b"));
console.log("range-out=" + t("^[a-c]$", "v", "d"));

// --- DOUBLE punctuators are reserved wholesale, even ones with no meaning ---
console.log("bangbang=" + both("[!!]"));
console.log("hashhash=" + both("[##]"));
console.log("dollardollar=" + both("[$$]"));
console.log("percentpercent=" + both("[%%]"));
console.log("starstar=" + both("[**]"));
console.log("plusplus=" + both("[++]"));
console.log("commacomma=" + both("[,,]"));
console.log("dotdot=" + both("[..]"));
console.log("coloncolon=" + both("[::]"));
console.log("semisemi=" + both("[;;]"));
console.log("ltlt=" + both("[<<]"));
console.log("eqeq=" + both("[==]"));
console.log("gtgt=" + both("[>>]"));
console.log("questquest=" + both("[??]"));
console.log("atat=" + both("[@@]"));
console.log("tildetilde=" + both("[~~]"));

// --- a SINGLE one of the same characters is an ordinary member ---
console.log("single-bang=" + t("^[!]$", "v", "!"));
console.log("single-star=" + t("^[*]$", "v", "*"));
console.log("single-at=" + t("^[@]$", "v", "@"));
console.log("single-tilde=" + t("^[~]$", "v", "~"));
console.log("u-bangbang=" + t("^[!!]$", "u", "!"));
console.log("u-amp=" + t("^[a&&b]$", "u", "&"));
console.log("u-lone-dash=" + t("^[-]$", "u", "-"));
