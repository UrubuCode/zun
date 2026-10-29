// Class expressions and names (anonymous and named, factories, immutable inner
// name binding, inferred names, TDZ in heritage), instanceof (prototype walk,
// bound functions, Symbol.hasInstance and its coercion, refusal of non-callable
// right-hand sides), and polymorphic dispatch over arrays of instances.

function hasOwn(o: object, k: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(o, k);
}

// --- basic class with super, accessors and a rename setter ---
class Box {
  value: string;
  constructor(value: string) {
    this.value = value;
  }
  label(): string {
    return "box:" + this.value;
  }
  get size(): number {
    return this.value.length;
  }
  set rename(next: string) {
    this.value = next;
  }
}
class GiftBox extends Box {
  kind: string;
  constructor(value: string, kind: string) {
    super(value);
    this.kind = kind;
  }
  label(): string {
    return super.label() + ":" + this.kind;
  }
}
const toy = new Box("toy");
console.log("box=" + toy.label() + " size=" + toy.size);
toy.rename = "drone";
console.log("renamed=" + toy.label());
const gift = new GiftBox("book", "gift");
console.log("gift=" + gift.label() + " is-box=" + (gift instanceof Box) + " is-gift=" + (gift instanceof GiftBox));

// --- class expressions ---
const Point = class {
  x: number;
  y: number;
  constructor(x: number, y: number) {
    this.x = x;
    this.y = y;
  }
  sum(): number {
    return this.x + this.y;
  }
  toString(): string {
    return "(" + this.x + "," + this.y + ")";
  }
};
const pt = new Point(2, 3);
console.log("expr=" + pt.sum() + " " + String(pt) + " " + (pt instanceof Point) + " name=" + Point.name + " length=" + Point.length);

const Node2 = class Inner {
  depth: number;
  constructor(depth: number) {
    this.depth = depth;
  }
  self(): string {
    return Inner.name + ":" + this.depth;
  }
  static make(d: number): Inner {
    return new Inner(d);
  }
};
console.log("named=" + new Node2(1).self() + " outer-name=" + Node2.name + " static=" + Node2.make(4).self() + " instance=" + (Node2.make(4) instanceof Node2));

const Base3 = class {
  tag(): string {
    return "base";
  }
};
const Derived3 = class extends Base3 {
  tag(): string {
    return "derived>" + super.tag();
  }
};
const d3 = new Derived3();
console.log("expr-extends=" + d3.tag() + " chain=" + (d3 instanceof Base3) + "/" + (d3 instanceof Derived3));

function makeClass(prefix: string) {
  return class {
    id: number;
    constructor(id: number) {
      this.id = id;
    }
    label(): string {
      return prefix + "-" + this.id;
    }
  };
}
const Alpha = makeClass("a");
const Beta = makeClass("b");
console.log("factory=" + new Alpha(1).label() + " " + new Beta(2).label() + " distinct=" + (Alpha !== Beta) + " cross=" + (new Alpha(1) instanceof Beta));
const factories = [makeClass("x"), makeClass("y")];
const built: string[] = [];
for (let i = 0; i < factories.length; i++) {
  built.push(new factories[i](i).label());
}
console.log("from-array=" + built.join(","));
console.log("iife=" + new (class {
  v(): string {
    return "inline";
  }
})().v());

// The name of an inner class expression is visible inside, not outside.
const Outer = class Inner {
  static ownName(): string {
    return Inner.name;
  }
  make(): any {
    return new Inner();
  }
};
const outerVal = new Outer();
console.log("scope=" + Outer.name + " " + Outer.ownName() + " " + (outerVal.make() instanceof Outer) + " global=" + typeof (globalThis as any).Inner);

// --- the class-name binding inside the body is immutable ---
class Declared {
  static rename(): string {
    try {
      // @ts-expect-error -- the inner class-name binding is const
      Declared = 1 as any;
      return "no-throw";
    } catch (e: any) {
      return e.constructor.name;
    }
  }
  static self(): boolean {
    return Declared === (this as any);
  }
  who(): string {
    return Declared.name;
  }
}
console.log("rename=" + Declared.rename() + " self=" + Declared.self() + " who=" + new Declared().who());

// The outer binding of a class declaration is an ordinary mutable let.
let alias: any = Declared;
alias = 5;
console.log("outer-rebind=" + typeof alias + " original=" + Declared.name);

const Expr = class Inner {
  static tryRename(): string {
    try {
      // @ts-expect-error -- the class-expression name binding is const
      Inner = null as any;
      return "no-throw";
    } catch (e: any) {
      return e.constructor.name;
    }
  }
  static selfRef(): boolean {
    return Inner === Expr;
  }
  tag(): string {
    return Inner.name;
  }
};
console.log("expr-binding=" + Expr.name + " " + Expr.tryRename() + " " + Expr.selfRef() + " " + new Expr().tag());

// The inner binding survives reassignment of the outer one.
let holder: any = class Named {
  static resolve(): string {
    return Named.name + ":" + typeof holder;
  }
};
const captured = holder;
holder = "gone";
console.log("survives=" + captured.resolve());

// Anonymous classes take names from their binding position.
const Anon = class {};
const memberHolder = { Member: class {} };
const arrHolder = [class {}];
console.log("inferred=" + Anon.name + "," + memberHolder.Member.name + ",[" + arrHolder[0].name + "],[" + (class {}).name + "]");

const nameDesc: any = Object.getOwnPropertyDescriptor(Anon, "name");
console.log("name-desc=w" + nameDesc.writable + ",c" + nameDesc.configurable);
Object.defineProperty(Anon, "name", { value: "Renamed" });
console.log("renamed=" + Anon.name);

// The class binding is in TDZ while its heritage expression is evaluated.
try {
  class Cyclic extends (Cyclic as any) {}
  console.log("tdz=no-throw");
} catch (e: any) {
  console.log("tdz=" + e.constructor.name);
}

class BaseName {}
class SubName extends BaseName {}
console.log("sub-name=" + SubName.name + " own=" + hasOwn(SubName, "name") + " head=" + Declared.toString().slice(0, 5) + " " + Expr.toString().slice(0, 5));

// --- instanceof: prototype walk along a chain ---
class A {}
class B extends A {}
class C extends B {}
class Other {}
const ia = new A();
const ib = new B();
const ic = new C();
const io = new Other();
console.log("walk-a=" + (ia instanceof A) + "," + (ia instanceof B) + "," + (ia instanceof C));
console.log("walk-b=" + (ib instanceof A) + "," + (ib instanceof B) + "," + (ib instanceof C));
console.log("walk-c=" + (ic instanceof A) + "," + (ic instanceof B) + "," + (ic instanceof C) + " other=" + (io instanceof A) + "," + (io instanceof Other));
console.log("walk-builtins=" + (ic instanceof Object) + "," + (ic instanceof Function) + "," + (C instanceof Function));
console.log("walk-prims=" + ((5 as unknown) instanceof A) + "," + (("s" as unknown) instanceof Object) + "," + ((null as unknown) instanceof Object));
console.log("walk-ctor=" + ic.constructor.name + "," + ib.constructor.name + "," + (ic.constructor === C) + " proto-of=" + (Object.getPrototypeOf(C.prototype) === B.prototype) + "," + (Object.getPrototypeOf(C) === B));
console.log("walk-isproto=" + A.prototype.isPrototypeOf(ic) + "," + C.prototype.isPrototypeOf(ib));
const sweep: string[] = [];
for (const item of [ia, ib, ic, io] as object[]) {
  if (item instanceof C) sweep.push("C");
  else if (item instanceof B) sweep.push("B");
  else if (item instanceof A) sweep.push("A");
  else sweep.push("?");
}
console.log("sweep=" + sweep.join(",") + " count-a=" + [ia, ib, ic, io].filter((x) => x instanceof A).length);

// --- instanceof and Symbol.hasInstance ---
class Animal {
  kind: string = "animal";
}
class Dog extends Animal {
  kind: string = "dog";
}
const dog = new Dog();
console.log("dog=" + (dog instanceof Animal) + "," + (dog instanceof Object) + "," + (dog instanceof (Function as any)));

const hiDesc: any = Object.getOwnPropertyDescriptor(Function.prototype, Symbol.hasInstance);
console.log("hasinstance=w" + hiDesc.writable + ",e" + hiDesc.enumerable + ",c" + hiDesc.configurable + "," + hiDesc.value.name + "," + hiDesc.value.length);
console.log("hasinstance-direct=" + (Function.prototype[Symbol.hasInstance] as any).call(Animal, dog));

const BoundDog: any = Dog.bind(null);
console.log("bound=" + hasOwn(BoundDog, "prototype") + " " + (dog instanceof BoundDog) + " " + (dog instanceof BoundDog.bind(null)) + " " + BoundDog.name);
const protoDesc: any = Object.getOwnPropertyDescriptor(Dog, "prototype");
console.log("class-prototype=w" + protoDesc.writable + ",c" + protoDesc.configurable);

function Maker(this: any): void {
  this.kind = "maker";
}
const madeBefore: any = new (Maker as any)();
(Maker as any).prototype = { kind: "replaced" };
console.log("swap-old=" + (madeBefore instanceof (Maker as any)));
const madeAfter: any = new (Maker as any)();
console.log("swap-new=" + (madeAfter instanceof (Maker as any)) + " kind=" + madeAfter.kind);

class Nothing {
  static [Symbol.hasInstance](): boolean {
    return false;
  }
}
class SubNothing extends Nothing {}
console.log("nothing=" + ((new Nothing() as any) instanceof Nothing) + " sub=" + ((new SubNothing() as any) instanceof SubNothing));

const seenTypes: string[] = [];
const Tracker: any = {
  [Symbol.hasInstance](v: any): any {
    seenTypes.push(typeof v);
    return "non-empty";
  },
};
console.log("tracker=" + (({} as any) instanceof Tracker) + "," + ((5 as any) instanceof Tracker) + "," + ((null as any) instanceof Tracker) + " seen=" + seenTypes.join(","));

const CustomEven: any = class {
  static [Symbol.hasInstance](x: any): number {
    return x && x.value % 2 === 0 ? 1 : 0;
  }
};
console.log("custom=" + (({ value: 4 } as any) instanceof CustomEven) + "," + (({ value: 5 } as any) instanceof CustomEven) + "," + ((null as any) instanceof CustomEven));

for (const rhs of [{}, 5, null]) {
  try {
    console.log("rhs=" + ((dog as any) instanceof (rhs as any)));
  } catch (e: any) {
    console.log("rhs=" + e.constructor.name);
  }
}

function NoProto(): void {}
(NoProto as any).prototype = 1;
try {
  console.log("prim-prototype=" + ((dog as any) instanceof (NoProto as any)));
} catch (e: any) {
  console.log("prim-prototype=" + e.constructor.name);
}

const BadTrap: any = function () {};
Object.defineProperty(BadTrap, Symbol.hasInstance, { value: 7, configurable: true });
try {
  console.log("bad-trap=" + ((dog as any) instanceof BadTrap));
} catch (e: any) {
  console.log("bad-trap=" + e.constructor.name);
}

const NullTrap: any = function (this: any) {};
NullTrap.prototype = Animal.prototype;
Object.defineProperty(NullTrap, Symbol.hasInstance, { value: undefined, configurable: true });
console.log("undefined-trap=" + (dog instanceof NullTrap));
console.log("wrapper=" + (("x" as any) instanceof String) + "," + (new String("x") instanceof String));

// --- polymorphic dispatch over an array of one base class ---
class Animal2 {
  name: string;
  constructor(name: string) {
    this.name = name;
  }
  speak(): string {
    return "...";
  }
  intro(): string {
    return this.name + " says " + this.speak();
  }
  legs(): number {
    return 4;
  }
}
class Dog2 extends Animal2 {
  speak(): string {
    return "woof";
  }
}
class Cat2 extends Animal2 {
  speak(): string {
    return "meow";
  }
}
class Bird2 extends Animal2 {
  speak(): string {
    return "tweet";
  }
  legs(): number {
    return 2;
  }
}
class Puppy2 extends Dog2 {
  speak(): string {
    return super.speak() + "!";
  }
}
const zoo: Animal2[] = [new Dog2("rex"), new Cat2("tom"), new Bird2("kiwi"), new Puppy2("bud"), new Animal2("blob")];
for (let i = 0; i < zoo.length; i++) {
  console.log("idx" + i + "=" + zoo[i].speak() + " / " + zoo[i].intro());
}
const viaOf: string[] = [];
for (const a of zoo) {
  viaOf.push(a.speak());
}
console.log("for-of=" + viaOf.join(","));
console.log("map=" + zoo.map((a) => a.speak()).join(",") + " legs=" + zoo.map((a) => a.legs()).join(","));
let total = 0;
for (let i = 0; i < zoo.length; i++) {
  total += zoo[i].legs();
}
console.log("total-legs=" + total + " four=" + zoo.filter((a) => a.legs() === 4).map((a) => a.name).join(","));
const nested: Animal2[][] = [[zoo[0], zoo[1]], [zoo[2]], [zoo[3], zoo[4]]];
const flat: string[] = [];
for (let i = 0; i < nested.length; i++) {
  for (let j = 0; j < nested[i].length; j++) {
    flat.push(nested[i][j].speak());
  }
}
console.log("nested=" + flat.join(","));
console.log("slot0=" + zoo[0].speak() + "/" + zoo[0].speak());
zoo[0] = new Cat2("swap");
console.log("after-swap=" + zoo[0].speak() + ":" + zoo[0].name);
const picked = zoo[3];
console.log("picked=" + picked.speak() + ":" + picked.intro());

// Overridden methods dispatch dynamically through base-class calls.
class OverrideBase {
  value(): number {
    return 2;
  }
  describe(): string {
    return "base:" + this.value();
  }
}
class OverrideChild extends OverrideBase {
  value(): number {
    return 5;
  }
}
const oc = new OverrideChild();
console.log("override=" + oc.value() + " " + oc.describe() + " " + (oc instanceof OverrideChild) + "," + (oc instanceof OverrideBase));
