// Member names and protocols: computed string and symbol method names, symbol
// keyed fields/accessors/generators/statics, Symbol.iterator and Symbol.toPrimitive
// on classes, generator and async methods (their shapes, descriptors, and the
// order they run in), and toString/valueOf coercion through class hierarchies.

function hasOwn(o: object, k: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(o, k);
}

// --- computed method names (strings, numbers, symbols, dynamic prototypes) ---
const KEY_NAME = "sum";
const iterKey = Symbol.iterator;
const reg = Symbol.for("demo");
class Ops {
  [KEY_NAME](a: number, b: number): number {
    return a + b;
  }
  [iterKey](): Iterator<number> {
    return [1, 2][Symbol.iterator]();
  }
  [reg](): string {
    return "ok";
  }
  [(1 + 1).toString()](): string {
    return "method named '2'";
  }
}
const ops: any = new Ops();
console.log("computed-sum=" + ops.sum(2, 3) + " iter=" + Array.from(ops).join(",") + " sym=" + ops[reg]() + " two=" + ops[2]());
console.log("computed-proto=" + Object.keys(Ops.prototype).length + " names=" + Object.getOwnPropertyNames(Ops.prototype).sort().join(","));

const PREFIX = "get";
const SUFFIX = "Value";
class Config {
  #data: Record<string, number> = { width: 100, height: 200, depth: 50 };
  [PREFIX + "Width" + SUFFIX]() {
    return this.#data.width;
  }
  [PREFIX + "Height" + SUFFIX]() {
    return this.#data.height;
  }
  [PREFIX + "Depth" + SUFFIX]() {
    return this.#data.depth;
  }
  [Symbol.toPrimitive](hint: string): any {
    if (hint === "string") return JSON.stringify(this.#data);
    return Object.values(this.#data).reduce((a, b) => a + b, 0);
  }
}
const cfg: any = new Config();
console.log("config=" + cfg.getWidthValue() + "," + cfg.getHeightValue() + "," + cfg.getDepthValue());
console.log("toprim=" + +cfg + " template=" + `${cfg}` + " concat=" + (cfg + "") + " gt=" + (cfg > 300));

const opNames = ["add", "sub", "mul"] as const;
const impl: Record<string, (a: number, b: number) => number> = {
  add: (a, b) => a + b,
  sub: (a, b) => a - b,
  mul: (a, b) => a * b,
};
class Calc {}
opNames.forEach((op) => {
  (Calc.prototype as any)[op] = impl[op];
});
const calc: any = new Calc();
console.log("dynamic=" + calc.add(3, 4) + "," + calc.sub(10, 3) + "," + calc.mul(3, 4));

// A computed key with a side effect is evaluated once, at definition time.
let keyCalls = 0;
const keyFn = () => {
  keyCalls++;
  return "run";
};
class Worker {
  [keyFn()](): number {
    return 7;
  }
}
const w: any = new Worker();
console.log("worker=" + w.run() + " key-calls=" + keyCalls + " keys=" + Object.keys(Worker.prototype).length);

// --- symbol-keyed members of every kind ---
const KEY = Symbol("key");
const ACC = Symbol("acc");
const FIELD = Symbol("field");
const STATIC = Symbol("static");
const GEN = Symbol("gen");
const REG = Symbol.for("registered");

class Holder {
  [FIELD]: string = "field-value";
  plain: string = "plain";
  [KEY](): string {
    return "method:" + this.plain;
  }
  get [ACC](): string {
    return "get:" + this.plain;
  }
  set [ACC](v: string) {
    this.plain = "set(" + v + ")";
  }
  *[GEN](): Generator<number, void, undefined> {
    yield 1;
    yield 2;
  }
  static [STATIC](): string {
    return "static-method";
  }
  [REG]: number = 42;
}

const h: any = new Holder();
console.log("method=" + h[KEY]() + " field=" + h[FIELD] + " registered=" + h[REG]);
console.log("accessor=" + h[ACC]);
h[ACC] = "x";
console.log("accessor-after=" + h.plain + " get2=" + h[ACC]);
console.log("generator=" + Array.from(h[GEN]()).join(",") + " static=" + (Holder as any)[STATIC]());
console.log("keys=" + Object.keys(h).join(",") + " names=" + Object.getOwnPropertyNames(h).join(",") + " json=" + JSON.stringify(h));

const instSyms = Object.getOwnPropertySymbols(h);
console.log("inst-symbols=" + instSyms.length + " field=" + (instSyms.indexOf(FIELD) >= 0) + " registered=" + (instSyms.indexOf(REG) >= 0) + " method=" + (instSyms.indexOf(KEY) >= 0));
const protoSyms = Object.getOwnPropertySymbols(Holder.prototype);
console.log("proto-symbols=" + protoSyms.length + " method=" + (protoSyms.indexOf(KEY) >= 0) + " accessor=" + (protoSyms.indexOf(ACC) >= 0) + " gen=" + (protoSyms.indexOf(GEN) >= 0));
console.log("class-symbols=" + Object.getOwnPropertySymbols(Holder).length);

const md: any = Object.getOwnPropertyDescriptor(Holder.prototype, KEY);
const ad: any = Object.getOwnPropertyDescriptor(Holder.prototype, ACC);
const fd: any = Object.getOwnPropertyDescriptor(h, FIELD);
console.log("method-desc=w" + md.writable + ",e" + md.enumerable + ",c" + md.configurable);
console.log("accessor-desc=e" + ad.enumerable + ",c" + ad.configurable + ",g" + typeof ad.get + ",s" + typeof ad.set);
console.log("field-desc=w" + fd.writable + ",e" + fd.enumerable + ",c" + fd.configurable);
console.log("names=" + JSON.stringify(md.value.name) + "," + JSON.stringify(ad.get.name) + "," + JSON.stringify(ad.set.name) + "," + JSON.stringify(((Holder as any)[STATIC] as any).name));

const ANON = Symbol();
class Anon {
  [ANON](): number {
    return 1;
  }
}
const anonDesc: any = Object.getOwnPropertyDescriptor(Anon.prototype, ANON);
console.log("anon-name=" + JSON.stringify(anonDesc.value.name));

class SubHolder extends Holder {
  [KEY](): string {
    return "sub+" + super[KEY]();
  }
}
const sh: any = new SubHolder();
console.log("sub-method=" + sh[KEY]() + " sub-field=" + sh[FIELD] + " in=" + (KEY in sh) + "," + (FIELD in sh) + " own-field=" + hasOwn(sh, FIELD));
console.log("sub-static=" + (SubHolder as any)[STATIC]() + " for=" + (h[Symbol.for("registered")] === 42) + " fresh=" + String(h[Symbol("field")]));

// --- Symbol.iterator as a generator method, and generator protocol details ---
class Seq {
  items: number[] = [1, 2, 3];
  *[Symbol.iterator](): Generator<number> {
    for (const it of this.items) yield it;
  }
  *pairs(): Generator<string> {
    let i = 0;
    while (i < this.items.length) {
      yield i + ":" + this.items[i];
      i = i + 1;
    }
  }
  static *count(n: number): Generator<number> {
    let i = 0;
    while (i < n) {
      yield i;
      i = i + 1;
    }
  }
  async load(): Promise<string> {
    return "loaded";
  }
  async *stream(): AsyncGenerator<number> {
    yield 1;
    yield 2;
  }
}
const seq = new Seq();
console.log("spread=" + [...seq].join(",") + " from=" + Array.from(seq).join(",") + " pairs=" + [...seq.pairs()].join("|") + " count=" + [...Seq.count(4)].join(","));

const gen = seq.pairs();
console.log("gen-tag=" + Object.prototype.toString.call(gen) + " async-tag=" + Object.prototype.toString.call(seq.stream()));
console.log("gen-next=" + JSON.stringify(gen.next()) + " ret=" + JSON.stringify(gen.return("stop" as any)) + " after=" + JSON.stringify(gen.next()));
const gen2 = seq.pairs();
gen2.next();
let thrown = "none";
try {
  gen2.throw(new RangeError("x"));
} catch (e: any) {
  thrown = e.constructor.name;
}
console.log("gen-throw=" + thrown);
const gen3 = seq.pairs();
console.log("gen-self-iter=" + ((gen3 as any)[Symbol.iterator]() === gen3) + " gen-proto=" + (Object.getPrototypeOf(seq.pairs()) === Seq.prototype.pairs.prototype));

const seqProto: any = Seq.prototype;
console.log("iterator-name=" + seqProto[Symbol.iterator].name + " pairs-name=" + seqProto.pairs.name + " count-name=" + Seq.count.name + " load-name=" + seqProto.load.name + " stream-name=" + seqProto.stream.name);
console.log("gen-ctor=" + seqProto.pairs.constructor.name + " async-ctor=" + seqProto.load.constructor.name + " stream-ctor=" + seqProto.stream.constructor.name);
console.log("gen-has-prototype=" + hasOwn(seqProto.pairs, "prototype") + " async-has-prototype=" + hasOwn(seqProto.load, "prototype"));
const genProtoDesc: any = Object.getOwnPropertyDescriptor(seqProto.pairs, "prototype");
console.log("gen-prototype-desc=w" + genProtoDesc.writable + ",e" + genProtoDesc.enumerable + ",c" + genProtoDesc.configurable);
try {
  new (seqProto.pairs as any)();
  console.log("new-gen=no-throw");
} catch (e: any) {
  console.log("new-gen=" + e.constructor.name);
}

// --- class methods of every flavour, descriptors and shapes ---
const log: string[] = [];

class Source {
  base: number = 10;

  async one(): Promise<number> {
    log.push("one-enter");
    const v = await this.base;
    log.push("one-exit");
    return v + 1;
  }

  static async build(n: number): Promise<string> {
    log.push("build:" + n);
    return "built-" + (await n);
  }

  async *pairs(): AsyncGenerator<string, string, undefined> {
    log.push("gen-start");
    yield "p" + this.base;
    log.push("gen-mid");
    yield "q" + (await 2);
    log.push("gen-end");
    return "done";
  }

  *plain(): Generator<number, void, undefined> {
    yield this.base;
    yield this.base + 1;
  }

  [Symbol.asyncIterator](): any {
    return this.pairs();
  }
}

class Derived extends Source {
  async one(): Promise<number> {
    log.push("derived-one");
    const fromBase = await super.one();
    return fromBase * 2;
  }
  arrowAsync: () => Promise<string> = async () => "arrow:" + this.base;
}

const src = new Source();
const pending = src.one();
console.log("returns-promise=" + (pending instanceof Promise) + " thenable=" + (typeof (pending as any).then === "function") + " ctor=" + pending.constructor.name);
const ag: any = src.pairs();
console.log("async-gen=" + (typeof ag.next === "function") + "," + (typeof ag[Symbol.asyncIterator] === "function") + "," + (ag[Symbol.asyncIterator]() === ag) + " sync-iter=" + typeof ag[Symbol.iterator]);

function desc(o: any, k: any): string {
  const d: any = Object.getOwnPropertyDescriptor(o, k);
  return "w" + d.writable + ",e" + d.enumerable + ",c" + d.configurable + ",fn" + (typeof d.value === "function");
}
console.log("desc-async=" + desc(Source.prototype, "one"));
console.log("desc-asyncgen=" + desc(Source.prototype, "pairs"));
console.log("desc-static=" + desc(Source, "build"));
console.log("name-async=" + Source.prototype.one.name + " name-asyncgen=" + Source.prototype.pairs.name);
console.log("proto-names=" + Object.getOwnPropertyNames(Source.prototype).sort().join(","));
console.log("proto-symbols=" + Object.getOwnPropertySymbols(Source.prototype).length);

let newAsync = "no-throw";
try {
  new (Source.prototype.one as any)();
} catch (e: any) {
  newAsync = e.constructor.name;
}
console.log("new-async=" + newAsync);
console.log("plain-gen=" + Array.from(src.plain()).join(","));

const syncGenProto = Object.getPrototypeOf(Object.getPrototypeOf(src.plain()));
console.log("gen-proto-tag=" + syncGenProto[Symbol.toStringTag]);

async function run(): Promise<void> {
  console.log("await-one=" + (await pending));

  const d = new Derived();
  console.log("derived-one=" + (await d.one()));
  console.log("derived-arrow=" + (await d.arrowAsync()));
  console.log("static-build=" + (await Source.build(4)));

  const seen: string[] = [];
  for await (const v of src.pairs()) {
    seen.push(v);
  }
  console.log("for-await=" + seen.join("|"));

  const viaClass: string[] = [];
  for await (const v of src as any) {
    viaClass.push(v);
  }
  console.log("via-class=" + viaClass.join("|"));

  const it: any = src.pairs();
  const s1 = await it.next();
  const s2 = await it.next();
  const s3 = await it.next();
  const s4 = await it.next();
  console.log("steps=" + s1.value + ":" + s1.done + "," + s2.value + ":" + s2.done + "," + s3.value + ":" + s3.done + "," + s4.value + ":" + s4.done);

  const it2: any = src.pairs();
  console.log("early=" + (await it2.next()).value);
  const closed = await it2.return("stopped");
  console.log("closed=" + closed.value + ":" + closed.done);

  const seq2: string[] = [];
  for await (const v of new Seq().stream()) {
    seq2.push(String(v));
  }
  console.log("stream=" + seq2.join(",") + " load=" + (await seq.load()));
  console.log("log=" + log.join(">"));
}
run().then(() => {
  console.log("tail=reached");
});
console.log("sync-end=true");

// --- toString / valueOf / coercion through a class hierarchy ---
class Money {
  amount: number;
  constructor(amount: number) {
    this.amount = amount;
  }
  toString(): string {
    return "$" + this.amount;
  }
  valueOf(): number {
    return this.amount;
  }
}
const money = new Money(5);
console.log("numeric=" + (+money) + " mul=" + (money * 2) + " sub=" + (money - 1) + " cmp=" + (money > 3) + " number=" + Number(money));
console.log("string=" + `${money}` + " " + String(money) + " concat=" + ("v:" + String(money)));
console.log("plus=" + (money + "") + " plus-num=" + (money + 1));

class Tag {
  v: string;
  constructor(v: string) {
    this.v = v;
  }
  toString(): string {
    return this.v;
  }
}
const tag = new Tag("42");
console.log("tag=" + `${tag}` + " num=" + +tag + " plus=" + ((tag as any) + 1));
const badTag = new Tag("abc");
console.log("bad-tag=" + +badTag + " " + ((badTag as any) + 1));

class Raw {
  valueOf(): number {
    return 9;
  }
}
const raw = new Raw();
console.log("raw=" + +raw + " " + `${raw}` + " " + String(raw));

class Cents extends Money {
  toString(): string {
    return super.toString() + "c";
  }
}
const cents = new Cents(3);
console.log("cents=" + `${cents}` + " " + (+cents) + " " + (cents + 1));
console.log("join=" + [new Money(1), new Money(2)].join("|"));
console.log("sorted=" + [new Money(3), new Money(1), new Money(2)].sort((a, b) => a.valueOf() - b.valueOf()).map((x) => x.toString()).join(","));
