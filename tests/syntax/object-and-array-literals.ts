// Object literals (prototype form, duplicates, accessors, key order, spread,
// reserved-word names) and array literals (holes, elision, trailing commas,
// the Array constructor, and what each array method does with a hole).

// --- __proto__ in a literal: only the bare `__proto__: v` form sets the prototype ---
const base: any = { inherited: "from-base" };
const withProto: any = { __proto__: base, own: 1 };
console.log("proto_is_base=" + (Object.getPrototypeOf(withProto) === base));
console.log("proto_own_keys=" + Object.getOwnPropertyNames(withProto).join(","));
console.log("proto_inherited=" + withProto.inherited);
const computedName = "__proto__";
const computed: any = { [computedName]: base, own: 2 };
console.log("computed_proto_is_object=" + (Object.getPrototypeOf(computed) === Object.prototype));
console.log("computed_own_keys=" + Object.getOwnPropertyNames(computed).join(","));
const asMethod: any = { __proto__() { return "method"; }, own: 3 };
console.log("method_proto_is_object=" + (Object.getPrototypeOf(asMethod) === Object.prototype));
console.log("method_proto_call=" + asMethod.__proto__());
const bare: any = { __proto__: null, k: "v" };
console.log("bare_proto=" + String(Object.getPrototypeOf(bare)) + "|hasOwn=" + typeof bare.hasOwnProperty);
console.log("bare_keys=" + Object.keys(bare).join(","));
const ignored: any = { __proto__: 42, k: 1 };
console.log("primitive_proto_ignored=" + (Object.getPrototypeOf(ignored) === Object.prototype) + "|" + Object.getOwnPropertyNames(ignored).join(","));

// --- duplicate keys: last value wins, one property, every value expression still runs ---
const dupes: any = { a: 1, b: 2, a: 3 };
console.log("dupes=" + dupes.a + "|" + Object.keys(dupes).join(","));
const evalOrder: string[] = [];
function note(label: string, v: any): any { evalOrder.push(label); return v; }
const dupeEval: any = { x: note("first", 1), y: note("mid", 2), x: note("last", 3) };
console.log("dupe_eval=" + evalOrder.join(",") + "|" + dupeEval.x + "|" + Object.keys(dupeEval).join(","));
const spellings: any = { "k": 1, k: 2 };
console.log("spellings=" + spellings.k + "|" + Object.keys(spellings).length);

// --- accessor pairs and data/accessor replacement ---
const accessorPair: any = {
  get pair(): string { return "got:" + this._v; },
  set pair(v: string) { this._v = v; },
};
accessorPair.pair = "set-value";
const pairDesc: any = Object.getOwnPropertyDescriptor(accessorPair, "pair");
console.log("accessor_pair=" + accessorPair.pair + "|" + typeof pairDesc.get + "/" + typeof pairDesc.set);
const replaced: any = { get z(): number { return 1; }, z: 99 };
const zDesc: any = Object.getOwnPropertyDescriptor(replaced, "z");
console.log("data_after_accessor=" + replaced.z + "|has_get=" + (zDesc.get !== undefined) + "|writable=" + zDesc.writable);
const replacedBack: any = { y: 5, get y(): number { return 7; } };
const yDesc: any = Object.getOwnPropertyDescriptor(replacedBack, "y");
console.log("accessor_after_data=" + replacedBack.y + "|has_get=" + (yDesc.get !== undefined));
const method: any = { m(): number { return 1; } };
const mDesc: any = Object.getOwnPropertyDescriptor(method, "m");
console.log("method_attrs=" + mDesc.enumerable + "/" + mDesc.writable + "|has_prototype=" + Object.prototype.hasOwnProperty.call(method.m, "prototype"));
const plain: any = { attr: 1 };
const attrDesc: any = Object.getOwnPropertyDescriptor(plain, "attr");
console.log("literal_attrs=" + attrDesc.writable + "/" + attrDesc.enumerable + "/" + attrDesc.configurable);

// --- key order: integer-like keys ascend first, then strings in insertion order ---
const ordered: any = { b: 1, 2: 2, a: 3, 1: 4, "0": 5 };
console.log("key_order=" + Object.keys(ordered).join(","));
const nearlyNumeric: any = { "01": 1, "1.5": 2, "-1": 3, "10": 4, "2": 5 };
console.log("nearly_numeric_order=" + Object.keys(nearlyNumeric).join(","));
const sym = Symbol("s");
const withSymbol: any = { [sym]: 1, plain: 2, 3: 3 };
console.log("symbol_keys=" + Object.keys(withSymbol).join(",") + "|names=" + Object.getOwnPropertyNames(withSymbol).join(",") + "|symbols=" + Object.getOwnPropertySymbols(withSymbol).length);
const collapsed: any = { [1]: "number-one", ["1"]: "string-one" };
console.log("collapsed=" + collapsed["1"] + "|" + Object.keys(collapsed).length);

// --- computed keys and values run in source order, interleaved ---
const keyOrder: string[] = [];
function keyName(name: string): string { keyOrder.push("key:" + name); return name; }
function val(name: string): number { keyOrder.push("val:" + name); return 1; }
const interleaved: any = { [keyName("a")]: val("a"), [keyName("b")]: val("b") };
console.log("computed_eval=" + keyOrder.join(",") + "|" + Object.keys(interleaved).join(","));

// --- shorthand takes the current value; spread copies enumerable own properties ---
let shorthandValue = "before";
const shorthand: any = { shorthandValue };
shorthandValue = "after";
console.log("shorthand=" + shorthand.shorthandValue);
const spreadSrc: any = { p: "from-spread", q: "q-spread" };
const spreadFirst: any = { ...spreadSrc, p: "explicit" };
const spreadLast: any = { p: "explicit", ...spreadSrc };
console.log("spread_order=" + spreadFirst.p + "|" + spreadFirst.q + "|" + spreadLast.p);
const merged: any = { base: 1, ...{ extra: 2 }, base: 3, tail: 4 };
console.log("spread_merged=" + JSON.stringify(merged));
const getterSource: any = {};
Object.defineProperty(getterSource, "hidden", { value: 1, enumerable: false });
Object.defineProperty(getterSource, "shown", { get() { return "read"; }, enumerable: true });
const spreadCopy: any = { ...getterSource };
const shownDesc: any = Object.getOwnPropertyDescriptor(spreadCopy, "shown");
console.log("spread_copy=" + Object.keys(spreadCopy).join(",") + "|data=" + (shownDesc.get === undefined) + "|" + spreadCopy.shown);

// --- reserved words are legal property names, and get/set are ordinary names unless followed by a name ---
const lit: any = {
  get: 1, set: 2, new: 3, class: 4, if: 5, else: 6, function: 7, return: 8,
  typeof: 9, delete: 10, in: 11, of: 12, instanceof: 13, null: 14, true: 15,
  false: 16, this: 17, super: 18, var: 19, const: 20, let: 21, static: 22,
  async: 23, await: 24, yield: 25, default: 26, case: 27, do: 28, for: 29, while: 30,
};
console.log("reserved_count=" + Object.keys(lit).length);
console.log("reserved_sum=" + Object.keys(lit).reduce((acc, k) => acc + lit[k], 0));
console.log("reserved_dot=" + lit.get + "|" + lit.class + "|" + lit.null + "|" + lit.true + "|" + lit.this + "|" + lit.default);
const methods: any = {
  get(): string { return "method-get"; },
  set(v: number): string { return "method-set:" + v; },
  new(): string { return "method-new"; },
  delete(): string { return "method-delete"; },
};
console.log("reserved_methods=" + methods.get() + "|" + methods.set(9) + "|" + methods.new() + "|" + methods.delete());
const accessors: any = {
  _v: 0,
  get get(): string { return "accessor-get:" + this._v; },
  set set(v: number) { this._v = v * 2; },
  get value(): number { return this._v; },
  set value(v: number) { this._v = v + 1; },
};
console.log("accessor_named_get=" + accessors.get);
accessors.set = 21;
console.log("after_set_named_set=" + accessors._v);
accessors.value = 10;
console.log("after_value_setter=" + accessors.value);
const shortOf = "OF", shortAsync = "ASYNC", shortGet = "GET";
console.log("shorthand_words=" + JSON.stringify({ of: shortOf, async: shortAsync, get: shortGet }));
class Reserved {
  static default = "static-default";
  if(): string { return "if-method"; }
  get class(): string { return "class-getter"; }
  new(): string { return "new-method"; }
}
const r = new Reserved();
console.log("class_members=" + r.if() + "|" + r.class + "|" + r.new() + "|" + Reserved.default);
const mixed: any = { 10: "ten", get: "g", 2: "two", "with space": "ws", 1: "one" };
console.log("mixed_order=" + Object.keys(mixed).join(",") + "|" + mixed["with space"]);
console.log("delete_keyword=" + (delete lit.class) + "|" + ("class" in lit) + "|" + ("typeof" in lit) + "|" + ("nothing" in lit));

// --- trailing commas are legal in literals, parameter lists and calls ---
function trailing(a: number, b: number,): number { return a + b; }
console.log("trailing=" + trailing(1, 2,) + "|" + [1, 2, 3,].length + "|" + Object.keys({ a: 1, b: 2, }).join(","));

// --- array literal holes: a hole is an absent index, not an undefined value ---
const holed: any[] = [1, , 3];
console.log("hole_length=" + holed.length + "|hole_value=" + String(holed[1]));
console.log("hole_in=" + (1 in holed) + "|filled_in=" + (0 in holed));
console.log("hole_own=" + Object.prototype.hasOwnProperty.call(holed, 1));
console.log("trailing_commas=" + [1, 2, ].length + "|" + [, ].length + "|" + [, , ].length + "|" + [].length);
console.log("hole_then_value=" + [, 9].length + "|" + String([, 9][0]) + "|" + [, 9][1]);
console.log("counts=" + [1].length + "," + [1, ].length + "," + [1, , ].length + "," + [1, , , ].length);
console.log("keys=" + Object.keys(holed).join(",") + "|names=" + Object.getOwnPropertyNames(holed).join(","));
const visited: string[] = [];
holed.forEach((v, idx) => { visited.push(idx + ":" + String(v)); });
console.log("forEach_visits=" + visited.join(","));
const mapped = holed.map((v) => "seen" + String(v));
console.log("map_keeps_hole=" + mapped.length + "|" + !(1 in mapped) + "|" + mapped.map((v) => String(v)).join(","));
console.log("filter_drops=" + holed.filter(() => true).length);
console.log("some_every=" + holed.some((v) => v === undefined) + "|" + holed.every((v) => typeof v === "number"));
console.log("find_index=" + holed.findIndex((v) => v === undefined));
console.log("join_string=" + JSON.stringify(holed.join("-")) + "|" + JSON.stringify(String(holed)));
console.log("includes_undefined=" + holed.includes(undefined) + "|indexOf=" + holed.indexOf(undefined));
const spreadHoles = [...holed];
console.log("spread_materialises=" + spreadHoles.length + "|" + (1 in spreadHoles));
const fromIter: string[] = [];
for (const v of holed) fromIter.push(String(v));
console.log("for_of_holes=" + fromIter.join(",") + "|" + Array.from(holed).length + "|" + (1 in Array.from(holed)));
const inKeys: string[] = [];
for (const k in holed) inKeys.push(k);
console.log("for_in_holes=" + inKeys.join(","));
console.log("json_hole=" + JSON.stringify(holed));
console.log("mixed_spread=" + [, ...[1, , 3]].length + "|" + (0 in [, ...[1, , 3]]) + "|" + (2 in [, ...[1, , 3]]));
console.log("flat_drops=" + [1, , [2, , 3]].flat().length);
console.log("reduce_skips=" + [1, , 3].reduce((a: any, b: any) => a + b, 0));
function reduceEmpty(): string {
  try {
    return String(new Array(3).reduce((a: any, b: any) => a + b));
  } catch (e) {
    return "threw:" + (e as any).constructor.name;
  }
}
console.log("reduce_all_holes=" + reduceEmpty());

// --- writes, deletes and length changes create or remove holes ---
const filled: any[] = [1, , 3];
filled[1] = "now-here";
console.log("filled=" + (1 in filled) + "|" + filled[1]);
const deleted: any[] = [1, 2, 3];
delete deleted[1];
console.log("deleted_hole=" + (1 in deleted) + "|length=" + deleted.length);
const grown: any[] = [1];
grown.length = 3;
console.log("grown=" + grown.length + "|two_in=" + (2 in grown) + "|keys=" + Object.keys(grown).join(","));

// --- constructors: Array(n) is all holes, Array.of and literals are not ---
const sized = new Array(3);
console.log("array_n=" + sized.length + "|zero_in=" + (0 in sized) + "|keys=" + Object.keys(sized).length);
console.log("array_of=" + Array.of(3).length + "|zero_in=" + (0 in Array.of(3)));
const refilled = new Array(3).fill("x");
console.log("fill=" + refilled.join(",") + "|one_in=" + (1 in refilled));
console.log("concat_holes=" + [1, , 3].concat([4, , 6]).length + "|" + !(1 in [1, , 3].concat([4, , 6])) + "|" + !(4 in [1, , 3].concat([4, , 6])));
console.log("slice_hole=" + [1, , 3, 4].slice(0, 3).length + "|" + !(1 in [1, , 3, 4].slice(0, 3)));
const nested: any[] = [[1, , 3], , [, 2]];
console.log("nested_holes=" + !(1 in nested) + "|" + !(1 in nested[0]) + "|" + !(0 in nested[2]));
console.log("new_array_spread=" + new Array(...[1, 2, 3]).join(",") + "|" + Array(...[4, 5]).join(","));
const single = new Array(...[10]);
console.log("single_spread=" + single.join(",") + "|len=" + single.length + "|empty=" + new Array(...[]).length);
