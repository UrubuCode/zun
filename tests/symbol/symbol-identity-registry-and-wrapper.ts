// Symbol identity, description, the global registry (Symbol.for / Symbol.keyFor),
// the constructor and the boxed wrapper, and the explicit string conversions.

function bad(label: string, fn: () => any): void {
  try { fn(); console.log(label + "=no_throw"); }
  catch (e: any) { console.log(label + "=" + e.constructor.name); }
}

// --- uniqueness and identity ---
const a1 = Symbol("alpha");
const a2 = Symbol("alpha");
console.log("unique_same_description=" + (a1 === a2));
console.log("unique_self=" + (a1 === a1));
console.log("unique_bare=" + (Symbol() === Symbol()));
console.log("unique_description_equal=" + (a1.description === a2.description));
console.log("unique_loose_eq=" + ((a1 as any) == (a2 as any)));
const setOfSymbols = new Set<any>([a1, a2, a1]);
console.log("set_size=" + setOfSymbols.size);
const mapOfSymbols = new Map<any, string>([[a1, "first"], [a2, "second"]]);
console.log("map_size=" + mapOfSymbols.size + ":" + mapOfSymbols.get(a1) + ":" + mapOfSymbols.get(a2));

// --- typeof and primitive-ness ---
const bare = Symbol();
const empty = Symbol("");
const undef = Symbol(undefined as any);
console.log("typeof_local=" + typeof a1);
console.log("typeof_bare=" + typeof bare);
console.log("typeof_registered=" + typeof Symbol.for("typeof-check"));
console.log("typeof_wellknown=" + typeof Symbol.iterator);
console.log("typeof_inside_array=" + typeof [a1][0]);
console.log("instanceof_symbol=" + ((a1 as any) instanceof Symbol));
console.log("instanceof_object=" + ((a1 as any) instanceof Object));
console.log("truthy=" + Boolean(a1) + ":" + (a1 ? "y" : "n") + ":not=" + !a1);
console.log("strict_eq_fresh=" + ((a1 as any) === Symbol("alpha")));

// --- description: a prototype accessor, with ToString applied to the argument ---
console.log("desc_named=" + a1.description);
console.log("desc_bare=" + String(bare.description) + ":" + typeof bare.description);
console.log("desc_empty=[" + empty.description + "]:" + (empty.description as string).length);
console.log("desc_undefined_arg=" + String(undef.description) + ":" + (bare.description === undef.description));
console.log("desc_number_arg=" + Symbol(7 as any).description);
console.log("desc_null_arg=" + Symbol(null as any).description);
console.log("desc_boolean_arg=" + Symbol(false as any).description);
console.log("desc_object_arg=" + Symbol({ toString() { return "od"; } } as any).description);
console.log("desc_own_property=" + Object.prototype.hasOwnProperty.call(a1, "description"));
const descDesc: any = Object.getOwnPropertyDescriptor(Symbol.prototype, "description");
console.log("desc_accessor=" + typeof descDesc.get + ":setter=" + (descDesc.set === undefined) +
  ":enumerable=" + descDesc.enumerable + ":configurable=" + descDesc.configurable + ":name=" + descDesc.get.name);
bad("desc_on_plain_receiver", () => descDesc.get.call({}));

// --- toString() and the explicit String() conversion ---
console.log("tostring_named=" + a1.toString());
console.log("tostring_bare=" + bare.toString());
console.log("tostring_empty=" + empty.toString());
console.log("tostring_registered=" + Symbol.for("claude-desc").toString());
console.log("tostring_wellknown=" + Symbol.iterator.toString());
console.log("string_named=" + String(a1));
console.log("string_bare=" + String(bare));
console.log("tostring_length=" + Symbol.prototype.toString.length);
bad("tostring_on_plain_receiver", () => Symbol.prototype.toString.call({}));

// --- the global registry: Symbol.for returns the same symbol for the same key ---
const shared1 = Symbol.for("shared-key");
const shared2 = Symbol.for("shared-key");
console.log("for_same=" + (shared1 === shared2) + ":" + (shared2 === Symbol.for("shared-key")));
console.log("for_differs_from_local=" + (shared1 === Symbol("shared-key")));
console.log("for_description=" + shared1.description);
console.log("for_keyFor=" + Symbol.keyFor(shared1));
console.log("for_tostring=" + shared1.toString());
console.log("for_empty_key_keyFor=" + JSON.stringify(Symbol.keyFor(Symbol.for(""))));

// --- Symbol.for coerces its key with ToString, so distinct arguments can collide ---
console.log("for_number_key=" + (Symbol.for(1 as any) === Symbol.for("1")));
console.log("for_undefined_key=" + (Symbol.for(undefined as any) === Symbol.for("undefined")));
console.log("for_null_key=" + (Symbol.for(null as any) === Symbol.for("null")));
console.log("for_boolean_key=" + (Symbol.for(true as any) === Symbol.for("true")));
console.log("for_no_argument_key=" + ((Symbol.for as any)() === Symbol.for("undefined")));
const keyish = { toString() { return "claude-obj-key"; } };
console.log("for_object_key=" + (Symbol.for(keyish as any) === Symbol.for("claude-obj-key")));
console.log("for_object_key_keyFor=" + Symbol.keyFor(Symbol.for(keyish as any)));

// --- a registry key is independent of a plain symbol with the same description ---
const reg = Symbol.for("claude-reg");
const plain = Symbol("claude-reg");
console.log("reg_vs_plain_identity=" + (plain === reg));
console.log("reg_vs_plain_description=" + (plain.description === reg.description));
console.log("plain_keyFor=" + String(Symbol.keyFor(plain)));

// --- keyFor is undefined for every symbol that is not registered ---
console.log("keyFor_local=" + String(Symbol.keyFor(Symbol("nope"))));
console.log("keyFor_bare=" + String(Symbol.keyFor(bare)));
console.log("keyFor_iterator=" + String(Symbol.keyFor(Symbol.iterator)));
console.log("keyFor_asyncIterator=" + String(Symbol.keyFor(Symbol.asyncIterator)));
console.log("keyFor_toPrimitive=" + String(Symbol.keyFor(Symbol.toPrimitive)));

// --- keyFor refuses anything that is not a symbol ---
bad("keyFor_string", () => (Symbol.keyFor as any)("x"));
bad("keyFor_number", () => (Symbol.keyFor as any)(1));
bad("keyFor_object", () => (Symbol.keyFor as any)({}));
bad("keyFor_null", () => (Symbol.keyFor as any)(null));
bad("keyFor_no_argument", () => (Symbol.keyFor as any)());
bad("keyFor_wrapper", () => (Symbol.keyFor as any)(Object(Symbol("w"))));

// --- a registered symbol is stable across lookups and works as a property key ---
const r1 = Symbol.for("claude-stable");
const r2 = Symbol.for("claude-stable");
console.log("stable=" + (r1 === r2) + ":" + (r2 === Symbol.for("claude-stable")) + ":" + typeof r1);
const holder: any = {};
holder[r1] = 42;
console.log("registered_as_key=" + holder[Symbol.for("claude-stable")]);
console.log("registered_own_symbols=" + Object.getOwnPropertySymbols(holder).length);
console.log("registered_own_keyFor=" + Symbol.keyFor(Object.getOwnPropertySymbols(holder)[0]));

// --- shape of the registry statics ---
console.log("for_length_name=" + Symbol.for.length + ":" + Symbol.for.name);
console.log("keyFor_length_name=" + Symbol.keyFor.length + ":" + Symbol.keyFor.name);
const forDesc: any = Object.getOwnPropertyDescriptor(Symbol, "for");
console.log("for_flags=" + forDesc.writable + ":" + forDesc.enumerable + ":" + forDesc.configurable);

// --- Symbol is a function you may call but never construct ---
bad("new_symbol", () => new (Symbol as any)());
bad("new_symbol_with_argument", () => new (Symbol as any)("x"));
bad("reflect_construct_symbol", () => Reflect.construct(Symbol as any, []));
console.log("plain_call_typeof=" + typeof Symbol("ok"));
console.log("call_with_this_null=" + (Symbol as any).call(null, "viaCall").description);
console.log("apply_with_array=" + (Symbol as any).apply(null, ["viaApply"]).description);
class SubSymbol extends (Symbol as any) {}
bad("subclass_new", () => new (SubSymbol as any)());
console.log("subclass_inherits_static=" + typeof (SubSymbol as any).iterator);
console.log("symbol_prototype_is_object=" + typeof Symbol.prototype);
console.log("symbol_prototype_tag=" + (Object.getOwnPropertyDescriptor(Symbol.prototype, Symbol.toStringTag) as any).value);

// --- the wrapper object created by Object(symbol) ---
const box: any = Object(a1);
console.log("box_typeof=" + typeof box);
console.log("box_instanceof_symbol=" + (box instanceof Symbol));
console.log("box_tag=" + Object.prototype.toString.call(box));
console.log("box_proto_is_symbol_prototype=" + (Object.getPrototypeOf(box) === Symbol.prototype));
console.log("box_valueOf_returns_primitive=" + (box.valueOf() === a1));
console.log("box_strict_eq_primitive=" + ((box as any) === (a1 as any)));
console.log("box_loose_eq_primitive=" + ((box as any) == (a1 as any)));
console.log("box_of_box_same=" + (Object(box) === box));
console.log("box_twice_distinct=" + (Object(a1) === Object(a1)));
console.log("box_description=" + box.description);
console.log("box_toString=" + box.toString());
console.log("box_own_property_names=" + Object.getOwnPropertyNames(box).length);
box.extra = 1;
console.log("box_extra=" + box.extra + ":reboxed_extra=" + String(Object(a1).extra));
const viaBox: any = {};
viaBox[box] = "by_box";
console.log("box_as_key_reads_primitive=" + viaBox[a1] + ":key_type=" + typeof Reflect.ownKeys(viaBox)[0]);

// --- Symbol.prototype brand checks and its own toPrimitive ---
bad("valueOf_plain_object", () => Symbol.prototype.valueOf.call({}));
bad("valueOf_string", () => Symbol.prototype.valueOf.call("x"));
bad("valueOf_wrapper", () => Symbol.prototype.valueOf.call(box));
console.log("valueOf_primitive=" + (Symbol.prototype.valueOf.call(a1) === a1));
bad("prototype_description_getter", () => (Symbol.prototype as any).description);
bad("prototype_toString_call", () => Symbol.prototype.toString());
const protoToPrimitive: any = (Symbol.prototype as any)[Symbol.toPrimitive];
console.log("toPrimitive_default=" + (protoToPrimitive.call(box, "default") === a1));
console.log("toPrimitive_string=" + (protoToPrimitive.call(box, "string") === a1));
console.log("toPrimitive_number=" + (protoToPrimitive.call(box, "number") === a1));
console.log("toPrimitive_name_length=" + protoToPrimitive.name + ":" + protoToPrimitive.length);

// --- the wrapper and the primitive answer the same description and tag ---
console.log("wrapper_description_matches=" + (box.description === a1.description));
console.log("wrapper_String_refused=" + (() => { try { String(box); return "no"; } catch (e: any) { return e.constructor.name; } })());
bad("wrapper_implicit_concat", () => box + "");
