"use strict";
// The sites where the engine itself raises TypeError, RangeError, URIError,
// SyntaxError and ReferenceError: argument ranges, URI coding, JSON, instanceof and
// in operands, non-constructible callables, collection and wrapper constructors,
// Array callbacks, TDZ bindings, throwing getters, toPrimitive and iterators,
// class-definition failures, frozen and sealed writes, and optional-chain
// boundaries. Only the constructor name (or ok:value) is printed, plus booleans
// that confirm instanceof of Error. Built-in messages are never printed.

function probe(fn: () => any): string {
  try {
    const v = fn();
    return "ok:" + String(v);
  } catch (e: any) {
    return e.constructor.name;
  }
}
function kind(fn: () => any): string {
  try {
    fn();
    return "no-throw";
  } catch (e: any) {
    return e.constructor.name + ":" + String(e instanceof Error);
  }
}

// 1. RangeError sites named by the specification, and the boundary values that must not throw.
{
  console.log("array-neg=" + probe(() => new Array(-1)));
  console.log("array-frac=" + probe(() => new Array(1.5)));
  console.log("array-huge=" + probe(() => new Array(4294967296)));
  console.log("array-max-ok=" + probe(() => new Array(4294967295).length));
  console.log("array-zero-ok=" + probe(() => new Array(0).length));
  console.log("array-string-ok=" + probe(() => new Array("3" as any).length));
  console.log("array-two-args-ok=" + probe(() => new Array(1, 2).length));
  console.log("length-neg=" + probe(() => { const a: any = []; a.length = -1; return a.length; }));
  console.log("length-frac=" + probe(() => { const a: any = []; a.length = 1.5; return a.length; }));
  console.log("length-nan=" + probe(() => { const a: any = []; a.length = NaN; return a.length; }));
  console.log("length-ok=" + probe(() => { const a: any = []; a.length = 3; return a.length; }));

  console.log("tofixed-101=" + probe(() => (1).toFixed(101)));
  console.log("tofixed-neg=" + probe(() => (1).toFixed(-1)));
  console.log("tofixed-100-ok=" + probe(() => (1).toFixed(100).length));
  console.log("tofixed-0-ok=" + probe(() => (1.6).toFixed(0)));
  console.log("toprecision-0=" + probe(() => (1).toPrecision(0)));
  console.log("toprecision-102=" + probe(() => (1).toPrecision(102)));
  console.log("toprecision-1-ok=" + probe(() => (123).toPrecision(1)));
  console.log("toprecision-100-ok=" + probe(() => (1).toPrecision(100).length));
  console.log("toprecision-undefined-ok=" + probe(() => (1.5).toPrecision(undefined)));
  console.log("toexponential-101=" + probe(() => (1).toExponential(101)));
  console.log("toexponential-neg=" + probe(() => (1).toExponential(-1)));
  console.log("toexponential-2-ok=" + probe(() => (1234).toExponential(2)));
  console.log("tostring-37=" + probe(() => (255).toString(37)));
  console.log("tostring-1=" + probe(() => (255).toString(1)));
  console.log("tostring-0=" + probe(() => (255).toString(0)));
  console.log("tostring-frac=" + probe(() => (255).toString(2.5)));
  console.log("tostring-2-ok=" + probe(() => (255).toString(2)));
  console.log("tostring-36-ok=" + probe(() => (255).toString(36)));
  console.log("repeat-neg=" + probe(() => "ab".repeat(-1)));
  console.log("repeat-inf=" + probe(() => "ab".repeat(Infinity)));
  console.log("repeat-0-ok=" + probe(() => JSON.stringify("ab".repeat(0))));
  console.log("repeat-frac-ok=" + probe(() => "ab".repeat(2.9)));
  console.log("padstart-ok=" + probe(() => "a".padStart(4, "-")));
  console.log("normalize-bad=" + probe(() => "a".normalize("NFX" as any)));
  console.log("normalize-ok=" + probe(() => "a".normalize("NFC")));
  console.log("fromcodepoint-neg=" + probe(() => String.fromCodePoint(-1)));
  console.log("fromcodepoint-frac=" + probe(() => String.fromCodePoint(1.5)));
  console.log("fromcodepoint-huge=" + probe(() => String.fromCodePoint(0x110000)));
  console.log("fromcodepoint-ok=" + probe(() => String.fromCodePoint(65)));
  console.log("bigint-frac=" + probe(() => BigInt(1.5)));
  console.log("bigint-nan=" + probe(() => BigInt(NaN)));
  console.log("bigint-int-ok=" + probe(() => BigInt(3)));
  console.log("bigint-asuintn-neg=" + probe(() => BigInt.asUintN(-1, 1n)));
  console.log("typedarray-neg=" + probe(() => new Uint8Array(-1)));
  console.log("typedarray-frac=" + probe(() => new Uint8Array(1.5).length));
  console.log("typedarray-ok=" + probe(() => new Uint8Array(2).length));
  console.log("arraybuffer-neg=" + probe(() => new ArrayBuffer(-1)));
  console.log("dataview-offset=" + probe(() => new DataView(new ArrayBuffer(4), 8)));
  console.log("dataview-ok=" + probe(() => new DataView(new ArrayBuffer(4), 2).byteLength));
  console.log("date-invalid-toiso=" + probe(() => new Date(NaN).toISOString()));
  console.log("date-valid-toiso-ok=" + probe(() => new Date(0).toISOString()));
  console.log("range-kind-tofixed=" + kind(() => (1).toFixed(101)));
  console.log("range-kind-radix=" + kind(() => (1).toString(37)));
  console.log("range-kind-repeat=" + kind(() => "a".repeat(-1)));
  console.log("range-kind-array=" + kind(() => new Array(-1)));
}

// 2. URIError: malformed escapes and lone surrogates; successful reserved-character splits.
{
  console.log("decodeuric-percent=" + kind(() => decodeURIComponent("%")));
  console.log("decodeuric-short=" + kind(() => decodeURIComponent("%2")));
  console.log("decodeuric-nonhex=" + kind(() => decodeURIComponent("%zz")));
  console.log("decodeuric-bad-utf8=" + kind(() => decodeURIComponent("%C0%80")));
  console.log("decodeuric-lone-cont=" + kind(() => decodeURIComponent("%80")));
  console.log("decodeuric-truncated=" + kind(() => decodeURIComponent("%E2%82")));
  console.log("decodeuri-nonhex=" + kind(() => decodeURI("%GG")));
  console.log("encodeuric-lone-high=" + kind(() => encodeURIComponent("\uD800")));
  console.log("encodeuric-lone-low=" + kind(() => encodeURIComponent("\uDC00")));
  console.log("encodeuric-reversed=" + kind(() => encodeURIComponent("\uDC00\uD800")));
  console.log("encodeuri-lone-high=" + kind(() => encodeURI("\uD800")));
  console.log("encodeuric-pair=" + probe(() => encodeURIComponent(String.fromCodePoint(0x1f600))));
  console.log("roundtrip=" + probe(() => decodeURIComponent(encodeURIComponent(String.fromCodePoint(0x1f600))).length));
  console.log("euric-reserved=" + encodeURIComponent(";/?:@&=+$,#"));
  console.log("euri-reserved=" + encodeURI(";/?:@&=+$,#"));
  console.log("euric-unreserved=" + encodeURIComponent("-_.!~*'()"));
  console.log("euric-space=" + encodeURIComponent(" ") + ",euri-space=" + encodeURI(" "));
  console.log("duric-reserved=" + decodeURIComponent("%3B%2F%3F%3A%40%26%3D%2B%24%2C%23"));
  console.log("duri-reserved=" + decodeURI("%3B%2F%3F%3A%40%26%3D%2B%24%2C%23"));
  console.log("unescape-percent=" + JSON.stringify(unescape("%")) + ",escape-lone=" + escape("\uD800"));
  console.log("coerce-number=" + probe(() => encodeURIComponent(1 as any)));
  console.log("coerce-symbol=" + probe(() => encodeURIComponent(Symbol("s") as any)));
  console.log("urierror-name=" + URIError.prototype.name + ",own=" + Object.prototype.hasOwnProperty.call(new URIError("x"), "name"));
  console.log("urierror-tostring=" + new URIError("mine").toString());
}

// 3. JSON: SyntaxError from the parser, TypeError from cycles and BigInt, user errors pass through.
{
  console.log("parse-empty=" + probe(() => JSON.parse("")));
  console.log("parse-trailing-comma=" + probe(() => JSON.parse("[1,]")));
  console.log("parse-single-quote=" + probe(() => JSON.parse("'a'")));
  console.log("parse-unquoted-key=" + probe(() => JSON.parse("{a:1}")));
  console.log("parse-undefined=" + probe(() => JSON.parse("undefined")));
  console.log("parse-nan=" + probe(() => JSON.parse("NaN")));
  console.log("parse-hex=" + probe(() => JSON.parse("0x10")));
  console.log("parse-comment=" + probe(() => JSON.parse("1 // tail")));
  console.log("parse-truncated=" + probe(() => JSON.parse('{"a":')));
  console.log("parse-bad-escape=" + probe(() => JSON.parse('"\\x41"')));
  console.log("parse-lone-surrogate-ok=" + probe(() => JSON.parse('"\\ud800"').length));
  console.log("parse-whitespace-ok=" + probe(() => JSON.parse(" \t\r\n1 ")));
  console.log("parse-undefined-arg=" + probe(() => JSON.parse(undefined as any)));

  let parsed: any = null;
  try {
    JSON.parse("{");
  } catch (e: any) {
    parsed = e;
  }
  console.log("syntax-instanceof=" + (parsed instanceof SyntaxError) + "," + (parsed instanceof Error) + ",name=" + parsed.name);

  const direct: any = {};
  direct.self = direct;
  console.log("cycle-direct=" + probe(() => JSON.stringify(direct)));
  const a: any = { name: "a" };
  const b: any = { name: "b", a: a };
  a.b = b;
  console.log("cycle-indirect=" + probe(() => JSON.stringify(a)));
  const arr: any[] = [1];
  arr.push(arr);
  console.log("cycle-array=" + probe(() => JSON.stringify(arr)));
  const shared: any = { v: 1 };
  console.log("shared-twice=" + probe(() => JSON.stringify({ x: shared, y: shared })));
  console.log("bigint-value=" + probe(() => JSON.stringify(1n as any)));
  console.log("bigint-nested=" + probe(() => JSON.stringify({ v: 2n } as any)));
  console.log("bigint-replaced=" + probe(() => JSON.stringify({ v: 3n } as any, (_k: string, x: any) => (typeof x === "bigint" ? String(x) : x))));

  const marker = new RangeError("mine-getter");
  const throwingGetter: any = {
    get boom(): any {
      throw marker;
    },
  };
  let caughtSame = "none";
  try {
    JSON.stringify(throwingGetter);
  } catch (e: any) {
    caughtSame = (e === marker) + ":" + e.constructor.name + ":" + e.message;
  }
  console.log("getter-identity=" + caughtSame);

  class BadJson {
    toJSON(): any {
      throw new EvalError("mine-tojson");
    }
  }
  console.log("tojson-throws=" + probe(() => JSON.stringify(new BadJson())));
  console.log("replacer-throws=" + probe(() => JSON.stringify({ a: 1 }, () => {
    throw new URIError("mine-replacer");
  })));
  console.log("reviver-throws=" + probe(() => JSON.parse('{"a":1}', () => {
    throw new ReferenceError("mine-reviver");
  })));
  console.log("replacer-number-ignored=" + probe(() => JSON.stringify({ a: 1 }, 5 as any)));
  console.log("reviver-number-ignored=" + probe(() => JSON.parse('{"a":1}', 5 as any).a));
  console.log("space-huge=" + probe(() => JSON.stringify({ a: 1 }, null, 100).length));
  console.log("space-negative=" + probe(() => JSON.stringify({ a: 1 }, null, -1)));
  console.log("tojson-not-callable=" + probe(() => JSON.stringify({ toJSON: 5, a: 1 })));
  console.log("symbol-value=" + String(JSON.stringify(Symbol("s") as any)));
  console.log("symbol-in-object=" + JSON.stringify({ s: Symbol("s") as any, k: 1 }));
  console.log("symbol-in-array=" + JSON.stringify([Symbol("s") as any]));
  console.log("undefined-value=" + String(JSON.stringify(undefined as any)));
}

// 4. instanceof and in: operand checks, Symbol.hasInstance, and private-name checks.
{
  class Thing {}
  const t = new Thing();
  console.log("inst-number=" + probe(() => (t as any) instanceof (1 as any)));
  console.log("inst-string=" + probe(() => (t as any) instanceof ("Thing" as any)));
  console.log("inst-null=" + probe(() => (t as any) instanceof (null as any)));
  console.log("inst-undefined=" + probe(() => (t as any) instanceof (undefined as any)));
  console.log("inst-symbol=" + probe(() => (t as any) instanceof (Symbol("s") as any)));
  console.log("inst-plain-object=" + probe(() => (t as any) instanceof ({} as any)));
  console.log("inst-array=" + probe(() => (t as any) instanceof ([] as any)));
  console.log("inst-left-number=" + probe(() => (1 as any) instanceof Thing));
  console.log("inst-left-null=" + probe(() => (null as any) instanceof Thing));
  console.log("inst-boxed-string=" + probe(() => new String("x") instanceof String));

  const acceptsNumbers: any = {
    [Symbol.hasInstance](v: any): boolean {
      return typeof v === "number";
    },
  };
  console.log("hasinstance-object=" + probe(() => (1 as any) instanceof acceptsNumbers));
  console.log("hasinstance-object-false=" + probe(() => ("1" as any) instanceof acceptsNumbers));
  console.log("hasinstance-not-callable=" + probe(() => (1 as any) instanceof ({ [Symbol.hasInstance]: 5 } as any)));
  const undefHasInstance: any = { [Symbol.hasInstance]: undefined };
  console.log("hasinstance-undefined=" + probe(() => (1 as any) instanceof undefHasInstance));
  const throwing: any = {
    [Symbol.hasInstance](): boolean {
      throw new RangeError("mine");
    },
  };
  console.log("hasinstance-throws=" + probe(() => (1 as any) instanceof throwing));

  function NoProto(): void {
    // nothing
  }
  (NoProto as any).prototype = 5;
  console.log("bad-prototype=" + probe(() => (t as any) instanceof (NoProto as any)));
  (NoProto as any).prototype = null;
  console.log("null-prototype=" + probe(() => (t as any) instanceof (NoProto as any)));

  class Branded {
    static [Symbol.hasInstance](v: any): boolean {
      return typeof v === "object" && v !== null && "brand" in v;
    }
  }
  console.log("branded-yes=" + probe(() => ({ brand: 1 } as any) instanceof Branded));
  console.log("branded-no=" + probe(() => ({} as any) instanceof Branded));

  console.log("in-number=" + probe(() => "x" in (1 as any)));
  console.log("in-string=" + probe(() => "0" in ("abc" as any)));
  console.log("in-null=" + probe(() => "x" in (null as any)));
  console.log("in-undefined=" + probe(() => "x" in (undefined as any)));
  console.log("in-boolean=" + probe(() => "x" in (true as any)));
  console.log("in-symbol=" + probe(() => "x" in (Symbol("s") as any)));
  console.log("in-boxed-string=" + probe(() => "0" in (new String("abc") as any)));
  console.log("in-own=" + probe(() => "brand" in ({ brand: 1 } as any)));
  console.log("in-inherited=" + probe(() => "toString" in ({} as any)));
  console.log("in-symbol-key=" + probe(() => Symbol.iterator in ([] as any)));
  console.log("in-array-hole=" + probe(() => 0 in ([, 1] as any)));

  class Priv {
    #secret: number = 1;
    static has(v: any): boolean {
      return #secret in v;
    }
    read(): number {
      return this.#secret;
    }
  }
  console.log("private-in-self=" + probe(() => Priv.has(new Priv())));
  console.log("private-in-plain=" + probe(() => Priv.has({})));
  console.log("private-in-primitive=" + probe(() => Priv.has(1)));
  console.log("private-in-null=" + probe(() => Priv.has(null)));
  console.log("private-read-offbrand=" + probe(() => (Priv.prototype.read as any).call({})));
}

// 5. Non-constructible callables, class calls, and the built-ins that refuse new or call.
{
  const ordinary = function (): void {
    // nothing
  };
  const arrow = (): number => 1;
  function* gen(): Generator<number, void, undefined> {
    yield 1;
  }
  async function asyncFn(): Promise<number> {
    return 1;
  }
  class Klass {}
  const holder: any = {
    shorthand(): number {
      return 1;
    },
    get accessor(): number {
      return 1;
    },
    set accessor(v: number) {
      void v;
    },
  };
  const accessorDesc: any = Object.getOwnPropertyDescriptor(holder, "accessor");

  console.log("new-ordinary=" + probe(() => new (ordinary as any)()));
  console.log("new-class=" + probe(() => new Klass()));
  console.log("new-arrow=" + probe(() => new (arrow as any)()));
  console.log("new-generator=" + probe(() => new (gen as any)()));
  console.log("new-async=" + probe(() => new (asyncFn as any)()));
  console.log("new-shorthand=" + probe(() => new (holder.shorthand as any)()));
  console.log("new-getter=" + probe(() => new (accessorDesc.get as any)()));
  console.log("new-setter=" + probe(() => new (accessorDesc.set as any)()));
  console.log("proto-ordinary=" + Object.prototype.hasOwnProperty.call(ordinary, "prototype"));
  console.log("proto-arrow=" + Object.prototype.hasOwnProperty.call(arrow, "prototype"));
  console.log("proto-shorthand=" + Object.prototype.hasOwnProperty.call(holder.shorthand, "prototype"));
  console.log("reflect-arrow=" + probe(() => Reflect.construct(arrow as any, [])));
  console.log("reflect-class=" + probe(() => Reflect.construct(Klass, [])));
  console.log("new-symbol=" + probe(() => new (Symbol as any)("x")));
  console.log("new-bigint=" + probe(() => new (BigInt as any)(1)));
  console.log("new-math-max=" + probe(() => new (Math.max as any)(1)));
  console.log("new-parseint=" + probe(() => new (parseInt as any)("1")));
  console.log("new-json-parse=" + probe(() => new (JSON.parse as any)("1")));
  console.log("new-object-keys=" + probe(() => new (Object.keys as any)({})));
  console.log("new-object-ok=" + probe(() => typeof new Object()));
  console.log("new-map-ok=" + probe(() => new Map().size));
  console.log("call-map=" + probe(() => (Map as any)()));
  console.log("call-promise=" + probe(() => (Promise as any)(() => undefined)));
  console.log("call-proxy=" + probe(() => (Proxy as any)({}, {})));
  console.log("call-weakmap=" + probe(() => (WeakMap as any)()));
  console.log("call-class=" + probe(() => (Klass as any)()));
  console.log("apply-class=" + probe(() => Reflect.apply(Klass as any, undefined, [])));
  console.log("bind-then-call=" + probe(() => (Klass as any).bind(null)()));
  console.log("bind-then-new-ok=" + probe(() => typeof new ((Klass as any).bind(null))()));
  console.log("call-undefined-method=" + probe(() => ({} as any).nope()));
  console.log("prop-of-undefined=" + probe(() => (undefined as any).x));
  console.log("prop-of-null=" + probe(() => (null as any).x));
  console.log("destructure-null=" + probe(() => { const { a } = null as any; return a; }));
  console.log("spread-null=" + probe(() => [...(null as any)]));
}

// 6. Operators on Symbol and BigInt, and the coercion failures they cause.
{
  const sym = Symbol("s");
  console.log("symbol-plus-string=" + probe(() => (sym as any) + "x"));
  console.log("symbol-template=" + probe(() => `${sym as any}`));
  console.log("symbol-number=" + probe(() => Number(sym as any)));
  console.log("symbol-string-ok=" + probe(() => String(sym)));
  console.log("bigint-plus-number=" + probe(() => (1n as any) + 1));
  console.log("bigint-number-mul=" + probe(() => (2n as any) * 3));
  console.log("bigint-compare-ok=" + probe(() => 1n < 2));
  console.log("bigint-number-ctor-ok=" + probe(() => Number(1n)));
  console.log("bigint-unary-plus=" + probe(() => +(1n as any)));
}

// 7. Built-in constructors and wrappers refusing their arguments.
{
  console.log("map-null-ok=" + probe(() => new Map(null as any).size));
  console.log("map-number=" + probe(() => new Map(5 as any)));
  console.log("map-object=" + probe(() => new Map({} as any)));
  console.log("map-string=" + probe(() => new Map("ab" as any)));
  console.log("set-null-ok=" + probe(() => new Set(null as any).size));
  console.log("set-number=" + probe(() => new Set(5 as any)));
  console.log("set-string-ok=" + probe(() => new Set("aab").size));
  console.log("set-arraylike=" + probe(() => new Set({ length: 2 } as any)));
  console.log("map-entry-number=" + probe(() => new Map([1] as any)));
  console.log("map-entry-null=" + probe(() => new Map([null] as any)));
  console.log("map-entry-string=" + probe(() => new Map(["ab"] as any)));
  console.log("map-entry-short-ok=" + probe(() => new Map([["k"]] as any).get("k")));
  console.log("map-entry-object-ok=" + probe(() => new Map([{ 0: "k", 1: "v" }] as any).get("k")));
  console.log("map-bad-iterator=" + probe(() => new Map({ [Symbol.iterator]: 5 } as any)));
  console.log("weakmap-number-key=" + probe(() => new WeakMap([[1, "v"]] as any)));
  console.log("weakmap-set-number=" + probe(() => new WeakMap().set(1 as any, "v")));
  console.log("weakmap-object-ok=" + probe(() => new WeakMap().set({}, "v").constructor.name));
  console.log("weakset-number=" + probe(() => new WeakSet().add(1 as any)));
  console.log("weakmap-has-primitive=" + probe(() => new WeakMap().has(1 as any)));
  console.log("promise-no-executor=" + probe(() => new (Promise as any)()));
  console.log("promise-number=" + probe(() => new (Promise as any)(5)));
  const executed: string[] = [];
  const pr = new Promise<number>((res) => {
    executed.push("sync");
    res(1);
  });
  console.log("promise-sync=" + executed.join(",") + ":" + (pr instanceof Promise));
  console.log("promise-throwing-executor=" + probe(() => {
    const failing = new Promise(() => {
      throw new RangeError("exec");
    });
    failing.catch(() => undefined);
    return failing instanceof Promise;
  }));
  console.log("proxy-no-args=" + probe(() => new (Proxy as any)()));
  console.log("proxy-primitive-target=" + probe(() => new Proxy(5 as any, {})));
  console.log("proxy-null-handler=" + probe(() => new Proxy({}, null as any)));
  console.log("proxy-ok=" + probe(() => typeof new Proxy({}, {})));
  console.log("dataview-object=" + probe(() => new DataView({} as any)));
  console.log("dataview-ok=" + probe(() => new DataView(new ArrayBuffer(4)).byteLength));
  console.log("typedarray-object-ok=" + probe(() => new Uint8Array({ length: 2 } as any).length));
  console.log("typedarray-null=" + probe(() => new Uint8Array(null as any).length));
  console.log("defineproperty-number=" + probe(() => Object.defineProperty(5 as any, "a", { value: 1 })));
  console.log("defineproperty-null=" + probe(() => Object.defineProperty(null as any, "x", { value: 1 })));
  console.log("defineproperty-bad-desc=" + probe(() => Object.defineProperty({}, "x", 5 as any)));
  console.log("defineproperty-both-kinds=" + probe(() => Object.defineProperty({}, "x", { value: 1, get() { return 1; } } as any)));
  console.log("defineproperties-string=" + probe(() => Object.defineProperties("s" as any, {})));
  console.log("keys-null=" + probe(() => Object.keys(null as any)));
  console.log("assign-null-target=" + probe(() => Object.assign(null as any, {})));
  console.log("getprototypeof-null=" + probe(() => Object.getPrototypeOf(null as any)));
  console.log("setprototypeof-primitive-ok=" + probe(() => Object.setPrototypeOf(5 as any, null)));
  console.log("reflect-ownkeys-number=" + probe(() => Reflect.ownKeys(5 as any)));
  console.log("reflect-get-number=" + probe(() => Reflect.get(5 as any, "a")));
  console.log("reflect-setproto-number=" + probe(() => Reflect.setPrototypeOf(5 as any, null)));
  console.log("map-get-on-set=" + probe(() => Map.prototype.get.call(new Set() as any, "a")));
  console.log("set-add-on-map=" + probe(() => Set.prototype.add.call(new Map() as any, "a")));
  console.log("date-wrong-receiver=" + probe(() => (Date.prototype.getTime as any).call({})));
  console.log("promise-then-on-object=" + probe(() => Promise.prototype.then.call({} as any, () => undefined)));
  console.log("bind-non-callable=" + probe(() => (Function.prototype.bind as any).call({}, null)));
}

// 8. Array higher-order callbacks: the check runs before any element is visited.
{
  const nums = [1, 2, 3, 4];
  console.log("map-missing=" + probe(() => (nums as any).map()));
  console.log("filter-missing=" + probe(() => (nums as any).filter()));
  console.log("reduce-missing=" + probe(() => (nums as any).reduce()));
  console.log("find-missing=" + probe(() => (nums as any).find()));
  console.log("flatmap-missing=" + probe(() => (nums as any).flatMap()));
  console.log("map-number=" + probe(() => (nums as any).map(5)));
  console.log("map-null=" + probe(() => (nums as any).map(null)));
  console.log("map-object=" + probe(() => (nums as any).map({})));
  console.log("sort-number=" + probe(() => nums.slice().sort(5 as any)));
  console.log("sort-null-ok=" + probe(() => [3, 1, 2].sort(null as any).join(",")));
  console.log("sort-undefined-ok=" + probe(() => [3, 1, 2].sort(undefined).join(",")));

  const touched: string[] = [];
  const watched: any = [1, 2];
  Object.defineProperty(watched, "0", {
    get(): number {
      touched.push("read0");
      return 1;
    },
    configurable: true,
    enumerable: true,
  });
  console.log("early-check=" + probe(() => watched.map(5)));
  console.log("early-touched=" + JSON.stringify(touched.join(",")));
  console.log("late-run=" + probe(() => watched.map((v: any) => v).join(",")));
  console.log("late-touched=" + touched.join(","));

  console.log("reduce-empty=" + probe(() => ([] as number[]).reduce((a, b) => a + b)));
  console.log("reduce-empty-init=" + probe(() => ([] as number[]).reduce((a, b) => a + b, 7)));
  console.log("reduceright-empty=" + probe(() => ([] as number[]).reduceRight((a, b) => a + b)));
  console.log("reduce-single=" + probe(() => [9].reduce((a, b) => a + b)));

  const marker = new RangeError("mine");
  const visited: number[] = [];
  let identity = "none";
  try {
    nums.map((v) => {
      visited.push(v);
      if (v === 3) throw marker;
      return v;
    });
  } catch (e: any) {
    identity = String(e === marker) + ":" + e.constructor.name + ":" + e.message;
  }
  console.log("throwing-identity=" + identity + ",visited=" + visited.join(","));
  console.log("foreach-throws=" + probe(() => nums.forEach(() => {
    throw new URIError("fe");
  })));
  console.log("sort-comparator-throws=" + probe(() => nums.slice().sort(() => {
    throw new EvalError("cmp");
  })));
  console.log("reduce-throws=" + probe(() => nums.reduce(() => {
    throw new SyntaxError("rd");
  }, 0)));
  console.log("from-bad-mapper=" + probe(() => Array.from([1, 2], 5 as any)));
  console.log("from-mapper-throws=" + probe(() => Array.from([1, 2], () => {
    throw new TypeError("fm2");
  })));
  console.log("from-null=" + probe(() => Array.from(null as any)));
  console.log("map-on-arraylike=" + probe(() => Array.prototype.map.call({ length: 2, 0: "a", 1: "b" } as any, (v: any) => v).join(",")));
  console.log("map-on-null=" + probe(() => Array.prototype.map.call(null as any, (v: any) => v)));
  console.log("foreach-on-number=" + probe(() => Array.prototype.forEach.call(5 as any, () => undefined)));

  const frozenArr = Object.freeze([1, 2]) as any;
  console.log("frozen-array-write=" + probe(() => { frozenArr[0] = 9; return frozenArr[0]; }));
  console.log("frozen-array-push=" + probe(() => frozenArr.push(3)));
}

// 9. Frozen, sealed and non-extensible writes; Reflect answers the same without throwing.
{
  const frozen: any = Object.freeze({ a: 1 });
  console.log("frozen-write=" + probe(() => { frozen.a = 2; return frozen.a; }));
  console.log("frozen-add=" + probe(() => { frozen.b = 1; return "added"; }));
  console.log("frozen-delete=" + probe(() => delete frozen.a));
  console.log("frozen-value=" + frozen.a);

  const sealed: any = Object.seal({ a: 1 });
  console.log("sealed-write=" + probe(() => { sealed.a = 2; return sealed.a; }));
  console.log("sealed-add=" + probe(() => { sealed.b = 1; return "added"; }));
  console.log("sealed-delete=" + probe(() => delete sealed.a));
  console.log("sealed-value=" + sealed.a);

  const nonExt: any = Object.preventExtensions({ a: 1 });
  console.log("nonext-add=" + probe(() => { nonExt.b = 1; return "added"; }));

  const locked: any = {};
  Object.defineProperty(locked, "k", { value: 1, configurable: false, writable: false });
  console.log("redefine-locked=" + probe(() => Object.defineProperty(locked, "k", { value: 2 })));
  console.log("redefine-same-ok=" + probe(() => Object.defineProperty(locked, "k", { value: 1 })));

  const pa: any = {};
  const pb: any = Object.create(pa);
  console.log("cyclic-proto=" + probe(() => Object.setPrototypeOf(pa, pb)));
  const sealedProto: any = Object.preventExtensions({});
  console.log("proto-on-nonext=" + probe(() => Object.setPrototypeOf(sealedProto, { x: 1 })));

  console.log("reflect-set-frozen=" + Reflect.set(frozen, "a", 2) + ":" + frozen.a);
  console.log("reflect-add-frozen=" + Reflect.set(frozen, "b", 1) + ":" + String(frozen.b));
  console.log("reflect-set-sealed=" + Reflect.set(sealed, "a", 3) + ":" + sealed.a);
  console.log("reflect-add-nonext=" + Reflect.set(nonExt, "d", 1) + ":" + String(nonExt.d));
  console.log("reflect-define-locked=" + Reflect.defineProperty(locked, "k", { value: 2 }) + ":" + locked.k);
  console.log("reflect-delete-frozen=" + Reflect.deleteProperty(frozen, "a"));
  console.log("reflect-setproto-cyclic=" + Reflect.setPrototypeOf(pa, pb));
  console.log("reflect-setproto-nonext=" + Reflect.setPrototypeOf(sealedProto, { x: 1 }));
}

// 10. Throws from getters, toPrimitive, toString and iterators propagate out of the built-in.
{
  const withGetter: any = {
    a: 1,
    get b(): number {
      throw new RangeError("getter");
    },
  };
  console.log("stringify-getter=" + probe(() => JSON.stringify(withGetter)));
  console.log("stringify-ok=" + probe(() => JSON.stringify({ a: 1 })));
  console.log("assign=" + probe(() => Object.assign({}, { get x(): number { throw new TypeError("assign"); } })));
  console.log("obj-spread=" + probe(() => ({ ...withGetter })));
  console.log("entries=" + probe(() => Object.entries(withGetter)));
  console.log("keys-safe=" + probe(() => Object.keys(withGetter).join(",")));

  const prim: any = {
    [Symbol.toPrimitive](hint: string): any {
      throw new RangeError("prim:" + hint);
    },
    toString(): string {
      return "never";
    },
    valueOf(): number {
      return 0;
    },
  };
  console.log("concat=" + probe(() => "x" + prim));
  console.log("template=" + probe(() => `${prim}`));
  console.log("plus=" + probe(() => (prim as any) + 1));
  console.log("number=" + probe(() => Number(prim)));
  console.log("string=" + probe(() => String(prim)));
  console.log("key=" + probe(() => ({} as any)[prim]));

  const hints: string[] = [];
  const hinted: any = {
    [Symbol.toPrimitive](hint: string): any {
      hints.push(hint);
      return 1;
    },
  };
  "" + hinted;
  `${hinted}`;
  Number(hinted);
  String(hinted);
  console.log("hints=" + hints.join(","));

  const order: string[] = [];
  const ordinary: any = {
    valueOf(): any {
      order.push("valueOf");
      return {};
    },
    toString(): any {
      order.push("toString");
      throw new TypeError("toString");
    },
  };
  console.log("ordinary=" + probe(() => "" + ordinary) + ",order=" + order.join(","));
  const neither: any = {
    valueOf(): any {
      return {};
    },
    toString(): any {
      return {};
    },
  };
  console.log("neither=" + probe(() => "" + neither));

  const badIterable: any = {
    [Symbol.iterator](): any {
      throw new RangeError("iter");
    },
  };
  console.log("spread=" + probe(() => [...badIterable]));
  console.log("array-from=" + probe(() => Array.from(badIterable)));
  console.log("destructure=" + probe(() => { const [x] = badIterable; return x; }));
  console.log("map-ctor=" + probe(() => new Map(badIterable)));
  const badNext: any = {
    [Symbol.iterator](): any {
      return {
        next(): any {
          throw new EvalError("next");
        },
      };
    },
  };
  console.log("bad-next=" + probe(() => [...badNext]));
}

// 11. TDZ: let, const and class bindings throw ReferenceError on any touch before initialisation.
{
  console.log("let-read=" + probe(() => {
    const r = probe(() => later as any);
    let later = 1;
    return r + "/then:" + later;
  }));
  console.log("typeof-tdz=" + probe(() => {
    const t = probe(() => typeof (pending as any));
    let pending = 1;
    return t + "/after:" + typeof pending;
  }));
  console.log("typeof-undeclared=" + typeof (globalThis as any).neverDeclaredAnywhere);
  console.log("let-write=" + probe(() => {
    const w = probe(() => {
      assignFirst = 5;
      return "wrote";
    });
    let assignFirst = 1;
    return w + "/final:" + assignFirst;
  }));
  console.log("const-tdz=" + probe(() => {
    const r = probe(() => fixed as any);
    const fixed = 2;
    return r + "/value:" + fixed;
  }));
  console.log("class-tdz=" + probe(() => {
    const r = probe(() => new (Later as any)());
    class Later {}
    return r + "/then:" + typeof Later;
  }));
  console.log("extends-self=" + probe(() => {
    class SelfRef extends (SelfRef as any) {}
    return typeof SelfRef;
  }));
  console.log("static-self-ok=" + probe(() => {
    class Ok {
      static via: string = "ok";
      static self: string = Ok.via + "-again";
    }
    return Ok.self;
  }));
  function forwardOrder(a: any = b as any, b: any = 2): string {
    return String(a) + "," + String(b);
  }
  console.log("param-forward=" + probe(() => forwardOrder()));
  console.log("param-forward-supplied=" + probe(() => forwardOrder(1)));
  function backwardOrder(a: any = 1, b: any = a + 1): string {
    return a + "," + b;
  }
  console.log("param-backward=" + probe(() => backwardOrder()));
  function shadowed(v: any = hidden as any): string {
    let hidden = "body";
    return String(v) + "/" + hidden;
  }
  console.log("param-body-let=" + probe(() => shadowed()));
  console.log("param-body-let-supplied=" + probe(() => shadowed("given")));
  const shared = "outer";
  console.log("block-shadow=" + probe(() => {
    const seen: string[] = [];
    {
      seen.push(probe(() => shared as any));
      const shared = "inner";
      seen.push("inner-value:" + shared);
    }
    seen.push("outer-value:" + shared);
    return seen.join("|");
  }));
  function inSwitch(n: number): string {
    switch (n) {
      case 0:
        return probe(() => declaredInCase as any);
      case 1:
        let declaredInCase = "one";
        return "declared:" + declaredInCase;
      default:
        return "none";
    }
  }
  console.log("switch-0=" + inSwitch(0) + ",switch-1=" + inSwitch(1) + ",switch-2=" + inSwitch(2));
  function hoisted(): string {
    const v = String(varName);
    const f = typeof named;
    var varName = 1;
    function named(): number {
      return 1;
    }
    return v + "/" + f + "/" + varName;
  }
  console.log("var-hoisted=" + hoisted());
  console.log("never-run=" + probe(() => {
    let touched = "no";
    if (false) {
      touched = unreached as any;
    }
    let unreached = "yes";
    return touched + "/" + unreached;
  }));
}

// 12. Exceptions raised while a class is defined or an instance is built.
{
  const log: string[] = [];
  console.log("computed-key=" + probe(() => {
    class Broken {
      [(log.push("k1"), "a")](): number {
        return 1;
      }
      [(() => {
        log.push("k2-throws");
        throw new RangeError("key");
      })()](): number {
        return 2;
      }
      [(log.push("k3"), "c")](): number {
        return 3;
      }
    }
    return typeof Broken;
  }));
  console.log("computed-log=" + log.join(","));

  const statics: string[] = [];
  console.log("static-field=" + probe(() => {
    class S {
      static first: string = (statics.push("first"), "1");
      static boom: string = (() => {
        statics.push("boom");
        throw new EvalError("static");
      })();
      static never: string = (statics.push("never"), "3");
    }
    return typeof S;
  }));
  console.log("static-log=" + statics.join(","));

  const fields: string[] = [];
  let escaped: any = null;
  class Partial {
    a: string = (fields.push("a"), "a-value");
    captured: string = ((escaped = this), "captured");
    b: string = (() => {
      fields.push("b-throws");
      throw new URIError("field");
    })();
    c: string = (fields.push("c"), "c-value");
    constructor() {
      fields.push("ctor");
    }
  }
  console.log("instance-field=" + probe(() => new Partial()));
  console.log("instance-log=" + fields.join(","));
  console.log("escaped-is-partial=" + (escaped instanceof Partial) + ",escaped-a=" + (escaped === null ? "none" : String(escaped.a)));

  const sideEffects: string[] = [];
  class Base {
    built: string = "base-built";
    constructor() {
      sideEffects.push("base-ctor");
    }
  }
  class DerivedBroken extends Base {
    ok: string = (sideEffects.push("derived-ok"), "ok");
    bad: string = (() => {
      sideEffects.push("derived-bad");
      throw new SyntaxError("derived");
    })();
    constructor() {
      super();
      sideEffects.push("derived-ctor");
    }
  }
  console.log("derived=" + probe(() => new DerivedBroken()));
  console.log("derived-log=" + sideEffects.join(","));

  const order2: string[] = [];
  class ThrowingBase {
    constructor() {
      order2.push("base");
      throw new EvalError("base-ctor");
    }
  }
  class OverThrowing extends ThrowingBase {
    field: string = (order2.push("derived-field"), "f");
    constructor() {
      super();
      order2.push("derived-ctor");
    }
  }
  console.log("throwing-base=" + probe(() => new OverThrowing()) + ",log=" + order2.join(","));
  console.log("second-attempt=" + probe(() => new OverThrowing()) + ",chain-intact=" + (Object.getPrototypeOf(OverThrowing.prototype) === ThrowingBase.prototype));

  const badKey: any = {
    [Symbol.toPrimitive](): string {
      throw new URIError("toprimitive");
    },
  };
  console.log("bad-key=" + probe(() => {
    class K {
      [badKey](): number {
        return 1;
      }
    }
    return typeof K;
  }));
  console.log("bad-heritage=" + probe(() => {
    class H extends (() => {
      throw new RangeError("heritage");
    })() {}
    return typeof H;
  }));
  console.log("final-log=" + log.join(",") + "/" + fields.join(","));
}

// 13. Optional chaining: null/undefined short-circuits the rest of the chain, but a
// plain `.` after the chain resumes on an undefined value still throws.
{
  const missing: any = undefined;
  const nulled: any = null;
  const present: any = { inner: { leaf: "leaf-value" }, fn: () => "called", notFn: 7 };
  const shallow: any = { inner: undefined };
  console.log("undef-deep=" + probe(() => missing?.a.b.c));
  console.log("undef-call=" + probe(() => missing?.a()));
  console.log("null-deep=" + probe(() => nulled?.a.b.c));
  console.log("resumed-plain=" + probe(() => shallow?.inner.leaf));
  console.log("resumed-optional=" + probe(() => shallow?.inner?.leaf));
  console.log("present-missing=" + probe(() => present?.nothing.leaf));
  console.log("present-missing-optional=" + probe(() => present?.nothing?.leaf));
  console.log("parens-break=" + probe(() => (missing?.a).b));
  console.log("parens-keep=" + probe(() => missing?.a.b));
  console.log("optcall-absent=" + probe(() => present.nothing?.()));
  console.log("optcall-not-function=" + probe(() => present.notFn?.()));
  console.log("call-absent-plain=" + probe(() => present.nothing()));
  const evaluated: string[] = [];
  const arg = (): number => {
    evaluated.push("arg");
    return 1;
  };
  console.log("skip-eval=" + probe(() => missing?.method(arg())) + ",evaluated=" + evaluated.length);
  console.log("on-number=" + probe(() => (0 as any)?.toFixed(1)));
  console.log("nullish-default=" + probe(() => missing?.a ?? "fallback"));
  console.log("nullish-after-throw=" + probe(() => missing.a ?? "fallback"));
  const receiver: any = {
    tag: "self",
    read(): string {
      return "read:" + this.tag;
    },
  };
  console.log("receiver-kept=" + probe(() => receiver?.read()));
  const detached = receiver.read;
  console.log("receiver-rebound=" + probe(() => detached.call({ tag: "other" })));
  console.log("receiver-empty=" + probe(() => detached.call({})));
  class Holder {
    get maybe(): any {
      return undefined;
    }
    get always(): any {
      return { deep: "deep-value" };
    }
  }
  const h = new Holder();
  console.log("class-optional=" + probe(() => h.maybe?.deep));
  console.log("class-plain=" + probe(() => h.maybe.deep));
  console.log("class-always-plain=" + probe(() => h.always.deep));
}
