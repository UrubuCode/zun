// Composition and heritage: mixins as functions returning class expressions
// (chains, super threading, independent applications), dynamic bases chosen at
// runtime, manual method decorators applied through defineProperty, a private
// constructor with a static factory, subclassing built-ins (Error, Map, Array)
// with their species and brand-check behaviour, and heritage values that are
// refused or accepted (null, Proxy, primitives, bound functions).

function hasOwn(o: object, k: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(o, k);
}

// --- classic mixins: Serializable(Validatable(User)) ---
type Constructor<T = {}> = new (...args: any[]) => T;

function Serializable<TBase extends Constructor>(Base: TBase) {
  return class extends Base {
    serialize(): string {
      return JSON.stringify(this);
    }
    static deserialize(json: string): any {
      return JSON.parse(json);
    }
  };
}

function Validatable<TBase extends Constructor>(Base: TBase) {
  return class extends Base {
    validate(): boolean {
      return Object.values(this as any).every((v) => v !== null && v !== undefined);
    }
  };
}

class User {
  name: string;
  age: number;
  constructor(name: string, age: number) {
    this.name = name;
    this.age = age;
  }
}
class RichUser extends Serializable(Validatable(User)) {}

const alice = new RichUser("Alice", 30);
console.log("validate=" + alice.validate() + " serialize=" + alice.serialize());
const bob = new RichUser("Bob", null as any);
console.log("validate-null=" + bob.validate());
console.log("mixin-instanceof=" + (alice instanceof User) + " static=" + JSON.stringify((RichUser as any).deserialize('{"k":1}')));
console.log("mixin-proto-chain-has-validate=" + (typeof (RichUser.prototype as any).validate) + " own=" + hasOwn(RichUser.prototype, "validate"));

// --- manual method decorators, applied with defineProperty ---
function readonlyDescriptor(_target: any, _key: string, descriptor: PropertyDescriptor): PropertyDescriptor {
  descriptor.writable = false;
  return descriptor;
}
function logCalls(_target: any, key: string, descriptor: PropertyDescriptor): PropertyDescriptor {
  const orig = descriptor.value;
  descriptor.value = function (this: any, ...args: any[]) {
    console.log("call:" + key + "(" + args.join(",") + ")");
    return orig.apply(this, args);
  };
  return descriptor;
}

class Calculator {
  add(a: number, b: number): number {
    return a + b;
  }
  mul(a: number, b: number): number {
    return a * b;
  }
  constant(): number {
    return 1;
  }
}
for (const name of ["add", "mul"]) {
  const wrapped = logCalls(Calculator.prototype, name, Object.getOwnPropertyDescriptor(Calculator.prototype, name)!);
  Object.defineProperty(Calculator.prototype, name, wrapped);
}
const frozenDesc = readonlyDescriptor(Calculator.prototype, "constant", Object.getOwnPropertyDescriptor(Calculator.prototype, "constant")!);
Object.defineProperty(Calculator.prototype, "constant", frozenDesc);
const calcInst = new Calculator();
console.log("calc=" + calcInst.add(2, 3) + " " + calcInst.mul(4, 5) + " constant=" + calcInst.constant());
console.log("constant-writable=" + Object.getOwnPropertyDescriptor(Calculator.prototype, "constant")!.writable);

// --- a private constructor behind a static factory ---
class Money {
  readonly amount: number;
  readonly currency: string;
  private constructor(amount: number, currency: string) {
    this.amount = amount;
    this.currency = currency;
  }
  static of(amount: number, currency: string): Money {
    if (amount < 0) throw new RangeError("negative amount");
    return new Money(amount, currency);
  }
  add(other: Money): Money {
    if (this.currency !== other.currency) throw new Error("currency mismatch");
    return Money.of(this.amount + other.amount, this.currency);
  }
  toString(): string {
    return this.amount + " " + this.currency;
  }
}
const usd10 = Money.of(10, "USD");
const usd5 = Money.of(5, "USD");
console.log("money=" + usd10.add(usd5).toString());
try {
  Money.of(-1, "USD");
} catch (e: any) {
  console.log("money-negative=" + e.constructor.name + ":" + e.message);
}
try {
  usd10.add(Money.of(1, "EUR"));
} catch (e: any) {
  console.log("money-mismatch=" + e.constructor.name + ":" + e.message);
}

// --- mixin factories with super threading and independent applications ---
const order: string[] = [];

function Serialisable(BaseCtor: any): any {
  return class Serialisable extends BaseCtor {
    serialisedBy: string = "serialisable";
    constructor(...args: any[]) {
      order.push("Serialisable.before");
      super(...args);
      order.push("Serialisable.after");
    }
    describe(): string {
      return "S<" + (super.describe ? super.describe() : "-") + ">";
    }
    static kind(): string {
      return "serialisable";
    }
  };
}

function Countable(BaseCtor: any): any {
  return class Countable extends BaseCtor {
    counted: number = 0;
    constructor(...args: any[]) {
      order.push("Countable.before");
      super(...args);
      this.counted = args.length;
      order.push("Countable.after");
    }
    describe(): string {
      return "C<" + (super.describe ? super.describe() : "-") + ">";
    }
  };
}

class Core {
  name: string = "";
  constructor(name: string) {
    order.push("Core.ctor");
    this.name = name;
  }
  describe(): string {
    return "Core(" + this.name + ")";
  }
}

const Composed: any = Countable(Serialisable(Core));
const composed = new Composed("thing", "extra");
console.log("composed=" + composed.describe() + " name=" + composed.name + " counted=" + composed.counted + " by=" + composed.serialisedBy);
console.log("composed-order=" + order.join(">") + " keys=" + Object.keys(composed).join(","));

function chainLength(o: any): number {
  let n = 0;
  let cur = Object.getPrototypeOf(o);
  while (cur !== null && n < 20) {
    n++;
    cur = Object.getPrototypeOf(cur);
  }
  return n;
}
console.log("composed-chain=" + chainLength(composed) + " kind=" + Composed.kind() + " own-kind=" + hasOwn(Composed, "kind"));
console.log("composed-instanceof=" + (composed instanceof Core) + "," + (composed instanceof Composed));

const mixA: any = Serialisable(Core);
const mixB: any = Serialisable(Core);
console.log("applications=" + (mixA !== mixB) + "," + (mixA.prototype !== mixB.prototype) + "," + (Object.getPrototypeOf(mixA.prototype) === Core.prototype));
const inA = new mixA("one");
console.log("apps-instanceof=" + (inA instanceof mixA) + "," + (inA instanceof mixB) + " describe=" + inA.describe());

class Leaf extends Composed {
  leafTag: string = "leaf";
  describe(): string {
    return "L<" + super.describe() + ">";
  }
}
order.length = 0;
const leaf = new Leaf("deep");
console.log("leaf=" + leaf.describe() + " order=" + order.join(">") + " keys=" + Object.keys(leaf).join(","));
console.log("leaf-chain=" + chainLength(leaf) + " instanceof=" + (leaf instanceof Core) + "," + (leaf instanceof Composed) + "," + (leaf instanceof Leaf) + " kind=" + (Leaf as any).kind());

const owners: string[] = [];
let walk: any = Object.getPrototypeOf(leaf);
while (walk !== null && owners.length < 10) {
  if (hasOwn(walk, "describe")) {
    owners.push(walk.constructor.name);
  }
  walk = Object.getPrototypeOf(walk);
}
console.log("describe-owners=" + owners.join(","));

// --- dynamic bases chosen at runtime ---
function Timestamped<T extends new (...a: any[]) => {}>(Base: T) {
  return class extends Base {
    readonly createdAt: string = new Date(0).toISOString();
  };
}
function Tagged(tag: string) {
  return <T extends new (...a: any[]) => {}>(Base: T) =>
    class extends Base {
      readonly tag: string = tag;
    };
}
class Person {
  name: string;
  constructor(name: string) {
    this.name = name;
  }
}
const TimestampedPerson = Timestamped(Person);
const tp: any = new TimestampedPerson("Carol");
console.log("timestamped=" + tp.name + " " + typeof tp.createdAt + " " + tp.createdAt + " " + (tp instanceof Person));
const AdminPerson: any = Tagged("admin")(TimestampedPerson);
const admin: any = new AdminPerson("Dan");
console.log("admin=" + admin.name + " " + admin.tag + " " + admin.createdAt + " " + (admin instanceof Person));

function createShape(kind: "circle" | "square") {
  const Base =
    kind === "circle"
      ? class {
          describe(): string {
            return "circle";
          }
        }
      : class {
          describe(): string {
            return "square";
          }
        };
  return class extends Base {
    area(size: number): number {
      return kind === "circle" ? Math.PI * size * size : size * size;
    }
  };
}
const CircleCls = createShape("circle");
const SquareCls = createShape("square");
const circle: any = new CircleCls();
const square: any = new SquareCls();
console.log("shapes=" + circle.describe() + " " + Math.round(circle.area(1) * 100) / 100 + " " + square.describe() + " " + square.area(4));

// A class whose base is an anonymous class, holding a null-prototype dictionary.
class PureDict extends (class {} as any) {
  #data: Record<string, string> = Object.create(null) as Record<string, string>;
  set(k: string, v: string): void {
    this.#data[k] = v;
  }
  get(k: string): string {
    return this.#data[k];
  }
  has(k: string): boolean {
    return k in this.#data;
  }
}
const dict = new PureDict();
dict.set("foo", "bar");
console.log("dict=" + dict.get("foo") + " " + dict.has("foo") + " " + dict.has("toString"));

// --- extends null and other heritage values ---
class NullProto extends null {}
console.log("null-proto=" + (Object.getPrototypeOf(NullProto.prototype) === null) + " ctor-proto=" + (Object.getPrototypeOf(NullProto) === Function.prototype) + " names=" + Object.getOwnPropertyNames(NullProto.prototype).join(","));
try {
  new (NullProto as any)();
  console.log("new-null=no-throw");
} catch (e: any) {
  console.log("new-null=" + e.constructor.name + " type=" + (e instanceof TypeError));
}
const bareNull: any = Object.create(NullProto.prototype);
bareNull.v = 1;
console.log("bare-null=" + ("toString" in bareNull) + " " + bareNull.v + " " + Object.prototype.toString.call(bareNull));

class Ticker {
  tag: string = "ticker";
}
const ProxiedBase: any = new Proxy(Ticker, {});
class ProxySub extends ProxiedBase {
  label: string = "ps";
}
const ps: any = new ProxySub();
console.log("proxy-heritage=" + ps.tag + " " + ps.label + " " + (ps instanceof Ticker) + " keys=" + Object.keys(ps).join(","));

for (const [label, heritageValue] of [
  ["arrow", () => 1],
  ["number", 7],
  ["undefined", undefined],
] as [string, any][]) {
  try {
    const made: any = class extends (heritageValue as any) {};
    console.log("extends-" + label + "=no-throw:" + typeof made);
  } catch (e: any) {
    console.log("extends-" + label + "=" + e.constructor.name);
  }
}

function WeirdProto(this: any): void {
  this.k = 1;
}
(WeirdProto as any).prototype = 5;
try {
  const weird: any = new (class extends (WeirdProto as any) {})();
  console.log("weird-proto=no-throw k=" + weird.k + " proto-is-object=" + (Object.getPrototypeOf(weird) !== Object.prototype));
} catch (e: any) {
  console.log("weird-proto=" + e.constructor.name);
}

// --- subclassing Error ---
class AppError extends Error {
  code: number = 0;
  constructor(message: string, code: number) {
    super(message);
    this.name = "AppError";
    this.code = code;
  }
  describe(): string {
    return this.name + "/" + this.code + "/" + this.message;
  }
}
class FatalError extends AppError {
  constructor(message: string) {
    super(message, 99);
  }
}
const appErr = new AppError("boom", 7);
console.log("err=" + appErr.describe() + " str=" + appErr.toString() + " tag=" + Object.prototype.toString.call(appErr));
console.log("err-instanceof=" + (appErr instanceof AppError) + "," + (appErr instanceof Error) + " proto=" + (Object.getPrototypeOf(appErr) === AppError.prototype));
console.log("err-chain=" + (Object.getPrototypeOf(AppError.prototype) === Error.prototype) + "," + (Object.getPrototypeOf(AppError) === Error) + " keys=" + Object.keys(appErr).sort().join(","));
console.log("err-own=" + hasOwn(appErr, "message") + "," + hasOwn(appErr, "name") + " ctor=" + (AppError.prototype.constructor === AppError));
const fatal = new FatalError("down");
console.log("fatal=" + fatal.describe() + " " + fatal.code + " " + (fatal instanceof AppError) + "," + (fatal instanceof Error));
try {
  throw new FatalError("thrown");
} catch (e: any) {
  console.log("caught=" + e.constructor.name + ":" + e.code + ":" + (e instanceof Error));
}

// --- subclassing Map: the built-in calls the subclass set() ---
const setCalls: string[] = [];
class CountingMap<K, V> extends Map<K, V> {
  writes: number = 0;
  set(key: K, value: V): this {
    this.writes = (this.writes || 0) + 1;
    setCalls.push(String(key));
    return super.set(key, value);
  }
}
const cm = new CountingMap<string, number>([
  ["a", 1],
  ["b", 2],
]);
console.log("cm-size=" + cm.size + " get=" + cm.get("a") + "," + cm.get("b") + " ctor-calls=" + setCalls.join("|"));
cm.set("c", 3);
console.log("cm-after=" + cm.size + " calls=" + setCalls.join("|") + " writes=" + cm.writes);
console.log("cm-instanceof=" + (cm instanceof CountingMap) + "," + (cm instanceof Map) + " tag=" + Object.prototype.toString.call(cm));
console.log("cm-entries=" + Array.from(cm.entries()).map((p) => p[0] + ":" + p[1]).join(",") + " keys=" + Array.from(cm.keys()).join(",") + " own=" + Object.keys(cm).join(","));
console.log("cm-names=" + Object.getOwnPropertyNames(CountingMap.prototype).sort().join(","));
console.log("species=" + ((Map as any)[Symbol.species] === Map) + "," + ((CountingMap as any)[Symbol.species] === CountingMap));
console.log("brand-ok=" + Map.prototype.has.call(cm, "a"));
try {
  Map.prototype.has.call({ size: 0 } as any, "a");
  console.log("brand-off=no-throw");
} catch (e: any) {
  console.log("brand-off=" + e.constructor.name);
}

// --- subclassing Array: species carries the subclass through derived methods ---
class MyArr<T> extends Array<T> {
  tag: string = "mine";
}
const ma = new MyArr<number>();
ma.push(1, 2, 3, 4);
console.log("myarr=" + ma.length + " " + ma.tag + " " + Array.isArray(ma) + " " + Object.prototype.toString.call(ma) + " " + ma.join(","));

const mapped: any = ma.map((x) => x * 2);
console.log("map=" + mapped.constructor.name + " " + (mapped instanceof MyArr) + " " + mapped.tag + " " + mapped.join(","));
const filtered: any = ma.filter((x) => x % 2 === 0);
console.log("filter=" + (filtered instanceof MyArr) + " " + filtered.join(","));
const sliced: any = ma.slice(1, 3);
console.log("slice=" + (sliced instanceof MyArr) + " " + sliced.join(","));
const spliced: any = ma.slice().splice(0, 2);
console.log("splice=" + (spliced instanceof MyArr) + " " + spliced.join(","));
const conc: any = ma.concat([9]);
console.log("concat=" + (conc instanceof MyArr) + " " + conc.join(","));
console.log("from-of=" + (MyArr.from([1, 2, 3]) instanceof MyArr) + "," + (MyArr.of(1, 2) instanceof MyArr));
const sortedCopy: any = ma.toSorted((a, b) => b - a);
console.log("to-sorted=" + sortedCopy.constructor.name + " " + sortedCopy.join(",") + " reversed=" + ma.toReversed().constructor.name);

class PlainOut<T> extends Array<T> {
  static get [Symbol.species](): any {
    return Array;
  }
}
const po = new PlainOut<number>();
po.push(1, 2, 3);
const poMapped: any = po.map((x) => x);
console.log("species-plain=" + poMapped.constructor.name + " " + (poMapped instanceof PlainOut) + " " + Array.isArray(poMapped) + " filter=" + (po.filter(() => true) instanceof PlainOut));

const grow: any = new MyArr<number>();
grow[3] = "x";
const growLen = grow.length;
grow.length = 1;
console.log("grow=" + growLen + " then " + grow.length + " hole=" + String(grow[3]));
const sized: any = new MyArr<number>(3);
console.log("sized=" + sized.length + " holes=" + (0 in sized) + " instanceof=" + (sized instanceof MyArr));

class Deeper<T> extends MyArr<T> {}
const dd = new Deeper<number>();
dd.push(5, 6);
const ddMapped: any = dd.map((x) => x);
console.log("deeper=" + ddMapped.constructor.name + " " + (ddMapped instanceof MyArr) + " " + ddMapped.tag);

const speciesDesc: any = Object.getOwnPropertyDescriptor(Array, Symbol.species);
console.log("array-species=" + typeof speciesDesc.get + "," + String(speciesDesc.set) + "," + (Array[Symbol.species] === Array) + "," + (MyArr[Symbol.species] === MyArr));
