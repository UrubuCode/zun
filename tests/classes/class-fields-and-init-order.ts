// Field semantics: declaration-order instance initialisation, what an
// initialiser can see, define-not-set (fields shadow inherited setters and
// read-only properties), per-instance fresh values, derived fields after
// super(), static fields and static blocks in one sequence, and the
// definition-time evaluation of computed keys and heritage expressions.

function hasOwn(o: object, k: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(o, k);
}

// --- declaration-order initialisation ---
const order: string[] = [];
function mark(tag: string, value: number): number {
  order.push(tag);
  return value;
}

class Rec {
  a: number = mark("a", 1);
  b: number = mark("b", this.a + 1);
  c: number = mark("c", this.a + this.b);
  d: string = "a=" + this.a + ",b=" + this.b + ",c=" + this.c;
  e: number;
  constructor(seed: number) {
    order.push("ctor");
    this.e = this.c + seed;
  }
}
const rec = new Rec(10);
console.log("order=" + order.join("|"));
console.log("values=" + rec.a + "," + rec.b + "," + rec.c + "," + rec.d + "," + rec.e);
console.log("rec-keys=" + Object.keys(rec).join(","));

// A field with no initialiser still exists, as undefined, and keeps its slot.
class Sparse {
  x: number = 1;
  y: number | undefined;
  z: number = 3;
}
const sparse: any = new Sparse();
console.log("sparse-keys=" + Object.keys(sparse).join(","));
console.log("sparse-y=" + String(sparse.y) + " own=" + hasOwn(sparse, "y") + " json=" + JSON.stringify(sparse));

// Object-valued initialisers run once per instance.
class Owner {
  items: number[] = [];
  constructor(n: number) {
    this.items.push(n);
  }
}
const o1 = new Owner(1);
const o2 = new Owner(2);
console.log("owners=" + o1.items.join(",") + " / " + o2.items.join(",") + " shared=" + (o1.items === o2.items));

// An own field shadows an inherited method of the same name.
class Shadowed {
  run(): string {
    return "method";
  }
}
class ShadowField extends Shadowed {
  run: () => string = () => "field";
}
const sf = new ShadowField();
console.log("shadow=" + sf.run() + " via-proto=" + Shadowed.prototype.run.call(sf) + " own=" + hasOwn(sf, "run"));

// --- what an initialiser can see ---
const outer = "outer-value";
const built: string[] = [];
class Scope {
  viaMethod: string = this.stamp("first");
  fromEarlier: string = "sees:" + this.viaMethod;
  fromLater: string = "later-is:" + String((this as any).declaredLast);
  fromClass: string = "class:" + Scope.label;
  fromOuter: string = outer;
  fromPrivate: string = this.#secret();
  declaredLast: string = "last";

  static label: string = "N";

  stamp(tag: string): string {
    built.push(tag);
    return "stamp-" + tag;
  }
  #secret(): string {
    return "private-ok";
  }
}
const sc1 = new Scope();
const sc2 = new Scope();
console.log("scope=" + sc1.viaMethod + "|" + sc1.fromEarlier + "|" + sc1.fromLater + "|" + sc1.fromClass + "|" + sc1.fromOuter + "|" + sc1.fromPrivate + "|" + sc1.declaredLast);
console.log("scope-built=" + built.join(","));
console.log("scope-keys=" + Object.keys(sc2).join(","));
const fieldDesc: any = Object.getOwnPropertyDescriptor(sc1, "viaMethod");
const methodDesc: any = Object.getOwnPropertyDescriptor(Scope.prototype, "stamp");
console.log("field-desc=w" + fieldDesc.writable + ",e" + fieldDesc.enumerable + ",c" + fieldDesc.configurable);
console.log("method-desc=w" + methodDesc.writable + ",e" + methodDesc.enumerable + ",c" + methodDesc.configurable);
console.log("method-on-instance=" + hasOwn(sc1, "stamp"));

// A derived initialiser runs after super() and sees what the base wrote.
class Base2 {
  seed: number = 1;
  constructor() {
    this.seed = 5;
    (this as any).fromCtor = "ctor-wrote";
  }
}
class Child2 extends Base2 {
  observed: string = "seed:" + this.seed + ",ctor:" + String((this as any).fromCtor);
  doubled: number = this.seed * 2;
}
const c2 = new Child2();
console.log("child=" + c2.observed + " doubled=" + c2.doubled + " keys=" + Object.keys(c2).join(","));

// Arrow initialisers capture the instance and are per-instance functions.
class Counter2 {
  n: number = 0;
  bump: () => number = () => ++this.n;
}
const cA = new Counter2();
const cB = new Counter2();
const detached = cA.bump;
console.log("arrow=" + detached() + "," + detached() + " other=" + cB.bump() + " distinct=" + (cA.bump !== cB.bump));
console.log("arrow-own=" + hasOwn(cA, "bump") + " name=" + cA.bump.name + " n=" + cA.n + "," + cB.n);

// --- define-not-set: fields do not call inherited setters ---
const calls: string[] = [];
class SetterBase {
  get shadowed(): string {
    return "base-getter";
  }
  set shadowed(v: string) {
    calls.push("setter:" + v);
  }
}
class FieldOver extends SetterBase {
  shadowed: string = "field-value";
}
class AssignOver extends SetterBase {
  constructor() {
    super();
    (this as any).shadowed = "assigned";
  }
}
const fo: any = new FieldOver();
console.log("field-over=" + fo.shadowed + " own=" + hasOwn(fo, "shadowed") + " calls=" + calls.length);
const foDesc: any = Object.getOwnPropertyDescriptor(fo, "shadowed");
console.log("field-over-desc=w" + foDesc.writable + ",e" + foDesc.enumerable + ",c" + foDesc.configurable);
const ao: any = new AssignOver();
console.log("assign-over=" + ao.shadowed + " own=" + hasOwn(ao, "shadowed") + " calls=" + calls.join("|"));

// A non-writable inherited data property: the field defines over it silently,
// while an ordinary assignment in strict code throws.
const frozenProto: any = {};
Object.defineProperty(frozenProto, "locked", { value: "proto", writable: false, enumerable: false, configurable: false });
class OverLocked {
  locked: string = "field";
}
Object.setPrototypeOf(OverLocked.prototype, frozenProto);
const ol: any = new OverLocked();
console.log("locked-field=" + ol.locked + " own=" + hasOwn(ol, "locked"));

class AssignLocked {
  result: string;
  constructor() {
    try {
      (this as any).locked = "assigned";
      this.result = "no-throw";
    } catch (e: any) {
      this.result = e.constructor.name;
    }
  }
}
Object.setPrototypeOf(AssignLocked.prototype, frozenProto);
const al: any = new AssignLocked();
console.log("locked-assign=" + al.result + " value=" + al.locked);

// A static field defines on the constructor, shadowing an inherited accessor.
class SBase2 {
  static get s(): string {
    return "sbase";
  }
  static set s(v: string) {
    calls.push("static-setter:" + v);
  }
}
calls.length = 0;
class SDerived2 extends SBase2 {
  static s: string = "sfield";
}
console.log("static-field=" + SDerived2.s + " base=" + SBase2.s + " calls=" + calls.length + " own=" + hasOwn(SDerived2, "s"));

// Field names that look special are ordinary instance properties.
class OddNames {
  constructorTag: string = "ct";
  prototype: string = "p";
}
const on: any = new OddNames();
console.log("odd-keys=" + Object.keys(on).join(",") + " prototype=" + on.prototype + " ctor=" + on.constructor.name);

// --- static fields, static blocks and private statics in one sequence ---
const seq: string[] = [];
class Sequence {
  static a = (seq.push("a"), 1);
  static {
    seq.push("block1:" + this.a);
    this.a += 2;
  }
  static b = (seq.push("b"), this.a * 2);
  static {
    seq.push("block2:" + this.b);
  }
  static #hidden = 3;
  static c = (seq.push("c:" + Sequence.#hidden), Sequence.#hidden + 1);
  static {
    Sequence.#hidden += 10;
    seq.push("block3:" + Sequence.#hidden);
  }
  static get hiddenNow(): number {
    return Sequence.#hidden;
  }
}
console.log("seq=" + seq.join("|"));
console.log("static-values=" + Sequence.a + "," + Sequence.b + "," + Sequence.c + "," + Sequence.hiddenNow);

class Counter3 {
  static first = 2;
  static second = this.first * 3;
  static third = Counter3.second + 1;
}
console.log("static-chain=" + Counter3.first + "," + Counter3.second + "," + Counter3.third + " keys=" + Object.keys(Counter3).join(","));

const thrown: string[] = [];
try {
  class Boom {
    static a = thrown.push("a");
    static {
      thrown.push("block");
      throw new Error("boom");
    }
    static b = thrown.push("b");
  }
  console.log("boom-class=" + typeof Boom);
} catch (e: any) {
  console.log("boom=" + e.message);
}
console.log("boom-log=" + thrown.join(","));

// --- definition-time evaluation: computed keys and heritage ---
const keyLog: string[] = [];
function k(name: string): string {
  keyLog.push("key:" + name);
  return name;
}
const objKey = {
  toString(): string {
    keyLog.push("toString");
    return "coerced";
  },
};
const sym = Symbol("s");
class Timed {
  [k("m1")](): string {
    return "m1";
  }
  [k("f1")]: string = (keyLog.push("init:f1"), "f1");
  static [k("sf1")]: string = (keyLog.push("init:sf1"), "sf1");
  get [k("g1")](): string {
    return "g1";
  }
  static {
    keyLog.push("static-block");
  }
  [objKey as any]: string = (keyLog.push("init:coerced"), "coerced-value");
  [sym](): string {
    return "sym";
  }
  // The same key twice: both expressions run, the last definition wins.
  [k("dup")](): string {
    return "first";
  }
  [k("dup")](): string {
    return "second";
  }
  static [k("sf2")]: string = (keyLog.push("init:sf2"), "sf2");
}
console.log("define-order=" + keyLog.join("|"));
keyLog.length = 0;

const tm: any = new Timed();
console.log("instance-order=" + keyLog.join("|"));
console.log("timed=" + tm.m1() + "," + tm.f1 + "," + tm.g1 + "," + tm.dup() + "," + tm.coerced + "," + tm[sym]());
console.log("timed-statics=" + (Timed as any).sf1 + "," + (Timed as any).sf2);
console.log("timed-proto=" + Object.getOwnPropertyNames(Timed.prototype).join(",") + " symbols=" + Object.getOwnPropertySymbols(Timed.prototype).length);
console.log("timed-keys=" + Object.keys(tm).join(","));
keyLog.length = 0;
const tm2: any = new Timed();
console.log("second-instance=" + keyLog.join("|") + " f1=" + tm2.f1);

// A throwing computed key aborts the class definition; earlier keys already ran.
const log2: string[] = [];
function k2(name: string): string {
  log2.push(name);
  return name;
}
try {
  class Bad {
    [k2("ok")](): number {
      return 1;
    }
    [(() => {
      log2.push("boom");
      throw new RangeError("key");
    })()](): number {
      return 2;
    }
    [k2("never")](): number {
      return 3;
    }
  }
  console.log("bad=no-throw");
} catch (e: any) {
  console.log("bad=" + e.constructor.name);
}
console.log("bad-log=" + log2.join("|"));

// The heritage expression is evaluated once, at definition time.
let heritageCalls = 0;
class Root {
  value(): number {
    return 6;
  }
}
function heritage(): typeof Root {
  heritageCalls++;
  return Root;
}
class HeritageChild extends heritage() {}
new HeritageChild();
new HeritageChild();
console.log("heritage-calls=" + heritageCalls + " value=" + new HeritageChild().value());

// It runs before any computed key or static element of the same body.
const heritageOrder: string[] = [];
function h2(): typeof Root {
  heritageOrder.push("heritage");
  return Root;
}
class Ordered extends h2() {
  [(heritageOrder.push("key"), "m")](): string {
    return "m";
  }
  static {
    heritageOrder.push("static-block");
  }
}
console.log("heritage-order=" + heritageOrder.join("|") + " m=" + (new Ordered() as any).m());

// --- own-key order and for-in on an instance ---
class KeyOrder {
  b: string = "b";
  a: string = "a";
  constructor() {
    (this as any)["10"] = "ten";
    (this as any)["2"] = "two";
    (this as any).z = "z";
    (this as any)["-1"] = "neg";
    (this as any)["01"] = "leading-zero";
    (this as any)["1.5"] = "frac";
    (this as any)[0] = "zero";
    (this as any)[Symbol.for("s1")] = "sym";
  }
  method(): string {
    return "m";
  }
}
const ko: any = new KeyOrder();
console.log("ko-keys=" + Object.keys(ko).join(","));
console.log("ko-names=" + Object.getOwnPropertyNames(ko).join(",") + " symbols=" + Object.getOwnPropertySymbols(ko).length);
console.log("ko-json=" + JSON.stringify(ko));
console.log("ko-entries=" + Object.entries(ko).map((p) => p[0] + ":" + p[1]).join("|"));
console.log("ko-spread=" + Object.keys({ ...ko }).join(",") + " assign=" + Object.keys(Object.assign({}, ko)).join(","));

// Delete and re-add: a string key moves to the end; an integer key keeps its slot.
delete ko.a;
ko.a = "a-again";
delete ko["2"];
ko["2"] = "two-again";
console.log("ko-redo=" + Object.keys(ko).join(","));

const forIn: string[] = [];
for (const k in ko) forIn.push(k);
console.log("ko-for-in=" + forIn.join(",") + " has-method=" + forIn.includes("method"));

// An enumerable property on the prototype appears in for-in after the own keys.
(KeyOrder.prototype as any).inherited = "from-proto";
const forIn2: string[] = [];
for (const k in ko) forIn2.push(k + (hasOwn(ko, k) ? "" : "(inherited)"));
console.log("ko-for-in2=" + forIn2.join(",") + " keys-unchanged=" + Object.keys(ko).length);

// A shadowing own key is visited once, at its own position.
ko.inherited = "own-wins";
const forIn3: string[] = [];
for (const k in ko) forIn3.push(k);
console.log("ko-shadow=" + forIn3.filter((k) => k === "inherited").length + " value=" + ko.inherited);

// A non-enumerable own property is invisible to keys and for-in, not to names.
Object.defineProperty(ko, "hidden", { value: 1, enumerable: false, writable: true, configurable: true });
const forIn4: string[] = [];
for (const k in ko) forIn4.push(k);
console.log("ko-hidden=" + Object.keys(ko).includes("hidden") + "," + forIn4.includes("hidden") + "," + Object.getOwnPropertyNames(ko).includes("hidden") + " value=" + ko.hidden);

// --- base constructor reads derived members; derived fields land after super ---
const csf: string[] = [];
class CsBase {
  x = (csf.push("base-field"), "base");
  constructor() {
    csf.push("base-ctor:" + this.x);
  }
}
class CsDerived extends CsBase {
  x = (csf.push("derived-field"), "derived");
  y = (csf.push("derived-y:" + this.x), this.x + "-y");
  constructor() {
    csf.push("before-super");
    super();
    csf.push("after-super:" + this.x + ":" + this.y);
  }
}
const cs = new CsDerived();
console.log("super-field=" + cs.x + ":" + cs.y + " log=" + csf.join("|"));

const bst: string[] = [];
class BaseReads {
  constructor() {
    bst.push("field=" + String((this as any).later));
    bst.push("static=" + String((this.constructor as any).ready));
  }
}
class DerivedReads extends BaseReads {
  later: string = "installed-after";
  static ready: string = "static-ready";
}
const dr = new DerivedReads();
console.log("base-reads=" + bst.join("|") + " later=" + dr.later);

// A base constructor that writes through an accessor: the derived setter runs,
// then the derived field defines over whatever the setter stored.
const acc: string[] = [];
class AccBase {
  constructor() {
    (this as any).value = "written-by-base";
    acc.push("base-wrote:" + String((this as any).value));
  }
}
class AccDerived extends AccBase {
  stored: string = "field-init";
  set value(v: string) {
    acc.push("setter:" + v);
    this.stored = "via-setter:" + v;
  }
  get value(): string {
    return "getter(" + this.stored + ")";
  }
}
const ad = new AccDerived();
console.log("acc=" + acc.join("|") + " stored=" + ad.stored + " value=" + ad.value + " keys=" + Object.keys(ad).join(",") + " own-value=" + hasOwn(ad, "value"));

// Reflect.construct with a newTarget that is not the class still runs its fields.
class WBase {
  which: string;
  constructor() {
    const nt: any = new.target;
    this.which = nt.name;
  }
}
class WLeaf extends WBase {}
class WithField extends WBase {
  marker: string = "field";
}
const odd: any = Reflect.construct(WithField, [], WLeaf);
console.log("odd=" + odd.which + " " + odd.marker + " proto=" + (Object.getPrototypeOf(odd) === WLeaf.prototype) + " keys=" + Object.keys(odd).join(","));
