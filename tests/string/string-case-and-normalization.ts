// Cross-runtime: the Unicode-aware transformations. Case MAPPING (toUpperCase and
// toLowerCase with SpecialCasing: one unit in, several out, not round-trippable) is
// a different operation from case FOLDING (the /iu regexp, length-preserving), and both
// are different from NORMALIZATION (NFC/NFD/NFKC/NFKD: canonical and compatibility
// decomposition, recomposition, singletons, canonical reordering, and algorithmic Hangul).
// Also: grapheme clusters, which JavaScript counts by code point and never by grapheme.
// Every non-ASCII character is built from code points so the source stays ASCII.

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

// code units as hex, space-separated
function cu(s: string): string {
  const out: string[] = [];
  for (let i = 0; i < s.length; i++) out.push(s.charCodeAt(i).toString(16));
  return out.join(" ");
}

// code points as hex, space-separated
function cps(s: string): string {
  const out: string[] = [];
  for (const ch of s) out.push((ch.codePointAt(0) as number).toString(16));
  return out.join(" ");
}

const ch = (n: number) => String.fromCharCode(n);

// --- case mapping: one unit can become several, and the result is not invertible ---
function up(s: string): string {
  const u = s.toUpperCase();
  return show(u) + "/" + u.length + "/" + cu(u);
}
function down(s: string): string {
  const d = s.toLowerCase();
  return show(d) + "/" + d.length + "/" + cu(d);
}
console.log("sharp-s-up=" + up(ch(0xdf)) + " in-word=" + up("stra" + ch(0xdf) + "e"));
console.log("sharp-s-roundtrip=" + (ch(0xdf).toUpperCase().toLowerCase() === ch(0xdf)));
console.log("capital-sharp-down=" + down(ch(0x1e9e)) + " up=" + up(ch(0x1e9e)));
console.log("fi-ligature=" + up(ch(0xfb01)) + " ffi=" + up(ch(0xfb03)) + " ffl=" + up(ch(0xfb04)));
console.log("ligature-down=" + down(ch(0xfb01)) + " st=" + up(ch(0xfb06)));
console.log("final-sigma-up=" + up(ch(0x3c2)) + " medial-up=" + up(ch(0x3c3)));
console.log("sigma-down=" + down(ch(0x3a3)) + " sigma-roundtrip=" + (ch(0x3c2).toUpperCase().toLowerCase() === ch(0x3c2)));
console.log("word-sigma-up=" + up("\u1f40\u03b4\u03c5\u03c3\u03c3\u03b5\u03cd\u03c2") + " down=" + down("\u039f\u0394\u03a5\u03a3\u03a3\u0395\u03a5\u03a3"));
console.log("dotless-up=" + up(ch(0x131)) + " dotted-down=" + down(ch(0x130)) + " dotted-up=" + up(ch(0x130)));
console.log("ascii-i-up=" + up("i") + " ascii-I-down=" + down("I"));
console.log("ypogegrammeni=" + up(ch(0x1f80)) + " n-preceded=" + up(ch(0x149)));
console.log("hebrew-yod-up=" + up(ch(0xfb49)) + " armenian=" + up(ch(0xfb34)));
console.log("deseret=" + "\u{10428}".toUpperCase().codePointAt(0)!.toString(16) + " " + "\u{10400}".toLowerCase().codePointAt(0)!.toString(16) + " len=" + "\u{10428}".toUpperCase().length);
console.log("no-case=" + up("123") + " cjk=" + up("\u4e2d\u6587") + " empty=" + show("".toUpperCase()));
console.log("emoji-len=" + "\u{1F600}".toUpperCase().length + " lone=" + cu("\uD800".toUpperCase()));
console.log("ascii-basic=" + down("Hello World") + " " + up("Hello World") + " mixed=" + down("HeLLo WoRLd"));
console.log("lossy-ligature=" + (ch(0xfb01).toUpperCase().toLowerCase() === ch(0xfb01)) + " lossy-dotted=" + ch(0x130).toLowerCase().toUpperCase().length);

// --- case folding (/iu) is length-preserving; it refuses what mapping accepts ---
function row(label: string, a: string, b: string): string {
  return label +
    " fold=" + new RegExp("^" + a + "$", "iu").test(b) +
    " foldNoU=" + new RegExp("^" + a + "$", "i").test(b) +
    " lower=" + (a.toLowerCase() === b.toLowerCase()) +
    " upper=" + (a.toUpperCase() === b.toUpperCase()) +
    " lenA=" + a.length + " lenB=" + b.length;
}
const KELVIN = ch(0x212a);
const ANGSTROM = ch(0x212b);
const LONG_S = ch(0x17f);
const MICRO = ch(0xb5);
const MU = ch(0x3bc);
const OHM = ch(0x2126);
const OMEGA = ch(0x3a9);
const SHARP_S = ch(0xdf);
const CAP_SHARP_S = ch(0x1e9e);
const FI = ch(0xfb01);
const DOTLESS_I = ch(0x131);
const DOTTED_I = ch(0x130);
console.log(row("kelvin-k", KELVIN, "k"));
console.log(row("angstrom-aring", ANGSTROM, ch(0xe5)));
console.log(row("longs-s", LONG_S, "s"));
console.log(row("micro-mu", MICRO, MU));
console.log(row("ohm-omega", OHM, OMEGA));
console.log(row("ascii", "A", "a"));
console.log(row("sharp-ss", SHARP_S, "ss"));
console.log(row("sharp-SS", SHARP_S, "SS"));
console.log(row("sharp-capital", SHARP_S, CAP_SHARP_S));
console.log(row("fi-ligature", FI, "fi"));
console.log(row("fi-ligature-upper", FI, "FI"));
console.log(row("dotless-i", DOTLESS_I, "i"));
console.log(row("dotted-I", DOTTED_I, "I"));
console.log(row("dotted-i", DOTTED_I, "i"));
const samples: [string, string][] = [
  ["kelvin", KELVIN], ["angstrom", ANGSTROM], ["longs", LONG_S], ["micro", MICRO],
  ["sharp", SHARP_S], ["fi", FI], ["dotted-I", DOTTED_I], ["dotless-i", DOTLESS_I], ["ascii-a", "a"],
];
for (const [name, s] of samples) {
  console.log(
    "trip-" + name +
      " up.down=" + (s.toUpperCase().toLowerCase() === s) +
      " down.up=" + (s.toLowerCase().toUpperCase() === s) +
      " upLen=" + s.toUpperCase().length +
      " downLen=" + s.toLowerCase().length +
      " selfFold=" + new RegExp("^" + s + "$", "iu").test(s),
  );
}
// a case-insensitive index search has no built-in: lowercasing finds what /iu does not
console.log("map-search-kelvin=" + ("a" + KELVIN + "b").toLowerCase().indexOf("k") + " fold-search-kelvin=" + ("a" + KELVIN + "b").search(/k/iu));
console.log("map-search-sharp=" + "aSSb".toLowerCase().indexOf(SHARP_S.toLowerCase()) + " fold-search-sharp=" + "aSSb".search(new RegExp(SHARP_S, "iu")));
console.log("map-search-longs=" + ("a" + LONG_S + "b").toLowerCase().indexOf("s") + " fold-search-longs=" + ("a" + LONG_S + "b").search(/s/iu));

// --- normalization: compatibility, canonical, and what NFKC throws away ---
function four(label: string, s: string): string {
  return label +
    " raw=" + cu(s) +
    " NFC=" + cu(s.normalize("NFC")) +
    " NFD=" + cu(s.normalize("NFD")) +
    " NFKC=" + cu(s.normalize("NFKC")) +
    " NFKD=" + cu(s.normalize("NFKD"));
}
console.log(four("super-2", ch(0xb2)));
console.log(four("super-n", ch(0x207f)));
console.log(four("sub-1", ch(0x2081)));
console.log(four("half", ch(0xbd)));
console.log(four("quarter", ch(0xbc)));
console.log(four("circled-1", ch(0x2460)));
console.log(four("circled-A", ch(0x24b6)));
console.log(four("paren-a", ch(0x249c)));
console.log(four("fullwidth-A", ch(0xff21)));
console.log(four("halfwidth-ka", ch(0xff76)));
console.log(four("fi-lig", ch(0xfb01)));
console.log(four("long-s", ch(0x17f)));
console.log(four("nbsp", ch(0xa0)));
console.log(four("en-space", ch(0x2002)));
console.log(four("narrow-nbsp", ch(0x202f)));
console.log(four("emoji-digit", "1"));
console.log(four("eacute", ch(0xe9)));
console.log(four("cjk", ch(0x4e2d)));
console.log(four("zwj", ch(0x200d)));
console.log(four("bom", ch(0xfeff)));
const HW_KA = ch(0xff76);
const HW_VOICED = ch(0xff9e);
console.log(four("hw-ga", HW_KA + HW_VOICED));
console.log("hw-ga-is-GA=" + ((HW_KA + HW_VOICED).normalize("NFKC") === ch(0x30ac)) + " nfkd-len=" + (HW_KA + HW_VOICED).normalize("NFKD").length);
console.log("nbsp-nfkc-space=" + (ch(0xa0).normalize("NFKC") === " ") + " nbsp-nfc-unchanged=" + (ch(0xa0).normalize("NFC") === ch(0xa0)));
console.log("half-nfkc-len=" + ch(0xbd).normalize("NFKC").length);
const SUPER2 = ch(0xb2);
console.log("k-lossy=" + (SUPER2.normalize("NFKC").normalize("NFC") === SUPER2) + " k-idempotent=" + (SUPER2.normalize("NFKC").normalize("NFKC") === SUPER2.normalize("NFKC")));
console.log("nfkd-then-nfkc=" + cu(SUPER2.normalize("NFKD").normalize("NFKC")));
console.log("k-sharp-kept=" + (SHARP_S.normalize("NFKC") === SHARP_S) + " micro-nfkc=" + (MICRO.normalize("NFKC") === MU));

// --- the argument of normalize: four exact strings, undefined means NFC, the rest throw ---
function norm(arg: any): string {
  try {
    return "ok:" + cu(ch(0xa0).normalize(arg));
  } catch (e: any) {
    return "!" + e.name;
  }
}
console.log("arg-NFC=" + norm("NFC") + " NFD=" + norm("NFD") + " NFKC=" + norm("NFKC") + " NFKD=" + norm("NFKD"));
console.log("arg-none=" + norm(undefined) + " arg-undef-explicit=" + norm(undefined as any));
console.log("arg-lower=" + norm("nfc") + " mixed=" + norm("Nfc") + " pad=" + norm(" NFC") + " empty=" + norm(""));
console.log("arg-null=" + norm(null) + " number=" + norm(1) + " object=" + norm({ toString: () => "NFD" }) + " symbol=" + norm(Symbol("NFC")));
const SEQ = "A" + ch(0x30a);
console.log("default-is-nfc=" + (SEQ.normalize() === SEQ.normalize("NFC")) + " len=" + SEQ.length + "->" + SEQ.normalize().length);
console.log("normalize-on-number=" + String.prototype.normalize.call(12 as any, "NFC") + " boxed=" + String.prototype.normalize.call(new String("a") as any, "NFC"));
console.log("normalize-on-null=" + attempt(() => String.prototype.normalize.call(null as any, "NFC")));

// --- singletons: NFC leaves the canonical target, never the source ---
const A_RING = ch(0xc5);
const RING = ch(0x30a);
console.log(four("angstrom", ANGSTROM));
console.log("angstrom-nfc-Aring=" + (ANGSTROM.normalize("NFC") === A_RING) + " Aring-nfc=" + (A_RING.normalize("NFC") === A_RING));
console.log("angstrom-not-restored=" + (A_RING.normalize("NFC") === ANGSTROM) + " raw-eq=" + (ANGSTROM === A_RING) + " nfc-eq=" + (ANGSTROM.normalize("NFC") === A_RING.normalize("NFC")));
console.log(four("ohm", OHM));
console.log("ohm-nfc-omega=" + (OHM.normalize("NFC") === OMEGA));
console.log(four("kelvin", KELVIN));
console.log("kelvin-nfc-K=" + (KELVIN.normalize("NFC") === "K") + " kelvin-unchanged=" + (KELVIN.normalize("NFC") === KELVIN));
console.log("angstrom-lower=" + cu(ANGSTROM.toLowerCase()));

// --- canonical ordering: marks of different combining class are sorted by class ---
const DOT_BELOW = ch(0x323);
const DOT_ABOVE = ch(0x307);
const ACUTE = ch(0x301);
console.log(four("q-below-above", "q" + DOT_BELOW + DOT_ABOVE));
console.log(four("q-above-below", "q" + DOT_ABOVE + DOT_BELOW));
console.log("reorder-nfd-equal=" + (("q" + DOT_BELOW + DOT_ABOVE).normalize("NFD") === ("q" + DOT_ABOVE + DOT_BELOW).normalize("NFD")));
console.log("raw-not-equal=" + (("q" + DOT_BELOW + DOT_ABOVE) === ("q" + DOT_ABOVE + DOT_BELOW)) + " nfc-equal=" + (("q" + DOT_BELOW + DOT_ABOVE).normalize("NFC") === ("q" + DOT_ABOVE + DOT_BELOW).normalize("NFC")));
console.log("same-class-differ=" + (("e" + ACUTE + DOT_ABOVE).normalize("NFD") !== ("e" + DOT_ABOVE + ACUTE).normalize("NFD")));
console.log("composition-blocked-len=" + ("s" + DOT_BELOW + DOT_ABOVE).normalize("NFC").length + " blocked=" + ("s" + DOT_ABOVE + DOT_BELOW).normalize("NFC").length);
console.log("excluded-344-len=" + ("a" + ch(0x344)).normalize("NFC").length + " A-ring-composes=" + (("A" + RING).normalize("NFC") === A_RING));
const DEVA_QA = ch(0x915) + ch(0x93c);
console.log(four("deva-nukta", DEVA_QA));
console.log("deva-nfd=" + cu(ch(0x958).normalize("NFD")) + " nfc-back=" + cu(ch(0x958).normalize("NFC")));
const idemSamples: string[] = [ANGSTROM, OHM, KELVIN, "q" + DOT_ABOVE + DOT_BELOW, "a" + ch(0x344), ch(0x958)];
const forms = ["NFC", "NFD", "NFKC", "NFKD"];
console.log("idempotent=" + idemSamples.map((s, i) => i + ":" + forms.every((f) => s.normalize(f).normalize(f) === s.normalize(f))).join(" "));
console.log("length-grow=" + A_RING.length + "->" + A_RING.normalize("NFD").length + " shrink=" + ("A" + RING).length + "->" + ("A" + RING).normalize("NFC").length + " same=" + ANGSTROM.length + "->" + ANGSTROM.normalize("NFC").length);

// --- algorithmic Hangul: S = 0xAC00 + (L*21 + V)*28 + T, not a table ---
const SBASE = 0xac00;
const LBASE = 0x1100;
const VBASE = 0x1161;
const TBASE = 0x11a7;
function syllable(l: number, v: number, t: number): string {
  return ch(SBASE + (l * 21 + v) * 28 + t);
}
function hangul(label: string, s: string): string {
  const nfd = s.normalize("NFD");
  const nfc = s.normalize("NFC");
  return label + " in=" + cps(s) + " nfd=" + cps(nfd) + "(" + nfd.length + ")" + " nfc=" + cps(nfc) + "(" + nfc.length + ")" + " round=" + (nfd.normalize("NFC") === nfc);
}
console.log(hangul("ga", syllable(0, 0, 0)));
console.log(hangul("gag", syllable(0, 0, 1)));
console.log(hangul("hih", syllable(18, 20, 27)));
console.log(hangul("first", ch(SBASE)));
console.log(hangul("last", ch(0xd7a3)));
const L = ch(LBASE);
const V = ch(VBASE);
const T = ch(TBASE + 1);
console.log(hangul("L+V", L + V));
console.log(hangul("L+V+T", L + V + T));
console.log(hangul("LV+T", ch(SBASE) + T));
console.log(hangul("L-alone", L));
console.log(hangul("V-alone", V));
console.log(hangul("L+T", L + T));
console.log(hangul("V+L", V + L));
console.log(hangul("L+syllable", L + syllable(0, 0, 0)));
console.log(hangul("syllable+V", syllable(0, 0, 0) + V));
console.log(hangul("two", syllable(0, 0, 0) + syllable(1, 1, 1)));
const word = syllable(0, 0, 0) + syllable(18, 20, 27) + syllable(11, 5, 0);
console.log("word-len=" + word.length + " nfd-len=" + word.normalize("NFD").length + " nfc-is-original=" + (word.normalize("NFD").normalize("NFC") === word));
console.log("mixed-nfc=" + cps(("a" + L + V + "b" + L + V + T).normalize("NFC")));
console.log("nfkc-eq-nfc=" + (syllable(0, 0, 1).normalize("NFKC") === syllable(0, 0, 1).normalize("NFC")));
const COMPAT_KIYEOK = ch(0x3131);
console.log("compat-nfc=" + cps(COMPAT_KIYEOK.normalize("NFC")) + " nfkc=" + cps(COMPAT_KIYEOK.normalize("NFKC")) + " nfkd=" + cps(COMPAT_KIYEOK.normalize("NFKD")));
console.log("halfwidth-kiyeok-nfkc=" + cps(ch(0xffa1).normalize("NFKC")) + " nfc=" + cps(ch(0xffa1).normalize("NFC")));
const compatRun = ch(0x3131) + ch(0x314f);
console.log("compat-run-nfkc=" + cps(compatRun.normalize("NFKC")) + " len=" + compatRun.normalize("NFKC").length);

// --- grapheme clusters: JavaScript counts code units, code points, and regexp atoms ---
const ZWJ = ch(0x200d);
const VS16 = ch(0xfe0f);
const KEYCAP = ch(0x20e3);
const MAN = String.fromCodePoint(0x1f468);
const WOMAN = String.fromCodePoint(0x1f469);
const BOY = String.fromCodePoint(0x1f466);
const WAVE = String.fromCodePoint(0x1f44b);
const SKIN4 = String.fromCodePoint(0x1f3fd);
const RI_B = String.fromCodePoint(0x1f1e7);
const RI_R = String.fromCodePoint(0x1f1f7);
const HEART = String.fromCodePoint(0x2764);
const FAMILY = MAN + ZWJ + WOMAN + ZWJ + BOY;
const clusters: [string, string][] = [
  ["family", FAMILY],
  ["wave-skin", WAVE + SKIN4],
  ["flag", RI_B + RI_R],
  ["heart-vs16", HEART + VS16],
  ["keycap", "1" + VS16 + KEYCAP],
  ["e-acute-seq", "e" + ACUTE],
  ["plain-astral", MAN],
  ["ascii", "ab"],
];
for (const [name, s] of clusters) {
  console.log(
    name +
      " units=" + s.length +
      " points=" + [...s].length +
      " dotU=" + (s.match(/./gu) as any).length +
      " dot=" + (s.match(/./gs) as any).length +
      " split=" + s.split("").length +
      " cps=" + cps(s),
  );
}
console.log("family-anchored=" + /^.$/u.test(FAMILY) + " three=" + /^...$/u.test(FAMILY) + " five=" + /^.....$/u.test(FAMILY));
console.log("family-class=" + new RegExp("^[" + MAN + "]", "u").test(FAMILY) + " zwj-space=" + /\s/.test(ZWJ) + " zwj-word=" + /\w/.test(ZWJ));
console.log("family-slice2-points=" + [...FAMILY.slice(0, 2)].length + " first-cp=" + [...FAMILY.slice(0, 2)][0].codePointAt(0)!.toString(16));
console.log("family-at0=" + (FAMILY.at(0) as string).charCodeAt(0).toString(16) + " first-point=" + [...FAMILY][0].codePointAt(0)!.toString(16));
const flagRev = [...(RI_B + RI_R)].reverse().join("");
console.log("flag-reversed-cps=" + cps(flagRev) + " wf=" + flagRev.isWellFormed());
const unitRev = (RI_B + RI_R).split("").reverse().join("");
console.log("unit-reversed-wf=" + unitRev.isWellFormed() + " len=" + unitRev.length);
console.log("accent-reversed-cps=" + cps([..."e" + ACUTE].reverse().join("")));
console.log("accent-nfc-points=" + [...("e" + ACUTE).normalize("NFC")].length + " family-nfc-points=" + [...FAMILY.normalize("NFC")].length);
console.log("keycap-nfkc-points=" + [...("1" + VS16 + KEYCAP).normalize("NFKC")].length + " heart-nfc-len=" + (HEART + VS16).normalize("NFC").length);
console.log("family-upper-len=" + FAMILY.toUpperCase().length + " accent-upper=" + cps(("e" + ACUTE).toUpperCase()));
console.log("indexOf-man=" + FAMILY.indexOf(MAN) + " indexOf-boy=" + FAMILY.indexOf(BOY) + " includes-woman=" + FAMILY.includes(WOMAN));
console.log("wave-in-skin=" + (WAVE + SKIN4).includes(WAVE) + " replace-part-points=" + [...FAMILY.replace(WOMAN, "")].length);
const padded = "x".padEnd(6, FAMILY);
console.log("pad-len=" + padded.length + " points=" + [...padded].length + " wf=" + padded.isWellFormed());
