// Object constructor and statics as values, Object.prototype.toString (built-in tags,
// Symbol.toStringTag rules and proxies), Object.is, valueOf, Object(value) boxing,
// typeof, and the own-vs-inherited answers of `in`, hasOwnProperty and Object.hasOwn,
// including null-prototype objects, boxed strings, sparse arrays and symbol keys.
export {};

const ts = Object.prototype.toString;

// ---------- 1. Statics readable as values, and working when detached ----------
console.log("statics=" + [
  typeof Object.defineProperty, typeof Object.defineProperties, typeof Object.fromEntries,
  typeof Object.create, typeof Object.setPrototypeOf, typeof Object.getOwnPropertyDescriptor,
  typeof Object.getOwnPropertySymbols, typeof Object.preventExtensions, typeof Object.isExtensible,
  typeof Object.keys, typeof Object.assign,
].join(","));
const dp = Object.defineProperty;
const target: any = {};
dp(target, "a", { value: 7 });
console.log("detached_define=" + target.a);
const fe = Object.fromEntries;
console.log("detached_fromEntries=" + JSON.stringify(fe([["k", 1]])));
const gopd = Object.getOwnPropertyDescriptor;
console.log("detached_gopd=" + (gopd({ a: 5 }, "a") as any).value);
const k = Object.keys;
console.log("detached_keys=" + k({ x: 1, y: 2 }).join(","));

// Object as a constructor and a function
const o1 = new Object();
console.log("new_empty=" + (Object.keys(o1).length === 0));
console.log("new_obj=" + new Object({ a: 1 }).a + ",call_obj=" + Object({ b: 2 }).b);
console.log("new_same=" + (new Object(o1 as any) === o1) + ",call_same=" + (Object(o1) === o1));
console.log("new_null=" + typeof new Object(null) + ",undef=" + typeof Object(undefined));

// ---------- 2. Object(value) boxes primitives; objects pass through ----------
const bn: any = Object(7);
console.log("box_num=" + typeof bn + "," + (bn instanceof Number) + "," + bn.valueOf());
const bs: any = Object("s");
console.log("box_str=" + typeof bs + "," + (bs instanceof String) + "," + bs.valueOf() + "," + bs.length);
const bb: any = Object(true);
console.log("box_bool=" + typeof bb + "," + (bb instanceof Boolean) + "," + bb.valueOf());
const bi: any = Object(10n);
console.log("box_big=" + typeof bi + "," + (bi instanceof BigInt) + "," + (bi.valueOf() === 10n));
const sym = Symbol("q");
const bsy: any = Object(sym);
console.log("box_sym=" + typeof bsy + "," + (bsy instanceof Symbol) + "," + (bsy.valueOf() === sym));
console.log("box_assign=" + typeof Object.assign(7 as any, { tag: "t" }) + "," + (Object.assign(7 as any, { tag: "t" }) as any).tag);

// valueOf: the identity for objects, the primitive for boxes
const plain = { a: 1 };
console.log("valueOf_same=" + (plain.valueOf() === plain));
console.log("valueOf_num=" + new Number(42).valueOf() + ",str=" + new String("hello").valueOf() + ",bool=" + new Boolean(true).valueOf());
console.log("valueOf_date=" + new Date(0).valueOf());

// ---------- 3. Object.is versus === ----------
const isPairs: any[][] = [[5, 5], [5, "5"], [NaN, NaN], [0, -0], [-0, -0], [Infinity, Infinity], [1, 1], ["1", 1], [{}, {}]];
console.log("is=" + isPairs.map(([a, b]) => Object.is(a, b)).join(","));
console.log("strict=" + isPairs.map(([a, b]) => a === b).join(","));
console.log("is_same_obj=" + Object.is(plain, plain));
console.log("is_null_undef=" + Object.is(null, undefined) + ",null_null=" + Object.is(null, null));

// ---------- 4. Object.prototype.toString: built-in tags ----------
console.log("tag_obj=" + ts.call({}) + ",arr=" + ts.call([]) + ",num=" + ts.call(5) + ",str=" + ts.call("hello"));
console.log("tag_bool=" + ts.call(true) + ",null=" + ts.call(null) + ",undef=" + ts.call(undefined));
console.log("tag_fn=" + ts.call(() => {}) + ",date=" + ts.call(new Date(0)) + ",regex=" + ts.call(/t/));
console.log("tag_map=" + ts.call(new Map()) + ",set=" + ts.call(new Set()) + ",promise=" + ts.call(Promise.resolve(1)));
console.log("tag_json=" + ts.call(JSON) + ",math=" + ts.call(Math) + ",boxed=" + ts.call(new String("x")));
console.log("tag_err=" + ts.call(new Error("e")) + ",args=" + ts.call((function () { return arguments; })()));
console.log("tag_bigint=" + ts.call(1n) + ",symbol=" + ts.call(Symbol("s")));
console.log("tag_nullproto=" + ts.call(Object.create(null)));
class Caixa { v = 1; }
console.log("tag_instance=" + ts.call(new Caixa()));
console.log("direct_obj=" + ({}).toString() + ",direct_arr=" + [1, 2].toString() + ",concat=" + ("" + {}));

// the idiom every library uses for type checks
function isArray(x: unknown): boolean { return ts.call(x) === "[object Array]"; }
console.log("is_array=" + isArray([1]) + "," + isArray({}) + "," + isArray("abc"));

// ---------- 5. Symbol.toStringTag rules ----------
console.log("tag_string=" + ts.call({ [Symbol.toStringTag]: "Custom" }));
console.log("tag_nonstring=" + [42, null, undefined, {}, "", "[object Nested]"].map((v) => ts.call({ [Symbol.toStringTag]: v } as any)).join("|"));
console.log("tag_symbol_value=" + ts.call({ [Symbol.toStringTag]: Symbol("x") } as any));
const taggedArr: any = [1, 2];
taggedArr[Symbol.toStringTag] = "NotArray";
console.log("tag_array_override=" + ts.call(taggedArr) + ",isArray=" + Array.isArray(taggedArr));
function tagged(): void { /* noop */ }
console.log("tag_fn_before=" + ts.call(tagged));
(tagged as any)[Symbol.toStringTag] = "Tagged";
console.log("tag_fn_after=" + ts.call(tagged));
console.log("tag_inherited=" + ts.call(Object.create({ [Symbol.toStringTag]: "Inherited" })));
const gt: any = {};
Object.defineProperty(gt, Symbol.toStringTag, { get() { return "FromGetter"; } });
console.log("tag_getter=" + ts.call(gt));
const throwingTag: any = {};
Object.defineProperty(throwingTag, Symbol.toStringTag, { get(): string { throw new RangeError("t"); } });
try {
  console.log("tag_throw=" + ts.call(throwingTag));
} catch (e: any) {
  console.log("tag_throw=throw:" + e.constructor.name);
}
class Tagged { get [Symbol.toStringTag](): string { return "MyClass"; } }
console.log("tag_class=" + ts.call(new Tagged()) + ",keys=" + Object.keys(new Tagged()).length);
const ctd = Object.getOwnPropertyDescriptor(Tagged.prototype, Symbol.toStringTag) as any;
console.log("tag_class_desc=e=" + ctd.enumerable + ",c=" + ctd.configurable);
console.log("tag_string_of=" + String({ [Symbol.toStringTag]: "Custom" }) + ",template=" + `${{ [Symbol.toStringTag]: "Custom" }}`);

// the tag is also the thing a proxy's get trap can invent
const invented: any = new Proxy({}, { get(_t, key) { return key === Symbol.toStringTag ? "ProxyTag" : undefined; } });
console.log("tag_proxy_invented=" + ts.call(invented));
console.log("tag_proxy_array=" + ts.call(new Proxy([1], {})) + ",fn=" + ts.call(new Proxy(tagged, { get() { return undefined; } })));
console.log("tag_proxy_nonstring=" + ts.call(new Proxy({}, { get(_t, key) { return key === Symbol.toStringTag ? 42 : undefined; } })));

// receivers that never look anything up
console.log("tag_null_receiver=" + ts.call(null) + ",undefined=" + ts.call(undefined));

// ---------- 6. typeof ----------
console.log("typeof=" + [
  typeof null, typeof undefined, typeof (() => {}), typeof Symbol(), typeof Symbol.iterator,
  typeof 1, typeof "s", typeof true, typeof {}, typeof [], typeof class {}, typeof NaN,
  typeof Math, typeof typeof null, typeof 1n,
].join(","));

// ---------- 7. `in`, hasOwnProperty, Object.hasOwn ----------
const proto = { inherited: 1 };
const obj: any = Object.create(proto);
obj.own = 2;
console.log("in_own=" + ("own" in obj) + ",in_inherited=" + ("inherited" in obj) + ",in_missing=" + ("zzz" in obj));
console.log("hop=" + obj.hasOwnProperty("own") + "," + obj.hasOwnProperty("inherited") + "," + obj.hasOwnProperty("toString"));
console.log("hasOwn=" + Object.hasOwn(obj, "own") + "," + Object.hasOwn(obj, "inherited") + "," + Object.hasOwn(obj, "zzz"));
console.log("in_builtin=" + ("toString" in {}) + "," + ("length" in []) + "," + ("push" in []) + "," + [].hasOwnProperty("push"));
console.log("in_array=" + (0 in [42]) + "," + (0 in [undefined]) + "," + (0 in new Array(3)) + ",undef_value=" + ({ a: undefined } as any).a);
const holed: any = [1, , 3];
console.log("holes=" + (0 in holed) + "," + (1 in holed) + "," + (2 in holed) + ",len=" + holed.length);
console.log("hasOwn_hole=" + Object.hasOwn(holed, 1) + ",len_own=" + Object.hasOwn(holed, "length"));
holed[1] = undefined;
console.log("hole_filled=" + Object.hasOwn(holed, 1) + ",value=" + holed[1]);
delete holed[1];
console.log("hole_deleted=" + Object.hasOwn(holed, 1));

// null prototype: hasOwn is the safe form; the method is not there
const bare: any = Object.create(null);
bare.k = 1;
bare["hasOwnProperty"] = "shadowed-but-a-string";
console.log("bare_hasOwn=" + Object.hasOwn(bare, "k") + ",borrowed=" + Object.prototype.hasOwnProperty.call(bare, "k"));
console.log("bare_own_names=" + Object.hasOwn(bare, "hasOwnProperty") + "," + Object.hasOwn(bare, "toString"));
try {
  console.log("bare_method=" + bare.hasOwnProperty("k"));
} catch (e: any) {
  console.log("bare_method=throw:" + e.constructor.name);
}
const shadowed: any = { hasOwnProperty: 1, real: 2 };
console.log("shadow_hasOwn=" + Object.hasOwn(shadowed, "real"));
try {
  console.log("shadow_method=" + shadowed.hasOwnProperty("real"));
} catch (e: any) {
  console.log("shadow_method=throw:" + e.constructor.name);
}

// boxed strings: indices and length are own; methods are not
const str = "abc";
console.log("str_own=" + Object.hasOwn(str as any, "0") + "," + Object.hasOwn(str as any, 2 as any) + "," + Object.hasOwn(str as any, "3") + "," + Object.hasOwn(str as any, "length") + "," + Object.hasOwn(str as any, "charAt"));
console.log("str_in=" + ("0" in Object(str)) + "," + ("charAt" in Object(str)));
console.log("str_names=" + Object.getOwnPropertyNames(Object(str)).join("|"));
console.log("str_empty_names=" + Object.getOwnPropertyNames(Object("")).join("|"));
console.log("box_num_names=" + Object.getOwnPropertyNames(Object(7)).length + ",bool=" + Object.getOwnPropertyNames(Object(true)).length);
console.log("box_sym_names=" + Object.getOwnPropertyNames(Object(Symbol("d"))).join("|"));
console.log("hasOwn_prims=" + Object.hasOwn("ab" as any, "1") + "," + Object.hasOwn(7 as any, "toFixed"));
attempt("hasOwn_null", () => Object.hasOwn(null as any, "a"));
attempt("hasOwn_undef", () => Object.hasOwn(undefined as any, "a"));

// key coercion and symbols
const numKeys: any = { 1: "one" };
console.log("coerce=" + Object.hasOwn(numKeys, 1) + "," + Object.hasOwn(numKeys, "1") + "," + Object.hasOwn({ 0: 1 } as any, -0));
const objKey: any = { toString() { return "made"; } };
console.log("coerce_obj=" + Object.hasOwn({ made: 1 } as any, objKey));
const s1 = Symbol("s");
console.log("sym_own=" + Object.hasOwn({ [s1]: 1 } as any, s1));

// getter-only property is still own; propertyIsEnumerable is own-only too
const accOnly: any = {};
Object.defineProperty(accOnly, "g", { get() { return 1; } });
console.log("acc_own=" + Object.hasOwn(accOnly, "g") + ",keys=" + Object.keys(accOnly).length + ",pie=" + accOnly.propertyIsEnumerable("g"));

// hasOwn on a proxy goes through getOwnPropertyDescriptor, `in` through has
const traps: string[] = [];
const px: any = new Proxy({ x: 1 }, {
  has(t, key) { traps.push("has:" + String(key)); return Reflect.has(t, key); },
  getOwnPropertyDescriptor(t, key) { traps.push("gopd:" + String(key)); return Reflect.getOwnPropertyDescriptor(t, key); },
});
console.log("proxy_hasOwn=" + Object.hasOwn(px, "x"));
console.log("proxy_in=" + ("x" in px));
console.log("proxy_traps=" + traps.join("|"));

function attempt(label: string, fn: () => unknown): void {
  try {
    console.log(label + "=ok:" + String(fn()));
  } catch (e: any) {
    console.log(label + "=throw:" + e.constructor.name);
  }
}

// ---------- 8. Coercion hooks: numeric hint uses valueOf first, string hint uses toString first ----------
const hintSeen: string[] = [];
const hint = {
  valueOf() { hintSeen.push("valueOf"); return 10; },
  toString() { hintSeen.push("toString"); return "S"; },
};
console.log("hint_plus=" + (+hint) + "," + hintSeen.join(","));
hintSeen.length = 0;
console.log("hint_string=" + String(hint) + "," + hintSeen.join(","));
