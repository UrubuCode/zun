// Static members and the two inheritance chains a class hierarchy has: statics
// are inherited through the constructor chain (read until shadowed, `this` is
// the receiver), static super calls, static fields/blocks with `this`, the
// instance chain vs the static chain, `extends null`, Object.create and
// setPrototypeOf rewiring, and the descriptor shapes of prototype/constructor.

function hasOwn(o: object, k: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(o, k);
}

// --- static methods through `this`, and static super ---
class Shape {
  static count: number = 0;
  static kind(): string {
    return "shape";
  }
  static describe(): string {
    return "kind=" + this.kind();
  }
  static make(n: number): string {
    return this.kind() + ":" + n;
  }
  static bump(): number {
    this.count = this.count + 1;
    return this.count;
  }
}
class Circle extends Shape {
  static kind(): string {
    return "circle";
  }
  static describe(): string {
    return "circle<" + super.describe() + ">";
  }
}
class SmallCircle extends Circle {
  static kind(): string {
    return "small-" + super.kind();
  }
  static describe(): string {
    return "small<" + super.describe() + ">";
  }
}
console.log("kinds=" + Shape.kind() + "," + Circle.kind() + "," + SmallCircle.kind());
console.log("describe=" + Shape.describe() + " | " + Circle.describe() + " | " + SmallCircle.describe());
console.log("make=" + Circle.make(3) + " " + SmallCircle.make(4) + " borrowed=" + Shape.describe.call(Circle));

console.log("count0=" + Shape.count + "," + Circle.count + " own-circle=" + hasOwn(Circle, "count"));
console.log("circle-bump=" + Circle.bump() + " counts=" + Circle.count + "," + Shape.count + " own-circle=" + hasOwn(Circle, "count"));
console.log("shape-bump=" + Shape.bump() + " counts=" + Shape.count + "," + Circle.count);

// --- statics are inherited, not copied; `this` picks the receiver ---
class Base {
  static count: number = 0;
  static label: string = "base";
  static tally: string[] = [];
  static bump(): string {
    (this as any).count = (this as any).count + 1;
    return (this as any).label + ":" + (this as any).count;
  }
  static whoAmI(): string {
    return (this as any).name;
  }
  static get computed(): string {
    return "computed-of-" + (this as any).label;
  }
}
class Sub extends Base {}
class SubSub extends Sub {
  static label: string = "subsub";
}
console.log("base-own-count=" + hasOwn(Base, "count") + " sub-own-count=" + hasOwn(Sub, "count"));
console.log("sub-reads=" + Sub.count + "," + Sub.label + " subsub=" + SubSub.count + "," + SubSub.label + " own-label=" + hasOwn(SubSub, "label"));
console.log("bump=" + Base.bump() + " " + Sub.bump() + " " + Sub.bump() + " " + SubSub.bump());
console.log("after=" + Base.count + "," + Sub.count + "," + SubSub.count);
console.log("who=" + Base.whoAmI() + "," + Sub.whoAmI() + "," + SubSub.whoAmI() + " detached=" + (Base.whoAmI as any).call({ name: "borrowed" }));
console.log("computed=" + Base.computed + "," + Sub.computed + "," + SubSub.computed);

Base.tally.push("from-base");
Sub.tally.push("from-sub");
console.log("tally=" + Base.tally.join(",") + " same=" + (Base.tally === Sub.tally));
(SubSub as any).tally = ["own"];
(SubSub as any).tally.push("more");
console.log("subsub-tally=" + (SubSub as any).tally.join(",") + " base-tally=" + Base.tally.join(","));

console.log("delete-shadow=" + Reflect.deleteProperty(SubSub, "label") + " label=" + SubSub.label + " own=" + hasOwn(SubSub, "label"));

// Static field initialisers and static blocks run with `this` = their own class.
class Derived extends Base {
  static seed: string = (this as any).label + "-seed";
  static self: boolean = (this as any) === Derived;
  static viaSuper: string = "n/a";
  static {
    (this as any).viaSuper = "block:" + (this as any).seed + ":" + (this as any).name;
  }
}
console.log("derived=" + Derived.seed + " " + Derived.self + " " + Derived.viaSuper);
console.log("derived-own=" + Object.getOwnPropertyNames(Derived).sort().join(","));

class WithSuper extends Base {
  static label: string = "with-super";
  static both(): string {
    return (this as any).label + "/" + (super.whoAmI as any).call(Base) + "/" + super.computed;
  }
}
console.log("with-super=" + WithSuper.both() + " computed=" + WithSuper.computed);

// Static getters and setters are looked up the same chain.
class SAcc {
  static #v = 1;
  static get v(): number {
    return SAcc.#v;
  }
  static set v(n: number) {
    SAcc.#v = n * 10;
  }
}
class SAccChild extends SAcc {}
SAccChild.v = 2;
console.log("sacc=" + SAcc.v + " child=" + SAccChild.v + " own=" + hasOwn(SAccChild, "v"));

// Static methods can be inherited under a shadowing field of the same name.
class SInBase {
  static value = 4;
  static read(): number {
    return this.value;
  }
}
class SInChild extends SInBase {
  static value = 9;
}
console.log("static-lookup=" + SInBase.read() + "," + SInChild.read() + " parent=" + (Object.getPrototypeOf(SInChild) === SInBase));

// Field-shaped statics: the derived value shadows, super reads the base.
class SuperVal {
  static value = 2;
  static calc(n: number): number {
    return this.value * n;
  }
}
class SuperValChild extends SuperVal {
  static value = 5;
  static calc(n: number): number {
    return super.calc(n) + 1;
  }
}
console.log("super-val=" + SuperValChild.calc(3) + "," + SuperVal.calc(3));

// --- counter hierarchy with accessors and an overridden instance method ---
class BaseCounter {
  label: string;
  _value: number;
  constructor(label: string, value: number) {
    this.label = label;
    this._value = value;
  }
  get value(): number {
    return this._value;
  }
  set value(next: number) {
    this._value = next;
  }
  bump(step: number): number {
    this._value += step;
    return this._value;
  }
  static seed(label: string): BaseCounter {
    return new BaseCounter(label, 1);
  }
}
class DoubleCounter extends BaseCounter {
  bump(step: number): number {
    return super.bump(step * 2);
  }
  info(): string {
    return this.label + ":" + this.value;
  }
}
const ac = BaseCounter.seed("alpha");
console.log("seed=" + ac.value + " bump=" + ac.bump(4));
ac.value = 12;
console.log("seed-after=" + ac.value);
const bc = new DoubleCounter("beta", 5);
console.log("double=" + bc.bump(3) + " " + bc.info() + " " + (bc instanceof BaseCounter) + "," + (bc instanceof DoubleCounter));

// --- instance chain and static chain ---
class Root {
  r(): string {
    return "r";
  }
  static sr(): string {
    return "sr";
  }
}
class Mid extends Root {
  m(): string {
    return "m";
  }
  static sm(): string {
    return "sm";
  }
}
class Leaf extends Mid {
  l(): string {
    return "l";
  }
}
const leaf = new Leaf();
const chain: string[] = [];
let cur: any = leaf;
for (let i = 0; i < 8; i++) {
  cur = Object.getPrototypeOf(cur);
  if (cur === null) {
    chain.push("null");
    break;
  }
  chain.push(
    cur === Leaf.prototype
      ? "Leaf.prototype"
      : cur === Mid.prototype
        ? "Mid.prototype"
        : cur === Root.prototype
          ? "Root.prototype"
          : cur === Object.prototype
            ? "Object.prototype"
            : "other",
  );
}
console.log("instance-chain=" + chain.join(">"));

const statics: string[] = [];
let sc: any = Leaf;
for (let i = 0; i < 8; i++) {
  sc = Object.getPrototypeOf(sc);
  if (sc === null) {
    statics.push("null");
    break;
  }
  statics.push(
    sc === Mid
      ? "Mid"
      : sc === Root
        ? "Root"
        : sc === Function.prototype
          ? "Function.prototype"
          : sc === Object.prototype
            ? "Object.prototype"
            : "other",
  );
}
console.log("static-chain=" + statics.join(">"));
console.log("leaf-methods=" + leaf.l() + leaf.m() + leaf.r() + " statics=" + Leaf.sr() + Leaf.sm());
console.log("is-proto-of=" + Root.prototype.isPrototypeOf(leaf) + "," + Mid.prototype.isPrototypeOf(leaf) + "," + Leaf.prototype.isPrototypeOf(new Mid()));
console.log("root-statics=" + (Object.getPrototypeOf(Root) === Function.prototype) + "," + (Object.getPrototypeOf(Root.prototype) === Object.prototype));

const pd: any = Object.getOwnPropertyDescriptor(Leaf, "prototype");
const cd: any = Object.getOwnPropertyDescriptor(Leaf.prototype, "constructor");
console.log("prototype-desc=w" + pd.writable + ",e" + pd.enumerable + ",c" + pd.configurable);
console.log("constructor-desc=w" + cd.writable + ",e" + cd.enumerable + ",c" + cd.configurable);
console.log("leaf-own-names=" + Object.getOwnPropertyNames(Leaf).sort().join(","));

// `extends null`: the instance chain stops at the class prototype itself.
class Bare extends null {
  constructor() {
    return Object.create(Bare.prototype);
  }
}
const bare: any = new Bare();
console.log("bare=" + (Object.getPrototypeOf(bare) === Bare.prototype) + "," + (Object.getPrototypeOf(Bare.prototype) === null) + "," + (Object.getPrototypeOf(Bare) === Function.prototype));
console.log("bare-tostring=" + typeof bare.toString + " instanceof=" + (bare instanceof Bare));

const rewired: any = Object.create(Root.prototype);
console.log("rewired=" + rewired.r() + " " + (rewired instanceof Root) + "," + (rewired instanceof Mid) + " ctor=" + (rewired.constructor === Root));

// setPrototypeOf on the static side moves static lookup, not instance lookup.
class Donor {
  static gift(): string {
    return "gift";
  }
}
class Taker {}
Object.setPrototypeOf(Taker, Donor);
console.log("taker-gift=" + (Taker as any).gift() + " instance-chain=" + (Object.getPrototypeOf(Taker.prototype) === Object.prototype) + " instance-gift=" + typeof (new Taker() as any).gift);
