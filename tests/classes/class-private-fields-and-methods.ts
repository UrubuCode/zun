// Private (#name) fields, methods and accessors: per-class slots (a subclass's
// #x is a different slot from the base's #x), private methods that call each
// other, private accessors with clamping and read-only pairs, private methods
// being non-writable, extracted methods keeping their brand, inheritance of
// private state through several levels, and private state in data structures.

function hasOwn(o: object, k: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(o, k);
}

// --- same-named private fields in base and subclass are distinct slots ---
class Shadowing {
  #x = 7;
  getX(): number {
    return this.#x;
  }
}
class ShadowingSub extends Shadowing {
  #x = 20;
  both(): string {
    return this.getX() + ":" + this.#x;
  }
}
const sub: any = new ShadowingSub();
console.log("both=" + sub.both() + " keys=" + Object.keys(sub).length + " names=" + Object.getOwnPropertyNames(sub).length);
console.log("string-key=" + String(sub["#x"]) + " in=" + ("#x" in sub));

// Same-named private methods are also per class.
class PA {
  #m(v: number): number {
    return v + 1;
  }
  call(v: number): number {
    return this.#m(v);
  }
}
class PB extends PA {
  #m(v: number): number {
    return v + 10;
  }
  both(v: number): string {
    return this.call(v) + ":" + this.#m(v);
  }
}
console.log("private-methods=" + new PB().both(5));

// --- private state with private step, reset and a static id generator ---
class Counter {
  #count = 0;
  #step: number;
  constructor(step = 1) {
    this.#step = step;
  }
  increment(): void {
    this.#count += this.#step;
  }
  decrement(): void {
    this.#count -= this.#step;
  }
  reset(): void {
    this.#count = 0;
  }
  get value(): number {
    return this.#count;
  }
}
const cnt = new Counter(2);
cnt.increment();
cnt.increment();
cnt.increment();
const v1 = cnt.value;
cnt.decrement();
const v2 = cnt.value;
cnt.reset();
console.log("counter=" + v1 + "," + v2 + "," + cnt.value + " keys=" + Object.keys(cnt).length);

class IdGen {
  static #next = 1;
  readonly id: number;
  constructor() {
    this.id = IdGen.#next++;
  }
  static reset(): void {
    IdGen.#next = 1;
  }
}
const ga = new IdGen();
const gb = new IdGen();
const gc = new IdGen();
IdGen.reset();
const gd = new IdGen();
console.log("ids=" + ga.id + "," + gb.id + "," + gc.id + " after-reset=" + gd.id);

// --- private methods calling private methods; private accessors ---
class Temperature {
  #kelvin: number;
  constructor(celsius: number) {
    this.#kelvin = this.#toKelvin(celsius);
  }
  #toKelvin(c: number): number {
    return c + 273.15;
  }
  #toCelsius(k: number): number {
    return k - 273.15;
  }
  get celsius(): number {
    return this.#toCelsius(this.#kelvin);
  }
  set celsius(v: number) {
    this.#kelvin = this.#toKelvin(v);
  }
  get kelvin(): number {
    return this.#kelvin;
  }
  get fahrenheit(): number {
    return (this.celsius * 9) / 5 + 32;
  }
}
const temp = new Temperature(100);
console.log("temp=" + temp.celsius + "," + temp.kelvin + "," + temp.fahrenheit);
temp.celsius = 0;
console.log("temp-after=" + temp.kelvin);

class Formatter {
  #prefix: string;
  constructor(prefix: string) {
    this.#prefix = prefix;
  }
  #pad(s: string, len: number): string {
    return s.padStart(len, " ");
  }
  #wrap(s: string): string {
    return "[" + this.#prefix + "] " + s;
  }
  format(msg: string, width = 20): string {
    return this.#pad(this.#wrap(msg), width);
  }
}
const fmt = new Formatter("INFO");
console.log("format=" + fmt.format("ok", 15).trimStart() + " len=" + fmt.format("x", 20).length);

class Clamped {
  #n: number = 0;
  #step: number = 1;
  get #value(): number {
    return this.#n;
  }
  set #value(v: number) {
    this.#n = v < 0 ? 0 : v;
  }
  #advance(): number {
    this.#value = this.#value + this.#step;
    return this.#value;
  }
  bump(): number {
    return this.#advance();
  }
  set(v: number): number {
    this.#value = v;
    return this.#value;
  }
  read(): number {
    return this.#value;
  }
  tryWriteMethod(): string {
    try {
      (this as any).#advance = 1;
      return "no-throw";
    } catch (e: any) {
      return e.constructor.name;
    }
  }
}
const cl = new Clamped();
console.log("clamped read0=" + cl.read() + " bump=" + cl.bump() + "," + cl.bump() + " set-neg=" + cl.set(-5) + " set-pos=" + cl.set(7) + " final=" + cl.read());
console.log("method-write=" + cl.tryWriteMethod());

// A getter-only private accessor refuses writes, even from inside the class.
class ReadOnlyPriv {
  #store: number = 3;
  get #view(): number {
    return this.#store;
  }
  read(): number {
    return this.#view;
  }
  write(): string {
    try {
      (this as any).#view = 9;
      return "no-throw";
    } catch (e: any) {
      return e.constructor.name;
    }
  }
}
const rp = new ReadOnlyPriv();
console.log("ro-read=" + rp.read() + " ro-write=" + rp.write());

// Compound assignment through a private accessor runs the setter exactly once.
class Doubler {
  #cell = 2;
  get #value(): number {
    return this.#cell;
  }
  set #value(v: number) {
    this.#cell = v * 2;
  }
  update(): number {
    return (this.#value += 3);
  }
  read(): number {
    return this.#cell;
  }
}
const dbl = new Doubler();
console.log("update=" + dbl.update() + " cell=" + dbl.read());

// An extracted private-reading method keeps its brand check.
class Secret {
  #value = 8;
  read(): number {
    return this.#value;
  }
}
const secret = new Secret();
const extracted = secret.read;
console.log("extracted-own=" + extracted.call(secret));
const brandChecks: boolean[] = [];
for (const receiver of [{}, Object.create(Secret.prototype)]) {
  try {
    extracted.call(receiver);
    brandChecks.push(false);
  } catch (e: any) {
    brandChecks.push(e instanceof TypeError);
  }
}
console.log("extracted-brand=" + brandChecks.join(","));

// --- private inheritance through several levels ---
class Animal {
  #name: string;
  #energy: number;
  constructor(name: string, energy = 100) {
    this.#name = name;
    this.#energy = energy;
  }
  #consume(amount: number): void {
    this.#energy = Math.max(0, this.#energy - amount);
  }
  eat(food: string, restore: number): string {
    this.#energy = Math.min(100, this.#energy + restore);
    return this.#name + " eats " + food;
  }
  move(cost: number): string {
    this.#consume(cost);
    return this.#name + " moves (energy:" + this.#energy + ")";
  }
  get name(): string {
    return this.#name;
  }
  get energy(): number {
    return this.#energy;
  }
}
class Dog extends Animal {
  #tricks: string[] = [];
  learn(trick: string): string {
    this.#tricks.push(trick);
    return this.name + " learned " + trick;
  }
  perform(): string {
    if (!this.#tricks.length) return this.name + " knows no tricks";
    const trick = this.#tricks[Math.floor(this.#tricks.length / 2)];
    return this.name + " performs " + trick;
  }
}
class GuideDog extends Dog {
  #owner: string;
  constructor(name: string, owner: string) {
    super(name);
    this.#owner = owner;
  }
  guide(): string {
    return this.name + " guides " + this.#owner;
  }
}
const rex = new Dog("Rex");
console.log(rex.eat("bone", 10));
console.log(rex.move(20));
console.log(rex.learn("sit"));
console.log(rex.learn("shake"));
console.log(rex.perform() + " energy=" + rex.energy);
const buddy = new GuideDog("Buddy", "Alice");
console.log(buddy.guide() + " | " + buddy.eat("kibble", 5) + " | " + buddy.move(10));
console.log("guide-instanceof=" + (buddy instanceof GuideDog) + "," + (buddy instanceof Dog) + "," + (buddy instanceof Animal));

// --- observable, value object and state machine with private state ---
class Observable<T> {
  #subscribers = new Set<(v: T) => void>();
  #lastValue: T | undefined;
  #hasValue = false;
  subscribe(fn: (v: T) => void): () => boolean {
    this.#subscribers.add(fn);
    if (this.#hasValue) fn(this.#lastValue as T);
    return () => this.#subscribers.delete(fn);
  }
  next(value: T): void {
    this.#lastValue = value;
    this.#hasValue = true;
    this.#subscribers.forEach((fn) => fn(value));
  }
  get subscriberCount(): number {
    return this.#subscribers.size;
  }
}
const obs = new Observable<number>();
const first: number[] = [];
const unsub = obs.subscribe((v) => first.push(v));
obs.next(1);
obs.next(2);
obs.next(3);
console.log("unsub=" + unsub() + " subscribers=" + obs.subscriberCount);
obs.next(4);
console.log("observed=" + first.join(","));
const late: number[] = [];
obs.subscribe((v) => late.push(v));
console.log("late=" + late.join(",") + " subscribers=" + obs.subscriberCount);

class Vec2 {
  #x: number;
  #y: number;
  constructor(x: number, y: number) {
    this.#x = x;
    this.#y = y;
  }
  get x(): number {
    return this.#x;
  }
  get y(): number {
    return this.#y;
  }
  add(other: Vec2): Vec2 {
    return new Vec2(this.#x + other.#x, this.#y + other.#y);
  }
  scale(s: number): Vec2 {
    return new Vec2(this.#x * s, this.#y * s);
  }
  dot(other: Vec2): number {
    return this.#x * other.#x + this.#y * other.#y;
  }
  get magnitude(): number {
    return Math.sqrt(this.#x ** 2 + this.#y ** 2);
  }
  toString(): string {
    return "Vec2(" + this.#x + "," + this.#y + ")";
  }
}
const va = new Vec2(3, 4);
const vb = new Vec2(1, 2);
console.log("vec=" + va.magnitude + " " + va.add(vb).toString() + " " + va.scale(2).toString() + " dot=" + va.dot(vb));
console.log("vec-keys=" + Object.keys(va).length + " json=" + JSON.stringify(va));

type State = "idle" | "running" | "paused" | "stopped";
class StateMachine {
  #state: State = "idle";
  #transitions: Map<State, State[]>;
  constructor() {
    this.#transitions = new Map<State, State[]>([
      ["idle", ["running"]],
      ["running", ["paused", "stopped"]],
      ["paused", ["running", "stopped"]],
      ["stopped", []],
    ]);
  }
  #canTransition(to: State): boolean {
    return (this.#transitions.get(this.#state) ?? []).includes(to);
  }
  transition(to: State): boolean {
    if (!this.#canTransition(to)) return false;
    this.#state = to;
    return true;
  }
  get state(): State {
    return this.#state;
  }
}
const sm = new StateMachine();
const trail: string[] = [sm.state];
for (const to of ["running", "idle", "paused", "stopped", "running"] as State[]) {
  trail.push(to + ":" + sm.transition(to));
}
console.log("machine=" + trail.join(" ") + " final=" + sm.state);
console.log("machine-own=" + hasOwn(sm, "state") + " json=" + JSON.stringify(sm));

// A private-name linked list with private helpers.
class LinkedList<T> {
  #head: { val: T; next: any } | null = null;
  #size = 0;
  #makeNode(val: T): { val: T; next: any } {
    return { val, next: null };
  }
  #findLast(): any {
    let cur: any = this.#head;
    while (cur?.next) cur = cur.next;
    return cur;
  }
  push(val: T): void {
    const node = this.#makeNode(val);
    const last = this.#findLast();
    if (last) last.next = node;
    else this.#head = node;
    this.#size++;
  }
  pop(): T | undefined {
    if (!this.#head) return undefined;
    if (!this.#head.next) {
      const only = this.#head.val;
      this.#head = null;
      this.#size--;
      return only;
    }
    let prev: any = null;
    let cur: any = this.#head;
    while (cur.next) {
      prev = cur;
      cur = cur.next;
    }
    prev.next = null;
    this.#size--;
    return cur.val;
  }
  get size(): number {
    return this.#size;
  }
  toArray(): T[] {
    const out: T[] = [];
    let cur: any = this.#head;
    while (cur) {
      out.push(cur.val);
      cur = cur.next;
    }
    return out;
  }
}
const ll = new LinkedList<number>();
ll.push(1);
ll.push(2);
ll.push(3);
const sizeBefore = ll.size;
const popped = ll.pop();
console.log("list=" + sizeBefore + " " + ll.toArray().join(",") + " popped=" + popped + " size=" + ll.size);
