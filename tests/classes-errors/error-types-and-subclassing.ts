"use strict";
// Built-in error constructors: name and message resolution, toString, prototype
// shape and instanceof chains, subclassing (inherited and own names, prototype
// repair, constructor return swaps, multi-level hierarchies) and Reflect.construct
// with a foreign newTarget. Only constructor names, own-property facts,
// author-chosen messages and booleans are printed. Stack text is never printed.

function ownFlag(o: any, key: string): string {
  return String(Object.prototype.hasOwnProperty.call(o, key));
}
function ownMsg(e: any): string {
  return ownFlag(e, "message") + ":" + JSON.stringify(e.message);
}
function desc(o: any, key: string): string {
  const d: any = Object.getOwnPropertyDescriptor(o, key);
  return "w" + d.writable + ",e" + d.enumerable + ",c" + d.configurable;
}
function probe(fn: () => any): string {
  try {
    const v = fn();
    return "ok:" + String(v);
  } catch (e: any) {
    return e.constructor.name;
  }
}

// 1. Built-in constructors: name, message, constructor identity.
{
  const samples: any[] = [
    new Error("e"),
    new TypeError("t"),
    new RangeError("r"),
    new SyntaxError("s"),
    new ReferenceError("f"),
    new EvalError("v"),
    new URIError("u"),
    new AggregateError([], "a"),
  ];
  const classes: any[] = [Error, TypeError, RangeError, SyntaxError, ReferenceError, EvalError, URIError, AggregateError];
  for (let i = 0; i < samples.length; i++) {
    const e = samples[i];
    console.log("sample" + i + "=" + e.constructor.name + "|" + e.name + "|" + e.message);
  }
  // instanceof matrix: row i is sample i tested against each constructor above.
  for (let i = 0; i < samples.length; i++) {
    const row = classes.map((C: any) => (samples[i] instanceof C ? "1" : "0")).join("");
    console.log("matrix" + i + "=" + row);
  }
  console.log("own-name-flags=" + [new TypeError("x"), new Error("x")].map((e: any) => ownFlag(e, "name")).join(","));
}

// 2. Prototype shape and intrinsic metadata.
{
  console.log("chain-type=" + (Object.getPrototypeOf(TypeError.prototype) === Error.prototype));
  console.log("chain-ctor-type=" + (Object.getPrototypeOf(TypeError) === Error));
  console.log("chain-error-ctor=" + (Object.getPrototypeOf(Error) === Function.prototype));
  console.log("chain-agg=" + (Object.getPrototypeOf(AggregateError.prototype) === Error.prototype));
  console.log("chain-agg-ctor=" + (Object.getPrototypeOf(AggregateError) === Error));
  console.log("proto-ctor=" + (Error.prototype.constructor === Error));
  console.log("proto-of-proto=" + (Object.getPrototypeOf(Error.prototype) === Object.prototype));
  console.log("lengths=" + Error.length + "," + TypeError.length + "," + AggregateError.length);
  console.log("error-prototype-desc=" + desc(Error, "prototype"));
  console.log("proto-name-desc=" + desc(Error.prototype, "name"));
  console.log("proto-message-desc=" + desc(Error.prototype, "message"));
  console.log("proto-name=" + Error.prototype.name + ",proto-message=" + JSON.stringify(Error.prototype.message));
  console.log("proto-names=" + [TypeError, RangeError, SyntaxError, ReferenceError, EvalError, URIError, AggregateError].map((C: any) => C.prototype.name).join(","));
  console.log("own-proto-name-flags=" + [TypeError, RangeError, SyntaxError, ReferenceError, EvalError, URIError].map((C: any) => ownFlag(C.prototype, "name")).join(","));
}

// 3. Message argument coercion: undefined leaves no own message, everything else is ToString'd.
{
  console.log("no-arg=" + ownMsg(new Error()));
  console.log("undefined-arg=" + ownMsg(new Error(undefined)));
  console.log("empty-arg=" + ownMsg(new Error("")));
  console.log("null-arg=" + ownMsg(new Error(null as any)));
  console.log("zero-arg=" + ownMsg(new Error(0 as any)));
  console.log("false-arg=" + ownMsg(new Error(false as any)));
  console.log("nan-arg=" + ownMsg(new Error(NaN as any)));
  console.log("array-arg=" + ownMsg(new Error([1, 2] as any)));
  console.log("object-arg=" + ownMsg(new Error({} as any)));
  console.log("bigint-arg=" + ownMsg(new Error(10n as any)));
  console.log("tostring-arg=" + ownMsg(new Error({ toString: () => "custom" } as any)));
  console.log("toprimitive-arg=" + ownMsg(new Error({ [Symbol.toPrimitive]: (h: string) => "prim:" + h } as any)));
  console.log("symbol-arg=" + probe(() => new Error(Symbol("s") as any).message));
  console.log("throwing-tostring=" + probe(() => new Error({ toString() { throw new RangeError("x"); } } as any)));
  console.log("bare-tostring=" + new Error().toString());
  console.log("bare-own-message=" + ownFlag(new Error(), "message"));
}

// 4. Calling the constructors without new.
{
  const called: any = (Error as any)("called");
  console.log("called-instanceof=" + (called instanceof Error) + ",msg=" + called.message + ",proto=" + (Object.getPrototypeOf(called) === Error.prototype));
  console.log("called-distinct=" + ((Error as any)("a") !== (Error as any)("a")));
  console.log("called-kinds=" + [(TypeError as any)("t"), (RangeError as any)("r"), (URIError as any)("u"), (EvalError as any)("e"), (SyntaxError as any)("s"), (ReferenceError as any)("f")].map((e: any) => e.constructor.name).join(","));
  console.log("extra-args=" + ownMsg((Error as any)("m", { cause: 1 }, "ignored", 5)));
  const calledAgg: any = (AggregateError as any)([], "m");
  console.log("agg-called=" + calledAgg.constructor.name + ":" + calledAgg.message);
}

// 5. instanceof and prototype queries on ordinary objects.
{
  const parent: any = { x: 10 };
  const child = Object.create(parent);
  console.log("child-proto=" + (Object.getPrototypeOf(child) === parent) + ",x=" + child.x);
  console.log("is-proto-of=" + parent.isPrototypeOf(child) + ":" + child.isPrototypeOf(parent));
  console.log("object-proto-of-array=" + Object.prototype.isPrototypeOf([]) + ",array-proto-of-array=" + Array.prototype.isPrototypeOf([]));
  const swapped: any = { a: 1 };
  Object.setPrototypeOf(swapped, { b: 2 });
  console.log("setproto=" + swapped.b + ",own-a=" + ownFlag(swapped, "a"));
  console.log("typeof=" + [typeof 1, typeof "x", typeof true, typeof undefined, typeof (() => 1), typeof {}, typeof [], typeof null].join(","));
  class BaseThing {}
  class SubThing extends BaseThing {}
  const item = new SubThing();
  console.log("inst=" + [item instanceof SubThing, item instanceof BaseThing, item instanceof Object].join(":"));
}

// 6. Own versus inherited name and message, and the Object.prototype.toString tags.
{
  const e = new Error("boom");
  console.log("own-flags=" + ownFlag(e, "name") + "," + ownFlag(e, "message"));
  console.log("proto-own-flags=" + ownFlag(Error.prototype, "name") + "," + ownFlag(Error.prototype, "message"));
  const md = desc(e, "message");
  console.log("own-message-desc=" + md);
  console.log("json=" + JSON.stringify(e) + ",keys=" + Object.keys(e).length);
  console.log("json-array=" + JSON.stringify([e]));
  console.log("tags=" + [Object.prototype.toString.call(e), Object.prototype.toString.call(new TypeError("x")), Object.prototype.toString.call(new AggregateError([])), Object.prototype.toString.call(Error.prototype)].join("|"));
}

// 7. toString branches: empty, undefined and coerced name and message, and borrowed receivers.
{
  const t: any = new Error("msg");
  t.name = "";
  console.log("ts-empty-name=" + JSON.stringify(t.toString()));
  t.name = "Custom";
  console.log("ts-custom=" + t.toString());
  t.message = "";
  console.log("ts-empty-message=" + JSON.stringify(t.toString()));
  t.name = "";
  console.log("ts-both-empty=" + JSON.stringify(t.toString()));
  const u: any = new Error("m");
  u.name = undefined;
  console.log("ts-undef-name=" + u.toString());
  const u2: any = new Error("m");
  u2.message = undefined;
  console.log("ts-undef-message=" + u2.toString());
  const c: any = new Error();
  c.name = 7;
  c.message = { toString: () => "objmsg" };
  console.log("ts-coerced=" + c.toString());
  console.log("ts-borrowed=" + Error.prototype.toString.call({ name: "N", message: "M" }));
  console.log("ts-borrowed-empty=" + JSON.stringify(Error.prototype.toString.call({})));
  console.log("ts-borrowed-primitive=" + probe(() => Error.prototype.toString.call("string" as any)));
}

// 8. Subclassing: inherited name, prototype-level name, own name, class field name.
{
  class Plain extends Error {}
  const p = new Plain("m");
  console.log("plain-name=" + p.name + ",own-name=" + ownFlag(p, "name") + ",ctor=" + p.constructor.name);
  console.log("plain-tostring=" + p.toString() + ",instanceof=" + (p instanceof Plain) + ":" + (p instanceof Error));

  class Named extends Error {}
  Named.prototype.name = "Named";
  const n = new Named("m");
  console.log("named=" + n.name + ",own=" + ownFlag(n, "name") + ",tostring=" + n.toString());
  console.log("named-keys=" + Object.keys(n).length + ",json=" + JSON.stringify(n));

  class OwnName extends Error {
    constructor(message: string) {
      super(message);
      this.name = "OwnName";
    }
  }
  const on = new OwnName("m");
  console.log("own-name=" + on.name + ",own=" + ownFlag(on, "name") + ",desc=" + desc(on, "name"));
  console.log("own-keys=" + Object.keys(on).join(",") + ",json=" + JSON.stringify(on));

  class FieldName extends Error {
    name: string = "FieldName";
  }
  const fn = new FieldName("m");
  console.log("field-name=" + fn.name + ",keys=" + Object.keys(fn).join(","));
}

// 9. Broken and repaired prototype chains.
{
  class Broken extends Error {}
  Object.setPrototypeOf(Broken.prototype, Object.prototype);
  const br: any = new Broken("m");
  console.log("broken-instanceof=" + (br instanceof Error) + ":" + (br instanceof Broken));
  console.log("broken-name=" + String(br.name) + ",tag=" + Object.prototype.toString.call(br));

  class Repaired extends Error {
    constructor(message: string) {
      super(message);
      Object.setPrototypeOf(this, Repaired.prototype);
    }
  }
  const rp = new Repaired("m");
  console.log("repaired=" + (rp instanceof Repaired) + ":" + (rp instanceof Error) + ",proto=" + (Object.getPrototypeOf(rp) === Repaired.prototype));
}

// 10. A constructor that returns a different error object.
{
  class Swapper extends Error {
    constructor() {
      super("ignored");
      return new TypeError("swapped") as any;
    }
  }
  const sw: any = new Swapper();
  console.log("swap-ctor=" + sw.constructor.name + ",instanceof-swapper=" + (sw instanceof Swapper));
  console.log("swap-instanceof-type=" + (sw instanceof TypeError) + ",message=" + sw.message);
}

// 11. Three-level hierarchy with per-level prototype names.
{
  class L1 extends Error {}
  L1.prototype.name = "L1";
  class L2 extends L1 {}
  L2.prototype.name = "L2";
  class L3 extends L2 {}
  const l3 = new L3("deep");
  console.log("l3-name=" + l3.name);
  console.log("l3-chain=" + [l3 instanceof L1, l3 instanceof L2, l3 instanceof L3, l3 instanceof Error, l3 instanceof TypeError].join(":"));
  console.log("l3-tostring=" + l3.toString() + ",ctor-chain=" + (Object.getPrototypeOf(L3) === L2));
}

// 12. Reflect.construct with a foreign newTarget: the prototype comes from newTarget,
// the internal slots from the constructor that ran.
{
  class Marker extends Error {}
  const foreign: any = Reflect.construct(Error, ["msg"], Marker);
  console.log("rc-proto=" + (Object.getPrototypeOf(foreign) === Marker.prototype) + ",instanceof=" + (foreign instanceof Marker) + ":" + (foreign instanceof Error));
  console.log("rc-message=" + foreign.message + ",name=" + foreign.name);

  class Tagged extends Error {
    tag: string = "tagged";
  }
  const asTagged: any = Reflect.construct(Error, ["m1"], Tagged);
  console.log("rc-tagged-field=" + String(asTagged.tag) + ",keys=" + Object.keys(asTagged).length + ",brand=" + Object.prototype.toString.call(asTagged));
  const realTagged: any = Reflect.construct(Tagged, ["m2"]);
  console.log("rc-real-tag=" + realTagged.tag + ",keys=" + Object.keys(realTagged).join(",") + ",message=" + realTagged.message);

  const crossed: any = Reflect.construct(TypeError, ["m3"], RangeError);
  console.log("crossed-name=" + crossed.name + ",tostring=" + crossed.toString());
  console.log("crossed-instanceof=" + [crossed instanceof RangeError, crossed instanceof TypeError, crossed instanceof Error].join(":"));

  function Odd(): void {
    // nothing
  }
  (Odd as any).prototype = 5;
  const fallback: any = Reflect.construct(RangeError, ["m7"], Odd);
  console.log("fallback=" + (Object.getPrototypeOf(fallback) === RangeError.prototype) + ",name=" + fallback.name);

  const arrow = (): number => 1;
  console.log("target-arrow=" + probe(() => Reflect.construct(arrow as any, [])));
  console.log("newtarget-arrow=" + probe(() => Reflect.construct(Error, [], arrow as any)));
  console.log("args-not-object=" + probe(() => Reflect.construct(Error, 5 as any)));
  console.log("args-arraylike=" + ownMsg(Reflect.construct(Error, { length: 1, 0: "from-arraylike" } as any)));
}

// 13. Built-in error subclass shapes through a user class with an extra field.
{
  class Wrapped extends Error {
    detail: string = "";
    constructor(message: string, detail: string) {
      super(message);
      this.name = "Wrapped";
      this.detail = detail;
    }
  }
  const w: any = new Wrapped("m8", "d8");
  console.log("wrapped=" + w.name + "|" + w.detail + "|" + w.message + "|" + Object.keys(w).sort().join(","));
  console.log("wrapped-instanceof=" + (w instanceof Wrapped) + "," + (w instanceof Error) + ",ctor=" + (Wrapped.prototype.constructor === Wrapped));
}
