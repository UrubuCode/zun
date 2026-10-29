// Prototype chains: Object.create / setPrototypeOf / __proto__ (accessor, literal forms,
// JSON), live lookup after re-parenting, own shadowing, cycle refusal, the null-prototype
// object, constructor-function inheritance, inherited setters that swallow assignments,
// accessor receivers across levels, and the lock that preventExtensions puts on the prototype.
export {};

function attempt(label: string, fn: () => unknown): void {
  try {
    console.log(label + "=ok:" + String(fn()));
  } catch (e: any) {
    console.log(label + "=throw:" + e.constructor.name);
  }
}

// ---------- 1. Object.create and the null prototype ----------
const bare: any = Object.create(null);
bare.a = 1;
console.log("bare_a=" + bare.a + ",proto=" + Object.getPrototypeOf(bare) + ",toString=" + (bare.toString === undefined));
console.log("bare_tag=" + Object.prototype.toString.call(bare));
console.log("create_obj_proto=" + (Object.getPrototypeOf(Object.create(Object.prototype)) === Object.prototype));
const parentC = { x: 10 };
const childC: any = Object.create(parentC);
console.log("child_x=" + childC.x + ",own=" + childC.hasOwnProperty("x"));
console.log("create_props=" + Object.create(null, { a: { value: 1, enumerable: true } } as any).a);
console.log("create_fn_proto=" + (Object.getPrototypeOf(Object.create(Math.max as any)) === Math.max));
attempt("create_undef_proto", () => Object.create(undefined as any));
attempt("create_number_proto", () => Object.create(7 as any));
attempt("create_string_proto", () => Object.create("s" as any));
attempt("create_symbol_proto", () => Object.create(Symbol("s") as any));
console.log("create_no_props=" + Reflect.ownKeys(Object.create(null, undefined as any)).length);
attempt("create_null_props", () => Reflect.ownKeys(Object.create(null, null as any)).length);
attempt("create_desc_primitive", () => Reflect.ownKeys(Object.create(null, { k: 1 } as any)).length);

// the properties map: own ENUMERABLE keys, symbols included, omitted attributes false
const dflt: any = Object.create(null, { k: { value: 1 } });
const dd = Object.getOwnPropertyDescriptor(dflt, "k") as any;
console.log("create_defaults=w=" + dd.writable + ",e=" + dd.enumerable + ",c=" + dd.configurable);
const sk = Symbol("sk");
const withSym: any = Object.create(null, { [sk]: { value: "S", enumerable: true } } as any);
console.log("create_sym=" + withSym[sk] + ",symbols=" + Object.getOwnPropertySymbols(withSym).map(String).join("|"));
const mapDesc: any = { visible: { value: 1, enumerable: true } };
Object.defineProperty(mapDesc, "skipped", { value: { value: 2, enumerable: true }, enumerable: false });
console.log("create_map_filter=" + Reflect.ownKeys(Object.create(null, mapDesc)).join("|"));
const inhDesc: any = Object.create({ fromProto: { value: 3, enumerable: true } });
inhDesc.own = { value: 4, enumerable: true };
console.log("create_map_inherited=" + Reflect.ownKeys(Object.create(null, inhDesc)).join("|"));
const reads: string[] = [];
const getterMap: any = {};
Object.defineProperty(getterMap, "b", { get() { reads.push("b"); return { value: 2, enumerable: true }; }, enumerable: true });
Object.defineProperty(getterMap, "1", { get() { reads.push("1"); return { value: 1, enumerable: true }; }, enumerable: true });
Object.defineProperty(getterMap, "a", { get() { reads.push("a"); return { value: 3, enumerable: true }; }, enumerable: true });
const built: any = Object.create(null, getterMap);
console.log("create_map_reads=" + reads.join("|") + ",result=" + Reflect.ownKeys(built).join("|"));

// the created object is extensible; a null-prototype object takes a prototype later
const child2: any = Object.create(null, { k: { value: 1, writable: true, configurable: true } });
child2.added = 2;
console.log("create_ext=" + Object.isExtensible(child2) + ",keys=" + Reflect.ownKeys(child2).join("|"));
console.log("create_setproto=" + Reflect.setPrototypeOf(child2, Object.prototype) + ",toString=" + typeof child2.toString);

// ---------- 2. __proto__: the accessor, the four literal spellings, JSON ----------
const pp: any = { kind: "PROTO" };
const litIdent: any = { __proto__: pp, own: 1 };
console.log("lit_ident=" + (Object.getPrototypeOf(litIdent) === pp) + ",own=" + Object.getOwnPropertyNames(litIdent).join("|") + ",kind=" + litIdent.kind);
const litString: any = { "__proto__": pp, own: 1 };
console.log("lit_string=" + (Object.getPrototypeOf(litString) === pp) + ",own=" + Object.getOwnPropertyNames(litString).join("|"));
const keyName = "__proto__";
const litComputed: any = { [keyName]: pp, own: 1 };
console.log("lit_computed=" + (Object.getPrototypeOf(litComputed) === Object.prototype) + ",own=" + Object.getOwnPropertyNames(litComputed).join("|") + ",read=" + (litComputed.__proto__ === pp));
const __proto__ = pp;
const litShort: any = { __proto__, own: 1 };
console.log("lit_shorthand=" + (Object.getPrototypeOf(litShort) === Object.prototype) + ",own=" + Object.getOwnPropertyNames(litShort).join("|"));
const litMethod: any = { __proto__() { return "m"; }, own: 1 };
console.log("lit_method=" + (Object.getPrototypeOf(litMethod) === Object.prototype) + ",own=" + Object.getOwnPropertyNames(litMethod).join("|") + ",call=" + litMethod.__proto__());

const protoDesc = Object.getOwnPropertyDescriptor(Object.prototype, "__proto__") as any;
console.log("proto_accessor=" + ("get" in protoDesc ? "accessor" : "data") + ",e=" + protoDesc.enumerable + ",c=" + protoDesc.configurable + ",get=" + typeof protoDesc.get + ",set=" + typeof protoDesc.set);
console.log("proto_getter_matches=" + (protoDesc.get.call(litIdent) === pp));

// assigning a primitive through the accessor is ignored; null is accepted
const q: any = { a: 1 };
q.__proto__ = 5;
q.__proto__ = "str";
console.log("proto_prim_ignored=" + (Object.getPrototypeOf(q) === Object.prototype) + ",own=" + Object.getOwnPropertyNames(q).join("|"));
q.__proto__ = null;
console.log("proto_null=" + Object.getPrototypeOf(q) + ",read=" + q.__proto__);

// on a null-prototype object the name is an ordinary property
const bareProto: any = Object.create(null);
bareProto.__proto__ = { injected: true };
console.log("bare_proto=" + Object.getPrototypeOf(bareProto) + ",own=" + Object.getOwnPropertyNames(bareProto).join("|") + ",injected=" + bareProto.__proto__.injected);

// defineProperty creates an own data slot that shadows the accessor
const shadow: any = {};
Object.defineProperty(shadow, "__proto__", { value: "own-value", enumerable: true, writable: true, configurable: true });
console.log("shadow_read=" + shadow.__proto__ + ",proto_intact=" + (Object.getPrototypeOf(shadow) === Object.prototype) + ",json=" + JSON.stringify(shadow));

// Object.setPrototypeOf and Object.create(getPrototypeOf) agree with the accessor
const base1: any = { type: "base", greet() { return "hi from " + this.type; } };
const obj: any = { type: "child" };
obj.__proto__ = base1;
console.log("setproto_greet=" + obj.greet() + ",identity=" + (Object.getPrototypeOf(obj) === base1));
const src: any = { x: 1 };
Object.setPrototypeOf(src, { double() { return this.x * 2; } });
console.log("setproto_method=" + src.double());
attempt("setproto_number", () => Object.setPrototypeOf({} as any, 7 as any));
attempt("setproto_undefined", () => Object.setPrototypeOf({} as any, undefined as any));
console.log("setproto_prim_ok=" + (Object.setPrototypeOf(7 as any, Number.prototype) as any));
attempt("setproto_null_null", () => Object.setPrototypeOf(null as any, null));

// ---------- 3. Live lookup: re-parenting, shadowing, deletion ----------
const a = { value: "a" };
const b = { value: "b", extra: 2 };
const live: any = Object.create(a);
console.log("live_before=" + live.value + "," + live.extra);
Object.setPrototypeOf(live, b);
console.log("live_after=" + live.value + "," + live.extra + "," + (Object.getPrototypeOf(live) === b));

const root = { x: 1, y: 2 };
const mid: any = Object.create(root);
mid.x = 3;
const leaf: any = Object.create(mid);
leaf.z = 4;
console.log("shadow_chain=" + leaf.x + "," + leaf.y + "," + leaf.z);
console.log("shadow_own=" + Object.hasOwn(leaf, "x") + ",in=" + ("x" in leaf));
delete mid.x;
console.log("shadow_deleted=" + leaf.x);

// the prototype's own non-enumerable properties still answer reads
const protoQ: any = { p1: 1, p2: 2 };
const kid: any = Object.create(protoQ);
Object.defineProperty(protoQ, "quiet", { value: "Q", enumerable: false, configurable: true });
console.log("quiet_read=" + kid.quiet + ",in=" + ("quiet" in kid));

// ---------- 4. Cycles and extensibility ----------
const cx: any = {};
const cy: any = {};
console.log("cycle_ok=" + Reflect.setPrototypeOf(cx, cy) + ",linked=" + (Object.getPrototypeOf(cx) === cy));
console.log("cycle_refused=" + Reflect.setPrototypeOf(cy, cx) + ",linked=" + (Object.getPrototypeOf(cy) === cx));
attempt("cycle_object_api", () => { Object.setPrototypeOf(cy, cx); return "ok"; });
console.log("self_cycle=" + Reflect.setPrototypeOf(cx, cx));

// preventExtensions locks the prototype: only the current value is accepted
const protoA: any = { tag: "A" };
const protoB: any = { tag: "B" };
const locked: any = Object.create(protoA);
locked.own = 1;
Object.preventExtensions(locked);
console.log("lock_same=" + Reflect.setPrototypeOf(locked, protoA) + ",other=" + Reflect.setPrototypeOf(locked, protoB) + ",null=" + Reflect.setPrototypeOf(locked, null));
attempt("lock_object_api", () => { Object.setPrototypeOf(locked, protoB); return "ok"; });
console.log("lock_tag=" + locked.tag);
const lockedBare: any = Object.preventExtensions(Object.create(null));
console.log("lock_bare_null=" + Reflect.setPrototypeOf(lockedBare, null) + ",other=" + Reflect.setPrototypeOf(lockedBare, protoA));
console.log("lock_write=" + Reflect.set(locked, "own", 2) + ",v=" + locked.own);
console.log("lock_delete=" + Reflect.deleteProperty(locked, "own") + ",readd=" + Reflect.set(locked, "own", 4) + ",has=" + Object.hasOwn(locked, "own"));
console.log("lock_empty_flags=" + Object.isSealed(locked) + "," + Object.isFrozen(locked));

// ---------- 5. Constructor-function inheritance ----------
function Animal(this: any, name: string) { this.name = name; }
(Animal.prototype as any).speak = function () { return this.name + " speaks"; };
function Dog(this: any, name: string) { Animal.call(this, name); }
Dog.prototype = Object.create(Animal.prototype);
(Dog.prototype as any).constructor = Dog;
(Dog.prototype as any).bark = function () { return this.name + " barks"; };
const dog: any = new (Dog as any)("Rex");
console.log("ctor_speak=" + dog.speak() + ",bark=" + dog.bark());
console.log("ctor_instanceof=" + (dog instanceof Dog) + "," + (dog instanceof Animal) + ",constructor=" + (dog.constructor === Dog));
console.log("ctor_own=" + dog.hasOwnProperty("name") + "," + dog.hasOwnProperty("speak"));
console.log("ctor_proto=" + (Object.getPrototypeOf(dog) === Dog.prototype) + "," + (Object.getPrototypeOf(Dog.prototype) === Animal.prototype));

function Shape(this: any, color: string) { this.color = color; }
(Shape.prototype as any).describe = function () { return this.color + " shape"; };
function Circle(this: any, color: string, r: number) { Shape.call(this, color); this.radius = r; }
Circle.prototype = Object.create(Shape.prototype, {
  constructor: { value: Circle, writable: true, configurable: true },
  area: { value: function (this: any) { return Math.PI * this.radius ** 2; }, writable: true, configurable: true },
});
const circ: any = new (Circle as any)("red", 5);
console.log("circle=" + circ.describe() + "," + Math.round(circ.area()) + "," + (circ instanceof Circle) + "," + (circ instanceof (Shape as any)));

// the prototype chain of a class hierarchy, walked by hand
class A0 { methodA() { return "A"; } }
class B0 extends A0 { methodB() { return "B"; } }
class C0 extends B0 { methodC() { return "C"; } }
const chain: string[] = [];
let walk: any = Object.getPrototypeOf(new C0());
while (walk) {
  if (walk.constructor && walk.constructor.name) chain.push(walk.constructor.name);
  walk = Object.getPrototypeOf(walk);
}
console.log("class_chain=" + chain.join("->"));
function protoDepth(o: any): number {
  let depth = 0;
  let p = Object.getPrototypeOf(o);
  while (p !== null) { depth++; p = Object.getPrototypeOf(p); }
  return depth;
}
console.log("depth=" + protoDepth({}) + "," + protoDepth([]) + "," + protoDepth(new Map()) + "," + protoDepth(Object.create(null)));

// ---------- 6. Inherited setters and read-only inheritance ----------
const store: string[] = [];
const sproto: any = {};
Object.defineProperty(sproto, "p", {
  get(this: any) { return "get<" + (this.tag || "?") + ">"; },
  set(this: any, v: string) { store.push(v + "@" + (this.tag || "?")); this._backing = v; },
  enumerable: true,
  configurable: true,
});
Object.defineProperty(sproto, "readonly", { get() { return "RO"; }, enumerable: true, configurable: true });
const sc: any = Object.create(sproto);
sc.tag = "CHILD";
sc.p = "v1";
console.log("setter_store=" + store.join(",") + ",own_p=" + Object.hasOwn(sc, "p") + ",backing=" + sc._backing + ",read=" + sc.p);
console.log("setter_keys=" + Object.keys(sc).join("|"));
const so: any = Object.create(sproto);
so.tag = "OTHER";
so.p = "v2";
console.log("setter_second=" + store.join(",") + ",other=" + so._backing + ",child=" + sc._backing);
console.log("getter_only_write=" + Reflect.set(sc, "readonly", "x") + ",own=" + Object.hasOwn(sc, "readonly") + ",read=" + sc.readonly);
Object.defineProperty(sc, "p", { value: "OWN", writable: true, enumerable: true, configurable: true });
console.log("shadowed=" + sc.p + ",write=" + Reflect.set(sc, "p", "OWN2") + ":" + sc.p + ",store=" + store.length);
console.log("other_still_accessor=" + so.p);
delete sc.p;
console.log("after_delete=" + sc.p + ",write=" + Reflect.set(sc, "p", "v3") + ",store=" + store.join(","));

const midS: any = Object.create(sproto);
const deepS: any = Object.create(midS);
deepS.tag = "DEEP";
deepS.p = "v4";
console.log("deep_setter=" + store[store.length - 1] + ",deep_own=" + Object.hasOwn(deepS, "_backing") + ",mid_own=" + Object.hasOwn(midS, "_backing"));

// inherited read-only data properties refuse writes from a child, in any mode
const frozenBase = Object.freeze({ ro: 1 });
const frozenChild: any = Object.create(frozenBase);
console.log("inherited_frozen_write=" + Reflect.set(frozenChild, "ro", 2) + ",read=" + frozenChild.ro + ",own=" + Object.hasOwn(frozenChild, "ro"));
console.log("inherited_new_key=" + Reflect.set(frozenChild, "fresh", 5) + ",own=" + Object.hasOwn(frozenChild, "fresh"));
const nwProto: any = {};
Object.defineProperty(nwProto, "nw", { value: 10, writable: false });
const nwChild: any = Object.create(nwProto);
const nwGrand: any = Object.create(nwChild);
console.log("inherited_nonwritable=" + Reflect.set(nwChild, "nw", 3) + "," + Reflect.set(nwGrand, "nw", 4) + ",read=" + nwGrand.nw);
Object.defineProperty(nwChild, "nw", { value: 7, writable: true, configurable: true });
console.log("shadow_then_write=" + Reflect.set(nwChild, "nw", 8) + ",read=" + nwChild.nw + ",grand=" + nwGrand.nw);
const getterOnly: any = {};
Object.defineProperty(getterOnly, "g", { get() { return 1; } });
const getterChild: any = Object.create(getterOnly);
console.log("getter_child_write=" + Reflect.set(getterChild, "g", 2) + ",own=" + Object.hasOwn(getterChild, "g"));

// a write through an inherited setter, via Object.assign or spread
const dest: any = Object.create(sproto);
dest.tag = "DEST";
Object.assign(dest, { p: "v5", z: 1 });
console.log("assign_setter=" + store[store.length - 1] + ",own=" + Object.getOwnPropertyNames(dest).join("|"));
const spreadDest: any = { ...({ p: "v6" }) };
console.log("spread_no_setter=" + store[store.length - 1] + ",own=" + Object.getOwnPropertyNames(spreadDest).join("|") + ":" + spreadDest.p);

// the prototype's __proto__ accessor is inherited by ordinary objects but not by null-prototype ones
console.log("inherits_accessor=" + ("__proto__" in {}) + ",bare=" + ("__proto__" in Object.create(null)));

// ---------- 7. Own-key bookkeeping over a long chain ----------
const chainRoot: any = { r: 1 };
const chainMid: any = Object.create(chainRoot);
chainMid.m = 2;
const chainLeaf: any = Object.create(chainMid);
chainLeaf.l = 3;
const seen: string[] = [];
for (const k in chainLeaf) seen.push(k);
console.log("chain_forin=" + seen.join("|") + ",keys=" + Object.keys(chainLeaf).join("|"));
console.log("chain_values=" + chainLeaf.r + chainLeaf.m + chainLeaf.l + ",own_r=" + Object.hasOwn(chainLeaf, "r"));

// ---------- 8. Object.create with descriptors, and a dynamic prototype swap ----------
const animal = { breathe() { return this.name + " breathes"; } };
const dogObj: any = Object.create(animal, {
  name: { value: "Rex", writable: true, enumerable: true, configurable: true },
  breed: { value: "Lab", writable: false, enumerable: true, configurable: false },
  _age: { value: 3, writable: true, enumerable: false, configurable: true },
});
console.log("dog=" + dogObj.breathe() + "," + Object.keys(dogObj).join("|") + ",proto_ok=" + (Object.getPrototypeOf(dogObj) === animal));
const swapBase1 = { greet() { return "hello from base1"; } };
const swapBase2 = { greet() { return "hello from base2"; } };
const swapping: any = Object.create(swapBase1);
const before = swapping.greet();
Object.setPrototypeOf(swapping, swapBase2);
console.log("swap=" + before + " -> " + swapping.greet());
