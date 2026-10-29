// Cross-runtime: a JavaScript string is a sequence of UTF-16 code units, and every
// operation makes a different choice about it. Index methods (charAt, charCodeAt,
// codePointAt, at, []) count units; iteration (spread, for-of, Array.from) counts
// code points; split("") counts units. This fixture pins those choices side by side
// on astral characters and lone surrogate halves, then the well-formedness helpers,
// the places where a lone half is an error (encodeURIComponent) or is escaped
// (JSON.stringify), the string iterator object itself, and UTF-16 ordering.
// Output is ASCII-only: every non-ASCII value is printed as hex code units.

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

// hex code units of a string, comma-separated
function units(s: string): string {
  const out: string[] = [];
  for (let i = 0; i < s.length; i++) out.push(s.charCodeAt(i).toString(16));
  return out.join(",");
}

const HI = String.fromCharCode(0xd83d);
const LO = String.fromCharCode(0xde00);
const GRIN = HI + LO; // U+1F600, two units
const S = "a" + GRIN + "b" + GRIN + "c"; // a HI LO b HI LO c

// --- length counts units; spread and for-of count code points ---
console.log("len=" + S.length + " cp-count=" + [...S].length);
console.log("len-grin=" + "\u{1F600}".length + " cp-grin=" + [...("\u{1F600}")].length);
console.log("len-double=" + "\u{1F600}\u{1F600}".length + " cp-double=" + [..."\u{1F600}\u{1F600}"].length);
console.log("len-mixed=" + "a\u{1F600}b".length + " cp-mixed=" + [..."a\u{1F600}b"].length);
console.log("len-empty=" + "".length);
console.log("units=" + units(S));
console.log("split-empty-len=" + S.split("").length + " spread-len=" + [...S].length);
console.log("split-empty-units=" + show(S.split("").map((c) => c.charCodeAt(0).toString(16))));
console.log("split-empty-lens=" + S.split("").map((c) => c.length).join(""));
console.log("split-empty-limit=" + units("a\u{1F600}b".split("", 3).join("")));
console.log("cp-of-half=" + HI.codePointAt(0)!.toString(16) + " cp-of-pair=" + GRIN.codePointAt(0)!.toString(16));
console.log("cp-points=" + [...S].map((c) => c.codePointAt(0)!.toString(16)).join(","));
console.log("cp-lengths=" + [...S].map((c) => c.length).join(""));
const pairs = [...("A\u{1F642}B")];
console.log("cp-array=" + show(pairs.map((c) => c.length)) + " " + units(pairs.join("|")));
console.log("cp-from-mapfn=" + Array.from("a\u{1F600}b", (c, i) => i + ":" + c.length).join("|"));
let forOf = "";
for (const ch of "a\u{1F600}b") forOf += ch.length;
console.log("cp-for-of-lens=" + forOf);
const [d0, d1, d2, d3] = "a\u{1F600}b";
console.log("cp-destructure=" + d0 + "|" + units(d1) + "|" + units(d2) + "|" + show(d3));

// --- index methods at every position of S: they disagree about what a unit is ---
for (let i = 0; i <= 3; i++) {
  console.log(
    "at" + i +
      " charAt=" + units(S.charAt(i)) +
      " bracket=" + units((S as any)[i]) +
      " at=" + units(S.at(i) as string) +
      " charCodeAt=" + S.charCodeAt(i).toString(16) +
      " codePointAt=" + S.codePointAt(i)!.toString(16),
  );
}
console.log("out-of-range=" + show(S.charAt(99)) + "/" + S.charCodeAt(99) + "/" + show(S.codePointAt(99)) + "/" + show(S.at(99)) + "/" + show((S as any)[99]));
console.log("negative=" + show(S.charAt(-1)) + "/" + show(S.at(-1)) + "/" + S.charCodeAt(-1) + "/" + show(S.codePointAt(-1)));
console.log("negative-bracket=" + show((S as any)[-1]) + " at-neg-2=" + units(S.at(-2) as string));
console.log("fractional=" + show(S.charAt(1.9)) + " at=" + units(S.at(1.9) as string) + " bracket=" + show((S as any)[1.9]));
console.log("empty-access=" + show("".charAt(0)) + "/" + show("".at(0)) + "/" + show(("" as any)[0]) + "/" + "".charCodeAt(0));
console.log("at-noarg=" + show(("hello" as any).at()) + " charAt-noarg=" + show("hello".charAt()));
console.log("bracket-canonical=" + show(("hello" as any)["1"]) + " " + show(("hello" as any)["01"]) + " " + show(("hello" as any)["1.0"]) + " " + show(("hello" as any)[" 1"]));
console.log("in-operator=" + (0 in Object("hello")) + " " + (5 in Object("hello")));

// --- fromCharCode wraps each input to an unsigned 16-bit unit; fromCodePoint does not ---
console.log("fcc-wrap=" + units(String.fromCharCode(65, 0x10041, -1, 3.9)));
console.log("fcc-zero=" + units(String.fromCharCode(0)) + " space=" + units(String.fromCharCode(32)));
console.log("fcc-astral-input=" + String.fromCharCode(0x1f600).length + " fcp-astral=" + String.fromCodePoint(0x1f600).length);
const mixedCp = String.fromCodePoint(0x41, 0x1f600, 0x20ac);
console.log("fcp-mixed=" + mixedCp.length + " cps=" + [...mixedCp].map((c) => c.codePointAt(0)!.toString(16)).join(","));
console.log("fcp-range=" + attempt(() => String.fromCodePoint(0x110000)) + " " + attempt(() => String.fromCodePoint(-1)) + " " + attempt(() => String.fromCodePoint(1.5)));

// --- search for a HALF succeeds at the index inside the pair ---
console.log("indexOf-hi=" + S.indexOf(HI) + " indexOf-lo=" + S.indexOf(LO));
console.log("lastIndexOf-hi=" + S.lastIndexOf(HI) + " lastIndexOf-lo=" + S.lastIndexOf(LO));
console.log("includes-hi=" + S.includes(HI) + " indexOf-pair=" + S.indexOf(GRIN) + " last-pair=" + S.lastIndexOf(GRIN));
console.log("indexOf-lo-b=" + S.indexOf(LO + "b") + " indexOf-b-hi=" + S.indexOf("b" + HI));
console.log("indexOf-from=" + S.indexOf(GRIN, 1) + "/" + S.indexOf(GRIN, 2) + "/" + S.indexOf(GRIN, 3));
console.log("startsWith-at=" + S.startsWith(GRIN, 1) + "/" + S.startsWith(LO, 2));
console.log("endsWith-at=" + S.endsWith(GRIN, 3) + "/" + S.endsWith(HI, 2));
console.log("search=" + S.search(new RegExp(GRIN)) + " search-lo=" + S.search(new RegExp(LO)));
console.log("regex-index-u=" + (new RegExp(GRIN, "u").exec(S) as any).index);
console.log("regex-dot=" + (S.match(/./g) as any).length + " dot-u=" + (S.match(/./gu) as any).length);
console.log("half-comparison=" + ("a" + GRIN + "x").startsWith(HI) + " " + ("x" + GRIN).endsWith(LO));

// --- slicing at an odd boundary tears the pair; well-formedness tells you so ---
console.log("slice-0-2=" + units(S.slice(0, 2)) + " wf=" + S.slice(0, 2).isWellFormed());
console.log("slice-2-4=" + units(S.slice(2, 4)) + " wf=" + S.slice(2, 4).isWellFormed());
console.log("slice-1-3=" + units(S.slice(1, 3)) + " wf=" + S.slice(1, 3).isWellFormed());
console.log("substring-1-2=" + units(S.substring(1, 2)) + " substr-2-1=" + units(S.substr(2, 1)));
console.log("slice-neg=" + units(S.slice(-1)) + " / " + units(S.slice(-2)) + " / " + units(S.slice(-3)));
console.log("replace-hi=" + units(S.replace(HI, "X")) + " wf=" + S.replace(HI, "X").isWellFormed());
console.log("replaceAll-lo=" + units(S.replaceAll(LO, "Y")));
console.log("split-on-hi=" + S.split(HI).length + " wf=" + S.split(HI).map((p) => String(p.isWellFormed())).join(","));
const padded = "x".padEnd(4, GRIN);
console.log("pad-units=" + units(padded) + " wf=" + padded.isWellFormed());
const joined = ("a" + HI) + (LO + "b");
console.log("joined-len=" + joined.length + " cps=" + [...joined].length + " wf=" + joined.isWellFormed() + " cp=" + joined.codePointAt(1)!.toString(16));
console.log("halves-alone-wf=" + ("a" + HI).isWellFormed() + "/" + (LO + "b").isWellFormed());
console.log("cmp-torn-prefix=" + (S.slice(0, 2) < S.slice(0, 3)) + " prefix=" + S.startsWith(S.slice(0, 2)));

// --- isWellFormed and toWellFormed: one lone unit becomes U+FFFD, length preserved ---
function probe(label: string, s: string): string {
  const fixed = s.toWellFormed();
  return label + " wf=" + s.isWellFormed() + " len=" + s.length + "/" + fixed.length + " fixed=" + units(fixed);
}
console.log(probe("ascii", "abc"));
console.log(probe("valid-pair", HI + LO));
console.log(probe("pair-in-text", "a" + HI + LO + "b"));
console.log(probe("lone-hi-mid", "a" + HI + "b"));
console.log(probe("lone-lo-start", LO + "ab"));
console.log(probe("reversed-pair", LO + HI));
console.log(probe("hi-hi", HI + HI));
console.log(probe("hi-hi-hi-lo", HI + HI + HI + LO));
console.log(probe("lone-then-pair", HI + HI + LO));
console.log(probe("pair-then-lone", HI + LO + LO));
console.log(probe("min-max-hi", String.fromCharCode(0xd800) + String.fromCharCode(0xdbff)));
console.log(probe("min-max-lo", String.fromCharCode(0xdc00) + String.fromCharCode(0xdfff)));
console.log(probe("adjacent", String.fromCharCode(0xd7ff) + String.fromCharCode(0xe000)));
console.log(probe("replacement", String.fromCharCode(0xfffd)));
const broken = "a" + HI + "b" + LO + "c";
console.log("idempotent=" + (broken.toWellFormed().toWellFormed() === broken.toWellFormed()));
console.log("fixed-is-wf=" + broken.toWellFormed().isWellFormed());
console.log("good-unchanged=" + (("a" + HI + LO + "b").toWellFormed() === "a" + HI + LO + "b"));
console.log("receiver-untouched=" + units(broken) + " len-kept=" + (broken.toWellFormed().length === broken.length));

// --- JSON.stringify is well-formed since ES2019: lone halves escape, pairs do not ---
console.log("json-pair=" + show(JSON.stringify(HI + LO)) + " codes=" + units(JSON.stringify(HI + LO)));
console.log("json-lone-hi=" + show(JSON.stringify(HI)) + " lone-lo=" + show(JSON.stringify(LO)));
console.log("json-lone-mid=" + show(JSON.stringify("a" + HI + "b")));
console.log("json-dbff=" + show(JSON.stringify(String.fromCharCode(0xdbff))) + " dfff=" + show(JSON.stringify(String.fromCharCode(0xdfff))));
console.log("json-adjacent=" + show(JSON.stringify(String.fromCharCode(0xd7ff))) + show(JSON.stringify(String.fromCharCode(0xe000))));
console.log("json-roundtrip=" + (JSON.parse(JSON.stringify(broken)) === broken) + " wf=" + JSON.parse(JSON.stringify(broken)).isWellFormed());
console.log("json-keys=" + show(JSON.stringify({ [HI]: 1 })) + " arr=" + show(JSON.stringify([HI, HI + LO])));
console.log("json-nested=" + show(JSON.stringify({ a: { b: [LO] } })));
console.log("json-controls=" + show(JSON.stringify("\b\f\n\r\t")) + " nul=" + show(JSON.stringify(String.fromCharCode(0))));
console.log("json-ctrl-1f=" + show(JSON.stringify(String.fromCharCode(0x1f))) + " 7f=" + show(JSON.stringify(String.fromCharCode(0x7f))));
console.log("json-2028=" + show(JSON.stringify(String.fromCharCode(0x2028))) + " nbsp=" + units(JSON.stringify(String.fromCharCode(0xa0))));
console.log("json-parse-lone=" + units(JSON.parse('"a\\ud800b"')) + " wf=" + JSON.parse('"a\\ud800b"').isWellFormed());
console.log("json-parse-pair=" + JSON.parse('"\\ud83d\\ude00"').length + " cp=" + JSON.parse('"\\ud83d\\ude00"').codePointAt(0).toString(16));
console.log("json-indent=" + show(JSON.stringify({ a: HI }, null, 1)));

// --- the URI functions: a lone half is a URIError; the repaired string encodes ---
console.log("euc-pair=" + encodeURIComponent(HI + LO) + " emoji=" + encodeURIComponent("\u{1F600}"));
console.log("euc-lead=" + attempt(() => encodeURIComponent("a" + HI + "b")));
console.log("euc-trail=" + attempt(() => encodeURIComponent("a" + LO + "b")));
console.log("euc-reversed=" + attempt(() => encodeURIComponent(LO + HI)));
console.log("eu-lone=" + attempt(() => encodeURI("a" + HI + "b")));
console.log("euc-repaired=" + encodeURIComponent(broken.toWellFormed()));
console.log("euc-roundtrip=" + (decodeURIComponent(encodeURIComponent(broken.toWellFormed())) === broken.toWellFormed()));
console.log("escape-number=" + encodeURIComponent(42 as any) + " undef=" + encodeURIComponent(undefined as any));

// --- the string iterator is an object with its own shape, and it snapshots the string ---
const it: any = S[Symbol.iterator]();
console.log("it-tag=" + show(Object.prototype.toString.call(it)) + " toStringTag=" + show(it[Symbol.toStringTag]));
console.log("it-self=" + (it[Symbol.iterator]() === it) + " next=" + typeof it.next + " return=" + typeof it.return + " throw=" + typeof it.throw);
console.log("it-own-names=" + show(Object.getOwnPropertyNames(it)));
const itProto: any = Object.getPrototypeOf(it);
const iterProto: any = Object.getPrototypeOf(itProto);
console.log("it-proto-next=" + Object.prototype.hasOwnProperty.call(itProto, "next") + " chain-root=" + (Object.getPrototypeOf(iterProto) === Object.prototype));
console.log("it-next-meta=" + itProto.next.name + "/" + itProto.next.length);
const tagDesc: any = Object.getOwnPropertyDescriptor(itProto, Symbol.toStringTag);
console.log("it-tag-desc=" + show(tagDesc.value) + "/" + tagDesc.writable + "/" + tagDesc.enumerable + "/" + tagDesc.configurable);
const step: any = S[Symbol.iterator]();
for (let i = 0; i < 5; i++) {
  const r: any = step.next();
  console.log("it-step" + i + " done=" + r.done + " value=" + (r.value === undefined ? "u" : units(r.value)) + " len=" + (r.value === undefined ? "-" : r.value.length));
}
console.log("it-result-keys=" + show(Object.keys(S[Symbol.iterator]().next())));
let mutable = "ab";
const snap: any = mutable[Symbol.iterator]();
console.log("snap1=" + snap.next().value);
mutable = "zz";
console.log("snap2=" + snap.next().value + " snap3=" + String(snap.next().value) + " var=" + mutable);
console.log("it-detached=" + attempt(() => itProto.next.call({})) + " on-string=" + attempt(() => itProto.next.call("ab")));
const spent: any = "a"[Symbol.iterator]();
spent.next();
console.log("spent1=" + JSON.stringify(spent.next()) + " spent2=" + JSON.stringify(spent.next()));
const empty: any = ""[Symbol.iterator]();
console.log("empty-iter=" + JSON.stringify(empty.next()) + " empty-spread=" + [...""].length);
const lone = "a" + HI + "b";
console.log("lone-iter-count=" + [...lone].length + " codes=" + [...lone].map(units).join("|"));
const reversedLone = LO + HI;
console.log("reversed-iter-count=" + [...reversedLone].length + " codes=" + [...reversedLone].map(units).join("|"));
console.log("boxed-spread=" + [...(new String("ab") as any)].join("|"));
console.log("iter-fn=" + String.prototype[Symbol.iterator].name + "/" + String.prototype[Symbol.iterator].length);
console.log("iter-same=" + ((new String("ab") as any)[Symbol.iterator] === String.prototype[Symbol.iterator]));

// --- code-unit order: <, sort and default compare by unit, not by code point ---
const FF3A = "\uff3a";
const GRIN2 = "\u{1F600}";
console.log("cp-order=" + (FF3A.codePointAt(0)! < GRIN2.codePointAt(0)!) + " unit-order=" + (FF3A < GRIN2) + " gt=" + (FF3A > GRIN2));
console.log("lone-vs-ff3a=" + (HI < FF3A) + " trail-vs-ff3a=" + (LO < FF3A) + " lead-vs-trail=" + (HI < LO));
console.log("grin-eq-pair=" + (GRIN2 === HI + LO));
const mixed = [GRIN2, FF3A, "z", "A", "\u00e9", "\ufffd"];
console.log("sorted-units=" + mixed.slice().sort().map(units).join(" "));
const chars = [...(FF3A + GRIN2)];
console.log("spread-sorted=" + chars.slice().sort().map(units).join(" "));
console.log("cp-sorted=" + chars.slice().sort((a, b) => a.codePointAt(0)! - b.codePointAt(0)!).map(units).join(" "));
