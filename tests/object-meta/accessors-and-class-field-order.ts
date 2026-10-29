// Accessors (object literal, class, defineProperty, static, inherited from a prototype, hidden
// behind non-enumerable keys) and the exact evaluation order of class fields, constructors,
// static fields and static blocks across an inheritance chain. Also obfuscator-style patterns
// (computed names, non-enumerable getter vault, guarded lookups, toString overrides) that must
// keep their plain semantics.
export {};

// ---------- 1. Object-literal accessors ----------
const lit = {
  _v: 0,
  get value() { return this._v; },
  set value(n: number) { this._v = n < 0 ? 0 : n; },
};
console.log("lit_initial=" + lit.value);
lit.value = 5;
console.log("lit_set=" + lit.value);
lit.value = -3;
console.log("lit_clamped=" + lit.value + ",keys=" + Object.keys(lit).join("|"));

// a getter-only property silently ignores writes in sloppy code; Reflect.set reports the refusal
const roLit: any = { get only() { return 1; } };
console.log("getter_only_write=" + Reflect.set(roLit, "only", 2) + ",read=" + roLit.only);

// ---------- 2. Class accessors and conversions ----------
class Temperature {
  private _c: number;
  constructor(c: number) { this._c = c; }
  get celsius() { return this._c; }
  set celsius(v: number) { this._c = v; }
  get fahrenheit() { return this._c * 9 / 5 + 32; }
  set fahrenheit(v: number) { this._c = (v - 32) * 5 / 9; }
}
const t = new Temperature(0);
console.log("temp_c=" + t.celsius + ",f=" + t.fahrenheit);
t.fahrenheit = 212;
console.log("temp_after_f=" + t.celsius);
console.log("temp_own=" + Object.keys(t).join("|") + ",proto_has_c=" + Object.prototype.hasOwnProperty.call(Temperature.prototype, "celsius"));

class Config {
  static get version() { return "1.0.0"; }
  static set mode(_v: string) { /* write-only in spirit */ }
}
console.log("static_getter=" + Config.version + ",mode_read=" + (Config as any).mode);

// ---------- 3. Accessor inheritance and override ----------
class Base {
  protected _x = 0;
  get x() { return this._x; }
  set x(v: number) { this._x = v; }
  describe() { return "base:" + this._x; }
}
class Child extends Base {
  get x() { return this._x * 2; }
  set x(v: number) { this._x = v; }
}
const b = new Base();
b.x = 5;
console.log("base_x=" + b.x + ",describe=" + b.describe());
const c = new Child();
c.x = 5;
console.log("child_x=" + c.x + ",describe=" + c.describe());

// an accessor defined on a prototype with defineProperty, behind a private backing field
class Widget { private _visible = true; }
Object.defineProperty(Widget.prototype, "visible", {
  get(this: any) { return this._visible; },
  set(this: any, v: boolean) { this._visible = v; },
});
const w: any = new Widget();
console.log("widget_visible=" + w.visible);
w.visible = false;
console.log("widget_after=" + w.visible + ",own=" + Object.getOwnPropertyNames(w).join("|"));

// accessor descriptors through the class and the instance
const tgd = Object.getOwnPropertyDescriptor(Temperature.prototype, "celsius") as any;
console.log("accessor_desc=get:" + typeof tgd.get + ",set:" + typeof tgd.set + ",e=" + tgd.enumerable + ",c=" + tgd.configurable);

// defineProperty accessor with a closure, outside any class
let secret = 0;
const o: any = {};
Object.defineProperty(o, "secret", {
  get() { return secret; },
  set(v: number) { secret = v * 2; },
});
o.secret = 5;
console.log("closure_accessor=" + o.secret + ",keys=" + Object.keys(o).length);

// ---------- 4. Class field and constructor evaluation order ----------
const log: string[] = [];
class FBase {
  x = (log.push("Base.x"), 1);
  y = (log.push("Base.y"), 2);
  constructor() { log.push("Base.constructor"); }
}
class FChild extends FBase {
  z = (log.push("Child.z"), 3);
  constructor() {
    super();
    log.push("Child.constructor");
    this.z = (log.push("Child.z.reassign"), 99);
  }
}
new FChild();
console.log("field_order=" + log.join(","));

const order: string[] = [];
class A {
  a = (order.push("a-field"), 10);
  b: number;
  c = (order.push("c-field"), 30);
  constructor() {
    order.push("constructor-start");
    this.b = (order.push("b-constructor"), 20);
    order.push("constructor-end");
  }
}
const aInst = new A();
console.log("field_order_2=" + order.join(",") + ",values=" + aInst.a + "," + aInst.b + "," + aInst.c);

// static fields and static blocks run in textual order, parent statics before child statics
const slog: string[] = [];
class S {
  static a = (slog.push("S.a"), 1);
  static b = (slog.push("S.b"), 2);
  static { slog.push("S.block"); }
  static c = (slog.push("S.c"), 3);
}
console.log("static_order=" + slog.join(","));
const ilog: string[] = [];
class P { static pv = (ilog.push("P.pv"), "p"); }
class C2 extends P { static cv = (ilog.push("C.cv"), "c"); }
console.log("static_inherit_order=" + ilog.join(",") + ",values=" + C2.pv + C2.cv);

// a field initializer can read an earlier field
class Counter {
  start = 10;
  end = this.start * 2;
  range = this.end - this.start;
}
const ct = new Counter();
console.log("counter=" + ct.start + "," + ct.end + "," + ct.range);

// an instance field shadows an inherited accessor (define semantics), while assignment would call the setter
class Shadowing {
  get value(): number { return 1; }
}
class ShadowKid extends Shadowing {
  value = 2;
}
const sk: any = new ShadowKid();
console.log("field_shadows_accessor=" + sk.value + ",own=" + Object.getOwnPropertyNames(sk).join("|"));

// ---------- 5. Obfuscator-style patterns that must keep plain semantics ----------
// Pattern 1: values hidden behind non-enumerable getters
const vault: any = {};
const codes = [0x61, 0x62, 0x63, 0x64, 0x65];
codes.forEach((code, i) => {
  Object.defineProperty(vault, "_" + i, {
    get() { return String.fromCharCode(code); },
    enumerable: false,
    configurable: false,
  });
});
console.log("vault_keys=" + Object.keys(vault).length + ",read=" + (vault._0 + vault._1 + vault._2 + vault._3 + vault._4));

// Pattern 2: computed property names assembled at runtime
const k1 = ["c", "o", "n", "s", "t", "r", "u", "c", "t", "o", "r"].join("");
const k2 = ["t", "o", "S", "t", "r", "i", "n", "g"].join("");
const holder: any = { hello: 42 };
console.log("computed_types=" + typeof holder[k1] + "," + typeof holder[k2] + ",value=" + holder["he" + "llo"]);

// Pattern 3: a guarded lookup that refuses prototype-ish keys
function safeGet(o: any, key: string): any {
  if (key === "__proto__" || key === "constructor" || key === "prototype") return undefined;
  return o[key];
}
const guardedData: any = { a: 1, b: 2 };
Object.defineProperty(guardedData, "__proto__", { value: { evil: true }, enumerable: true, configurable: true, writable: true });
console.log("guard_a=" + safeGet(guardedData, "a") + ",proto=" + safeGet(guardedData, "__proto__") + ",b=" + safeGet(guardedData, "b"));
console.log("guard_plain_proto=" + (Object.getPrototypeOf(guardedData) === Object.prototype));

// Pattern 4: overriding toString on a single function instance
function fn42() { return 42; }
const origStr = Function.prototype.toString;
(fn42 as any).toString = () => "function fn42() { [obfuscated] }";
console.log("fn_tostring=" + (fn42 as any).toString() + ",call=" + fn42());
console.log("fn_proto_unchanged=" + (Function.prototype.toString === origStr));

// ---------- 6. Extra constructor arguments are ignored, but still evaluated ----------
let argEffects = 0;
function effect(v: string): string {
  argEffects = argEffects + 1;
  return v;
}
class Point2 {
  v: unknown;
  constructor(a: unknown) { this.v = a; }
}
const p2: any = new (Point2 as any)(1, effect("x"), effect("y"));
console.log("extra_user=" + p2.v + ",evaluated=" + argEffects);
const mapEx: any = new (Map as any)([["k", 1]], effect("m"));
console.log("extra_map=" + mapEx.get("k"));
const setEx: any = new (Set as any)([1, 2], effect("s"));
console.log("extra_set=" + setEx.size);
const urlEx: any = new (URL as any)("https://example.com/path", undefined, effect("u"));
console.log("extra_url=" + urlEx.pathname + ",evaluated=" + argEffects);
