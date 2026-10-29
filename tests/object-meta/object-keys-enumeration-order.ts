// Own-property enumeration: ordering, enumerability filters, and agreement between
// Object.keys / values / entries / for..in / JSON / spread / getOwnPropertyNames.
// Spec order: array-index keys ascending, then other string keys in insertion
// order, then symbols (never reported by the string-key APIs).
export {};

// 1. Insertion order for plain string keys
const plain = { c: 3, a: 1, b: 2 };
console.log("plain_keys=" + Object.keys(plain).join(","));
console.log("plain_values=" + Object.values(plain).join(","));
console.log("plain_entries=" + Object.entries(plain).map((e) => e[0] + ":" + e[1]).join(","));

// 2. Integer-like keys come first, ascending, regardless of insertion
const numeric = { 2: "b", 1: "a", 3: "c" };
console.log("numeric_keys=" + Object.keys(numeric).join(","));
console.log("numeric_values=" + Object.values(numeric).join(","));

const mixed = { b: 2, 1: "one", a: 1, 2: "two" };
console.log("mixed_keys=" + Object.keys(mixed).join(","));
console.log("mixed_json=" + JSON.stringify(mixed));
console.log("mixed_forin=" + (() => { const out: string[] = []; for (const k in mixed) out.push(k); return out.join(","); })());

// 3. Index-like string forms that are NOT canonical array indices stay in insertion order
const oddKeys: Record<string, number> = { "01": 1, "1.5": 2, "-1": 3, "2": 4, x: 5 };
console.log("odd_keys=" + Object.keys(oddKeys).join(","));

// 4. Symbols are excluded from every string-key API and reported only by getOwnPropertySymbols
const s1 = Symbol("one");
const s2 = Symbol("two");
const withSym: any = { a: 1, [s1]: "hidden1", b: 2 };
withSym[s2] = "hidden2";
console.log("sym_keys=" + Object.keys(withSym).join(","));
console.log("sym_names=" + Object.getOwnPropertyNames(withSym).join(","));
console.log("sym_count=" + Object.getOwnPropertySymbols(withSym).length);
console.log("sym_has1=" + Object.getOwnPropertySymbols(withSym).includes(s1));
console.log("sym_json=" + JSON.stringify(withSym));
console.log("sym_reflect_order=" + Reflect.ownKeys(withSym).map((k) => String(k)).join(","));

// 5. Non-enumerable own properties: hidden from keys/values/entries/JSON/spread/for-in,
//    still reported by getOwnPropertyNames and Reflect.ownKeys
const obj: any = { a: 1, b: 2 };
Object.defineProperty(obj, "hidden", { value: 3, enumerable: false, configurable: true });
Object.defineProperty(obj, "0", { value: "zero", enumerable: false, configurable: true });
console.log("ne_keys=" + Object.keys(obj).join(","));
console.log("ne_values=" + Object.values(obj).join(","));
console.log("ne_entries=" + Object.entries(obj).length);
console.log("ne_json=" + JSON.stringify(obj));
console.log("ne_spread=" + JSON.stringify({ ...obj }));
console.log("ne_names=" + Object.getOwnPropertyNames(obj).join(","));
console.log("ne_reflect=" + Reflect.ownKeys(obj).join(","));
console.log("ne_forin=" + (() => { const out: string[] = []; for (const k in obj) out.push(k); return out.join(","); })());
console.log("ne_hidden_value=" + obj.hidden);

// 6. Enumerable via prototype is NOT own: for..in sees it, keys/values/entries do not
const proto = { inherited: "P" };
const child: any = Object.create(proto);
child.own = "C";
const forinChild: string[] = [];
for (const k in child) forinChild.push(k);
console.log("proto_forin=" + forinChild.join(","));
console.log("proto_keys=" + Object.keys(child).join(","));
console.log("proto_values=" + Object.values(child).join(","));
console.log("proto_json=" + JSON.stringify(child));

// 7. Non-enumerable prototype key is skipped by for..in too
Object.defineProperty(proto, "quiet", { value: "Q", enumerable: false, configurable: true });
const forinQuiet: string[] = [];
for (const k in child) forinQuiet.push(k);
console.log("quiet_forin=" + forinQuiet.join(","));
console.log("quiet_value=" + child.quiet);

// 8. Shadowing: a non-enumerable own key hides an enumerable inherited one
const shadowProto = { k: "P", visible: 1 };
const shadowChild: any = Object.create(shadowProto);
Object.defineProperty(shadowChild, "k", { value: "C", enumerable: false, configurable: true });
shadowChild.own = 2;
const shadowKeys: string[] = [];
for (const k in shadowChild) shadowKeys.push(k);
console.log("shadow_forin=" + shadowKeys.join(","));
console.log("shadow_keys=" + Object.keys(shadowChild).join(","));
console.log("shadow_value=" + shadowChild.k);

// 9. Arrays: indices and length; length is non-enumerable
const arr = [10, 20, 30];
(arr as any).extra = "x";
console.log("arr_keys=" + Object.keys(arr).join(","));
console.log("arr_names=" + Object.getOwnPropertyNames(arr).join(","));
console.log("arr_json=" + JSON.stringify(arr));

// 10. Strings and numbers as boxed objects
console.log("boxed_str_keys=" + Object.keys(new String("hi")).join(","));
console.log("boxed_str_names=" + Object.getOwnPropertyNames(new String("hi")).join(","));
console.log("boxed_num_keys=" + Object.keys(new Number(42)).length);
console.log("primitive_keys=" + Object.keys("ab").join(","));

// 11. Empty and null-prototype objects
console.log("empty_keys=" + Object.keys({}).length + ":" + Object.values({}).length + ":" + Object.entries({}).length);
const bare = Object.create(null);
bare.z = 1;
bare.a = 2;
console.log("bare_keys=" + Object.keys(bare).join(","));
console.log("bare_names=" + Object.getOwnPropertyNames(bare).join(","));

// 12. Object.keys and for..in agree on a plain object after deletion and re-insertion
const churn: any = { a: 1, b: 2, c: 3 };
delete churn.b;
churn.b = 4;
console.log("churn_keys=" + Object.keys(churn).join(","));
console.log("churn_forin=" + (() => { const out: string[] = []; for (const k in churn) out.push(k); return out.join(","); })());

// 13. Integer key order also applies to arrays with extra integer-like props
const sparse: any = [];
sparse[5] = "five";
sparse[2] = "two";
sparse.beta = "b";
sparse[0] = "zero";
console.log("sparse_keys=" + Object.keys(sparse).join(","));
console.log("sparse_length=" + sparse.length);

// 14. Object.entries pairs and Object.fromEntries round trip preserve order
const rt = Object.fromEntries(Object.entries({ q: 1, 0: "zero", p: 2 }));
console.log("roundtrip_keys=" + Object.keys(rt).join(","));

// 15. JSON.stringify follows the same enumerable own key order
console.log("json_order=" + JSON.stringify({ z: 1, 10: 2, 9: 3, y: 4 }));

// 16. Spread copies enumerable own string and symbol keys in the same order
const spreadSrc: any = { m: 1, [s1]: "S", 1: "one" };
const spreadOut: any = { ...spreadSrc };
console.log("spread_keys=" + Object.keys(spreadOut).join(","));
console.log("spread_sym=" + (spreadOut[s1] === "S"));
