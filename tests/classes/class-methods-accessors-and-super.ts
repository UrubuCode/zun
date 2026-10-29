// Methods, accessors and super: detached `this`, arrow vs function receivers,
// method/accessor/constructor descriptors and names, getter/setter pairs and
// overrides, getter-only strictness, super property reads and writes (receiver
// split), super chains across levels, and [[HomeObject]] rules.

function hasOwn(o: object, k: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(o, k);
}

class WriterUtil {
  static write(target: any, key: string, value: any): string {
    try {
      target[key] = value;
      return "no-throw";
    } catch (e: any) {
      return e.constructor.name;
    }
  }
}

// --- detached methods lose `this`; arrow fields keep it ---
class Counter {
  n: number;
  constructor(n: number) {
    this.n = n;
  }
  get(): number {
    return this.n;
  }
  safe(): string {
    return this === undefined ? "no-this" : "has-this:" + this.n;
  }
  arrow: () => string = () => "arrow:" + this.n;
}
const c = new Counter(7);
console.log("attached=" + c.get() + " safe=" + c.safe());

const m = c.get;
try {
  m();
  console.log("detached=no-throw");
} catch (e: any) {
  console.log("detached=" + e.constructor.name);
}
const s = c.safe;
console.log("detached-safe=" + s());
console.log("call=" + m.call(c) + " apply=" + m.apply(c, []) + " bind=" + m.bind(c)());
console.log("bind-twice=" + m.bind(c).bind(new Counter(99))());
console.log("borrow=" + m.call({ n: 42 }) + " other=" + m.call(new Counter(5)));

const arrowFn = c.arrow;
console.log("arrow-detached=" + arrowFn() + " arrow-call-other=" + arrowFn.call(new Counter(1)));

try {
  console.log("cb-method=" + [1].map(c.get).join(","));
} catch (e: any) {
  console.log("cb-method=" + e.constructor.name);
}
console.log("cb-arrow=" + [1].map(c.arrow).join(","));
console.log("cb-bound=" + [1].map(c.get.bind(c)).join(","));
console.log("cb-wrapped=" + [1].map(() => c.get()).join(","));

const pool = [new Counter(11), new Counter(22)];
const pooled = pool[0].get;
try {
  pooled();
  console.log("elem-detached=no-throw");
} catch (e: any) {
  console.log("elem-detached=" + e.constructor.name);
}
console.log("elem-bound=" + pooled.call(pool[1]));

const { get: destructured } = c;
try {
  destructured();
  console.log("destructured=no-throw");
} catch (e: any) {
  console.log("destructured=" + e.constructor.name);
}

// --- arrow vs function receivers inside methods ---
class ThisBox {
  value: number;
  constructor(v: number) {
    this.value = v;
  }
  arrowReader(): () => number {
    return () => this.value;
  }
  functionReader(): () => string {
    return function (this: any): string {
      return typeof this === "undefined" ? "undefined_this" : "has_this";
    };
  }
  selfReader(): () => number {
    const self = this;
    return function (): number {
      return self.value;
    };
  }
  boundReader(): () => number {
    return function (this: ThisBox): number {
      return this.value;
    }.bind(this);
  }
  localReader(): () => number {
    const snapshot = this.value;
    return () => snapshot;
  }
  arrowBumper(): () => number {
    return () => {
      this.value += 1;
      return this.value;
    };
  }
  mapped(): string {
    return [1, 2].map((n) => n + this.value).join(",");
  }
  deepArrow(): () => () => number {
    return () => () => this.value;
  }
  static staticArrow(): string {
    const f = () => (typeof this === "function" ? "ctor" : "other");
    return f();
  }
}
const tb = new ThisBox(10);
console.log("arrow=" + tb.arrowReader()() + " function=" + tb.functionReader()() + " self=" + tb.selfReader()());
console.log("bound=" + tb.boundReader()() + " mapped=" + tb.mapped() + " deep=" + tb.deepArrow()()());
console.log("static-arrow=" + ThisBox.staticArrow());
const arrowR = tb.arrowReader();
const localR = tb.localReader();
tb.value = 20;
console.log("after-mutation arrow=" + arrowR() + " local=" + localR());
const bumper = tb.arrowBumper();
console.log("bump=" + bumper() + "," + bumper() + " field=" + tb.value);
const other = new ThisBox(100);
console.log("instance-bound=" + arrowR() + " other=" + other.arrowReader()());

class Button {
  count = 0;
  click = () => ++this.count;
}
const btn = new Button();
const clickFn = btn.click;
console.log("click=" + clickFn() + "," + clickFn() + " count=" + btn.count);
console.log("click-call=" + clickFn.call({ count: 100 }) + " count=" + btn.count);

// `this` from a base method's arrow follows the subclass instance.
class ArrowBase {
  tag(): () => string {
    return () => this.constructor.name;
  }
}
class ArrowDerived extends ArrowBase {}
console.log("inherited-arrow=" + new ArrowDerived().tag()());

// Passing a receiver explicitly to a method extracted from another class.
class Adder {
  base: number;
  constructor(base: number) {
    this.base = base;
  }
  add(n: number): number {
    return this.base + n;
  }
}
const addM = new Adder(5).add;
console.log("add-call=" + addM.call({ base: 20 }, 3) + "," + addM.call({ base: -2 }, 4));

// --- descriptors and names of methods, accessors and constructors ---
class Shape {
  side: number = 3;
  area(a: number, b: number): number {
    return a * b;
  }
  get width(): number {
    return this.side;
  }
  set width(v: number) {
    this.side = v;
  }
  static make(n: number): Shape {
    const shape = new Shape();
    shape.side = n;
    return shape;
  }
  *rows(): Generator<number> {
    yield this.side;
  }
}

function desc(o: any, k: string): string {
  const d: any = Object.getOwnPropertyDescriptor(o, k);
  if (d === undefined) return "none";
  const isAccessor = d.get !== undefined || d.set !== undefined;
  return (
    (isAccessor ? "accessor" : "data") +
    ",e=" + String(d.enumerable) +
    ",c=" + String(d.configurable) +
    (isAccessor ? ",g=" + String(d.get !== undefined) + ",s=" + String(d.set !== undefined) : ",w=" + String(d.writable))
  );
}
const P: any = Shape.prototype;
console.log("proto-area=" + desc(P, "area"));
console.log("proto-width=" + desc(P, "width"));
console.log("proto-rows=" + desc(P, "rows"));
console.log("proto-ctor=" + desc(P, "constructor"));
console.log("static-make=" + desc(Shape, "make"));
console.log("ctor-prototype=" + desc(Shape, "prototype"));
console.log("ctor-length=" + desc(Shape, "length"));
console.log("ctor-name=" + desc(Shape, "name"));
console.log("proto-names=" + Object.getOwnPropertyNames(P).sort().join(","));
console.log("proto-keys=" + Object.keys(P).length + " ctor-names=" + Object.getOwnPropertyNames(Shape).sort().join(","));

const shape = new Shape();
console.log("inst-keys=" + Object.keys(shape).join(",") + " json=" + JSON.stringify(shape));
console.log("area-name=" + Shape.prototype.area.name + " length=" + Shape.prototype.area.length + " make=" + Shape.make.name);
console.log("class-name=" + Shape.name + " class-length=" + Shape.length);
const wd: any = Object.getOwnPropertyDescriptor(P, "width");
console.log("accessor-names=" + wd.get.name + "|" + wd.set.name + " lengths=" + wd.get.length + "," + wd.set.length);

try {
  new (Shape.prototype.area as any)();
  console.log("new-method=no-throw");
} catch (e: any) {
  console.log("new-method=" + e.constructor.name);
}
try {
  new (wd.get as any)();
  console.log("new-getter=no-throw");
} catch (e: any) {
  console.log("new-getter=" + e.constructor.name);
}
console.log("method-has-prototype=" + ("prototype" in (Shape.prototype.area as any)) + " generator-has-prototype=" + ("prototype" in (Shape.prototype.rows as any)));
console.log("proto-tostring=" + Object.prototype.toString.call(P));

// A shorthand method is non-enumerable-on-class and non-constructible; a
// function-valued object property is enumerable and constructible.
const shorthandDesc: any = Object.getOwnPropertyDescriptor({ m(): number { return 1; } }, "m");
const propDesc: any = Object.getOwnPropertyDescriptor({ m: function (): number { return 1; } }, "m");
console.log("shorthand-enum=" + shorthandDesc.enumerable + " prop-enum=" + propDesc.enumerable);
console.log("shorthand-proto=" + hasOwn(shorthandDesc.value, "prototype") + " prop-proto=" + hasOwn(propDesc.value, "prototype"));
console.log("shorthand-name=" + shorthandDesc.value.name + " prop-name=" + propDesc.value.name);
try {
  new (shorthandDesc.value as any)();
  console.log("new-shorthand=no-throw");
} catch (e: any) {
  console.log("new-shorthand=" + e.constructor.name);
}
console.log("new-prop=" + typeof new (propDesc.value as any)());

class Mixed {
  asMethod(): number {
    return 1;
  }
  asField: () => number = function (): number {
    return 2;
  };
}
const mi: any = new Mixed();
console.log("mixed-own=" + hasOwn(mi, "asMethod") + "," + hasOwn(mi, "asField") + " keys=" + Object.keys(mi).join(","));
console.log("field-proto=" + hasOwn(mi.asField, "prototype") + " field-name=" + JSON.stringify(mi.asField.name) + " method-name=" + Mixed.prototype.asMethod.name);

// --- accessor pairs, overrides and inheritance ---
class Temp {
  _c: number;
  constructor(c: number) {
    this._c = c;
  }
  get celsius(): number {
    return this._c;
  }
  set celsius(v: number) {
    this._c = v < -273 ? -273 : v;
  }
  get fahrenheit(): number {
    return (this._c * 9) / 5 + 32;
  }
  set fahrenheit(v: number) {
    this.celsius = ((v - 32) * 5) / 9;
  }
}
const t = new Temp(0);
console.log("c0=" + t.celsius + " f0=" + t.fahrenheit);
t.celsius = 100;
console.log("c1=" + t.celsius + " f1=" + t.fahrenheit);
t.fahrenheit = 32;
console.log("c2=" + t.celsius + " backing=" + t._c);
t.celsius = -1000;
console.log("clamped=" + t.celsius);
t.fahrenheit = -500;
console.log("clamped-via-f=" + t.celsius);
console.log("own-celsius=" + hasOwn(t, "celsius") + " own-backing=" + hasOwn(t, "_c"));

class Kelvin extends Temp {
  get kelvin(): number {
    return this.celsius + 273;
  }
  set kelvin(v: number) {
    this.celsius = v - 273;
  }
}
const kv = new Kelvin(10);
console.log("kelvin=" + kv.celsius + "," + kv.kelvin + " inherited-f=" + kv.fahrenheit);
kv.kelvin = 300;
console.log("kelvin-after=" + kv.celsius);

class Doubled extends Temp {
  get celsius(): number {
    return this._c * 2;
  }
  set celsius(v: number) {
    this._c = v;
  }
}
const dbl = new Doubled(5);
console.log("doubled-get=" + dbl.celsius);
dbl.celsius = 8;
console.log("doubled-after=" + dbl.celsius + " backing=" + dbl._c);

class ReadOnly {
  _v: number;
  constructor(v: number) {
    this._v = v;
  }
  get v(): number {
    return this._v;
  }
}
const ro = new ReadOnly(7);
console.log("ro-get=" + ro.v + " write=" + WriterUtil.write(ro, "v", 1) + " value=" + ro.v);

// Getter-only accessors refuse assignment in strict code.
class GetterOnly {
  get v(): number {
    return 1;
  }
}
const go = new GetterOnly();
console.log("getter-only-write=" + WriterUtil.write(go, "v", 2) + " value=" + go.v);

// A getter and a setter on different objects of one chain are never merged.
const half: any = {};
Object.defineProperty(half, "half", {
  get(): string {
    return "get-half";
  },
  configurable: true,
});
const halfChild: any = Object.create(half);
console.log("half-strict=" + WriterUtil.write(halfChild, "half", 1));
Object.defineProperty(half, "half", {
  get(): string {
    return "get-half";
  },
  set(this: any, v: any): void {
    this.stored = v;
  },
  configurable: true,
});
halfChild.half = 9;
console.log("half-after=" + halfChild.half + ":" + halfChild.stored);

const lower: any = {};
Object.defineProperty(lower, "split", { get: () => "lower-get", configurable: true });
const upper: any = Object.create(lower);
Object.defineProperty(upper, "split", {
  set(this: any, v: any) {
    this.saw = v;
  },
  configurable: true,
});
console.log("split-read=" + String(upper.split) + " split-write=" + WriterUtil.write(upper, "split", 3) + ":" + upper.saw);

// A pair declared out of order is still one accessor property.
class OutOfOrder {
  set p(v: number) {
    (this as any).store = v * 2;
  }
  get p(): number {
    return (this as any).store;
  }
}
const oo: any = new OutOfOrder();
oo.p = 4;
const ood: any = Object.getOwnPropertyDescriptor(OutOfOrder.prototype, "p");
console.log("out-of-order=" + oo.p + " pair=" + typeof ood.get + ":" + typeof ood.set);

// A getter redeclared later replaces only the get half.
class Redeclared {
  get q(): string {
    return "first";
  }
  set q(v: string) {
    (this as any).seen = v;
  }
  get q(): string {
    return "second";
  }
}
const rd: any = new Redeclared();
const rdd: any = Object.getOwnPropertyDescriptor(Redeclared.prototype, "q");
console.log("redeclared-get=" + rd.q + " set=" + (rdd.set === undefined ? "dropped" : "kept"));
console.log("redeclared-write=" + WriterUtil.write(rd, "q", "x") + " seen=" + rd.seen);

// --- super property reads: [[HomeObject]] and receiver ---
class SBase {
  tag: string = "base";
  get label(): string {
    return "base:" + (this as any).tag;
  }
  set label(v: string) {
    (this as any).tag = "set(" + v + ")";
  }
  get readOnly(): string {
    return "ro";
  }
  greet(): string {
    return "base-greet:" + (this as any).tag;
  }
  static origin(): string {
    return "base-static";
  }
}
class SDerived extends SBase {
  tag2: string = "d";
  viaSuper(): string {
    return super.label;
  }
  viaSuperArrow(): string {
    const f = () => super.label + "|" + super.greet();
    return f();
  }
  writeSuper(v: string): string {
    super.label = v;
    return this.tag;
  }
  ownWrite(v: string): string {
    (this as any).label = v;
    return this.tag;
  }
  breakReadOnly(): string {
    try {
      (this as any).readOnly = "x";
      return "no-throw";
    } catch (e: any) {
      return e.constructor.name;
    }
  }
  static origin(): string {
    return "derived+" + super.origin();
  }
}
const sd = new SDerived();
console.log("via-super=" + sd.viaSuper());
console.log("via-super-arrow=" + sd.viaSuperArrow());
console.log("write-super=" + sd.writeSuper("A"));
console.log("own-write=" + sd.ownWrite("B"));
console.log("read-only=" + sd.breakReadOnly());
console.log("static-super=" + SDerived.origin());
console.log("sd-keys=" + Object.keys(sd).join(","));

// `super.x = v` finds the setter on the home object's prototype but stores on
// the receiver; a plain data write lands as an own property of the instance.
class RBase {
  baseData: string = "base-data";
  get accessor(): string {
    return "base-accessor";
  }
  set accessor(v: string) {
    (this as any).stored = "stored-" + v;
  }
}
(RBase.prototype as any).plainOnProto = "proto-value";
class RDerived extends RBase {
  stored: string = "";
  writePlain(v: string): string {
    super["plainOnProto"] = v;
    return "own=" + hasOwn(this, "plainOnProto") + ",proto=" + (RBase.prototype as any).plainOnProto + ",read=" + (this as any).plainOnProto;
  }
  writeThroughSetter(v: string): string {
    super.accessor = v;
    return "stored=" + this.stored + ",own-accessor=" + hasOwn(this, "accessor");
  }
  viaArrow(): string {
    const f = () => super.accessor;
    return "arrow=" + f();
  }
  readComputed(key: string): string {
    return "computed=" + String(super[key as any]);
  }
}
const rdv = new RDerived();
console.log("plain-write=" + rdv.writePlain("written"));
console.log("proto-after=" + (RBase.prototype as any).plainOnProto + " second=" + String((new RDerived() as any).plainOnProto));
console.log("setter-write=" + rdv.writeThroughSetter("v1"));
console.log("via-arrow=" + rdv.viaArrow());
console.log("computed-accessor=" + rdv.readComputed("accessor") + " computed-plain=" + rdv.readComputed("plainOnProto") + " computed-missing=" + rdv.readComputed("nothing"));

class FrozenUser extends RBase {
  attempt(): string {
    try {
      super["plainOnProto"] = "x";
      return "no-throw";
    } catch (e: any) {
      return e.constructor.name;
    }
  }
}
const fz = new FrozenUser();
Object.freeze(fz);
console.log("frozen-write=" + fz.attempt() + " own=" + hasOwn(fz, "plainOnProto") + " read=" + (fz as any).plainOnProto);

// Reflect.set spells out the same split: target for lookup, receiver for store.
const reflTarget: any = Object.create(RBase.prototype);
const receiver: any = {};
console.log("reflect-set=" + Reflect.set(reflTarget, "plainOnProto", "via-reflect", receiver) + " target-own=" + hasOwn(reflTarget, "plainOnProto") + " receiver-own=" + hasOwn(receiver, "plainOnProto") + " value=" + receiver.plainOnProto);
console.log("reflect-setter=" + Reflect.set(reflTarget, "accessor", "rv", receiver) + " stored=" + receiver.stored);

// A derived getter reads derived state through the receiver.
class Reader extends RBase {
  own: string = "derived-own";
  get accessorPlus(): string {
    return super.accessor + "+" + this.own;
  }
}
console.log("receiver=" + new Reader().accessorPlus);

// --- super chains through several levels ---
const trace: string[] = [];
class L1 {
  describe(): string {
    trace.push("L1-describe");
    return "L1";
  }
  tag(): string {
    return "l1";
  }
}
class L2 extends L1 {
  describe(): string {
    trace.push("L2-describe");
    return "L2[" + super.describe() + "]";
  }
  tag(): string {
    return super.tag() + ">l2";
  }
}
class L3 extends L2 {
  describe(): string {
    trace.push("L3-describe");
    return "L3{" + super.describe() + "}";
  }
  tag(): string {
    return super.tag() + ">l3";
  }
}
const c3 = new L3();
console.log("chain-describe=" + c3.describe() + " trace=" + trace.join("|"));
console.log("chain-tag=" + c3.tag());
trace.length = 0;
console.log("borrowed=" + (L2.prototype.describe as any).call(c3) + " trace=" + trace.join("|"));
console.log("direct-l1=" + L1.prototype.describe.call(c3));

// --- [[HomeObject]]: super follows where the method was defined ---
class HBase {
  who(): string {
    return "Base.who";
  }
}
class HMiddle extends HBase {
  who(): string {
    return "Middle(" + super.who() + ")";
  }
}
class HOther extends HBase {
  who(): string {
    return "Other.who";
  }
  tag(): string {
    return "other-tag";
  }
}
const hm = new HMiddle();
const borrowedWho = HMiddle.prototype.who;
const ho = new HOther();
console.log("borrowed-on-other=" + borrowedWho.call(ho) + " other-tag=" + ho.tag());

class Unrelated {
  tag(): string {
    return "unrelated-tag";
  }
}
(Unrelated.prototype as any).who = HMiddle.prototype.who;
const un: any = new Unrelated();
console.log("copied-who=" + un.who() + " instanceof-base=" + (un instanceof HBase) + " tag=" + un.tag());

// Moving the home object's prototype changes where super resolves.
class Replacement {
  who(): string {
    return "Replacement.who";
  }
}
const savedParent = Object.getPrototypeOf(HMiddle.prototype);
Object.setPrototypeOf(HMiddle.prototype, Replacement.prototype);
console.log("after-reparent=" + hm.who());
Object.setPrototypeOf(HMiddle.prototype, savedParent);
console.log("restored=" + hm.who());

const literal: any = {
  who(): string {
    return "literal(" + super.who() + ")";
  },
};
Object.setPrototypeOf(literal, HBase.prototype);
console.log("literal-base=" + literal.who());
Object.setPrototypeOf(literal, HOther.prototype);
console.log("literal-other=" + literal.who());

// A static super reaches the parent constructor's statics, and a borrowed
// static keeps its home object.
class SS1 {
  static value: number = 2;
  static calc(n: number): number {
    return (this as any).value * n;
  }
}
class SS2 extends SS1 {
  static value: number = 5;
  static calc(n: number): number {
    return super.calc(n) + 1;
  }
}
console.log("static-super=" + SS2.calc(3) + "," + SS1.calc(3) + " borrowed=" + (SS2.calc as any).call(SS1, 3));
console.log("static-parent=" + (Object.getPrototypeOf(SS2) === SS1));

// A static setter reached through super writes onto the derived class.
class SSetBase {
  static get slot(): string {
    return "s-base";
  }
  static set slot(v: string) {
    (this as any).kept = v;
  }
}
class SWriter extends SSetBase {
  static write(v: string): string {
    super.slot = v;
    return "kept=" + String((this as any).kept) + ",own=" + hasOwn(SWriter, "kept") + ",base-own=" + hasOwn(SSetBase, "kept");
  }
}
console.log("static-write=" + SWriter.write("sv") + " read=" + SWriter.slot);

// super[key()](arg()): the key, then the argument, then the call.
const seen: string[] = [];
class KBase {
  run(value: number): number {
    seen.push("method:" + value);
    return value * 2;
  }
}
class KChild extends KBase {
  test(): number {
    const key = () => {
      seen.push("key");
      return "run";
    };
    const arg = () => {
      seen.push("arg");
      return 4;
    };
    return super[key()](arg());
  }
}
console.log("super-key=" + new KChild().test() + " order=" + seen.join(","));
