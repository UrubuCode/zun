// Cross-runtime: how the backtracking matcher chooses and what it leaves in the
// capture slots. Covers first-match (not longest) alternation, greedy vs lazy,
// per-iteration capture reset inside quantifiers, backreferences to groups that
// did not participate or are not reached yet, Annex B octal and \k fallbacks,
// lookahead and lookbehind captures, and variable-width lookbehind.

function m(re: RegExp, s: string): string {
  const r = re.exec(s);
  if (r === null) return "null";
  const slots: string[] = [];
  for (let i = 0; i < r.length; i++) slots.push(r[i] === undefined ? "u" : JSON.stringify(r[i]));
  return "[" + slots.join(",") + "]@" + r.index;
}

function ex(re: RegExp, s: string): string {
  const r: any = re.exec(s);
  if (r === null) return "null";
  const parts: string[] = [];
  for (let i = 0; i < r.length; i++) parts.push(i + ":" + String(r[i]));
  return "@" + r.index + " " + parts.join(" ");
}

function syn(src: string, flags: string): string {
  try {
    new RegExp(src, flags);
    return "ok";
  } catch (e: any) {
    return "!" + e.constructor.name;
  }
}

// --- alternation takes the FIRST alternative that lets the whole pattern succeed ---
console.log("first-wins=" + m(/ab|abcd/, "abcd"));
console.log("longest-if-first=" + m(/abcd|ab/, "abcd"));
console.log("first-fails-then=" + m(/abx|abcd/, "abcd"));
console.log("anchored-forces-long=" + m(/^(?:ab|abcd)$/, "abcd"));
console.log("empty-alt-first=" + m(/|a/, "a"));
console.log("empty-alt-last=" + m(/a|/, "a"));
console.log("nested-alt=" + m(/(a|ab)(c|bcd)/, "abcd"));
console.log("nested-alt-swap=" + m(/(ab|a)(bcd|c)/, "abcd"));

// --- the match is LEFTMOST: an earlier short match beats a later long one ---
console.log("leftmost=" + m(/a+/, "ba aaa"));
console.log("leftmost-alt=" + m(/x|aaa/, "aaax"));

// --- greedy gives characters back one at a time; lazy takes them one at a time ---
console.log("greedy=" + m(/^(a+)(a*)$/, "aaa"));
console.log("lazy=" + m(/^(a+?)(a*)$/, "aaa"));
console.log("greedy-backoff=" + m(/^(a+)a$/, "aaa"));
console.log("greedy-dot=" + m(/"(.*)"/, 'say "a" and "b" now'));
console.log("lazy-dot=" + m(/"(.*?)"/, 'say "a" and "b" now'));
console.log("greedy-then-fixed=" + m(/^(.*)(\d)$/, "a1b2c3"));
console.log("lazy-then-fixed=" + m(/^(.*?)(\d)/, "a1b2c3"));
console.log("greedy-backoff-digits=" + ex(/(\d+)(\d)/, "12345"));
console.log("lazy-forward-digits=" + ex(/(\d+?)(\d)/, "12345"));
console.log("greedy-star=" + ex(/a(.*)a/, "abaca"));
console.log("lazy-star=" + ex(/a(.*?)a/, "abaca"));
console.log("greedy-angle=" + ex(/<(.+)>/, "<a><b>"));
console.log("lazy-angle=" + ex(/<(.+?)>/, "<a><b>"));
console.log("global-lazy=" + JSON.stringify("<a><b><c>".match(/<.*?>/g)));
console.log("global-greedy=" + "<a><b><c>".match(/<.*>/)![0]);
console.log("bounded-greedy=" + "aaaa".match(/a{2,3}/g)!.join("|"));
console.log("brace-literal=" + ex(/a{,2}/, "a{,2}"));
console.log("brace-open=" + ex(/a{2/, "a{2"));

// --- nested quantifiers: the inner one is satisfied before the outer repeats ---
console.log("nested-quant=" + m(/^(?:(a*))*$/, "aaa"));
console.log("nested-groups=" + m(/^((a)(b)?)+$/, "aab"));
console.log("nested-star-plus=" + m(/^(a+)+$/, "aaa"));
console.log("nested-alt-quant=" + m(/^(?:a|aa)+$/, "aaaa"));
console.log("nested-inner-last=" + ex(/((\w)+)+/, "abc"));
console.log("nested-loop-last=" + ex(/(?:(?:(\w)(\w))+;)+/, "ab;cd;"));

// --- a quantified group keeps only the LAST iteration; its groups reset each pass ---
console.log("last-iteration=" + m(/^(?:(\w)-)+$/, "a-b-c-"));
console.log("last-iteration-two=" + m(/^(?:(\w)(\d))+$/, "a1b2"));
console.log("zero-iterations=" + m(/^(?:(x))*$/, ""));
console.log("alt-ab=" + ex(/(?:(a)|(b))+/, "ab"));
console.log("alt-ba=" + ex(/(?:(a)|(b))+/, "ba"));
console.log("alt-aa=" + ex(/(?:(a)|(b))+/, "aa"));
console.log("alt-once=" + ex(/(?:(a)|(b))/, "a"));
console.log("opt-reset=" + ex(/(?:(z)x)?(a)/, "a"));
console.log("opt-taken=" + ex(/(?:(z)x)?(a)/, "zxa"));
console.log("exact=" + ex(/(?:(\w)){3}/, "abcd"));
console.log("range=" + ex(/(?:(\w)){2,3}/, "abcd"));
console.log("range-lazy=" + ex(/(?:(\w)){2,3}?/, "abcd"));
console.log("atmost=" + ex(/(?:(\w)){0,2}/, "ab"));
console.log("global-repl=" + "ab ba".replace(/(?:(a)|(b))+/g, (...args: any[]) =>
  "[" + String(args[1]) + "," + String(args[2]) + "]"));
console.log("matchall-caps=" + [..."a1b2".matchAll(/(?:(\w)(\d))/g)].map((x: any) => x[1] + x[2]).join(" "));

// --- an OPTIONAL group that matched then backtracked out is undefined, not "" ---
console.log("backtracked-out=" + m(/^(a)?a$/, "a"));
console.log("kept-in=" + m(/^(a)?a$/, "aa"));
console.log("empty-vs-undefined=" + m(/^(a*)a?$/, "a"));
console.log("opt-quest=" + ex(/(a)?b/, "b"));
console.log("opt-quest-hit=" + ex(/(a)?b/, "ab"));
console.log("backtrack=" + ex(/^(a+)(a)$/, "aaa"));
console.log("backtrack-alt=" + ex(/^(?:(ab)|(a))(b?)$/, "ab"));

// --- zero-width bodies inside * terminate after one empty pass ---
console.log("empty-loop=" + m(/^(?:)*$/, ""));
console.log("empty-loop-group=" + m(/^(a?)*$/, "aa"));
console.log("empty-loop-b=" + m(/(?:a*)*b/, "aaab"));
console.log("empty-star=" + ex(/(a?)*/, "b"));
console.log("empty-star-hit=" + ex(/(a?)*/, "aa"));
console.log("empty-plus=" + ex(/(a?)+/, "b"));
console.log("empty-group-star=" + ex(/(?:()){2}/, "x"));
console.log("empty-inner=" + ex(/(a|)+/, "aa"));
console.log("small-nested=" + m(/^(?:a+)+$/, "aaaaaaaa"));
console.log("small-nested-fail=" + m(/^(?:a+)+b$/, "aaaaaaaa"));

// --- global iteration takes successive first-matches; replace inherits the choice ---
console.log("global-first=" + JSON.stringify("abcd".match(/ab|abcd/g)));
console.log("global-long=" + JSON.stringify("abcdabcd".match(/abcd|ab/g)));
console.log("global-overlap=" + JSON.stringify("aaaa".match(/aa/g)));
console.log("replace-first=" + "abcd".replace(/ab|abcd/, "<$&>"));
console.log("replace-long=" + "abcd".replace(/abcd|ab/, "<$&>"));

// --- lookahead can force the longest alternative, and emulate possessiveness ---
console.log("lookahead-force=" + m(/(?=(abcd))(?:ab|abcd)/, "abcd"));
console.log("possessive-emulation=" + m(/(?=(a+))\1b/, "aaab"));
console.log("possessive-fails=" + m(/(?=(a+))\1a/, "aaa"));
console.log("atomic=" + ex(/^(?=(a+))\1b$/, "aaab"));
console.log("atomic-fail=" + ex(/^(?=(a+))\1ab$/, "aaab"));

// --- a backreference to a group that did NOT participate matches the EMPTY string ---
console.log("basic=" + ex(/(a)\1/, "aa"));
console.log("basic-miss=" + ex(/(a)\1/, "ab"));
console.log("quote=" + ex(/(['"]).*?\1/, "say 'hi' end"));
console.log("double=" + ex(/(\w)(\w)\2\1/, "xabba"));
console.log("nonpart=" + ex(/(?:(a)|b)\1/, "b"));
console.log("nonpart-then=" + ex(/(?:(a)|b)\1c/, "bc"));
console.log("nonpart-alt-a=" + ex(/(?:(a)|b)\1c/, "aac"));
console.log("optional=" + ex(/(z)?\1/, "q"));
console.log("optional-anchored=" + ex(/^(z)?\1$/, ""));
console.log("lookahead-group=" + ex(/(?!(a))\1b/, "b"));
console.log("other-alt=" + m(/^(?:(a)|b)\1$/, "b"));
console.log("other-alt-taken=" + m(/^(?:(a)|b)\1$/, "aa"));
console.log("after-reset=" + m(/^(?:(a)x)?\1b$/, "b"));

// --- a FORWARD reference (group not reached yet) is empty, not a failure ---
console.log("forward=" + m(/(\1a)/, "a"));
console.log("forward-anchored=" + m(/^\1a$/, "a"));
console.log("forward-then-group=" + m(/^\1(a)$/, "a"));
console.log("forward-quantified=" + m(/^\1*a(b)$/, "ab"));
console.log("forward-quant-group=" + ex(/(?:\1(a))+/, "aa"));
console.log("forward-named=" + m(/\k<x>(?<x>a)/, "a"));
console.log("forward-named-anchored=" + m(/^\k<x>(?<x>ab)$/, "ab"));
console.log("la-forward=" + m(/^(?=\1)(a)$/, "a"));
console.log("lb-forward=" + m(/^(a)(?<=\2)(b)$/, "ab"));

// --- a group can refer to itself across iterations: empty first, filled after ---
console.log("self-ref=" + m(/(a\1)+/, "aa"));
console.log("self-ref-anchored=" + m(/^(a\1?)+$/, "aaa"));
console.log("self-ref-nested=" + m(/^((a)\2)+$/, "aaaa"));
console.log("self-ref-fails=" + m(/^(a\1)$/, "aa"));

// --- named and case-insensitive backreferences ---
console.log("named=" + ex(/(?<q>['"]).*?\k<q>/, "x \"hi\" y"));
console.log("named-nonpart=" + ex(/(?:(?<a>x)|y)\k<a>/, "y"));
console.log("named-repl=" + "aa".replace(/(?<c>a)\k<c>/, "[$<c>]"));
console.log("ci=" + ex(/(a)\1/i, "aA"));
console.log("ci-word=" + ex(/(\w+) \1/i, "Hello hello"));
console.log("cs=" + ex(/(a)\1/, "aA"));
console.log("lb-backref=" + ex(/(?<=(a)\1)c/, "aac"));
console.log("lb-backref-miss=" + ex(/(?<=(a)\1)c/, "abc"));
console.log("backref-fold-length=" + ex(/^(ab)\1{2}$/, "ababab"));
console.log("backref-quant-short=" + ex(/^(ab)\1{2}$/, "abab"));
console.log("backref-astral-u=" + ex(/^(.)\1$/u, "\u{1F600}\u{1F600}"));
console.log("backref-astral-nou=" + ex(/^(..)\1$/, "\u{1F600}\u{1F600}"));
console.log("empty-capture=" + ex(/^(a*)b\1$/, "b"));
console.log("empty-capture-tail=" + ex(/^(a*)b\1$/, "aabaa"));
console.log("empty-capture-mismatch=" + ex(/^(a*)b\1$/, "aaba"));

// --- Annex B: without u an unmatched \N is an octal escape or an identity escape ---
console.log("no-group-1=" + syn("\\1", "") + "/" + syn("\\1", "u") + "/" + syn("\\1", "v"));
console.log("no-group-1-matches=" + /^\1$/.test(String.fromCharCode(1)));
console.log("no-group-8=" + syn("\\8", "") + "/" + syn("\\8", "u"));
console.log("no-group-8-matches=" + /^\8$/.test("8"));
console.log("no-group-9-matches=" + /^\9$/.test("9"));
console.log("octal-77=" + /^\77$/.test("?"));
console.log("octal-101=" + /^\101$/.test("A"));
console.log("octal-400=" + /^\400$/.test(String.fromCharCode(0x20) + "0") + "/" + syn("\\400", ""));
console.log("nul-escape=" + /^\0$/.test(String.fromCharCode(0)) + "/" + syn("\\0", "u"));
console.log("nul-then-digit=" + syn("\\01", "") + "/" + syn("\\01", "u"));
console.log("over-count=" + syn("(a)\\2", "") + "/" + syn("(a)\\2", "u"));
console.log("over-count-match=" + /^(a)\2$/.test("a" + String.fromCharCode(2)));
console.log("in-count=" + m(/^(a)(b)\2$/, "abb"));
console.log("bare-k=" + syn("\\k", "") + "/" + syn("\\k", "u"));
console.log("bare-k-match=" + /^\k$/.test("k"));
console.log("bare-k-with-named=" + syn("\\k(?<x>a)", ""));
console.log("unknown-name=" + syn("\\k<y>(?<x>a)", "") + "/" + syn("\\k<y>(?<x>a)", "u"));
console.log("oob-u=" + syn("(a)\\2", "u"));
console.log("oob-nou-src=" + new RegExp("(a)\\2").source);

// --- lookahead captures persist after the cursor has not moved ---
console.log("la-cap=" + m(/a(?=(b))/, "ab"));
console.log("la-cap-two=" + m(/a(?=(b)(c))/, "abc"));
console.log("la-zero-width=" + m(/^(?=(\w+))\w\w$/, "ab"));
console.log("la-then-use=" + m(/(?=(a+))\1/, "aaa"));
console.log("la-nested=" + m(/a(?=(b(?=(c))))/, "abc"));
console.log("la-backtrack=" + m(/(?=(a*)aa)a/, "aaa"));
console.log("la-backtrack-none=" + m(/(?=(a*)b)/, "aab"));
console.log("quant-la=" + m(/^(?:(?=(\w))\w)+$/, "abc"));
console.log("quant-lb=" + m(/^(?:\w(?<=(\w)))+$/, "abc"));

// --- a negative lookaround resets its captures, even when it succeeds ---
console.log("nla-success=" + m(/a(?!(b))/, "ac"));
console.log("nla-fails=" + m(/a(?!(b))/, "ab"));
console.log("nla-double=" + m(/a(?!(?!(b)))/, "ab"));
console.log("nlb-success=" + m(/(?<!(a))b/, "xb"));
console.log("nlb-fails=" + m(/(?<!(a))b/, "ab"));

// --- lookbehind matches RIGHT TO LEFT: the last alternative and greedy quantifier reached first ---
console.log("lb-cap=" + m(/(?<=(b))c/, "bc"));
console.log("lb-cap-two=" + m(/(?<=(a)(b))c/, "abc"));
console.log("lb-greedy=" + m(/(?<=^(\w+))c/, "abc"));
console.log("lb-quantified=" + m(/(?<=(\w)+)$/, "abc"));
console.log("lb-quantified-fwd=" + m(/^(?:(\w))+/, "abc"));
console.log("lb-alt-order=" + m(/(?<=(ab|b))c/, "abc"));
console.log("lb-backref-order=" + m(/(?<=(\w)\1)c/, "aac"));
console.log("fwd-backref-order=" + m(/^(\w)\1/, "aac"));
console.log("rtl-greedy=" + ex(/(?<=(a+))b/, "aaab"));
console.log("rtl-lazy=" + ex(/(?<=(a+?))b/, "aaab"));
console.log("rtl-order=" + ex(/(?<=(\d)(\d))x/, "12x"));
console.log("rtl-anchor=" + ex(/(?<=^(a+))b/, "aaab"));
console.log("lb-cap-named=" + String((/(?<=(?<h>\w))b/.exec("ab") as any).groups.h));

// --- lookbehind shapes: fixed, alternation, quantified (variable width) ---
console.log("fixed=" + ex(/(?<=\$)\d+/, "cost: $42"));
console.log("fixed-miss=" + ex(/(?<=\$)\d+/, "cost: 42"));
console.log("fixed-at-start=" + ex(/(?<=^)a/, "ab"));
console.log("fixed-replace=" + "$42 and 7".replace(/(?<=\$)\d+/g, "N"));
console.log("neg=" + ex(/(?<!\$)\d+/, "$42 7"));
console.log("neg-replace=" + "$42 7".replace(/(?<!\$)\d+/g, "N"));
console.log("neg-empty-prefix=" + ex(/(?<!x)a/, "a"));
console.log("neg-hit=" + ex(/(?<!x)a/, "xa"));
console.log("var-alt=" + ex(/(?<=ab|c)d/, "cd"));
console.log("var-alt2=" + ex(/(?<=ab|c)d/, "abd"));
console.log("var-quant=" + ex(/(?<=a+)b/, "xaaab"));
console.log("var-star=" + ex(/(?<=a*)b/, "b"));
console.log("var-opt=" + ex(/(?<=xa?)b/, "xb"));
console.log("var-unbounded=" + ex(/(?<=\d+px)!/, "1234px!"));
console.log("both=" + ex(/(?<=a)b(?=c)/, "abc"));
console.log("both-miss=" + ex(/(?<=a)b(?=c)/, "abd"));
console.log("zero-width=" + "abcabc".replace(/(?<=b)/g, "-"));
console.log("zero-width-neg=" + "abc".replace(/(?<!^)(?=[a-z])/g, "."));
console.log("thousands=" + "1234567".replace(/\B(?=(\d{3})+(?!\d))/g, ","));
console.log("thousands-short=" + "12".replace(/\B(?=(\d{3})+(?!\d))/g, ","));
console.log("start=" + ex(/(?<=a)b/, "b"));
console.log("start-neg=" + ex(/(?<!a)b/, "b"));
console.log("sliced=" + ex(/(?<=a)b/, "ab".slice(1)));
console.log("in-class-literal=" + /[(?<=a)]/.test("<"));
console.log("source=" + /(?<=a+)b/.source);
console.log("nested-lb=" + ex(/(?<=a(?<=[a-z]))b/, "ab"));

const sticky = /(?<=a)b/y;
sticky.lastIndex = 1;
console.log("sticky-lb=" + sticky.test("ab") + ":" + sticky.lastIndex);
const stickyFail = /(?<=a)b/y;
stickyFail.lastIndex = 0;
console.log("sticky-lb-fail=" + stickyFail.test("ab") + ":" + stickyFail.lastIndex);

const lbRe = /(?<=\.\s*)[a-z]+/g;
const text = "a. foo b.x c.  bar";
const steps: string[] = [];
let step;
while ((step = lbRe.exec(text)) !== null) steps.push(step[0] + "@" + step.index + ">" + lbRe.lastIndex);
console.log("lb-walk=" + steps.join(" "));
const jumped = /(?<=\.\s*)[a-z]+/g;
jumped.lastIndex = 5;
const landed: any = jumped.exec(text);
console.log("lb-jumped=" + landed[0] + "@" + landed.index + ">" + jumped.lastIndex);
console.log("lb-matchall=" + JSON.stringify([...text.matchAll(/(?<=\.\s*)[a-z]+/g)].map((x) => [x[0], x.index])));
console.log("lb-quant-global=" + JSON.stringify(text.match(/(?<=\.\s*)[a-z]+/g)));
console.log("lb-neg-global=" + JSON.stringify("a. foo".match(/(?<!\.\s*)[a-z]+/g)));
console.log("lb-quant-hit=" + JSON.stringify("cab".match(/(?<=a+)b/)));
console.log("lb-quant-miss=" + JSON.stringify("b".match(/(?<=a+)b/)));
console.log("lb-range-hit=" + JSON.stringify("xxy".match(/(?<=x{1,3})y/)));
console.log("lb-range-miss=" + JSON.stringify("y".match(/(?<=x{1,3})y/)));
console.log("lb-zero-reps=" + JSON.stringify("abc".match(/(?<=x*)a/)));
console.log("lb-leading-space=" + JSON.stringify("  a a".match(/(?<=\s*)a/g)));
console.log("lb-star-anchor=" + JSON.stringify("aab".match(/(?<=^a*)b/)));
console.log("lb-end-plus=" + JSON.stringify("aaa".match(/(?<=a+)$/)));
console.log("lb-group-after=" + JSON.stringify("a.  foo".match(/(?<=\.\s*)([a-z]+)/)));
console.log("lb-named-word=" + JSON.stringify("a.  foo".match(/(?<=\.\s*)(?<word>[a-z]+)/)!.groups!.word));
