// Private brand checks and private statics: `#x in obj` answers without
// throwing, direct access on a wrong-brand receiver is a TypeError, private
// names never leak into reflection, private static state lives only on the
// declaring constructor (not on subclasses), a private constructor guards a
// registry, and private names are lexically scoped (nested classes).

function hasOwn(o: object, k: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(o, k);
}

// --- brand checks with `#x in` ---
class Point {
  #x: number;
  #y: number;
  constructor(x: number, y: number) {
    this.#x = x;
    this.#y = y;
  }
  static isPoint(v: unknown): v is Point {
    return v !== null && typeof v === "object" && #x in (v as any);
  }
  distanceTo(other: unknown): number {
    if (!Point.isPoint(other)) throw new TypeError("not a Point");
    return Math.sqrt((this.#x - other.#x) ** 2 + (this.#y - other.#y) ** 2);
  }
  toString(): string {
    return "(" + this.#x + "," + this.#y + ")";
  }
}
const p1 = new Point(0, 0);
const p2 = new Point(3, 4);
console.log("is-point=" + Point.isPoint(p1) + "," + Point.isPoint(p2) + "," + Point.isPoint({}) + "," + Point.isPoint(null));
console.log("distance=" + p1.distanceTo(p2) + " str=" + String(p2));
try {
  p1.distanceTo({ x: 3, y: 4 });
  console.log("distance-plain=no-throw");
} catch (e: any) {
  console.log("distance-plain=" + e.constructor.name + ":" + e.message);
}

class Cat {
  #name: string;
  constructor(name: string) {
    this.#name = name;
  }
  static isCat(v: any): boolean {
    return v !== null && typeof v === "object" && #name in v;
  }
  speak(): string {
    return this.#name + " says meow";
  }
}
class Dog {
  #name: string;
  constructor(name: string) {
    this.#name = name;
  }
  static isDog(v: any): boolean {
    return v !== null && typeof v === "object" && #name in v;
  }
  speak(): string {
    return this.#name + " says woof";
  }
}
const cat = new Cat("Whiskers");
const dog = new Dog("Rex");
console.log("cats=" + Cat.isCat(cat) + "," + Cat.isCat(dog) + " dogs=" + Dog.isDog(dog) + "," + Dog.isDog(cat));
console.log(cat.speak() + " / " + dog.speak());

// --- the brand check and the access are different operations ---
class Wallet {
  #balance: number = 10;
  #secret(): string {
    return "s" + this.#balance;
  }
  static #count: number = 0;
  static #bump(): number {
    Wallet.#count = Wallet.#count + 1;
    return Wallet.#count;
  }
  static hasBrand(o: unknown): boolean {
    return #balance in (o as any);
  }
  static hasMethodBrand(o: unknown): boolean {
    return #secret in (o as any);
  }
  static hasStaticBrand(o: unknown): boolean {
    return #count in (o as any);
  }
  static bump(): number {
    return Wallet.#bump();
  }
  peek(other: Wallet): number {
    return other.#balance;
  }
  read(): number {
    return this.#balance;
  }
  callSecret(): string {
    return this.#secret();
  }
  raw(o: any): number {
    return o.#balance;
  }
}
class Decoy {
  #balance: number = 99;
  read(): number {
    return this.#balance;
  }
}
const wa = new Wallet();
const wb = new Wallet();
const decoy = new Decoy();
console.log("wallet=" + wa.read() + " peek=" + wa.peek(wb) + " secret=" + wa.callSecret());
console.log("brand=" + Wallet.hasBrand(wa) + "," + Wallet.hasBrand(wb) + "," + Wallet.hasBrand(decoy) + "," + Wallet.hasBrand({ balance: 1 }));
console.log("brand-proto=" + Wallet.hasBrand(Wallet.prototype) + " brand-null-proto=" + Wallet.hasBrand(Object.create(null)));
console.log("method-brand=" + Wallet.hasMethodBrand(wa) + " method-brand-proto=" + Wallet.hasMethodBrand(Wallet.prototype));
console.log("static-brand=" + Wallet.hasStaticBrand(Wallet) + "," + Wallet.hasStaticBrand(wa));
console.log("bump=" + Wallet.bump() + "," + Wallet.bump());

try {
  wa.raw(decoy);
  console.log("wrong-brand=no-throw");
} catch (e: any) {
  console.log("wrong-brand=" + e.constructor.name + " type=" + (e instanceof TypeError));
}
for (const bad of [{ balance: 5 }, Wallet.prototype]) {
  try {
    wa.raw(bad);
    console.log("bad-receiver=no-throw");
  } catch (e: any) {
    console.log("bad-receiver=" + e.constructor.name);
  }
}

// Privates are invisible to every reflection surface.
console.log("reflect=" + Object.getOwnPropertyNames(wa).length + " symbols=" + Object.getOwnPropertySymbols(wa).length + " keys=" + Object.keys(wa).length + " json=" + JSON.stringify(wa));

// The brand lives on the instance, so it survives a prototype swap.
const swapped: any = new Wallet();
Object.setPrototypeOf(swapped, Decoy.prototype);
console.log("swapped-brand=" + Wallet.hasBrand(swapped) + " instanceof=" + (swapped instanceof Wallet));
try {
  console.log("swapped-read=" + swapped.read());
} catch (e: any) {
  console.log("swapped-read=" + e.constructor.name);
}

// Brand checks through a Proxy: the proxy itself has no private slots.
class Box {
  #value = 1;
  static has(value: any): boolean {
    return #value in value;
  }
}
const box = new Box();
console.log("box-has=" + Box.has(box) + "," + Box.has({}) + "," + Box.has(Object.create(Box.prototype)) + "," + Box.has(new Proxy(box, {})));

// --- private static state: shared across instances, not inherited ---
class Ticket {
  static #next = 1;
  id = Ticket.#next++;
  static current(): number {
    return Ticket.#next;
  }
}
const t1 = new Ticket();
const t2 = new Ticket();
console.log("tickets=" + t1.id + "," + t2.id + " current=" + Ticket.current());

class Counter {
  static #n = 0;
  #id: number;
  constructor() {
    this.#id = ++Counter.#n;
  }
  static count(): number {
    return Counter.#n;
  }
  static hasBrand(x: any): boolean {
    return #id in x;
  }
  label(): string {
    return "C" + this.#id + "/" + Counter.#n;
  }
}
const ca = new Counter();
const cb = new Counter();
console.log("labels=" + ca.label() + " " + cb.label() + " count=" + Counter.count());
console.log("counter-brand=" + Counter.hasBrand(ca) + ":" + Counter.hasBrand({}));

class Registry {
  static #instances = new Map<string, Registry>();
  static #count = 0;
  readonly #name: string;
  readonly #id: number;
  private constructor(name: string) {
    this.#name = name;
    this.#id = ++Registry.#count;
  }
  static get(name: string): Registry {
    if (!Registry.#instances.has(name)) {
      Registry.#instances.set(name, new Registry(name));
    }
    return Registry.#instances.get(name)!;
  }
  static #validate(name: string): boolean {
    return typeof name === "string" && name.length > 0;
  }
  static create(name: string): Registry {
    if (!Registry.#validate(name)) throw new TypeError("invalid name");
    return Registry.get(name);
  }
  static get totalCreated(): number {
    return Registry.#count;
  }
  toString(): string {
    return "Registry(" + this.#name + "#" + this.#id + ")";
  }
}
const r1 = Registry.create("alpha");
const r2 = Registry.create("beta");
const r3 = Registry.get("alpha");
console.log("registry=" + r1.toString() + " " + r2.toString() + " same=" + (r1 === r3) + " total=" + Registry.totalCreated);
try {
  Registry.create("");
  console.log("empty-name=no-throw");
} catch (e: any) {
  console.log("empty-name=" + e.constructor.name + ":" + e.message);
}

// Static state, static methods and a static block over a private table.
class LookupTable {
  static #table: Map<number, string>;
  static #reverse: Map<string, number>;
  static {
    const entries: [number, string][] = [
      [1, "one"],
      [2, "two"],
      [3, "three"],
      [4, "four"],
      [5, "five"],
    ];
    LookupTable.#table = new Map(entries);
    LookupTable.#reverse = new Map(entries.map(([k, v]) => [v, k] as [string, number]));
  }
  static toWord(n: number): string {
    return LookupTable.#table.get(n) ?? "unknown";
  }
  static toNum(s: string): number {
    return LookupTable.#reverse.get(s) ?? -1;
  }
}
console.log("lookup=" + LookupTable.toWord(3) + "," + LookupTable.toNum("five") + "," + LookupTable.toWord(99) + "," + LookupTable.toNum("six"));

// A static private on the base: `this` in a static method decides the brand.
class StaticHolder {
  static #total: number = 100;
  static #tick(): number {
    StaticHolder.#total = StaticHolder.#total + 1;
    return StaticHolder.#total;
  }
  static get #half(): number {
    return StaticHolder.#total / 2;
  }
  static tick(): number {
    return StaticHolder.#tick();
  }
  static half(): number {
    return StaticHolder.#half;
  }
  static tickOnThis(this: any): string {
    try {
      this.#total = this.#total + 1;
      return "ok:" + this.#total;
    } catch (e: any) {
      return e.constructor.name;
    }
  }
  static hasBrand(o: any): boolean {
    return #total in o;
  }
}
class SubHolder extends StaticHolder {}
console.log("tick=" + StaticHolder.tick() + "," + StaticHolder.tick() + " half=" + StaticHolder.half());
console.log("static-brand=" + StaticHolder.hasBrand(StaticHolder) + "," + StaticHolder.hasBrand(SubHolder));
console.log("this-on-base=" + StaticHolder.tickOnThis() + " this-on-sub=" + (SubHolder as any).tickOnThis());
console.log("sub-tick=" + SubHolder.tick() + " total-after=" + StaticHolder.half() * 2);
console.log("sub-proto=" + (Object.getPrototypeOf(SubHolder) === StaticHolder));

// Derived classes do not get a copy of a base static private; the inherited
// static method reaches the base slot through the base name, but fails on
// `this` when the receiver is the subclass.
class SecretBase {
  static #value = 7;
  static read(): number {
    return (this as any).#value;
  }
  static readBase(): number {
    return SecretBase.#value;
  }
}
class SecretChild extends SecretBase {}
console.log("secret-base=" + SecretBase.read() + " via-name=" + SecretChild.readBase());
try {
  SecretChild.read();
  console.log("secret-child=no-throw");
} catch (e: any) {
  console.log("secret-child=" + e.constructor.name);
}

// Private names are lexical: an inner class sees the outer #n and its own #m.
class Outer {
  #n: string = "outer";
  makeInner(): any {
    const self = this;
    class Inner {
      #m: string = "inner";
      both(): string {
        return this.#m + "/" + self.#n;
      }
      static innerBrand(o: any): boolean {
        return #m in o;
      }
      static outerBrand(o: any): boolean {
        return #n in o;
      }
    }
    return Inner;
  }
  static outerBrand(o: any): boolean {
    return #n in o;
  }
}
const outer = new Outer();
const InnerClass: any = outer.makeInner();
const inner = new InnerClass();
console.log("nested=" + inner.both());
console.log("nested-brands=" + InnerClass.innerBrand(inner) + "," + InnerClass.innerBrand(outer) + " outer-from-inner=" + InnerClass.outerBrand(inner) + "," + InnerClass.outerBrand(outer));
console.log("outer-brand=" + Outer.outerBrand(inner) + "," + Outer.outerBrand(outer) + " own=" + hasOwn(outer, "makeInner"));
