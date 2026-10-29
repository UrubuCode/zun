// Cross-runtime: the platform helper surface. Sections: numeric and URI globals,
// Object helpers and property order, the constructor roster (real constructors,
// prototype methods, subclassing), web-platform instances (URL, Headers, Blob,
// TextEncoder, AbortController), crypto.getRandomValues and randomUUID contracts
// (only their deterministic properties), and idioms that bundlers and
// obfuscators emit. Only outputs that are the same on every conforming runtime
// are printed.

function t(f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name;
  }
}

// ---------------------------------------------------------------------------
// 1. Numeric and URI globals.
// ---------------------------------------------------------------------------
console.log("parseInt_hex=" + parseInt("0x1F") + " hex_radix16=" + parseInt("1F", 16) + " radix10_of_08=" + parseInt("08"));
console.log("parseInt_trailing=" + parseInt("  12px") + " leading_sign=" + parseInt("-42") + " plus=" + parseInt("+7"));
console.log("parseInt_empty=" + parseInt("") + " junk=" + parseInt("abc") + " radix_bad=" + parseInt("10", 1));
console.log("parseInt_float_cut=" + parseInt("3.99") + " big=" + parseInt("9007199254740993"));
console.log("parseFloat=" + parseFloat("3.14abc") + " leading_dot=" + parseFloat(".5e2") + " neg_inf=" + parseFloat("-Infinity"));
console.log("parseFloat_junk=" + parseFloat("x1") + " spaces=" + parseFloat("   2.5  ") + " hex_not_taken=" + parseFloat("0x10"));
console.log("isNaN_global=" + isNaN("abc") + " vs_Number=" + Number.isNaN("abc") + " undefined=" + isNaN(undefined as any));
console.log("isFinite_global=" + isFinite("12") + " vs_Number=" + Number.isFinite("12") + " inf=" + isFinite(Infinity));
console.log("encodeURIComponent=" + encodeURIComponent("a b&c/é"));
console.log("encodeURI=" + encodeURI("http://x.y/a b?q=1&r=é#h"));
console.log("encodeURI_reserved=" + encodeURI(";,/?:@&=+$#") + " component_reserved=" + encodeURIComponent(";,/?:@&=+$#"));
console.log("decodeURIComponent=" + decodeURIComponent("%E2%82%AC") + " ascii=" + decodeURIComponent("a%20b"));
console.log("decodeURI_keeps_reserved=" + decodeURI("%2F%20") + " component_decodes=" + decodeURIComponent("%2F%20"));
console.log("roundtrip_ok=" + (decodeURIComponent(encodeURIComponent("ü/ß?#&=✓")) === "ü/ß?#&=✓"));
console.log("malformed_uri=" + t(function () { return decodeURIComponent("%E0%A4%A"); }));
console.log("lone_surrogate=" + t(function () { return encodeURIComponent("\uD800"); }));
console.log("number_coercion=" + Number("") + "," + Number(" 12 ") + "," + Number("12px") + "," + Number(null) + "," + Number(undefined) + "," + Number([5]));
console.log("int_conversions=" + (~~-3.9) + "," + (2 ** 32 + 5 | 0) + "," + (-1 >>> 0) + "," + (7.9 >> 0));

// ---------------------------------------------------------------------------
// 2. Object helpers and property order.
// ---------------------------------------------------------------------------
console.log("hasOwn=" + Object.hasOwn({ a: 1 }, "a") + "," + Object.hasOwn({ a: 1 }, "toString"));
console.log("hasOwn_array=" + Object.hasOwn([1], "length") + "," + Object.hasOwn([1], 0));
console.log("hasOwn_null_proto=" + Object.hasOwn(Object.create(null), "x"));
console.log("URL.canParse=" + URL.canParse("/x", "https://example.com") + "," + URL.canParse("not a url") + "," + URL.canParse("https://ok.example/"));

const sym = Symbol("z");
const ordered: any = {};
ordered["20"] = "twenty";
ordered["3"] = "three";
ordered["aa"] = "aa";
ordered["10"] = "ten";
ordered["b"] = "b";
ordered[sym] = "sym";
console.log("keys=" + Object.keys(ordered).join(","));
console.log("names=" + Object.getOwnPropertyNames(ordered).join(","));
console.log("symbols=" + Object.getOwnPropertySymbols(ordered).length);
console.log("json=" + JSON.stringify(ordered));
console.log("for_in=" + (function (): string {
  const seen: string[] = [];
  for (const k in ordered) seen.push(k);
  return seen.join(",");
})());
console.log("negative_and_float_keys=" + Object.keys({ "-1": 1, "1.5": 2, "4294967295": 3, "0": 4, "01": 5 }).join(","));
console.log("delete_then_readd=" + (function (): string {
  const o: any = { a: 1, b: 2, c: 3 };
  delete o.a;
  o.a = 4;
  return Object.keys(o).join(",");
})());
console.log("assign_order_retained=" + (function (): string {
  const o: any = { x: 1, y: 2 };
  o.x = 9;
  return Object.keys(o).join(",") + "=" + o.x;
})());

// ---------------------------------------------------------------------------
// 3. The constructor roster: real constructors with wired prototypes.
// ---------------------------------------------------------------------------
const roster: string[] = [
  "URL", "URLSearchParams", "TextEncoder", "TextDecoder", "Blob", "Headers", "Request", "Response",
  "FormData", "AbortController", "AbortSignal", "DOMException", "Event", "EventTarget",
  "MessageChannel", "MessagePort", "Promise", "Map", "Set", "WeakMap", "WeakSet", "WeakRef",
  "FinalizationRegistry", "ArrayBuffer", "SharedArrayBuffer", "DataView", "BigInt64Array",
];
for (const name of roster) {
  const C: any = (globalThis as any)[name];
  if (typeof C !== "function") {
    console.log("class[" + name + "]=" + typeof C);
    continue;
  }
  const wired = C.prototype !== undefined && C.prototype !== null && C.prototype.constructor === C;
  let withoutNew = "accepted";
  try {
    C();
  } catch (e: any) {
    withoutNew = e.constructor.name;
  }
  console.log("class[" + name + "]=name:" + C.name + " wired:" + wired + " noNew:" + withoutNew + " protoIsObject:" + (typeof C.prototype === "object"));
}
console.log("proxy_no_prototype=" + (Proxy.prototype === undefined) + " name=" + Proxy.name + " len=" + Proxy.length);
console.log("proxy_instanceof=" + t(function () { return ({} as any) instanceof Proxy; }));
console.log("arity=" + ["URL", "URLSearchParams", "TextEncoder", "TextDecoder", "Blob", "Headers", "Response", "AbortController", "DOMException", "Event", "EventTarget", "Promise", "Map", "Set", "WeakRef", "ArrayBuffer", "DataView"].map(function (n) {
  return n + ":" + (globalThis as any)[n].length;
}).join(" "));
console.log("no_custom_hasInstance=" + roster.filter(function (n) {
  const C: any = (globalThis as any)[n];
  return typeof C === "function" && Object.prototype.hasOwnProperty.call(C, Symbol.hasInstance);
}).length);

// A prototype method lives on the PROTOTYPE, never as an own property of an instance.
const owned: Array<[string, any, string]> = [
  ["URL", new URL("https://e.example/"), "toString"],
  ["URLSearchParams", new URLSearchParams("a=1"), "append"],
  ["Headers", new Headers(), "append"],
  ["Blob", new Blob(), "slice"],
  ["TextEncoder", new TextEncoder(), "encode"],
  ["TextDecoder", new TextDecoder(), "decode"],
  ["Map", new Map(), "get"],
  ["AbortController", new AbortController(), "abort"],
];
for (const o of owned) {
  const instance: any = o[1];
  const key = o[2];
  console.log("method[" + o[0] + "." + key + "]=own:" + Object.prototype.hasOwnProperty.call(instance, key) + " onProto:" + Object.prototype.hasOwnProperty.call(Object.getPrototypeOf(instance), key) + " callable:" + (typeof instance[key] === "function"));
}

const instances: Array<[string, any]> = [
  ["URL", new URL("https://e.example/")],
  ["URLSearchParams", new URLSearchParams("a=1")],
  ["Headers", new Headers({ a: "1" })],
  ["Blob", new Blob(["x"])],
  ["Response", new Response()],
  ["AbortController", new AbortController()],
  ["DOMException", new DOMException("m", "AbortError")],
  ["EventTarget", new EventTarget()],
];
for (const i of instances) {
  const v: any = i[1];
  const C: any = (globalThis as any)[i[0]];
  console.log("instance[" + i[0] + "]=is:" + (v instanceof C) + " ctor:" + (v.constructor === C) + " ownKeys:" + Object.keys(v).length);
}

// ---------------------------------------------------------------------------
// 4. Web-platform instances and subclassing.
// ---------------------------------------------------------------------------
const u = new URL("https://user:pw@e.example:8080/a/b?x=1&y=2#frag");
console.log("url_parts=" + u.protocol + "|" + u.hostname + "|" + u.port + "|" + u.pathname + "|" + u.search + "|" + u.hash);
console.log("url_origin=" + u.origin + " username=" + u.username);
console.log("url_relative=" + new URL("../c?z", "https://e.example/a/b/").href);
console.log("url_invalid=" + t(function () { return new URL("nope"); }));
console.log("url_searchparams=" + u.searchParams.get("y") + " " + [...u.searchParams.keys()].join(","));
const params = new URLSearchParams("a=1&b=2&a=3");
console.log("usp_getall=" + params.getAll("a").join(",") + " has=" + params.has("b") + " size=" + params.size);
params.append("c", "x y");
params.sort();
console.log("usp_serialized=" + params.toString());
console.log("usp_encoding=" + new URLSearchParams({ q: "a&b=c" }).toString());

const headers = new Headers({ "Content-Type": "text/plain" });
headers.append("X-Multi", "1");
headers.append("x-multi", "2");
console.log("headers_get=" + headers.get("content-type") + " multi=" + headers.get("X-MULTI") + " missing=" + headers.get("nope"));
console.log("headers_has=" + headers.has("CONTENT-TYPE") + " sorted=" + [...headers.keys()].join(","));

const blob = new Blob(["abc", new Uint8Array([100, 101])]);
console.log("blob_size=" + blob.size + " type=" + JSON.stringify(blob.type));
console.log("blob_slice_size=" + blob.slice(1, 4).size);

const enc = new TextEncoder();
const bytes = enc.encode("é€");
console.log("encode=" + Array.from(bytes).join(",") + " encoding=" + enc.encoding);
console.log("decode=" + new TextDecoder().decode(bytes) + " len=" + new TextDecoder().decode(bytes).length);
console.log("decode_replacement=" + JSON.stringify(new TextDecoder().decode(new Uint8Array([0xff, 0x41]))));

const ac = new AbortController();
let abortCount = 0;
ac.signal.addEventListener("abort", function () {
  abortCount++;
});
console.log("signal_before=" + ac.signal.aborted);
ac.abort();
ac.abort();
console.log("signal_after=" + ac.signal.aborted + " fired=" + abortCount);

const et = new EventTarget();
let seen = "";
et.addEventListener("ping", function (e: any) {
  seen += "a:" + e.type;
});
et.dispatchEvent(new Event("ping"));
console.log("event_dispatch=" + seen);

class TaggedURL extends URL {
  tag = "t";
}
const sub = new TaggedURL("https://e.example/p");
console.log("subclass_url=" + sub.pathname + " tag=" + sub.tag + " isURL=" + (sub instanceof URL) + " isSub=" + (sub instanceof TaggedURL));
console.log("subclass_proto=" + (Object.getPrototypeOf(TaggedURL.prototype) === URL.prototype) + " ctor_chain=" + (Object.getPrototypeOf(TaggedURL) === URL));
class TaggedHeaders extends Headers {}
const subHeaders = new TaggedHeaders({ a: "1" });
console.log("subclass_headers=" + subHeaders.get("a") + " isHeaders=" + (subHeaders instanceof Headers) + " ctorName=" + subHeaders.constructor.name);
const viaReflect: any = Reflect.construct(URL, ["https://e.example/z"], TaggedURL);
console.log("reflect_construct=" + viaReflect.pathname + " isSub=" + (viaReflect instanceof TaggedURL) + " hasField=" + ("tag" in viaReflect));

// ---------------------------------------------------------------------------
// 5. crypto: deterministic contract only (values are never printed).
// ---------------------------------------------------------------------------
const c: any = (globalThis as any).crypto;
console.log("crypto_global=" + typeof c + " tag=" + Object.prototype.toString.call(c));
console.log("crypto_members=" + ["getRandomValues", "randomUUID", "subtle"].map(function (k) { return k + ":" + typeof c[k]; }).join(" "));
console.log("crypto_methods_not_constructible=" + t(function () { return new (c.getRandomValues as any)(new Uint8Array(1)); }));
console.log("returns_same_object=" + (function (): string {
  const a = new Uint8Array(8);
  return String(c.getRandomValues(a) === a) + "/" + a.length;
})());
console.log("empty_array=" + (function (): string {
  const a = new Uint8Array(0);
  return String(c.getRandomValues(a) === a) + "/" + a.length;
})());
console.log("max_size=" + c.getRandomValues(new Uint8Array(65536)).length);

const integerKinds = ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array", "Uint32Array"];
for (const name of integerKinds) {
  console.log("kind_" + name + "=" + t(function () {
    const ctor: any = (globalThis as any)[name];
    const a = ctor.from(new ctor(64));
    c.getRandomValues(a);
    const values: number[] = Array.from(a);
    const bits = ctor.BYTES_PER_ELEMENT * 8;
    const signed = name.charAt(0) === "I";
    const lo = signed ? -Math.pow(2, bits - 1) : 0;
    const hi = signed ? Math.pow(2, bits - 1) - 1 : Math.pow(2, bits) - 1;
    const inRange = values.every(function (v) { return v >= lo && v <= hi; });
    return "inRange:" + inRange + " integers:" + values.every(function (v) { return Number.isInteger(v); });
  }));
}
console.log("bigint_kinds=" + ["BigInt64Array", "BigUint64Array"].map(function (name) {
  return name + ":" + t(function () {
    const ctor: any = (globalThis as any)[name];
    const a = new ctor(4);
    c.getRandomValues(a);
    return String(Array.from(a).every(function (v: any) { return typeof v === "bigint"; }));
  });
}).join(" "));
console.log("plain_array=" + t(function () { return c.getRandomValues([1, 2, 3]); }));
console.log("string_arg=" + t(function () { return c.getRandomValues("abc"); }));
console.log("object_arg=" + t(function () { return c.getRandomValues({ length: 4 }); }));
console.log("respects_window=" + (function (): string {
  const backing = new Uint8Array(16);
  const win = backing.subarray(4, 8);
  c.getRandomValues(win);
  const before = Array.from(backing.subarray(0, 4)).join(",");
  const after = Array.from(backing.subarray(8)).join(",");
  return "before:" + (before === "0,0,0,0") + " after:" + (after === "0,0,0,0,0,0,0,0") + " len:" + win.length;
})());
console.log("detached=" + t(function () {
  const buf = new ArrayBuffer(8);
  const a = new Uint8Array(buf);
  buf.transfer();
  return c.getRandomValues(a).length;
}));

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const sampleUuid: string = c.randomUUID();
console.log("uuid_shape=" + sampleUuid.length + "/" + uuidPattern.test(sampleUuid) + "/" + typeof sampleUuid + "/" + sampleUuid.charAt(14));
console.log("uuid_hyphens=" + sampleUuid.split("-").map(function (p) { return String(p.length); }).join(","));
console.log("uuid_lowercase=" + (sampleUuid === sampleUuid.toLowerCase()));
const uuidSeen = new Set<string>();
let allValid = true;
for (let i = 0; i < 16; i++) {
  const next: string = c.randomUUID();
  uuidSeen.add(next);
  if (!uuidPattern.test(next)) allValid = false;
}
console.log("uuid_unique=" + uuidSeen.size + " all_valid=" + allValid);

// ---------------------------------------------------------------------------
// 6. Idioms that bundlers, transpilers and obfuscators emit.
// ---------------------------------------------------------------------------
// tsc __assign / __rest / __spreadArray.
function __assign(target: any, ...sources: any[]) {
  for (const src of sources) {
    for (const key of Object.keys(src)) target[key] = src[key];
  }
  return target;
}
const merged = __assign({}, { a: 1 }, { b: 2 }, { a: 99 });
console.log("assign_helper=" + merged.a + "," + merged.b);

function __rest(source: any, excluded: string[]) {
  const result: any = {};
  for (const key of Object.keys(source)) {
    if (!excluded.includes(key)) result[key] = source[key];
  }
  return result;
}
const restSrc = { a: 1, b: 2, c: 3 };
console.log("rest_helper=" + Object.keys(__rest(restSrc, ["a"])).join(",") + " destructured=" + (function () {
  const { a: _a, ...rest } = restSrc;
  return Object.keys(rest).join(",");
})());

function __spreadArray(to: any[], from: any[]) {
  return to.concat(Array.prototype.slice.call(from));
}
console.log("spread_helper=" + __spreadArray([1, 2], [3, 4]).join(","));

// tsc __extends: prototype wiring through a helper constructor.
function __extends(d: any, b: any) {
  Object.setPrototypeOf(d, b);
  function Ctor(this: any) { this.constructor = d; }
  Ctor.prototype = b.prototype;
  d.prototype = new (Ctor as any)();
}
function Base2(this: any) { this.type = "base"; }
(Base2 as any).prototype.greet = function () { return "hello from " + this.type; };
function Derived(this: any) { Base2.call(this); this.type = "derived"; }
__extends(Derived, Base2);
const inst: any = new (Derived as any)();
console.log("extends_helper=" + inst.greet() + " instanceof=" + (inst instanceof Derived));

// UMD-style guards and feature flags folded at build time.
const IS_PROD = true;
if (IS_PROD) {
  console.log("dce_prod_branch=taken");
} else {
  console.log("dce_dev_branch=taken");
}
const FEATURE_X = false;
FEATURE_X && console.log("feature_x=enabled");
!FEATURE_X && console.log("feature_x=disabled");
const __DEV__ = false;
function warn(msg: string) {
  if (__DEV__) console.log("[warn] " + msg);
}
warn("never printed");
console.log("typeof_guard=" + (typeof globalThis !== "undefined"));

// CommonJS-to-ESM and namespace shims.
const moduleExports: Record<string, any> = {};
(function (exports: Record<string, any>) {
  exports.add = (a: number, b: number) => a + b;
  exports.mul = (a: number, b: number) => a * b;
  exports.PI = 3.14159;
})(moduleExports);
console.log("cjs_exports=" + moduleExports.add(2, 3) + "," + moduleExports.mul(4, 5) + "," + moduleExports.PI);
function __toESM(mod: any) {
  return Object.assign(Object.create(null), mod, { default: mod });
}
const ns = __toESM({ foo: 1, bar: 2 });
console.log("namespace_shim=" + ns.foo + "," + ns.default.foo + " proto_null=" + (Object.getPrototypeOf(ns) === null));
const lazy: any = {};
let loaded = false;
let value = 0;
Object.defineProperty(lazy, "value", {
  get() {
    if (!loaded) { loaded = true; value = 100; }
    return value;
  },
});
console.log("lazy_getter=" + lazy.value + "," + lazy.value);
const exportsObj: any = {};
Object.defineProperty(exportsObj, "__esModule", { value: true });
exportsObj.hello = "world";
console.log("esmodule_flag=" + exportsObj.__esModule + " " + exportsObj.hello);
const interop = { default: 42, __esModule: true };
console.log("default_interop=" + (interop.__esModule ? interop.default : interop));

// Obfuscator output: XOR decoding, rolling keys, charcode and escape decoding.
function xorDecode(encoded: number[], key: number): string {
  return encoded.map(function (c) { return String.fromCharCode(c ^ key); }).join("");
}
console.log("xor_hello=" + xorDecode([0x2a, 0x27, 0x2e, 0x2e, 0x2d], 0x42));
console.log("xor_world=" + xorDecode([0x62, 0x6a, 0x7f, 0x79, 0x69], 0x15));
function rollingXor(encoded: number[], keys: number[]): string {
  return encoded.map(function (c, i) { return String.fromCharCode(c ^ keys[i % keys.length]); }).join("");
}
console.log("xor_rolling=" + rollingXor([0x7e, 0x46, 0x57, 0x74, 0x47, 0x56, 0x50, 0x43, 0x57, 0x75], [0x11, 0x22, 0x33]));
console.log("charcodes=" + [74, 97, 118, 97, 83, 99, 114, 105, 112, 116].map(function (c) { return String.fromCharCode(c); }).join(""));
console.log("hex_escape=\x68\x65\x6c\x6c\x6f\x20\x52\x54\x53");
console.log("unicode_escape=RTS runs");

// Obfuscator dispatch tables keyed by escaped method names.
type StrOp = (s: string, ...args: any[]) => any;
const sops: Record<string, StrOp> = {
  "\x73\x70": function (s) { return s.split(""); },
  "\x6a\x6e": function (s, sep) { return s.split("").join(sep); },
  "\x72\x76": function (s) { return s.split("").reverse().join(""); },
  "\x75\x70": function (s) { return s.toUpperCase(); },
  "\x6c\x6f": function (s) { return s.toLowerCase(); },
};
console.log("dispatch_rv=" + sops["\x72\x76"]("hello") + " up=" + sops["\x75\x70"]("world") + " jn=" + sops["\x6a\x6e"]("abc", "-") + " lo=" + sops["\x6c\x6f"]("ABC"));

// Number-literal obfuscation: ~~, bit tricks and arithmetic encodings.
const zero = 1 - 1;
const one = !!1 | 0;
const two = one + one;
const ten = one + one + one + one + one + one + one + one + one + one;
console.log("encoded_numbers=" + zero + "," + one + "," + two + "," + ten);
console.log("bit_tricks=" + (~~42) + "," + (~~3.9) + "," + (~~-3.9) + "," + (42 ^ 0) + "," + (42 ^ 42) + "," + (0 ^ 99));
console.log("shifts=" + (1 << 8) + "," + (1024 >> 2) + "," + (7 << 3) + "," + (100 >>> 1));
const packed = 0xDEADBEEF;
console.log("bytes=" + (packed & 0xFF).toString(16) + "," + ((packed >> 8) & 0xFF).toString(16) + "," + ((packed >> 16) & 0xFF).toString(16) + "," + ((packed >>> 24) & 0xFF).toString(16));

// Opcode dispatcher with outlined functions.
type Op = (...args: number[]) => number;
const ops: Record<number, Op> = {
  0x01: function (a, b) { return a + b; },
  0x02: function (a, b) { return a - b; },
  0x03: function (a, b) { return a * b; },
  0x04: function (a, b) { return a / b; },
  0x06: function (a, b) { return a ** b; },
  0x07: function (a) { return -a; },
  0x08: function (a, b) { return a > b ? a : b; },
  0x0A: function (a) { return Math.abs(a); },
};
function exec(opcode: number, ...args: number[]): number {
  return ops[opcode](...args);
}
console.log("dispatch_expr=" + exec(0x04, exec(0x03, exec(0x01, 3, 4), exec(0x02, 10, 2)), 4));
console.log("dispatch_pow=" + exec(0x06, 2, 10) + " neg=" + exec(0x07, -42) + " max=" + exec(0x08, 3, 7) + " abs=" + exec(0x0A, -99));

// Dead-code injection: opaque predicates and branches that never execute.
function compute(x: number): number {
  if (typeof x === "undefined" && x > 0) {
    return -999;
  }
  const doubled = x * 2;
  if (false && doubled < 0) {
    throw new Error("impossible");
  }
  if ((0).toString() !== "0") {
    return -1;
  }
  return doubled + 1;
}
console.log("dead_inject=" + compute(5) + "," + compute(0) + "," + compute(-3));
console.log("opaque=" + (function (n: number) { return n * n >= 0; })(5) + "," + (function (n: number) { return n * n >= 0; })(-5));
let guard = 0;
const neverRun = false;
while (neverRun) {
  guard++;
  if (guard > 1e9) break;
}
console.log("guard=" + guard);
function safe(n: number): number {
  if (typeof n !== "number") return safe(0);
  return n + 1;
}
console.log("safe_recursion=" + safe(41));
