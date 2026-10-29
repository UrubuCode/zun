// Cross-runtime: the string-shaping methods. split (separator kinds, the limit as
// ToUint32 with wrap-around, captures occupying output slots, split("") by code unit,
// undefined versus absent separator, Symbol.split dispatch), padStart/padEnd (fill
// repetition and truncation, width counted in code units), repeat (count coercion
// by truncation, the two RangeError families, receiver coerced before the count),
// and trim/trimStart/trimEnd over the full WhiteSpace and LineTerminator set.
// Output is ASCII-only.

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

// units of a string as hex, for trim tests where characters are invisible
function units(s: string): string {
  const out: string[] = [];
  for (let i = 0; i < s.length; i++) out.push(s.charCodeAt(i).toString(16));
  return out.join(",");
}

// --- split by a string separator, with and without a limit ---
const csv = "a,b,c,d,e";
console.log("split-all=" + show(csv.split(",")) + " join=" + csv.split(",").join("|"));
console.log("split-lim-2=" + show(csv.split(",", 2)) + " lim-0=" + show(csv.split(",", 0)) + " lim-10=" + csv.split(",", 10).length);
console.log("split-space=" + show("hello world foo bar".split(" ")) + " lim-2=" + show("hello world foo bar".split(" ", 2)));
console.log("split-absent=" + show("abc".split("z")) + " empty-src=" + show("".split(",")) + " empty-sep=" + show("".split("")));
console.log("split-trailing=" + show("a,,b,".split(",")) + " frac-lim=" + show("a,,b,".split(",", 2.9)));

// --- the empty separator splits by code UNIT, so astral pairs are torn ---
const emoji = "a\u{1F600}b";
console.log("split-units-len=" + emoji.split("").length + " spread-len=" + [...emoji].length);
console.log("split-units=" + emoji.split("").map((c) => c.charCodeAt(0).toString(16)).join(","));
console.log("split-piece-lens=" + emoji.split("").map((c) => c.length).join(""));
console.log("split-rejoin=" + (emoji.split("").join("") === emoji));
console.log("split-half-cp=" + emoji.split("")[1].codePointAt(0)!.toString(16) + " full-cp=" + emoji.codePointAt(1)!.toString(16));
console.log("split-spread-cps=" + [...emoji].map((c) => c.codePointAt(0)!.toString(16)).join(","));
console.log("split-empty-limit=" + units(emoji.split("", 3).join("")) + " limit-0=" + emoji.split("", 0).length);
console.log("split-two-pairs=" + "\u{1F600}\u{1F601}".split("").length + " vs " + [..."\u{1F600}\u{1F601}"].length);
console.log("split-u10000=" + "\u{10000}".split("").length + " ffff=" + String.fromCharCode(0xffff).split("").length);
console.log("split-combining=" + ("e" + String.fromCharCode(0x301)).split("").length + " spread=" + [..."e" + String.fromCharCode(0x301)].length);
console.log("split-ascii=" + "abc".split("").join("|"));

// --- a regexp separator, with captures (which occupy output slots) and a limit ---
console.log("re-split=" + show("a1b2c3".split(/\d/)) + " lim-2=" + show("a1b2c3".split(/\d/, 2)));
console.log("re-capture=" + show("abc".split(/(b)/)) + " dash=" + show("a-b-c".split(/(-)/)));
console.log("re-date=" + show("2023-01-02".split(/(\d+)/)));
console.log("re-empty-match=" + show("xxx".split(/x*/)) + " empty-regex=" + show("abc".split(/(?:)/)));
console.log("re-cap-lim=" + show("a1b2c".split(/(\d)/, 2)) + " " + show("a1b2c".split(/(\d)/, 3)) + " " + show("a1b2c".split(/(\d)/, 4)));
console.log("re-unicode=" + show("\u{1F642}x".split(/(?:)/u)) + " no-u-len=" + "\u{1F642}x".split(/(?:)/).length);
console.log("re-split-captures=" + show("a1b22c".split(/(\d+)/)) + " limited=" + show("a1b22c".split(/(\d+)/, 4)));

// --- the limit is ToUint32: negatives wrap, 2^32 is zero, non-numbers are coerced ---
function j(a: string[]): string {
  return "[" + a.join("|") + "]#" + a.length;
}
console.log("lim-undef=" + j(csv.split(",", undefined)) + " lim-noarg=" + j(csv.split(",")));
console.log("lim-neg1=" + j(csv.split(",", -1)) + " lim-neg5=" + j(csv.split(",", -5)) + " lim-negzero=" + j(csv.split(",", -0)));
console.log("lim-2p32=" + j(csv.split(",", 4294967296)) + " 2p32p3=" + j(csv.split(",", 4294967299)));
console.log("lim-2p32m1-len=" + csv.split(",", 4294967295).length);
console.log("lim-str2=" + j(csv.split(",", "2" as any)) + " str-bad=" + j(csv.split(",", "zz" as any)));
console.log("lim-nan=" + j(csv.split(",", NaN as any)) + " inf=" + j(csv.split(",", Infinity as any)) + " neginf=" + j(csv.split(",", -Infinity as any)));
console.log("lim-bools=" + j(csv.split(",", true as any)) + " false=" + j(csv.split(",", false as any)) + " null=" + j(csv.split(",", null as any)));
console.log("lim-frac=" + j(csv.split(",", 2.9 as any)) + " negfrac=" + j(csv.split(",", -0.5 as any)));
const boxed: any = { valueOf() { return 2; } };
console.log("lim-obj=" + j(csv.split(",", boxed)) + " lim-arr=" + j(csv.split(",", [3] as any)));
console.log("lim-0-nomatch=" + j("abc".split("z", 0)) + " lim-0-regex=" + j("abc".split(/b/, 0)) + " lim-0-undef-sep=" + j("abc".split(undefined as any, 0)));
console.log("empty-lim0=" + j("".split(",", 0)) + " empty-lim1=" + j("".split(",", 1)) + " empty-empty-sep=" + j("".split("")));
console.log("lim-empty-sep-0=" + j("abc".split("", 0)));

// --- split(undefined) versus split(): one element, the whole string ---
const abc = "a,b,c";
console.log("noarg=" + abc.split().length + " " + abc.split()[0] + " undef=" + abc.split(undefined).length + " eq-src=" + (abc.split()[0] === abc));
console.log("literal-undefined=" + show("aundefinedb".split("undefined")));
const sep: string | undefined = undefined;
console.log("var-undef=" + abc.split(sep as any).length + " null=" + show("anullb".split(null as any)) + " null-nomatch=" + show("abc".split(null as any)));
console.log("undef-limit0=" + abc.split(undefined, 0).length + " undef-limit1=" + show(abc.split(undefined, 1)));
console.log("empty-undef=" + show("".split(undefined)) + " is-array=" + Array.isArray(abc.split(undefined)));

// --- Symbol.split receives (string, limit) with the limit passed through raw ---
const spl: any = {};
spl[Symbol.split] = function (str: string, lim: any) { return [str, typeof lim, show(lim)]; };
console.log("protocol-split=" + "abc".split(spl).join(","));
console.log("protocol-split-lim=" + "abc".split(spl, 3).join(","));
console.log("protocol-split-lim0=" + "abc".split(spl, 0).join(","));
const arrayish: any = { length: 2, 0: "x", 1: "y" };
arrayish[Symbol.split] = function () { return ["FROM-PROTOCOL"]; };
console.log("protocol-arrayish=" + "abc".split(arrayish).join(","));
console.log("re-direct-split=" + /,/[Symbol.split]("a,b,c", 2).join("|"));

// --- padStart / padEnd: the fill repeats and is truncated to the exact width ---
console.log("pad-start-default=" + show("a".padStart(5)) + " zeros=" + show("5".padStart(3, "0")) + " zeros-short=" + show("7".padStart(4, "0")));
console.log("pad-end-default=" + show("a".padEnd(5)) + " dash=" + show("a".padEnd(5, "-")) + " dots=" + show("7".padEnd(4, ".")));
console.log("pad-no-op=" + show("abc".padStart(2)) + " " + show("abc".padEnd(2)) + " " + show("already".padStart(3, "z")));
console.log("pad-multi-fill=" + show("ab".padStart(6, "123")) + " " + show("ab".padEnd(7, "12")) + " " + show("x".padStart(5, "ab")));
console.log("pad-truncate=" + show("7".padStart(6, "abc")) + " " + show("xy".padEnd(7, "01")) + " " + show("same".padEnd(4, "")));
console.log("pad-empty-src=" + show("".padStart(3, "x")) + " " + show("".padEnd(3, "yo")));
console.log("pad-unicode-fill=" + show("5".padStart(3, "\u00e9")) + " " + show("cafe".padStart(6, "*")));
console.log("pad-units-width=" + "\u{1F389}".length + " " + "\u{1F389}".padStart(3, "-").length);
console.log("pad-astral-fill=" + "x".padStart(4, "\u{1F49A}").length + " " + units("x".padStart(4, "\u{1F49A}")));
console.log("pad-astral-tear=" + "n".padStart(5, "\u{1F49A}\u{1F525}").length + " " + units("n".padStart(5, "\u{1F49A}\u{1F525}")));
console.log("pad-length-arg=" + show("a".padStart(3.9, "-")) + " " + show("a".padEnd(-5, "-")) + " " + show("a".padEnd(NaN as any, "-")));
console.log("pad-fill-undef=" + show("a".padStart(3, undefined as any)) + " fill-num=" + show("a".padStart(3, 0 as any)));

// --- repeat: count is ToIntegerOrInfinity, then two RangeError families ---
console.log("repeat=" + show("abc".repeat(3)) + " 0=" + show("abc".repeat(0)) + " 1=" + show("abc".repeat(1)));
console.log("repeat-empty=" + show("".repeat(5)) + " space=" + show(" ".repeat(5) + "x"));
console.log("repeat-line=" + show("-".repeat(10)) + " indent=" + show("  ".repeat(3) + "text"));
console.log("repeat-100=" + "x".repeat(100).length + " pattern=" + show("ab".repeat(5)));
console.log("repeat-frac-low=" + attempt(() => "ab".repeat(2.1)) + " frac-high=" + attempt(() => "ab".repeat(2.9)));
console.log("repeat-just-under=" + attempt(() => "ab".repeat(0.9)) + " neg-frac=" + attempt(() => "ab".repeat(-0.9)));
console.log("repeat-neg-one=" + attempt(() => "ab".repeat(-1)) + " neg-tiny=" + attempt(() => "ab".repeat(-1e-9)));
console.log("repeat-string=" + attempt(() => "ab".repeat("3" as any)) + " string-frac=" + attempt(() => "ab".repeat("2.7" as any)));
console.log("repeat-string-empty=" + attempt(() => "ab".repeat("" as any)) + " string-bad=" + attempt(() => "ab".repeat("x" as any)));
console.log("repeat-hex-string=" + attempt(() => "ab".repeat("0x2" as any)) + " null=" + attempt(() => "ab".repeat(null as any)));
console.log("repeat-undef=" + attempt(() => "ab".repeat(undefined as any)) + " no-arg=" + attempt(() => ("ab" as any).repeat()));
console.log("repeat-bools=" + attempt(() => "ab".repeat(true as any)) + " false=" + attempt(() => "ab".repeat(false as any)));
console.log("repeat-nan=" + attempt(() => "ab".repeat(NaN)));
console.log("repeat-array=" + attempt(() => "ab".repeat([2] as any)) + " empty=" + attempt(() => "ab".repeat([] as any)) + " two=" + attempt(() => "ab".repeat([1, 2] as any)));
console.log("repeat-object=" + attempt(() => "ab".repeat({ valueOf: () => 2 } as any)));
console.log("repeat-symbol=" + attempt(() => "ab".repeat(Symbol("2") as any)) + " bigint=" + attempt(() => "ab".repeat(2n as any)));
console.log("repeat-infinity=" + attempt(() => "ab".repeat(Infinity)) + " neg-infinity=" + attempt(() => "ab".repeat(-Infinity)));
console.log("repeat-over-limit=" + attempt(() => "ab".repeat(1e10).length) + " 2p31=" + attempt(() => "ab".repeat(2147483648).length));
console.log("repeat-max-safe=" + attempt(() => "ab".repeat(Number.MAX_SAFE_INTEGER).length));
console.log("repeat-empty-huge=" + attempt(() => "".repeat(1e10).length) + " empty-max=" + attempt(() => "".repeat(Number.MAX_SAFE_INTEGER).length));
console.log("repeat-empty-infinity=" + attempt(() => "".repeat(Infinity)) + " empty-neg=" + attempt(() => "".repeat(-1)) + " empty-zero=" + attempt(() => "".repeat(0)));
console.log("repeat-zero-huge-receiver=" + attempt(() => "x".repeat(1000).repeat(0).length));
const order: string[] = [];
const badThis: any = { toString: () => { order.push("this"); throw new RangeError("this"); } };
const badCount: any = { valueOf: () => { order.push("count"); throw new TypeError("count"); } };
const okThis: any = { toString: () => { order.push("this"); return "ab"; } };
const okCount: any = { valueOf: () => { order.push("count"); return 2; } };
console.log("repeat-both-throw=" + attempt(() => String.prototype.repeat.call(badThis, badCount)) + " order=" + order.join(","));
order.length = 0;
console.log("repeat-count-throws=" + attempt(() => String.prototype.repeat.call(okThis, badCount)) + " order=" + order.join(","));
order.length = 0;
console.log("repeat-this-throws=" + attempt(() => String.prototype.repeat.call(badThis, okCount)) + " order=" + order.join(","));
order.length = 0;
console.log("repeat-both-ok=" + attempt(() => String.prototype.repeat.call(okThis, okCount)) + " order=" + order.join(","));
console.log("repeat-null-this=" + attempt(() => String.prototype.repeat.call(null as any, 2)) + " undef-this=" + attempt(() => String.prototype.repeat.call(undefined as any, -1)));
console.log("repeat-number-this=" + attempt(() => String.prototype.repeat.call(12 as any, 2)) + " bool=" + attempt(() => String.prototype.repeat.call(true as any, 2)));
console.log("repeat-astral=" + attempt(() => "\u{1F600}".repeat(2).length) + " wf=" + "\u{1F600}".repeat(3).isWellFormed());
console.log("repeat-lone-wf=" + "\uD83D".repeat(2).isWellFormed() + " len=" + "\uD83D".repeat(2).length);
console.log("repeat-meta=" + String.prototype.repeat.name + "/" + String.prototype.repeat.length);

// --- trim family: the ECMAScript WhiteSpace and LineTerminator set, one char at a time ---
const wsCodes: number[] = [0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0xa0, 0x1680, 0x2000, 0x2003, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff];
console.log("trim-wrapped=" + wsCodes.map((c) => {
  const ch = String.fromCharCode(c);
  return (ch + "x" + ch).trim() === "x" ? "y" : "n";
}).join(""));
const notWs: number[] = [0x200b, 0x200c, 0x200d, 0x00, 0x85];
console.log("trim-not-ws=" + notWs.map((c) => {
  const ch = String.fromCharCode(c);
  return units((ch + "x" + ch).trim());
}).join(" | "));
const mixedWs = [0x09, 0x0a, 0x20, 0xa0, 0xfeff, 0x2003, 0x2028, 0x3000].map((c) => String.fromCharCode(c)).join("");
console.log("trim-all-ws=" + mixedWs.trim().length + "/" + mixedWs.trimStart().length + "/" + mixedWs.trimEnd().length);
const exotic = String.fromCharCode(0xa0, 0xfeff, 0x2028) + "x" + String.fromCharCode(0x2029, 0x3000);
console.log("trim-exotic-raw=" + units(exotic));
console.log("trim-exotic-start=" + units(exotic.trimStart()));
console.log("trim-exotic-end=" + units(exotic.trimEnd()));
console.log("trim-exotic-both=" + units(exotic.trim()));
console.log("trim-interior=" + units((" a" + String.fromCharCode(0xa0) + "b ").trim()));
console.log("trim-zwsp-blocks=" + units((" " + String.fromCharCode(0x200b) + " x").trimStart()));
console.log("trim-empty=" + "".trim().length + " nbsp-only=" + (String.fromCharCode(0xa0) + String.fromCharCode(0xa0)).trim().length + " bom-only=" + String.fromCharCode(0xfeff).trim().length);
console.log("trim-formats=" + show("\t\thello\t\t".trim()) + " " + show("\n\nhello\n\n".trim()) + " " + show(" \t\n hello \n\t ".trim()));
console.log("trim-sides=" + show("  hello world  ".trimStart()) + " " + show("  hello world  ".trimEnd()) + " " + show("  hello world  ".trim()));
console.log("trim-no-mutate=" + "  x  ".trim().length);
const tsrc = " x ";
tsrc.trim();
console.log("trim-receiver-kept=" + tsrc.length + " alias=" + String.prototype.trim.call(" q " as any));
