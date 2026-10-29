// Cross-runtime: the surface of single-character matching in Annex B (no u)
// syntax. Covers character-class grammar corners ([] vs [^], dash and caret
// placement, \b as backspace inside a class), shorthand classes, ignoreCase on
// ranges, `.` against the four ECMAScript line terminators, the `m` anchors
// splitting at exactly those four, CRLF as two breaks, and \s membership.

const LF = String.fromCharCode(0x0a);
const CR = String.fromCharCode(0x0d);
const LS = String.fromCharCode(0x2028);
const PS = String.fromCharCode(0x2029);
const NEL = String.fromCharCode(0x85);
const VT = String.fromCharCode(0x0b);
const FF = String.fromCharCode(0x0c);
const NBSP = String.fromCharCode(0xa0);

function t(re: RegExp, s: string): string {
  return String(re.test(s));
}

// --- the empty class matches nothing; the negated empty class matches everything ---
console.log("empty=" + t(/[]/, "a"));
console.log("empty-empty=" + t(/[]/, ""));
console.log("empty-any=" + t(/a[]b/, "ab"));
console.log("negempty-letter=" + t(/[^]/, "a"));
console.log("negempty-newline=" + t(/[^]/, "\n"));
console.log("negempty-astral=" + "\u{1F600}".replace(/[^]/g, "-"));
console.log("negempty-len=" + "a\nb".match(/[^]/g)?.length);
console.log("negempty-lf=" + /^[^]$/.test(LF) + " ls=" + /^[^]$/.test(LS));

// --- `.` excludes exactly LF, CR, LS, PS unless the s flag is set ---
console.log("dot-newline=" + t(/a.b/, "a\nb"));
console.log("dot-s=" + t(/a.b/s, "a\nb"));
console.log("dot-cr=" + t(/a.b/, "a\rb"));
console.log("dot-ls=" + t(/a.b/, "a b"));
console.log("dot-tab=" + t(/a.b/, "a\tb"));
console.log("dot-astral-units=" + ("\u{1F600}".match(/./g) as any).length);
console.log("dot-astral-u=" + ("\u{1F600}".match(/./gu) as any).length);
console.log("dot-s-repl=" + ("a" + LF + "b").replace(/./gs, "-") + " plain=" + ("a" + LF + "b").replace(/./g, "-").length);

const terminators: any[][] = [
  ["lf", LF], ["cr", CR], ["ls", LS], ["ps", PS],
  ["nel", NEL], ["vt", VT], ["ff", FF], ["nbsp", NBSP],
  ["tab", "\t"], ["space", " "],
];

for (let i = 0; i < terminators.length; i++) {
  const name = terminators[i][0];
  const ch = terminators[i][1];
  console.log(
    "dot-" + name +
      " plain=" + /^.$/.test(ch) +
      " s=" + /^.$/s.test(ch) +
      " u=" + /^.$/u.test(ch) +
      " su=" + /^.$/su.test(ch),
  );
}

// --- under m, ^ and $ anchor at exactly the same four terminators ---
for (let i = 0; i < terminators.length; i++) {
  const name = terminators[i][0];
  const ch = terminators[i][1];
  console.log(
    "anchor-" + name +
      " parts=" + ("a" + ch + "b").split(/^/m).length +
      " matches=" + JSON.stringify(("a" + ch + "b").match(/^\w$/gm)),
  );
}

// --- without m, ^ and $ are the whole-string ends only ---
console.log("no-m-caret=" + ("a" + LF + "b").split(/^/).length);
console.log("no-m-dollar=" + JSON.stringify(("a" + LF + "b").match(/^\w$/g)));
console.log("no-m-whole=" + /^a$/.test("a" + LF));
console.log("no-m-whole-strict=" + /^a$/.test("a"));

// --- CRLF is two terminators, so it yields an empty line between them ---
const crlf = "a" + CR + LF + "b";
console.log("crlf-lines=" + JSON.stringify(crlf.split(/^/m).map((s) => s.length)));
console.log("crlf-empty-line=" + JSON.stringify(crlf.match(/^.*$/gm)));
console.log("crlf-dollar-count=" + (crlf.match(/$/gm) as any).length);
console.log("lf-dollar-count=" + (("a" + LF + "b").match(/$/gm) as any).length);

// --- $ sits before the terminator and ^ after it: neither consumes it ---
const probe: any = /^(b)$/m.exec("a" + LF + "b");
console.log("m-index=" + probe.index + " len=" + probe[0].length);
const probe2: any = /^(a)$/m.exec("a" + LF + "b");
console.log("m-first-index=" + probe2.index);
console.log("m-empty-at-end=" + JSON.stringify(("a" + LF).match(/^$/gm)));
console.log("m-dot=" + /^.$/m.test(LF) + " s-anchor=" + ("a" + LF + "b").split(/^/ms).length);
console.log("ms-both=" + JSON.stringify(("a" + LF + "b").match(/^.$/gms)));

// --- \s is the full white-space class, and it includes every terminator ---
for (let i = 0; i < terminators.length; i++) {
  console.log("s-class-" + terminators[i][0] + "=" + /^\s$/.test(terminators[i][1]));
}

// --- a class containing an explicit terminator matches it in every mode ---
console.log("explicit-lf=" + /^[\n]$/.test(LF));
console.log("explicit-range=" + new RegExp("^[" + LS + "]$").test(LS));

// --- \b means backspace inside a class and a word boundary outside it ---
console.log("boundary=" + t(/\bab\b/, "ab"));
console.log("boundary-mid=" + t(/\bb/, "ab"));
console.log("backspace=" + t(/[\b]/, "\b"));
console.log("backspace-not-b=" + t(/[\b]/, "b"));
console.log("backspace-code=" + "x\bx".replace(/[\b]/, "-"));
console.log("nonboundary=" + t(/\Bb/, "ab"));
console.log("word-boundary-lines=" + JSON.stringify(("a" + LF + "b").match(/\b\w\b/g)));
console.log("word-boundary-repl=" + "ab cd".replace(/\b/g, "|"));

// --- a dash is literal at either edge or when escaped; a caret only negates first ---
console.log("dash-first=" + t(/[-a]/, "-"));
console.log("dash-last=" + t(/[a-]/, "-"));
console.log("dash-escaped=" + t(/[a\-z]/, "-"));
console.log("dash-escaped-range=" + t(/[a\-z]/, "m"));
console.log("dash-range=" + t(/[a-z]/, "m"));
console.log("dash-after-class=" + t(/[\d-a]/, "-"));
console.log("caret-first=" + t(/[^a]/, "a"));
console.log("caret-inner=" + t(/[a^]/, "^"));
console.log("caret-only=" + t(/[^]/, "^"));

// --- a closing bracket must be escaped; an opening one need not be ---
console.log("open-bracket=" + t(/[[]/, "["));
console.log("close-bracket=" + t(/[\]]/, "]"));
console.log("both-brackets=" + t(/[[\]]/, "]"));
console.log("source-roundtrip=" + /[-a\]]/.source);

// --- shorthand classes work inside a class and combine with ranges ---
console.log("digit-in-class=" + t(/[\d]/, "5"));
console.log("union=" + t(/[\dA-F]/, "E"));
console.log("negated-shorthand=" + t(/[\D]/, "x"));
console.log("both-shorthands=" + t(/[\d\D]/, "\n"));
console.log("s-class=" + t(/[\s]/, " "));
console.log("S-class=" + t(/[\S]/, " "));
console.log("w-underscore=" + t(/^\w+$/, "a_1"));
console.log("w-accent=" + t(/^\w$/, "é"));

// --- ignoreCase widens ASCII ranges in both directions ---
console.log("i-range-up=" + t(/[a-z]/i, "Q"));
console.log("i-range-down=" + t(/[A-Z]/i, "q"));
console.log("i-negated=" + t(/[^a-z]/i, "Q"));
console.log("i-sharp-s=" + t(/ss/i, "ß"));

// --- ranges compare code units; a reversed range is a SyntaxError ---
try {
  console.log("reversed=" + new RegExp("[z-a]").test("m"));
} catch (e: any) {
  console.log("reversed!" + e.constructor.name);
}
console.log("unit-range=" + t(/[\u0000-￿]/, "\uD800"));
console.log("hex-range=" + t(/[\x41-\x43]/, "B"));
console.log("octal-ish=" + t(/[\101]/, "A"));

// --- an unknown escape is the identity character without u, an error with it ---
console.log("unknown-escape=" + t(/[\q]/, "q"));
try {
  console.log("unknown-escape-u=" + new RegExp("[\\q]", "u").test("q"));
} catch (e: any) {
  console.log("unknown-escape-u!" + e.constructor.name);
}

// --- literal dot and escaped dot ---
console.log("dot-literal=" + t(/a\.b/, "a.b") + "/" + t(/a\.b/, "axb"));
console.log("dot-in-class=" + t(/[.]/, "x") + "/" + t(/[.]/, "."));
