// Constructor semantics: derived-constructor `this` TDZ, super() call rules,
// return-value overrides, implicit and explicit argument forwarding, new.target
// across super / Reflect.construct / arrows, [[Call]] refusal, and bound
// constructors (name, length, new.target, extends, Reflect.construct).

function hasOwn(o: object, k: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(o, k);
}

const trace: string[] = [];

class Base {
  tag: string = "base";
  constructor() {
    trace.push("base-ctor");
  }
}

// `this` is in TDZ until super() returns: reading it throws ReferenceError.
class TdzProbe extends Base {
  constructor() {
    try {
      trace.push("read=" + (this as any).tag);
    } catch (e: any) {
      trace.push("read-throws=" + e.constructor.name);
    }
    super();
    trace.push("after-super=" + this.tag);
  }
}
new TdzProbe();
console.log("tdz=" + trace.join("|"));
trace.length = 0;

class DoubleSuper extends Base {
  constructor() {
    super();
    super();
  }
}
try {
  new DoubleSuper();
  console.log("double=no-throw");
} catch (e: any) {
  console.log("double=" + e.constructor.name + ",ref=" + (e instanceof ReferenceError));
}

class NoSuper extends Base {
  constructor() {
    trace.push("nosuper-body");
  }
}
try {
  new NoSuper();
  console.log("nosuper=no-throw");
} catch (e: any) {
  console.log("nosuper=" + e.constructor.name);
}
console.log("nosuper-trace=" + trace.join("|"));
trace.length = 0;

// A method call before super() is still a `this` access.
class EarlyMethod extends Base {
  constructor() {
    let outcome = "no-throw";
    try {
      (this as any).describe();
    } catch (e: any) {
      outcome = e.constructor.name;
    }
    super();
    this.tag = outcome;
  }
  describe(): string {
    return "d";
  }
}
console.log("early-method=" + new EarlyMethod().tag);

// super() may be called from a nested arrow inside the constructor.
class ArrowSuper extends Base {
  constructor() {
    const go = () => {
      super();
    };
    go();
    this.tag = "arrow";
  }
}
console.log("arrow-super=" + new ArrowSuper().tag);

// new.target is readable before super(); it is not part of `this`.
class TargetBeforeSuper extends Base {
  constructor() {
    const nt: any = new.target;
    super();
    this.tag = nt.name;
  }
}
console.log("target-before-super=" + new TargetBeforeSuper().tag);

// --- return values from derived constructors ---
class ReturnObject extends Base {
  constructor() {
    super();
    this.tag = "ignored";
    return { tag: "replacement", extra: 1 } as any;
  }
}
const ro: any = new ReturnObject();
console.log("ret-object=" + ro.tag + "," + ro.extra + "," + (ro instanceof ReturnObject));

class ReturnBypass extends Base {
  constructor() {
    return { tag: "bypass" } as any;
  }
}
const bp0: any = new ReturnBypass();
console.log("ret-bypass=" + bp0.tag + "," + (bp0 instanceof ReturnBypass));

class ReturnUndefined extends Base {
  constructor() {
    super();
    return undefined as any;
  }
}
const ru = new ReturnUndefined();
console.log("ret-undefined=" + ru.tag + "," + (ru instanceof ReturnUndefined));

class ReturnPrimitive extends Base {
  constructor() {
    super();
    return 42 as any;
  }
}
try {
  new ReturnPrimitive();
  console.log("ret-prim=no-throw");
} catch (e: any) {
  console.log("ret-prim=" + e.constructor.name + ",type=" + (e instanceof TypeError));
}

class ReturnNullNoSuper extends Base {
  constructor() {
    return null as any;
  }
}
try {
  new ReturnNullNoSuper();
  console.log("ret-null=no-throw");
} catch (e: any) {
  console.log("ret-null=" + e.constructor.name);
}

// --- implicit and explicit forwarding ---
class ArgRecorder {
  args: string;
  count: number;
  nt: string;
  constructor(first: any, second: any, ...rest: any[]) {
    this.args = [first, second].concat(rest).map((v) => String(v)).join(",");
    this.count = arguments.length;
    this.nt = new.target === undefined ? "none" : (new.target as any).name;
  }
}
class Level1 extends ArgRecorder {}
class Level2 extends Level1 {
  tag: string = "c";
}
class Level3 extends Level2 {}
class Level4 extends Level3 {}

const l4 = new Level4(1, 2, 3, 4);
console.log("implicit-args=" + l4.args + " count=" + l4.count + " nt=" + l4.nt + " tag=" + l4.tag);
const l4short = new Level4("only");
console.log("implicit-short=" + l4short.args + " count=" + l4short.count);
const l1none = new Level1();
console.log("implicit-none=" + JSON.stringify(l1none.args) + " count=" + l1none.count);
const spreadArgs = [7, 8, 9];
console.log("implicit-spread=" + new Level2(...spreadArgs).args);
console.log("implicit-lengths=" + ArgRecorder.length + "," + Level1.length + "," + Level4.length);
console.log("proto-ctor=" + (Level1.prototype.constructor === Level1) + "," + (Level4.prototype.constructor === Level4));
console.log("proto-own-names=" + Object.getOwnPropertyNames(Level3.prototype).join(","));
console.log("implicit-distinct=" + (Level1.prototype.constructor !== ArgRecorder) + "," + (Level1.prototype.constructor !== Level3.prototype.constructor));

const viaReflectArgs: any = Reflect.construct(Level4, ["p", "q", "r"]);
console.log("reflect-implicit=" + viaReflectArgs.args + " count=" + viaReflectArgs.count);

class ExplicitForward extends ArgRecorder {
  constructor(...args: any[]) {
    super(...args);
  }
}
const ef: any = new ExplicitForward("e1", "e2", "e3");
console.log("explicit-forward=" + ef.args + " len=" + ExplicitForward.length + " implicit-len=" + Level1.length);

// Derived constructor transforms arguments before super().
class Pair {
  x: number;
  y: number;
  constructor(x: number, y: number) {
    this.x = x;
    this.y = y;
  }
}
class Shifted extends Pair {
  extra: number;
  constructor(x: number, y: number) {
    super(x + 1, y + 2);
    this.extra = this.x + this.y;
  }
}
const sh = new Shifted(3, 4);
console.log("shifted=" + sh.x + "," + sh.y + "," + sh.extra);

// --- base constructor calls an overridden method across three levels ---
const order: string[] = [];
function mark(tag: string, value: string): string {
  order.push(tag);
  return value;
}
class L1 {
  a: string = mark("L1.a", "a1");
  b: string = mark("L1.b", "b1");
  constructor() {
    order.push("L1.ctor describe=" + this.describe());
  }
  describe(): string {
    return "L1(" + this.a + "," + this.b + ")";
  }
}
class L2 extends L1 {
  c: string = mark("L2.c", "c2");
  d: string = mark("L2.d", "a-was:" + this.a);
  describe(): string {
    return "L2[" + String(this.c) + "|" + super.describe() + "]";
  }
}
class L3 extends L2 {
  e: string = mark("L3.e", "e3");
  describe(): string {
    return "L3{" + String(this.e) + "+" + super.describe() + "}";
  }
}
const inst = new L3();
for (let i = 0; i < order.length; i++) {
  console.log("order" + i + "=" + order[i]);
}
console.log("final-describe=" + inst.describe());
console.log("inst-keys=" + Object.keys(inst).join(","));

// --- new.target across super, Reflect.construct, functions, arrows, methods ---
const seen: string[] = [];
class NT {
  which: string;
  constructor() {
    const nt: any = new.target;
    this.which = nt === undefined ? "undefined" : nt.name;
    seen.push(this.which);
  }
}
class NTMid extends NT {}
class NTLeaf extends NTMid {}
console.log("nt-base=" + new NT().which + " mid=" + new NTMid().which + " leaf=" + new NTLeaf().which);

const viaReflect: any = Reflect.construct(NT, [], NTLeaf);
console.log("reflect-which=" + viaReflect.which);
console.log("reflect-proto=" + (Object.getPrototypeOf(viaReflect) === NTLeaf.prototype));
console.log("reflect-instanceof=" + (viaReflect instanceof NTLeaf) + "," + (viaReflect instanceof NT));
console.log("reflect-default=" + Reflect.construct(NT, []).which);

function Fn(this: any) {
  this.nt = new.target === undefined ? "undefined" : "Fn";
}
console.log("fn-new=" + (new (Fn as any)()).nt);
const bareObj: any = {};
(Fn as any).call(bareObj);
console.log("fn-call=" + bareObj.nt);

function Weird() {}
(Weird as any).prototype = 7;
const fallback: any = Reflect.construct(NT, [], Weird as any);
console.log("fallback-proto=" + (Object.getPrototypeOf(fallback) === Object.prototype) + " which=" + fallback.which);

function Point2(this: any, x: number) {
  this.x = x;
}
class OtherClass {}
const rcOther: any = Reflect.construct(Point2, [7], OtherClass);
console.log("rc-other=" + rcOther.x + ",base=" + (rcOther instanceof Point2) + ",other=" + (rcOther instanceof OtherClass));

class ArrowHolder {
  probe: string;
  constructor() {
    const f = () => (new.target === undefined ? "undefined" : (new.target as any).name);
    this.probe = f();
  }
}
class ArrowSub extends ArrowHolder {}
console.log("arrow-new-target=" + new ArrowHolder().probe + "," + new ArrowSub().probe);

const holder = {
  m() {
    return new.target === undefined ? "undefined" : "set";
  },
};
console.log("method-new-target=" + holder.m());

// --- [[Call]] refusal for classes ---
class OnlyNew {
  value = 1;
}
const refusals: string[] = [];
try {
  (OnlyNew as any)();
  refusals.push("no-throw");
} catch (e: any) {
  refusals.push(e.constructor.name);
}
try {
  OnlyNew.call({});
  refusals.push("no-throw");
} catch (e: any) {
  refusals.push(e.constructor.name);
}
try {
  Reflect.apply(OnlyNew as any, {}, []);
  refusals.push("no-throw");
} catch (e: any) {
  refusals.push(e.constructor.name);
}
console.log("call-refusals=" + refusals.join(","));
console.log("new-only-value=" + new OnlyNew().value);

// --- bound class constructors ---
const seenBound: string[] = [];
class Point {
  x: number = 0;
  y: number = 0;
  target: string = "";
  constructor(x: number, y: number) {
    this.x = x;
    this.y = y;
    this.target = new.target === undefined ? "none" : (new.target as any).name;
    seenBound.push("Point(" + x + "," + y + ")");
  }
  sum(): number {
    return this.x + this.y;
  }
}
const BoundPoint: any = Point.bind(null, 10);
console.log("bound-name=" + BoundPoint.name + " length=" + BoundPoint.length);
console.log("bound-has-prototype=" + hasOwn(BoundPoint, "prototype") + " typeof=" + typeof BoundPoint);

const bp = new BoundPoint(5);
console.log("bp-xy=" + bp.x + "," + bp.y + " sum=" + bp.sum() + " target=" + bp.target);
console.log("bp-instanceof=" + (bp instanceof Point) + "," + (bp instanceof BoundPoint));
console.log("bp-proto-is-point=" + (Object.getPrototypeOf(bp) === Point.prototype));

const Bound2: any = BoundPoint.bind(null, 7);
const bp2: any = new Bound2();
console.log("bound2-name=" + Bound2.name + " length=" + Bound2.length);
console.log("bp2-xy=" + bp2.x + "," + bp2.y + " target=" + bp2.target);

let plainCall = "no-throw";
try {
  BoundPoint(1);
} catch (e: any) {
  plainCall = e.constructor.name;
}
console.log("bound-plain-call=" + plainCall);

const rcBound: any = Reflect.construct(BoundPoint, [1]);
console.log("rc-bound=" + rcBound.x + "," + rcBound.y + " target=" + rcBound.target);

class OtherC {}
const rcBound2: any = Reflect.construct(BoundPoint, [2], OtherC);
console.log("rc-bound2=" + rcBound2.x + "," + rcBound2.y + " target=" + rcBound2.target + " proto=" + (Object.getPrototypeOf(rcBound2) === OtherC.prototype));

class Point3 extends Point {
  z: number = 0;
  constructor(x: number, y: number, z: number) {
    super(x, y);
    this.z = z;
  }
}
const p3 = new Point3(1, 2, 3);
console.log("p3-target=" + p3.target + " z=" + p3.z);
const BoundP3: any = Point3.bind(null, 4, 5);
const bp3: any = new BoundP3(6);
console.log("bp3=" + bp3.target + " z=" + bp3.z + " instanceof=" + (bp3 instanceof Point3) + "," + (bp3 instanceof Point));

// A bound function has no `prototype`, so `extends` refuses it until one is
// put back; the derived chain then follows the original class.
try {
  class BoundSub extends BoundPoint {}
  console.log("extends-bound=no-throw");
} catch (e: any) {
  console.log("extends-bound=" + e.constructor.name);
}
const Restored: any = Point.bind(null);
Object.defineProperty(Restored, "prototype", { value: Point.prototype, writable: false });
class FromRestored extends Restored {
  tag: string = "restored";
}
const fr: any = new FromRestored(8, 9);
console.log("restored-xy=" + fr.x + "," + fr.y + " tag=" + fr.tag + " target=" + fr.target);
console.log("restored-instanceof=" + (fr instanceof FromRestored) + "," + (fr instanceof Point));

console.log("seen=" + seenBound.join("|"));
console.log("seen-new=" + seen.join("|"));
