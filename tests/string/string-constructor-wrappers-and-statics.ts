// Cross-runtime: the String constructor used three ways -- as a function (primitive
// result), as an object (the String exotic object: own indices with frozen-in-place
// descriptors, hidden length, == but never ===), and as a namespace of static
// helpers. Also: generic `this` on String.prototype methods (ToString after
// RequireObjectCoercible), statics detached and aliased as first-class values,
// String.raw's length arithmetic, and the Annex B HTML methods with their escaping.
// Output is ASCII-only: non-ASCII is escaped by show().

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

// --- String() as a function returns a primitive ---
console.log("call-typeof=" + show(typeof String("ab")));
console.log("call-eq=" + (String("ab") === "ab"));
console.log("call-num=" + show(String(5)));
console.log("call-func-null=" + show(String(null)));

// --- new String: a String exotic object ---
const w: any = new String("ab");
console.log("w-typeof=" + show(typeof w) + " prim=" + show(typeof "ab"));
console.log("w-loose-eq=" + (w == "ab"));
console.log("w-strict-eq=" + (w === "ab"));
console.log("w-self-loose=" + (w == new String("ab")));
console.log("w-valueOf=" + show(w.valueOf()));
console.log("w-toString=" + show(w.toString()));
console.log("w-tag=" + show(Object.prototype.toString.call(w)));
console.log("w-instanceof=" + (w instanceof String));
console.log("w-proto=" + (Object.getPrototypeOf(w) === String.prototype));
console.log("w-concat=" + show("x" + w));
console.log("w-template=" + show(`${w}`));
console.log("w-json=" + show(JSON.stringify(w)));
console.log("w-json-nested=" + show(JSON.stringify({ a: new String("z") })));
console.log("w-indices=" + show(w[0]) + show(w[1]) + show(w[2]));
console.log("w-length=" + w.length);
console.log("w-keys=" + show(Object.keys(w)));
console.log("w-own-names=" + show(Object.getOwnPropertyNames(w)));
console.log("w-has=" + ("0" in w) + "/" + ("2" in w) + "/" + Object.prototype.hasOwnProperty.call(w, "length"));

// --- descriptors: indices are enumerable but locked in place; length is hidden ---
const d0: any = Object.getOwnPropertyDescriptor(w, "0");
console.log("w-desc0=" + show(d0.value) + "/" + d0.writable + "/" + d0.enumerable + "/" + d0.configurable);
const dl: any = Object.getOwnPropertyDescriptor(w, "length");
console.log("w-desc-length=" + show(dl.value) + "/" + dl.writable + "/" + dl.enumerable + "/" + dl.configurable);

// --- writes: an existing index and length refuse; a new property is accepted ---
console.log("w-set-idx=" + Reflect.set(w, "0", "z"));
console.log("w-idx0-after=" + show(w[0]));
console.log("w-set-length=" + Reflect.set(w, "length", 9));
console.log("w-length-after=" + w.length);
console.log("w-set-new=" + Reflect.set(w, "extra", 7) + " extra=" + w.extra);
console.log("w-keys-after=" + show(Object.keys(w)));
console.log("w-delete-idx=" + Reflect.deleteProperty(w, "0"));
console.log("w-delete-extra=" + Reflect.deleteProperty(w, "extra"));

// --- extensibility and integrity levels ---
console.log("w-extensible=" + Object.isExtensible(new String("ab")));
console.log("w-frozen=" + Object.isFrozen(new String("ab")) + " frozen-empty=" + Object.isFrozen(new String("")));
console.log("w-sealed-empty=" + Object.isSealed(Object.preventExtensions(new String(""))));

// --- iteration over a wrapper: for-in sees indices; spread and Array.from use code points ---
const seen: string[] = [];
for (const k in new String("abc")) seen.push(k);
console.log("w-forin=" + show(seen));
console.log("w-spread=" + show([...(new String("ab") as any)]));
console.log("w-from=" + show(Array.from(new String("ab") as any)));

// --- boxing: no double wrapping, empty wrapper is truthy, numbers box too ---
console.log("w-wrap-empty-len=" + new String("").length);
console.log("w-wrap-number=" + show(new String(42).valueOf()));
console.log("w-rewrap=" + show(new String(new String("ab") as any).valueOf()));
console.log("w-cond-empty=" + (new String("") ? "truthy" : "falsy"));

// --- generic this: primitives coerce, objects go through ToString ---
const upper = String.prototype.toUpperCase;
const slice = String.prototype.slice;
const indexOf = String.prototype.indexOf;
console.log("this-num=" + attempt(() => upper.call(42 as any)));
console.log("this-float=" + attempt(() => slice.call(3.5 as any, 1)));
console.log("this-bool=" + attempt(() => upper.call(true as any)));
console.log("this-bigint=" + attempt(() => slice.call(10n as any, 0, 1)));
console.log("this-array=" + attempt(() => indexOf.call([1, 2, 3] as any, ",")));
console.log("this-array-upper=" + attempt(() => upper.call(["a", "b"] as any)));
console.log("this-plain=" + attempt(() => upper.call({} as any)));
console.log("this-tostring=" + attempt(() => upper.call({ toString() { return "hi"; } } as any)));
console.log("this-valueOf-only=" + attempt(() => upper.call({ valueOf() { return "vo"; } } as any)));
console.log("this-regexp=" + attempt(() => indexOf.call(/ab/g as any, "b")));
console.log("this-wrapper=" + attempt(() => upper.call(new String("ab") as any)));
console.log("this-wrapper-slice=" + attempt(() => slice.call(new String("abc") as any, 1)));

// --- nullish or symbol receivers, throwing or missing toString ---
console.log("this-null=" + attempt(() => upper.call(null as any)));
console.log("this-undefined=" + attempt(() => upper.call(undefined as any)));
console.log("this-noarg=" + attempt(() => (0, String.prototype.trim)()));
console.log("this-symbol=" + attempt(() => upper.call(Symbol("s") as any)));
console.log("this-throwing=" + attempt(() => upper.call({ toString() { throw new RangeError("x"); } } as any)));
console.log("this-no-tostring=" + attempt(() => upper.call(Object.create(null) as any)));

// --- arguments are coerced independently of the receiver ---
console.log("arg-obj=" + attempt(() => indexOf.call("a,b", { toString() { return ","; } } as any)));
console.log("arg-symbol=" + attempt(() => indexOf.call("ab", Symbol("s") as any)));
console.log("arg-null=" + attempt(() => indexOf.call("anull", null as any)));

// --- prototype facts: length 0, valueOf is brand-checked ---
console.log("proto-length=" + String.prototype.length + " typeof=" + typeof String.prototype);
console.log("proto-valueOf-obj=" + attempt(() => String.prototype.valueOf.call({} as any)));
console.log("proto-valueOf-wrapper=" + attempt(() => String.prototype.valueOf.call(new String("z") as any)));
console.log("proto-valueOf-prim=" + attempt(() => String.prototype.valueOf.call("z" as any)));
console.log("proto-iter-num=" + attempt(() => [...(String.prototype[Symbol.iterator].call(123 as any) as any)].join("|")));
console.log("proto-iter-null=" + attempt(() => String.prototype[Symbol.iterator].call(null as any)));

// --- static helpers as first-class values: aliasing ignores `this` ---
const cc = String.fromCharCode;
console.log("static-alias=" + show(cc(66)) + " multi=" + show(cc(67, 68, 69)) + " none=" + show(cc()));
const holder: any = { f: String.fromCharCode };
console.log("static-prop=" + show(holder.f(71)));
console.log("static-map=" + show([73, 74, 75].map((n) => String.fromCharCode(n)).join("")));
console.log("static-identity=" + (cc === String.fromCharCode) + "/" + (holder.f === String.fromCharCode));
console.log("static-call=" + show(String.fromCharCode.call(null as any, 83)));
console.log("static-apply=" + show(String.fromCharCode.apply(null, [84, 85])));
console.log("static-bind=" + show(String.fromCharCode.bind(null as any)(86)));
console.log("static-bind-partial=" + show(String.fromCharCode.bind(null as any, 87)(88)));
const fcp = String.fromCodePoint;
console.log("static-fcp-alias=" + show(fcp(98)) + " astral-len=" + fcp(0x1f600).length);
console.log("static-meta=" + typeof cc + "/" + cc.name + "/" + cc.length + "/" + fcp.name + "/" + fcp.length);

// --- String.raw: the tag reads the raw segments and interleaves substitutions ---
const name = "test";
console.log("raw-tag=" + show(String.raw`a\nb`));
console.log("raw-tag-sub=" + show(String.raw`a${1}b${2}c`));
console.log("raw-tag-empty=" + show(String.raw``) + " only-sub=" + show(String.raw`${1}`));
console.log("raw-path=" + show(String.raw`C:\Users\name\file.txt`));
console.log("raw-sub-name=" + show(String.raw`Hello ${name}!`));
console.log("raw-vs-cooked=" + show(`a\tb`) + " / " + show(String.raw`a\tb`));
console.log("raw-backslash-len=" + String.raw`\\`.length);
console.log("raw-newline-len=" + String.raw`a
b`.length);
const raw: any = String.raw;
console.log("raw-exact=" + attempt(() => raw({ raw: ["a", "b", "c"] }, 1, 2)));
console.log("raw-extra-subs=" + attempt(() => raw({ raw: ["a", "b"] }, 1, 2, 3)));
console.log("raw-one-segment=" + attempt(() => raw({ raw: ["a"] }, 1, 2)));
console.log("raw-no-subs=" + attempt(() => raw({ raw: ["a", "b", "c"] })));
console.log("raw-one-sub-three-segs=" + attempt(() => raw({ raw: ["a", "b", "c"] }, "X")));
console.log("raw-empty-raw=" + attempt(() => raw({ raw: [] }, 1, 2)));
console.log("raw-trailing=" + attempt(() => raw({ raw: ["<", ">"] }, "MID")));
console.log("raw-leading=" + attempt(() => raw({ raw: ["", "!"] }, "X")));
console.log("raw-string-raw=" + attempt(() => raw({ raw: "xyz" }, 1, 2)));
console.log("raw-arraylike=" + attempt(() => raw({ raw: { length: 2, 0: "a", 1: "b" } }, "-")));
console.log("raw-holes=" + attempt(() => raw({ raw: { length: 3, 0: "a", 2: "c" } }, "-", "-")));
console.log("raw-frac-length=" + attempt(() => raw({ raw: { length: 2.9, 0: "a", 1: "b", 2: "c" } }, "-", "-")));
console.log("raw-neg-length=" + attempt(() => raw({ raw: { length: -1, 0: "a" } }, "-")));
console.log("raw-no-length=" + attempt(() => raw({ raw: { 0: "a", 1: "b" } }, "-")));
console.log("raw-num-segments=" + attempt(() => raw({ raw: [1, 2] }, 9)));
console.log("raw-obj-sub=" + attempt(() => raw({ raw: ["<", ">"] }, { toString: () => "T" })));
console.log("raw-null-sub=" + attempt(() => raw({ raw: ["<", ">"] }, null)));
console.log("raw-undefined-sub=" + attempt(() => raw({ raw: ["<", ">"] }, undefined)));
console.log("raw-symbol-sub=" + attempt(() => raw({ raw: ["<", ">"] }, Symbol("s"))));
console.log("raw-no-raw=" + attempt(() => raw({})));
console.log("raw-null-raw=" + attempt(() => raw({ raw: null })));
console.log("raw-no-arg=" + attempt(() => raw()));
console.log("raw-throwing-sub=" + attempt(() => raw({ raw: ["<", ">"] }, { toString: () => { throw new RangeError("x"); } })));
console.log("raw-meta=" + raw.name + "/" + raw.length);

// --- Annex B HTML methods: tag-only ones, attribute ones, and CreateHTML escaping ---
const S: any = "x";
console.log("annexb-tags=" + show([S.big(), S.blink(), S.bold(), S.fixed(), S.italics(), S.small(), S.strike(), S.sub(), S.sup()].join(" ")));
console.log("annexb-anchor=" + show(S.anchor("n")));
console.log("annexb-link=" + show(S.link("u")));
console.log("annexb-fontcolor=" + show(S.fontcolor("red")) + " fontsize=" + show(S.fontsize(3)));
console.log("annexb-quote-in-value=" + show(S.anchor('a"b')));
console.log("annexb-quote-twice=" + show(S.anchor('"a"')));
console.log("annexb-lt-in-value=" + show(S.link("<b>")));
console.log("annexb-amp-in-value=" + show(S.link("a&b")));
console.log("annexb-single-quote=" + show(S.link("a'b")));
console.log("annexb-newline-value=" + show(S.link("a\nb")));
console.log("annexb-quote-in-body=" + show(('a"b' as any).anchor("n")));
console.log("annexb-lt-in-body=" + show(("<b>" as any).bold()));
console.log("annexb-null-value=" + show(S.link(null)) + " undef=" + show(S.link(undefined)) + " noarg=" + show(S.link()));
console.log("annexb-object-value=" + show(S.link({ toString: () => "o" })));
console.log("annexb-array-value=" + show(S.link(["a", "b"])));
console.log("annexb-symbol-value=" + attempt(() => S.link(Symbol("s"))));
console.log("annexb-throwing-value=" + attempt(() => S.link({ toString: () => { throw new RangeError("x"); } })));
console.log("annexb-num-receiver=" + show(String.prototype.big.call(12)));
console.log("annexb-num-value=" + show(S.fontsize(12)));
console.log("annexb-null-this=" + attempt(() => String.prototype.big.call(null as any)));
console.log("annexb-undefined-this=" + attempt(() => String.prototype.anchor.call(undefined as any, "n")));
console.log("annexb-boxed-this=" + show(String.prototype.bold.call(new String("y") as any)));
console.log("annexb-array-this=" + show(String.prototype.italics.call(["a", "b"] as any)));
const order: string[] = [];
console.log("annexb-order-result=" + attempt(() => String.prototype.link.call(
  { toString: () => { order.push("this"); return "b"; } } as any,
  { toString: () => { order.push("value"); return "v"; } } as any,
)));
console.log("annexb-order=" + order.join(","));
const htmlNames: string[] = ["big", "blink", "bold", "fixed", "italics", "small", "strike", "sub", "sup", "anchor", "link", "fontcolor", "fontsize"];
console.log("annexb-shapes=" + htmlNames.map((n) => {
  const f: any = (String.prototype as any)[n];
  return n + ":" + f.length + ":" + f.name;
}).join(" "));
const annexDesc: any = Object.getOwnPropertyDescriptor(String.prototype, "anchor");
console.log("annexb-desc=" + annexDesc.writable + "/" + annexDesc.enumerable + "/" + annexDesc.configurable);
console.log("annexb-nested=" + show((S.bold() as string).italics()) + " len=" + S.anchor("n").length);
console.log("annexb-astral-body=" + ("\u{1F600}" as any).bold().length);
