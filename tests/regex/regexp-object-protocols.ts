// Cross-runtime: the RegExp object as a value. Covers the flags getter's fixed
// order and the boolean accessors, `source` escaping so that /source/ always
// re-parses, the constructor's copy and flag-replacement rules, IsRegExp via
// Symbol.match, RegExp.prototype as a non-regex with its own exemptions, regex
// identity and coercion (no Symbol.toPrimitive; ToString only), JSON and
// enumeration, and subclasses that override exec or set Symbol.species.

function attempt(f: () => any): string {
  try {
    const v = f();
    return v === undefined ? "undefined" : JSON.stringify(v);
  } catch (e: any) {
    return "!" + e.constructor.name;
  }
}

function bad(src: string, f: string): string {
  try {
    return "ok:" + new RegExp(src, f).flags;
  } catch (e: any) {
    return "!" + e.constructor.name;
  }
}

// --- flags is assembled in a fixed canonical order, never echoed back ---
console.log("literal-scrambled=" + /a/yusimgd.flags);
console.log("literal-plain=" + /a/.flags);
console.log("ctor-scrambled=" + new RegExp("a", "yusimgd").flags);
console.log("ctor-reverse=" + new RegExp("a", "dgimsuy").flags);
console.log("v-instead-of-u=" + new RegExp("a", "yvsimgd").flags);
console.log("subset=" + /a/mi.flags + " " + /a/gi.flags);
console.log("bools=" + [/a/dgimsy.hasIndices, /a/dgimsy.global, /a/dgimsy.ignoreCase, /a/dgimsy.multiline,
  /a/dgimsy.dotAll, /a/dgimsy.sticky, /a/dgimsy.unicode, /a/dgimsy.unicodeSets].join(","));
console.log("u-bools=" + /a/u.unicode + "," + /a/u.unicodeSets);
console.log("v-bools=" + /a/v.unicode + "," + /a/v.unicodeSets);

// --- flags and source are accessors on the prototype, not own data properties ---
console.log("own-flags=" + Object.prototype.hasOwnProperty.call(/a/g, "flags"));
console.log("own-source=" + Object.prototype.hasOwnProperty.call(/a/g, "source"));
const dFlags: any = Object.getOwnPropertyDescriptor(RegExp.prototype, "flags");
console.log("desc-flags=" + (typeof dFlags.get) + "/" + dFlags.set + "/" + dFlags.enumerable + "/" + dFlags.configurable);
const dSource: any = Object.getOwnPropertyDescriptor(RegExp.prototype, "source");
console.log("desc-source=" + (typeof dSource.get) + "/" + (dSource.set === undefined) + "/" + dSource.enumerable + "/" + dSource.configurable);
console.log("getter-name=" + dSource.get.name + " length=" + dSource.get.length);
console.log("own-source-flag-keys=" + Object.keys(/a/g).length);

// --- flags is generic: it reads the boolean accessors as plain properties ---
console.log("flags-on-fake=" + attempt(() => dFlags.get.call({ global: true, sticky: true, dotAll: true })));
console.log("flags-order=" + attempt(() => dFlags.get.call({
  sticky: true, unicode: true, multiline: true, ignoreCase: true, global: true, hasIndices: true, dotAll: true,
})));
console.log("flags-truthy=" + attempt(() => dFlags.get.call({ global: 1, ignoreCase: "x" })));
console.log("flags-primitive=" + attempt(() => dFlags.get.call(1)));
console.log("flags-null=" + attempt(() => dFlags.get.call(null)));
const fakeFlags: any = { global: true, ignoreCase: false, multiline: true, dotAll: false,
  unicode: false, unicodeSets: false, sticky: true, hasIndices: false };
console.log("fake-flags=" + (dFlags.get.call(fakeFlags) as string));

// --- source escapes whatever would break the /.../ round trip ---
console.log("empty-ctor-src=" + new RegExp("").source);
console.log("empty-tostring=" + String(new RegExp("")));
console.log("empty-matches=" + new RegExp("").test("x"));
console.log("empty-roundtrip=" + new RegExp(new RegExp("").source).source);
console.log("empty-group-src=" + /(?:)/.source + " equal=" + (new RegExp("").source === /(?:)/.source));
console.log("slash-in-class=" + /[/]/.source + " " + String(/[/]/));
console.log("slash-escaped=" + /\//.source);
console.log("ctor-slash=" + new RegExp("/").source + " " + String(new RegExp("/")));
console.log("escaped-slash-kept=" + new RegExp("a\\/b").source);
console.log("slash-mid=" + new RegExp("a/b").source + " " + new RegExp("///").source);
console.log("slash-roundtrip=" + new RegExp(/a\/b/).source + " test=" + new RegExp(/a\/b/).test("a/b"));
console.log("newline-src=" + JSON.stringify(new RegExp("a\nb").source));
console.log("newline-codes=" + new RegExp("\n").source.length + " " + new RegExp("\n").source.charCodeAt(0).toString(16));
console.log("cr-codes=" + new RegExp("\r").source.charCodeAt(0).toString(16));
console.log("ls-codes=" + new RegExp(" ").source.charCodeAt(0).toString(16));
console.log("ps-codes=" + new RegExp(" ").source.charCodeAt(0).toString(16));
console.log("newline-matches=" + new RegExp(new RegExp("\n").source).test("\n"));
console.log("tab-kept=" + new RegExp("\t").source.length + " nul-kept=" + new RegExp("\u0000").source.length);
console.log("dot-src=" + /a.b/.source + " caret-class=" + /[^a]/.source + " bs=" + /\\/.source.length);
console.log("src-passthrough=" + new RegExp("\\d+", "g").source + " " + new RegExp("(?<x>a)").source + " " + new RegExp("a{2,3}").source);

// --- toString is exactly "/" + source + "/" + flags, and round-trips ---
const cases: RegExp[] = [/a/gi, /a\/b/m, new RegExp("x", "suy"), /(?:)/g, /[\]]/];
for (let i = 0; i < cases.length; i++) {
  const re = cases[i];
  const rebuilt = new RegExp(re.source, re.flags);
  console.log("case" + i + " str=" + String(re) + " same=" + (String(rebuilt) === String(re)) + " parts=" + (String(re) === "/" + re.source + "/" + re.flags));
}

// --- the constructor copies pattern and flags, and does not carry lastIndex ---
const src = /ab+c/gi;
src.lastIndex = 3;
const copy = new RegExp(src);
console.log("copy=" + copy.source + " " + copy.flags + " identity=" + (copy === src) + " li=" + copy.lastIndex);
const over = new RegExp(src, "m");
console.log("over=" + over.source + " " + over.flags + " global=" + over.global);
console.log("over-empty=" + "[" + new RegExp(src, "").flags + "]");
console.log("over-add=" + new RegExp(/a/, "gy").flags);
console.log("undef-flags=" + new RegExp(src, undefined).flags);
console.log("ctor-coerce-flags=" + new RegExp("a", { toString() { return "gi"; } } as any).flags);
console.log("ctor-num-flags=" + attempt(() => new RegExp("a", 1 as any).flags));
console.log("ctor-symbol-flags=" + attempt(() => new RegExp("a", Symbol("g") as any).flags));
console.log("ctor-flags-bad=" + bad("a", "gg") + " " + bad("a", "q") + " " + bad("a", "uv") + " " + bad("a", "G") + " " + bad("a", " g") + " " + bad("a", ""));
const orig = /a/g;
const clone = new RegExp(orig);
orig.lastIndex = 1;
console.log("indep=" + clone.test("aa") + ":" + clone.lastIndex + ":" + orig.lastIndex);
console.log("indep2=" + orig.test("aa") + ":" + orig.lastIndex);

// --- without new: same object when no flags are supplied, a copy when they are ---
console.log("call-identity=" + (RegExp(src) === src) + " li=" + RegExp(src).lastIndex);
console.log("call-flags=" + (RegExp(src, "i") === src) + " " + RegExp(src, "i").flags);
console.log("call-same-literal=" + ((RegExp as any)(/a/g) === (RegExp as any)(/a/g)));
console.log("call-string=" + (RegExp("a") instanceof RegExp));

// --- constructor arguments that are not patterns ---
console.log("noarg=" + new RegExp(undefined).source + "|" + "[" + new RegExp(undefined).flags + "]");
console.log("noargs=" + new (RegExp as any)().source);
console.log("null-pattern=" + new RegExp(null as any).source + " " + new RegExp(null as any).test("null"));
console.log("number-pattern=" + new RegExp(12 as any).source);

// --- IsRegExp: a plain object is only a regex when it declares Symbol.match ---
const impostor: any = { source: "zz", flags: "g", toString() { return "q+"; } };
console.log("impostor-source=" + new RegExp(impostor).source + " flags=[" + new RegExp(impostor).flags + "]");
console.log("impostor-match=" + new RegExp(impostor).test("qqq"));
const matchy: any = { source: "z+", flags: "i", toString() { return "NEVER"; } };
matchy[Symbol.match] = true;
console.log("matchy-source=" + new RegExp(matchy).source + " flags=" + new RegExp(matchy).flags);
console.log("matchy-override=" + new RegExp(matchy, "g").flags);
console.log("matchy-call-identity=" + (RegExp(matchy) === matchy));
console.log("startsWith-match-true=" + attempt(() => "a".startsWith({ [Symbol.match]: true } as any)));
console.log("startsWith-match-false=" + attempt(() => "a".startsWith({ [Symbol.match]: false, toString: () => "a" } as any)));

// --- every regex literal is a NEW object; lastIndex is per object ---
function make(): RegExp {
  return /a/g;
}
console.log("literal-identity=" + (make() === make()));
console.log("ctor-identity=" + (new RegExp("a") === new RegExp("a")));
console.log("string-eq=" + (String(/a/g) === String(/a/g)));
console.log("typeof=" + typeof /a/ + " call=" + typeof (/a/ as any).call);
const first = make();
first.lastIndex = 2;
console.log("fresh-lastIndex=" + make().lastIndex + "/" + first.lastIndex);

// --- as Map and Set members a regex is compared by identity ---
const map = new Map<any, number>();
map.set(/a/, 1);
console.log("map-get-new=" + map.get(/a/));
const key = /a/;
map.set(key, 2);
console.log("map-get-same=" + map.get(key) + " size=" + map.size);
console.log("set-size=" + new Set<any>([/a/, /a/, key, key]).size);

// --- coercion: no Symbol.toPrimitive, no own valueOf, so ToString does everything ---
console.log("no-toPrimitive=" + ((/a/ as any)[Symbol.toPrimitive] === undefined));
console.log("valueOf-inherited=" + !Object.prototype.hasOwnProperty.call(RegExp.prototype, "valueOf"));
console.log("plus=" + +(/a/ as any) + " number=" + Number(/12/) + " digits=" + Number(new RegExp("12")));
console.log("concat=" + ("x" + /a\/b/gi) + " template=" + `${/a\/b/gi}`);
console.log("string-fn=" + String(/a/y) + " boolean=" + Boolean(new RegExp("")));
console.log("loose-eq-string=" + ((/a/ as any) == "/a/") + " array=" + (([/a/] as any) == "/a/"));
console.log("join=" + [/a/, /b/g].join("+"));
const list: any[] = [/b/, /a/g, /a/, /c/i];
console.log("sorted=" + list.sort().map(String).join(" "));

// --- JSON and enumeration see no enumerable own state ---
console.log("json=" + JSON.stringify(/a\/b/gi));
console.log("json-nested=" + JSON.stringify({ re: /a/g, n: 1 }));
console.log("json-array=" + JSON.stringify([/a/]));
console.log("keys=" + JSON.stringify(Object.keys(/a/g)) + " entries=" + JSON.stringify(Object.entries(/a/g)));
console.log("spread=" + JSON.stringify({ ...(/a/g as any) }) + " assign=" + JSON.stringify(Object.assign({}, /a/g)));
const withToJson: any = /a/g;
withToJson.toJSON = function () {
  return this.source + "|" + this.flags;
};
console.log("json-hook=" + JSON.stringify(withToJson));
const ext: any = /a/;
console.log("extensible=" + Object.isExtensible(ext));
console.log("add-prop=" + Reflect.set(ext, "note", "hi") + "/" + ext.note);
console.log("keys-after=" + JSON.stringify(Object.keys(ext)) + " json-after=" + JSON.stringify(ext));
console.log("sealed=" + Object.isSealed(Object.seal(/a/)));
console.log("tag=" + Object.prototype.toString.call(/a/) + " toStringTag=" + String((/a/ as any)[Symbol.toStringTag]));
console.log("instanceof=" + (/a/ instanceof Object) + " proto=" + (Object.getPrototypeOf(/a/) === RegExp.prototype));

// --- RegExp.prototype is an ordinary object, not a RegExp: no matcher slot ---
const P: any = RegExp.prototype;
const FLAGS = ["hasIndices", "global", "ignoreCase", "multiline", "dotAll", "unicode", "unicodeSets", "sticky"];
console.log("proto-tag=" + Object.prototype.toString.call(P) + " instance=" + (P instanceof RegExp));
console.log("proto-of-proto=" + (Object.getPrototypeOf(P) === Object.prototype) + " ctor=" + (P.constructor === RegExp));
console.log("proto-lastIndex-own=" + Object.prototype.hasOwnProperty.call(P, "lastIndex"));
console.log("proto-source=" + P.source + " flags=[" + P.flags + "] toString=" + P.toString());
for (let i = 0; i < FLAGS.length; i++) {
  console.log("proto-" + FLAGS[i] + "=" + String(P[FLAGS[i]]));
}
console.log("proto-exec=" + attempt(() => P.exec("a")));
console.log("proto-test=" + attempt(() => P.test("a")));
console.log("proto-match=" + attempt(() => "a".match(P)));
console.log("proto-search=" + attempt(() => "a".search(P)));
console.log("proto-replace=" + attempt(() => "a".replace(P, "-")));
console.log("proto-split=" + attempt(() => "a".split(P)));
console.log("proto-matchAll=" + attempt(() => [..."a".matchAll(P)].length));

// --- a plain object with the right own properties is still not a regex ---
const fake: any = { source: "a", flags: "g", global: true, lastIndex: 0 };
for (let i = 0; i < FLAGS.length; i++) {
  const g: any = Object.getOwnPropertyDescriptor(RegExp.prototype, FLAGS[i]);
  console.log("fake-" + FLAGS[i] + "=" + attempt(() => g.get.call(fake)));
}
const gs: any = Object.getOwnPropertyDescriptor(RegExp.prototype, "source");
console.log("fake-source=" + attempt(() => gs.get.call(fake)));
console.log("fake-flags-getter=" + attempt(() => dFlags.get.call(fake)));
console.log("fake-exec=" + attempt(() => RegExp.prototype.exec.call(fake, "a")));
console.log("tostring-fake=" + RegExp.prototype.toString.call({ source: "x", flags: "y" }));
console.log("tostring-coerce=" + RegExp.prototype.toString.call({ source: 1, flags: 2 }));
console.log("tostring-missing=" + RegExp.prototype.toString.call({}));
const inst = /a/gi;
console.log("instance-own=" + FLAGS.map((f) => Object.prototype.hasOwnProperty.call(inst, f)).join(","));
const dGlobal: any = Object.getOwnPropertyDescriptor(RegExp.prototype, "global");
console.log("global-desc=" + (typeof dGlobal.get) + "/" + (dGlobal.set === undefined) + "/" + dGlobal.enumerable + "/" + dGlobal.configurable);

// --- subclasses: exec is the hook that every String method reaches through ---
class Counting extends RegExp {
  calls = 0;
  exec(s: string): any {
    this.calls++;
    return super.exec(s);
  }
}
const c0 = new Counting("a", "gi");
console.log("sub-source=" + c0.source + " flags=" + c0.flags + " global=" + c0.global + " ic=" + c0.ignoreCase);
console.log("sub-instanceof=" + (c0 instanceof RegExp) + ":" + (c0 instanceof Counting));
console.log("sub-tostring=" + String(c0) + " tag=" + Object.prototype.toString.call(c0));
const r1 = new Counting("a", "g");
console.log("sub-replace=" + "aba".replace(r1 as any, "-") + ":" + r1.calls);
const r2 = new Counting("a", "g");
console.log("sub-match=" + (("aba".match(r2 as any) as any) || []).join(",") + ":" + r2.calls);
const r3 = new Counting("a", "g");
console.log("sub-test=" + r3.test("aba") + ":" + r3.calls);
const r4 = new Counting("b", "");
console.log("sub-search=" + "aba".search(r4 as any) + ":" + r4.calls);
const r5 = new Counting("a", "g");
console.log("sub-matchAll=" + [..."aba".matchAll(r5 as any)].length + ":" + r5.calls);
const r6 = new Counting("a", "g");
console.log("sub-replaceAll=" + "aba".replaceAll(r6 as any, "-") + ":" + r6.calls);
console.log("sub-own-exec=" + Object.prototype.hasOwnProperty.call(Counting.prototype, "exec") + " li-own=" + Object.prototype.hasOwnProperty.call(c0, "lastIndex"));
console.log("sub-ctor-name=" + c0.constructor.name + " chain=" + (Object.getPrototypeOf(Counting.prototype) === RegExp.prototype));

class Loud extends RegExp {
  exec(s: string): any {
    const m = super.exec(s);
    if (m) m[0] = "<" + m[0] + ">";
    return m;
  }
}
console.log("loud-replace=" + "abc".replace(new Loud("b") as any, "[$&]"));
console.log("loud-match=" + ("abc".match(new Loud("b") as any) as any)[0]);
console.log("loud-split=" + "abc".split(new Loud("b") as any).join("|"));

const dead: any = /x/g;
dead.exec = () => null;
console.log("dead-replace=" + "xxx".replace(dead, "-") + " match=" + String("xxx".match(dead)) + " test=" + dead.test("xxx") + " search=" + "xxx".search(dead));
const shadowed: any = /b/;
shadowed.exec = 42;
console.log("noncallable-exec=" + "abc".replace(shadowed, "-"));
const bogus: any = /b/;
bogus.exec = () => 7;
console.log("bogus-exec=" + attempt(() => "abc".replace(bogus, "-")));
const fakeExec: any = /zzz/;
fakeExec.exec = (s: string) => {
  const arr: any = ["bc", "c"];
  arr.index = 1;
  arr.input = s;
  arr.groups = undefined;
  return arr;
};
console.log("fake-exec-replace=" + "abc".replace(fakeExec, "[$&|$1]"));
console.log("fake-exec-match=" + ("abc".match(fakeExec) as any).index + " search=" + "abc".search(fakeExec));

// --- Symbol.species drives the clone made by split; species can be overridden ---
console.log("species-RegExp=" + ((RegExp as any)[Symbol.species] === RegExp));
class Sub extends RegExp {}
console.log("split-subclass=" + "a,b,c".split(new Sub(",") as any).join("|") + " sub-species=" + ((Sub as any)[Symbol.species] === Sub));
const seen: string[] = [];
class Spy extends RegExp {
  constructor(p: any, f: any) {
    super(p, f);
    seen.push(String(p) + "|" + String(f));
  }
  static get [Symbol.species]() {
    return Spy;
  }
}
console.log("spy-gi=" + JSON.stringify("a1b2c".split(new Spy("\\d", "gi") as any)) + " args=" + seen.join(" ; "));
seen.length = 0;
console.log("spy-y=" + JSON.stringify("a1b".split(new Spy("\\d", "y") as any)) + " args=" + seen.join(" ; "));
seen.length = 0;
console.log("spy-plain=" + JSON.stringify("a1b".split(new Spy("\\d", "") as any)) + " args=" + seen.join(" ; "));
class Wrapped extends RegExp {
  static get [Symbol.species]() {
    return RegExp;
  }
}
console.log("species-regexp=" + JSON.stringify("a1b".split(new Wrapped("\\d", "") as any)));
const orig2 = /,/g;
orig2.lastIndex = 3;
console.log("split-orig-untouched=" + JSON.stringify("a,b,c".split(orig2)) + " li=" + orig2.lastIndex);
