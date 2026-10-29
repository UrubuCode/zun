// Object-oriented and reactive patterns: observers, singletons, factories,
// commands, fluent and immutable builders, Proxy-based dependency tracking,
// metadata inheritance, lazy iteration, async serialization, immutable undo
// history with structural diff, and JSON Pointer / deep merge.

// ---------------------------------------------------------------- observer

class EventEmitter {
  private listeners = new Map<string, Function[]>();
  on(event: string, fn: Function) {
    if (!this.listeners.has(event)) this.listeners.set(event, []);
    (this.listeners.get(event) as Function[]).push(fn);
    return this;
  }
  off(event: string, fn: Function) {
    const list = this.listeners.get(event) ?? [];
    this.listeners.set(event, list.filter((f) => f !== fn));
    return this;
  }
  emit(event: string, ...args: any[]) {
    (this.listeners.get(event) ?? []).forEach((f) => f(...args));
    return this;
  }
  count(event: string) { return (this.listeners.get(event) ?? []).length; }
}

(() => {
  const emitter = new EventEmitter();
  const log: string[] = [];
  const handler = (v: number) => log.push("got:" + v);
  emitter.on("data", handler);
  emitter.emit("data", 1).emit("data", 2);
  emitter.off("data", handler);
  emitter.emit("data", 3);
  console.log("observer.log=" + log.join(","));
  console.log("observer.count=" + emitter.count("data"));
  console.log("observer.unknown=" + emitter.emit("nothing").count("nothing"));
})();

// Once-listeners that remove themselves and add new listeners during emit.
(() => {
  class Emitter {
    listeners: Function[] = [];
    on(fn: Function) { this.listeners.push(fn); }
    once(fn: Function) {
      const wrap = (...args: any[]) => {
        this.listeners = this.listeners.filter((x) => x !== wrap);
        fn(...args);
      };
      this.on(wrap);
    }
    emit(v: string) {
      for (const fn of [...this.listeners]) fn(v);
    }
  }
  const log: string[] = [];
  const e = new Emitter();
  e.on((v: string) => log.push("a" + v));
  e.once((v: string) => {
    log.push("once" + v);
    e.on((x: string) => log.push("late" + x));
  });
  e.emit("1");
  e.emit("2");
  console.log("once.log=" + log.join(","));
  console.log("once.listeners=" + e.listeners.length);
})();

// ---------------------------------------------------------------- singleton, factory, command

class Config {
  private static inst: Config | null = null;
  private data: Record<string, string> = {};
  private constructor() {}
  static getInstance(): Config {
    if (Config.inst === null) Config.inst = new Config();
    return Config.inst;
  }
  set(k: string, v: string) { this.data[k] = v; }
  get(k: string): string | undefined { return this.data[k]; }
}
const c1 = Config.getInstance();
const c2 = Config.getInstance();
c1.set("env", "prod");
console.log("singleton.env=" + c2.get("env"));
console.log("singleton.same=" + (c1 === c2));
console.log("singleton.missing=" + String(c2.get("nope")));

interface Shape {
  area(): number;
  perimeter(): number;
}
class Circle implements Shape {
  private r: number;
  constructor(r: number) { this.r = r; }
  area() { return Math.PI * this.r * this.r; }
  perimeter() { return 2 * Math.PI * this.r; }
}
class Rect implements Shape {
  private w: number;
  private h: number;
  constructor(w: number, h: number) {
    this.w = w;
    this.h = h;
  }
  area() { return this.w * this.h; }
  perimeter() { return 2 * (this.w + this.h); }
}
function createShape(type: "circle" | "rect", ...args: number[]): Shape {
  if (type === "circle") return new Circle(args[0]);
  return new Rect(args[0], args[1]);
}
const circle = createShape("circle", 5);
const rect = createShape("rect", 3, 4);
console.log("factory.circle.area=" + Math.round(circle.area() * 100) / 100);
console.log("factory.rect.area=" + rect.area());
console.log("factory.rect.perimeter=" + rect.perimeter());
console.log("factory.square=" + createShape("rect", 2, 2).area());

interface Command {
  execute(): void;
  undo(): void;
}
class Counter {
  value = 0;
  history: Command[] = [];
  run(cmd: Command) {
    cmd.execute();
    this.history.push(cmd);
  }
  undo() {
    const cmd = this.history.pop();
    if (cmd) cmd.undo();
  }
}
const counter = new Counter();
counter.run({ execute() { counter.value += 5; }, undo() { counter.value -= 5; } });
counter.run({ execute() { counter.value *= 2; }, undo() { counter.value /= 2; } });
console.log("command.value=" + counter.value);
counter.undo();
console.log("command.undo1=" + counter.value);
counter.undo();
console.log("command.undo2=" + counter.value);
counter.undo();
console.log("command.undo_empty=" + counter.value + " depth=" + counter.history.length);

// ---------------------------------------------------------------- fluent and immutable builders

class QueryBuilder {
  #table = "";
  #conditions: string[] = [];
  #columns: string[] = ["*"];
  #orderBy = "";
  #limit = 0;
  #offset = 0;

  from(table: string) { this.#table = table; return this; }
  select(...cols: string[]) { this.#columns = cols; return this; }
  where(cond: string) { this.#conditions.push(cond); return this; }
  order(col: string, dir: "ASC" | "DESC" = "ASC") { this.#orderBy = col + " " + dir; return this; }
  take(n: number) { this.#limit = n; return this; }
  skip(n: number) { this.#offset = n; return this; }

  build(): string {
    let q = "SELECT " + this.#columns.join(", ") + " FROM " + this.#table;
    if (this.#conditions.length) q += " WHERE " + this.#conditions.join(" AND ");
    if (this.#orderBy) q += " ORDER BY " + this.#orderBy;
    if (this.#limit) q += " LIMIT " + this.#limit;
    if (this.#offset) q += " OFFSET " + this.#offset;
    return q;
  }
}
console.log("query.full=" + new QueryBuilder()
  .from("users")
  .select("id", "name", "email")
  .where("age > 18")
  .where("active = true")
  .order("name")
  .take(10)
  .skip(20)
  .build());
console.log("query.minimal=" + new QueryBuilder().from("t").build());
console.log("query.desc.zero=" + new QueryBuilder().from("t").order("x", "DESC").take(0).build());

class Vec3 {
  readonly x: number;
  readonly y: number;
  readonly z: number;
  constructor(x: number, y: number, z: number) {
    this.x = x;
    this.y = y;
    this.z = z;
  }
  withX(x: number) { return new Vec3(x, this.y, this.z); }
  withY(y: number) { return new Vec3(this.x, y, this.z); }
  withZ(z: number) { return new Vec3(this.x, this.y, z); }
  add(other: Vec3) { return new Vec3(this.x + other.x, this.y + other.y, this.z + other.z); }
  scale(s: number) { return new Vec3(this.x * s, this.y * s, this.z * s); }
  get magnitude() { return Math.sqrt(this.x ** 2 + this.y ** 2 + this.z ** 2); }
  toString() { return "Vec3(" + this.x + "," + this.y + "," + this.z + ")"; }
}
const base3 = new Vec3(1, 2, 3);
const v = base3.withX(10).scale(2).add(new Vec3(0, 1, 0));
console.log("vec.chain=" + v.toString());
console.log("vec.base_untouched=" + base3.toString());
console.log("vec.magnitude=" + Math.round(new Vec3(3, 4, 0).magnitude));
console.log("vec.withZ=" + base3.withZ(-1).toString() + " withY=" + base3.withY(0).toString());
console.log("vec.new_instances=" + (base3.withX(1) !== base3));

class FetchBuilder {
  #url = "";
  #headers: Record<string, string> = {};
  #timeout = 5000;

  url(u: string) { this.#url = u; return this; }
  header(k: string, v: string) { this.#headers[k] = v; return this; }
  timeout(ms: number) { this.#timeout = ms; return this; }
  describe(): string {
    return "GET " + this.#url + " headers:" + Object.keys(this.#headers).join(",") + " timeout:" + this.#timeout;
  }
}
console.log("fetch.describe=" + new FetchBuilder()
  .url("https://api.example.com/users")
  .header("Authorization", "Bearer token")
  .header("Accept", "application/json")
  .timeout(3000)
  .describe());
console.log("fetch.defaults=" + new FetchBuilder().url("/x").describe());

// ---------------------------------------------------------------- Proxy receiver and dependency tracking

(() => {
  const deps = new Map<string, Set<() => void>>();
  let active: (() => void) | null = null;
  const state = new Proxy({ price: 2, quantity: 3 }, {
    get(target, key, receiver) {
      if (active) {
        const bucket = deps.get(String(key)) || new Set<() => void>();
        bucket.add(active);
        deps.set(String(key), bucket);
      }
      return Reflect.get(target, key, receiver);
    },
    set(target, key, value, receiver) {
      const changed = Reflect.set(target, key, value, receiver);
      const subs = deps.get(String(key));
      if (subs) subs.forEach((fn) => fn());
      return changed;
    },
  });
  const seen: number[] = [];
  const effect = (fn: () => void) => {
    active = fn;
    fn();
    active = null;
  };
  effect(() => seen.push(state.price * state.quantity));
  state.price = 4;
  state.quantity = 5;
  console.log("reactive.seen=" + seen.join(","));
  console.log("reactive.keys=" + [...deps.keys()].join(","));
})();

// Nested effects: each effect records its own dependency set while it runs.
(() => {
  const deps = new Map<string, Set<Function>>();
  let active: Function | null = null;
  const state: any = new Proxy({ a: 1, b: 2 }, {
    get(t, k: string) {
      if (active) {
        if (!deps.has(k)) deps.set(k, new Set());
        (deps.get(k) as Set<Function>).add(active);
      }
      return t[k];
    },
    set(t, k: string, v) {
      t[k] = v;
      const subs = deps.get(k);
      if (subs) subs.forEach((fn) => fn());
      return true;
    },
  });
  const log: string[] = [];
  const effect = (fn: Function) => {
    active = fn;
    fn();
    active = null;
  };
  effect(() => log.push("sum=" + (state.a + state.b)));
  state.a = 5;
  state.b = 7;
  console.log("tracker.log=" + log.join("|"));
  console.log("tracker.deps=" + [...deps.entries()].map(([k, s]) => k + ":" + s.size).join(","));
})();

// Optional calls through a Proxy keep the receiver and skip argument evaluation.
(() => {
  const seen: string[] = [];
  const target = {
    value: 5,
    run(n: number) { return this.value + n; },
  };
  const proxy: any = new Proxy(target, {
    get(t, key, receiver) {
      seen.push("get:" + String(key));
      return Reflect.get(t, key, receiver);
    },
  });
  let args = 0;
  const arg = () => {
    args++;
    return 3;
  };
  console.log("optcall.run=" + proxy.run?.(arg()));
  console.log("optcall.missing=" + proxy.missing?.(arg()));
  console.log("optcall.args=" + args);
  console.log("optcall.seen=" + seen.join("|"));
})();

// ---------------------------------------------------------------- graph serialization and metadata

(() => {
  const root: any = { name: "root" };
  root.left = { name: "child", parent: root };
  root.right = root.left;
  const ids = new WeakMap<object, number>();
  let next = 1;
  const encoded = JSON.stringify(root, function (_key, value) {
    if (value && typeof value === "object") {
      if (ids.has(value)) return { $ref: ids.get(value) };
      ids.set(value, next++);
    }
    return value;
  });
  console.log("graph.json=" + encoded);
  console.log("graph.next=" + next);
  const list: any[] = [1, 2];
  list.push(list);
  const cyc = JSON.stringify(list, function (_key, value) {
    if (value && typeof value === "object") {
      if (ids.has(value)) return "[cycle]";
      ids.set(value, 99);
    }
    return value;
  });
  console.log("graph.cycle=" + cyc);
})();

(() => {
  const metadata = new WeakMap<object, Map<string, any>>();
  function define(target: object, key: string, value: any) {
    let map = metadata.get(target);
    if (!map) {
      map = new Map<string, any>();
      metadata.set(target, map);
    }
    map.set(key, value);
  }
  function read(target: object, key: string): any {
    for (let cursor: any = target; cursor; cursor = Object.getPrototypeOf(cursor)) {
      const map = metadata.get(cursor);
      if (map?.has(key)) return map.get(key);
    }
    return undefined;
  }
  class Base {}
  class Child extends Base {}
  define(Base.prototype, "role", "base");
  define(Child.prototype, "own", 7);
  define(Child.prototype, "role", "child");
  console.log("meta.child_role=" + read(Child.prototype, "role") + " own=" + read(Child.prototype, "own"));
  console.log("meta.instance=" + read(new Child(), "role") + " " + read(new Base(), "own"));
  console.log("meta.missing=" + String(read(new Child(), "nope")));
})();

// ---------------------------------------------------------------- lazy iteration

(() => {
  const seen: string[] = [];
  const source: any = {
    get 0() { seen.push("get0"); return "a"; },
    get 1() { seen.push("get1"); return "b"; },
    length: 2,
  };
  const iterable = {
    *[Symbol.iterator]() {
      try {
        for (let i = 0; i < source.length; i++) yield source[i];
      } finally {
        seen.push("close");
      }
    },
  };
  try {
    for (const value of iterable) {
      seen.push("value:" + value);
      if (value === "a") throw new Error("stop");
    }
  } catch (e: any) {
    seen.push("catch:" + e.message);
  }
  console.log("lazy.seen=" + seen.join("|"));
})();

// ---------------------------------------------------------------- immutable undo history with diff

type State = { items: string[]; sel: number; flags: { [k: string]: boolean } };

function freezeDeep<T>(v: T): T {
  if (v && typeof v === "object") {
    Object.getOwnPropertyNames(v).forEach((k) => freezeDeep((v as any)[k]));
    Object.freeze(v);
  }
  return v;
}

function makeStore(initial: State) {
  const past: State[] = [];
  const future: State[] = [];
  let present = freezeDeep(initial);
  return {
    get: () => present,
    apply(label: string, f: (s: State) => State) {
      past.push(present);
      future.length = 0;
      present = freezeDeep(f(present));
      return label + " -> " + JSON.stringify(present);
    },
    undo() {
      if (past.length === 0) return "undo:empty";
      future.push(present);
      present = past.pop() as State;
      return "undo -> " + JSON.stringify(present);
    },
    redo() {
      if (future.length === 0) return "redo:empty";
      past.push(present);
      present = future.pop() as State;
      return "redo -> " + JSON.stringify(present);
    },
    depth: () => past.length + "/" + future.length,
  };
}

const store = makeStore({ items: ["a"], sel: 0, flags: { dirty: false } });
console.log("undo.add_b=" + store.apply("add-b", (s) => ({ ...s, items: [...s.items, "b"] })));
console.log("undo.select=" + store.apply("select-1", (s) => ({ ...s, sel: 1 })));
console.log("undo.mark=" + store.apply("mark", (s) => ({ ...s, flags: { ...s.flags, dirty: true } })));
console.log("undo.depth=" + store.depth());
console.log("undo.1=" + store.undo());
console.log("undo.2=" + store.undo());
console.log("undo.depth_mid=" + store.depth());
console.log("undo.redo=" + store.redo());
console.log("undo.depth_after_redo=" + store.depth());
console.log("undo.add_c=" + store.apply("add-c", (s) => ({ ...s, items: [...s.items, "c"] })));
console.log("undo.future_cleared=" + store.depth());
console.log("undo.redo_empty=" + store.redo());
while (store.undo() !== "undo:empty") { /* drain history */ }
console.log("undo.drained=" + store.depth() + " " + JSON.stringify(store.get()));

const cur: any = store.get();
console.log("frozen.top=" + Object.isFrozen(cur) + " nested=" + Object.isFrozen(cur.flags) + " arr=" + Object.isFrozen(cur.items));
console.log("frozen.set_refused=" + Reflect.set(cur, "sel", 99) + " still=" + cur.sel);
console.log("frozen.delete_refused=" + Reflect.deleteProperty(cur, "sel") + " still=" + cur.sel);
console.log("frozen.push=" + (() => {
  try {
    cur.items.push("z");
    return "no-throw";
  } catch (e: any) {
    return e.name;
  }
})());

function diff(a: any, b: any, path: string, out: string[]): string[] {
  if (a === b) return out;
  const aObj = a && typeof a === "object";
  const bObj = b && typeof b === "object";
  if (!aObj || !bObj) {
    out.push(path + ": " + JSON.stringify(a) + " -> " + JSON.stringify(b));
    return out;
  }
  const keys = new Set<string>();
  Object.keys(a).forEach((k) => keys.add(k));
  Object.keys(b).forEach((k) => keys.add(k));
  for (const k of Array.from(keys).sort()) {
    const inA = Object.prototype.hasOwnProperty.call(a, k);
    const inB = Object.prototype.hasOwnProperty.call(b, k);
    if (!inA) out.push(path + "/" + k + ": <added> " + JSON.stringify(b[k]));
    else if (!inB) out.push(path + "/" + k + ": <removed> " + JSON.stringify(a[k]));
    else diff(a[k], b[k], path + "/" + k, out);
  }
  return out;
}
const v1 = { items: ["a", "b"], sel: 0, flags: { dirty: false, pinned: true } };
const v2 = { items: ["a", "B", "c"], sel: 0, flags: { dirty: true } };
diff(v1, v2, "", []).forEach((line, i) => console.log("diff." + i + "=" + line));
console.log("diff.self=" + JSON.stringify(diff(v1, v1, "", [])));
console.log("diff.prim=" + diff(1, "1", "", []).join("|"));
console.log("diff.null_obj=" + diff({ a: null }, { a: {} }, "", []).join("|"));

// ---------------------------------------------------------------- JSON Pointer and deep merge

const doc: any = {
  "": "empty-key",
  "a/b": "slash-key",
  "m~n": "tilde-key",
  " ": "space-key",
  users: [
    { id: 1, name: "ana", tags: ["x", "y"], meta: { active: true } },
    { id: 2, name: "bo", tags: [], meta: { active: false, note: null } },
  ],
  nested: { deep: { deeper: { value: 42 } } },
};

function unescapeToken(tok: string): string {
  return tok.split("~1").join("/").split("~0").join("~");
}

function resolvePointer(root: any, pointer: string): string {
  if (pointer === "") return "ROOT";
  if (pointer.charAt(0) !== "/") return "ERR:no-leading-slash";
  const toks = pointer.split("/").slice(1).map(unescapeToken);
  let cur = root;
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (cur === null || typeof cur !== "object") return "ERR:not-container@" + i;
    if (Array.isArray(cur)) {
      if (t === "-") return "ERR:dash";
      if (!/^(0|[1-9][0-9]*)$/.test(t)) return "ERR:bad-index:" + t;
      const n = Number(t);
      if (n >= cur.length) return "ERR:oob:" + t;
      cur = cur[n];
    } else {
      if (!Object.prototype.hasOwnProperty.call(cur, t)) return "ERR:missing:" + JSON.stringify(t);
      cur = cur[t];
    }
  }
  return JSON.stringify(cur);
}

const pointers = [
  "", "/", "/a~1b", "/m~0n", "/ ", "/nested/deep/deeper/value",
  "/users/0/name", "/users/1/meta/note", "/users/0/tags/1", "/users/2",
  "/users/-", "/users/01", "/users/0/nope", "/nested/deep/deeper/value/more",
  "no-slash", "/users/0/tags",
];
for (const p of pointers) console.log("pointer." + JSON.stringify(p) + "=" + resolvePointer(doc, p));

function merge(a: any, b: any, arrays: string): any {
  if (Array.isArray(a) && Array.isArray(b)) {
    if (arrays === "replace") return b.slice();
    if (arrays === "concat") return a.concat(b);
    const out = a.slice();
    for (let i = 0; i < b.length; i++) out[i] = i < a.length ? merge(a[i], b[i], arrays) : b[i];
    return out;
  }
  if (a && b && typeof a === "object" && typeof b === "object" && !Array.isArray(a) && !Array.isArray(b)) {
    const out: any = {};
    for (const k of Object.keys(a)) out[k] = a[k];
    for (const k of Object.keys(b)) {
      out[k] = Object.prototype.hasOwnProperty.call(a, k) ? merge(a[k], b[k], arrays) : b[k];
    }
    return out;
  }
  return b;
}

const baseDoc = { z: 1, a: { p: [1, 2, 3], q: "keep" }, 2: "two", 10: "ten", 1: "one" };
const over = { a: { p: [9], r: "new" }, y: 2, 3: "three" };
for (const strat of ["replace", "concat", "index"]) {
  const m = merge(baseDoc, over, strat);
  console.log("merge." + strat + ".keys=" + Object.keys(m).join(","));
  console.log("merge." + strat + ".json=" + JSON.stringify(m));
}
console.log("merge.null_over=" + JSON.stringify(merge({ a: 1 }, { a: null }, "replace")));
console.log("merge.undef_over=" + JSON.stringify(merge({ a: 1 }, { a: undefined }, "replace")));
console.log("merge.prim_over_obj=" + JSON.stringify(merge({ a: { b: 1 } }, { a: 5 }, "replace")));
console.log("merge.obj_over_prim=" + JSON.stringify(merge({ a: 5 }, { a: { b: 1 } }, "replace")));
console.log("merge.arr_over_obj=" + JSON.stringify(merge({ a: { b: 1 } }, { a: [1] }, "replace")));
