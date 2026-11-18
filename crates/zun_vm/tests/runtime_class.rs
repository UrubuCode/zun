//! Run-time classes: a class that is a value -- produced once per call of a
//! factory, or declared over a base that is only known at run time (a mixin).
//!
//! Such a class is lowered by the front end to a constructor function plus a
//! prototype object (see `zun_syntax`'s `runtime_class` and the prelude's
//! `builtin/runtime_class.zun`). Every case checks both directions against
//! what node answers: identity and reflection (what the class *lists*) and
//! behaviour (what constructing and calling it *does*).

mod common;

use common::run_oxc;

fn run(src: &str) -> Vec<String> {
    run_oxc(src)
}

#[test]
fn each_evaluation_is_its_own_class() {
    let out = run(r#"function make(tag: string): any { return class { label: string = tag; }; }
        function main() {
            const A: any = make("a"), B: any = make("b");
            console.log((A !== B) + ":" + (A.prototype !== B.prototype) + ":" + (A === A));
            console.log(new A().label + new B().label);
            console.log((new A() instanceof A) + ":" + (new A() instanceof B));
            console.log((new A().constructor === A) + ":" + (A.prototype.constructor === A));
            console.log((Object.getPrototypeOf(new A()) === A.prototype) + ":"
                + (Object.getPrototypeOf(A.prototype) === Object.prototype) + ":"
                + (Object.getPrototypeOf(A) === Function.prototype));
        }
        main();"#);
    assert_eq!(
        out,
        ["true:true:true", "ab", "true:false", "true:true", "true:true:true"]
    );
}

#[test]
fn members_close_over_the_factory_scope() {
    let out = run(r#"function counter(start: number): any {
            let shared = start;
            return class {
                static next(): number { return ++shared; }
                value = shared;
                bump(): number { shared += 10; return shared; }
            };
        }
        function main() {
            const C1: any = counter(1), C2: any = counter(100);
            const a = new C1();
            a.bump();
            console.log(C1.next() + ":" + C2.next() + ":" + new C1().value + ":" + new C2().value);
        }
        main();"#);
    assert_eq!(out, ["12:101:12:101"]);
}

#[test]
fn reflection_lists_what_a_class_lists() {
    let out = run(r#"function make(prefix: string): any {
            return class Full {
                id: number;
                extra = prefix + "!";
                static count = 0;
                constructor(id: number, second?: string) { this.id = id; }
                label(): string { return prefix + "-" + this.id; }
                get doubled(): number { return this.id * 2; }
                set doubled(v: number) { this.id = v / 2; }
                static get kind(): string { return "full:" + prefix; }
            };
        }
        function main() {
            const F: any = make("x");
            console.log(F.name + ":" + F.length + ":" + typeof F);
            console.log(Object.getOwnPropertyNames(F.prototype).join(","));
            console.log("[" + Object.keys(F.prototype).join(",") + "][" + Object.keys(F).join(",")
                + "][" + Object.keys(new F(1)).join(",") + "]");
            const keys: string[] = [];
            for (const k in new F(1)) keys.push(k);
            console.log(keys.join(","));
            const m: any = Object.getOwnPropertyDescriptor(F.prototype, "label");
            console.log(typeof m.value + ":" + m.writable + ":" + m.enumerable + ":" + m.configurable);
            const a: any = Object.getOwnPropertyDescriptor(F.prototype, "doubled");
            console.log(typeof a.get + ":" + typeof a.set + ":" + a.enumerable + ":" + a.configurable);
            const p: any = Object.getOwnPropertyDescriptor(F, "prototype");
            console.log(p.writable + ":" + p.enumerable + ":" + p.configurable);
            const inst = new F(4);
            const before = inst.doubled;
            inst.doubled = 20;
            console.log(inst.label() + ":" + inst.extra + ":" + before + ":" + inst.id + ":" + F.kind);
        }
        main();"#);
    assert_eq!(
        out,
        [
            "Full:2:function",
            "constructor,label,doubled",
            "[][count][id,extra]",
            "id,extra",
            "function:true:false:true",
            "function:function:false:true",
            "false:false:false",
            "x-10:x!:8:10:full:x",
        ]
    );
}

#[test]
fn a_mixin_reaches_its_run_time_base() {
    let out = run(r#"const order: string[] = [];
        class Core {
            name: string = "";
            constructor(name: string) { order.push("Core"); this.name = name; }
            describe(): string { return "Core(" + this.name + ")"; }
            static make(): string { return "made"; }
        }
        function mix(Base: any): any {
            return class Mixed extends Base {
                tag: string = "t";
                constructor(...args: any[]) {
                    order.push("before");
                    super(...args);
                    this.tag = this.tag + args.length;
                    order.push("after");
                }
                describe(): string { return "M<" + super.describe() + ">"; }
                static who(): string { return "mixed/" + super.make(); }
            };
        }
        function main() {
            const M1: any = mix(Core), M2: any = mix(Core);
            const m = new M1("n");
            console.log(m.describe() + ":" + m.name + ":" + m.tag + ":" + Object.keys(m).join(","));
            console.log(order.join(">"));
            console.log((M1 !== M2) + ":" + (m instanceof M1) + ":" + (m instanceof M2) + ":" + (m instanceof Core));
            console.log((Object.getPrototypeOf(M1.prototype) === Core.prototype) + ":"
                + (Object.getPrototypeOf(M1) === Core) + ":" + (m.constructor === M1));
            console.log(M1.who() + ":" + M1.make());
            const twice = new (mix(mix(Core)))("z");
            console.log(twice.describe() + ":" + (twice instanceof Core));
        }
        main();"#);
    assert_eq!(
        out,
        [
            "M<Core(n)>:n:t1:name,tag",
            "before>Core>after",
            "true:true:false:true",
            "true:true:true",
            "mixed/made:made",
            "M<M<Core(z)>>:true",
        ]
    );
}

#[test]
fn a_class_declared_over_a_run_time_base_is_usable_from_typed_code() {
    let out = run(r#"class Core {
            name: string = "";
            constructor(name: string) { this.name = name; }
            describe(): string { return "Core(" + this.name + ")"; }
        }
        function mix(Base: any): any {
            return class extends Base { extra = 1; };
        }
        const Composed: any = mix(Core);
        class Leaf extends Composed {
            leaf: string = "leaf";
            describe(): string { return "L<" + super.describe() + ">"; }
        }
        function main() {
            const leaf = new Leaf("deep");
            console.log(leaf.describe() + ":" + leaf.leaf + ":" + leaf.extra + ":" + leaf.name);
            console.log((leaf instanceof Leaf) + ":" + (leaf instanceof Composed) + ":" + (leaf instanceof Core));
            console.log(Leaf.name + ":" + leaf.constructor.name + ":" + Object.keys(leaf).join(","));
        }
        main();"#);
    assert_eq!(
        out,
        ["L<Core(deep)>:leaf:1:deep", "true:true:true", "Leaf:Leaf:name,extra,leaf"]
    );
}

#[test]
fn a_run_time_class_can_extend_a_built_in() {
    let out = run(r#"function mix(Base: any): any { return class extends Base { extra = 1; }; }
        function main() {
            const M: any = mix(Map);
            const m = new M([["a", 1]]);
            console.log(m.size + ":" + m.get("a") + ":" + m.extra + ":" + (m instanceof Map) + ":" + (m instanceof M));
            const A: any = mix(Array);
            const a = new A();
            a.push(1, 2);
            console.log(a.length + ":" + a.extra + ":" + Array.isArray(a) + ":" + (a instanceof A));
            const E: any = mix(Error);
            const e = new E("boom");
            console.log(e.message + ":" + e.extra + ":" + (e instanceof Error) + ":" + (e instanceof E));
        }
        main();"#);
    assert_eq!(out, ["1:1:1:true:true", "2:1:true:true", "boom:1:true:true"]);
}

#[test]
fn heritage_and_construction_errors_are_thrown() {
    let out = run(r#"class Core { name: string = ""; constructor(name: string) { this.name = name; } }
        function attempt(f: () => any): string {
            try { f(); return "no-throw"; } catch (e: any) { return e.constructor.name; }
        }
        function main() {
            const values: any[] = [7, undefined, "s"];
            for (const value of values) {
                console.log(attempt(() => { const made: any = class extends (value as any) {}; return made; }));
            }
            const base: any = Core;
            const NoSuper: any = class extends base { constructor() { } };
            console.log(attempt(() => new NoSuper()));
            const Plain: any = (function (): any { return class {}; })();
            console.log(attempt(() => Plain()));
            const Nully: any = (function (): any { return class extends (null as any) {}; })();
            console.log((Object.getPrototypeOf(Nully.prototype) === null) + ":" + attempt(() => new Nully()));
        }
        main();"#);
    assert_eq!(
        out,
        [
            "TypeError",
            "TypeError",
            "TypeError",
            "ReferenceError",
            "TypeError",
            "true:TypeError",
        ]
    );
}

#[test]
fn inherited_methods_take_part_in_conversion_and_static_dispatch() {
    let out = run(r#"function make(): any {
            return class Money {
                amount: number;
                constructor(amount: number) { this.amount = amount; }
                toString(): string { return this.amount + " USD"; }
                valueOf(): number { return this.amount; }
                static of(amount: number): any { return new this(amount); }
                static same(): boolean { return this === Money; }
            };
        }
        function main() {
            const Money: any = make();
            const m = Money.of(5);
            console.log(String(m) + ":" + `${m}` + ":" + (m + 1) + ":" + (m instanceof Money) + ":" + Money.same());
        }
        main();"#);
    assert_eq!(out, ["5 USD:5 USD:6:true:true"]);
}

// A class *declared* (or bound by a declarator) inside a function whose
// members mention the function's bindings is a closure too. A static TypeDef
// has no environment, so such a class is lowered like a class expression.
// Every expected line below is node's answer.

#[test]
fn a_declared_class_reads_the_enclosing_function_scope() {
    let out = run(r#"function mk(): any { const k = 5; class I { get(): number { return k } } return I }
        function mkp(p: number): any { class J { val(): number { return p * 2 } } return J }
        function bound(): any { let n = 1; const K = class { read(): number { return n } }; const k = new K(); n = 7; return k.read() + ":" + K.name }
        function callee(p: number): any { return new (class { v(): number { return p + 1 } })().v() }
        function main() {
            console.log(new (mk())().get() + ":" + new (mk())().get());
            console.log(new (mkp(21))().val() + ":" + new (mkp(4))().val());
            const A: any = mkp(1), B: any = mkp(100);
            console.log(new A().val() + ":" + new B().val() + ":" + (A === B) + ":" + (new A() instanceof B));
            console.log(bound() + ":" + bound());
            console.log(callee(1) + ":" + callee(2));
            let total = 0;
            for (let i = 0; i < 5000; i++) { total += new (mkp(i))().val() + new (mk())().get(); }
            console.log(total);
        }
        main();"#);
    assert_eq!(
        out,
        ["5:5", "42:8", "2:200:false:false", "7:K:7:K", "2:3", "25020000"]
    );
}

#[test]
fn a_declared_class_captures_by_reference_not_by_copy() {
    // The binding is shared: a write after the class is created is seen by
    // its methods, and a method and an arrow of the same scope see each
    // other's writes.
    let out = run(r#"function mut(): any { let n = 1; class K { read(): number { return n } } const k = new K(); n = 99; return k.read() }
        function shared(): any[] { let c = 0; class C { inc(): number { return ++c } } return [new C(), () => c] }
        function wr(): any {
            let c = 10;
            const peek = () => c;
            class W { set(v: number): void { c = v } add(): number { c += 5; return c } }
            const w = new W();
            w.set(1);
            const a = peek();
            const b = w.add();
            return [a, b, c, peek()].join(",");
        }
        function hot(): any { let total = 0; class H { add(x: number): number { total += x; return total } } const h = new H(); for (let i = 0; i < 5000; i++) h.add(1); return total }
        function main() {
            console.log(mut() + ":" + mut());
            for (let round = 0; round < 2; round++) {
                const [o, peek] = shared();
                console.log(o.inc() + "," + o.inc() + "," + peek());
            }
            console.log(wr() + ":" + wr());
            console.log(hot() + ":" + hot());
            let sum = 0;
            for (let i = 0; i < 5000; i++) { const [o, peek] = shared(); o.inc(); o.inc(); sum += peek() + mut(); }
            console.log(sum);
        }
        main();"#);
    assert_eq!(
        out,
        ["99:99", "1,2,2", "1,2,2", "1,6,6,6:1,6,6,6", "5000:5000", "505000"]
    );
}

#[test]
fn every_member_kind_of_a_declared_class_captures() {
    let out = run(r#"function kinds(seed: number): any {
            let log: string[] = [];
            function helper(x: number): number { return x + seed }
            class M {
                f: number = seed * 2;
                static s: number = seed + 1;
                constructor(extra: number) { log.push("ctor" + extra + ":" + seed) }
                get g(): number { return helper(1) }
                set g(v: number) { seed = v }
                static sm(): number { return helper(100) }
                m(): string { return log.join("|") }
            }
            const o = new M(3);
            const before = o.g; o.g = 50;
            return [o.f, M.s, before, o.g, M.sm(), o.m(), seed].join(",")
        }
        class Host { tag = "host"; make(): any { const self = this; class In { who(): string { return self.tag } } return new In().who() } }
        function inh(p: number): any {
            class A { a(): number { return p } }
            class B extends A { b(): number { return this.a() + p } }
            const b = new B();
            return [b.b(), b instanceof A, b instanceof B, B.name, Object.getPrototypeOf(B) === A].join(",")
        }
        function loop(): any { const out: any[] = []; for (let i = 0; i < 3; i++) { class L { v(): number { return i } } out.push(new L()) } return out.map((o: any) => o.v()).join(",") }
        function nest(a: number): any { class O { mk(b: number): any { class I { sum(): number { return a + b } } return new I() } } return new O().mk(2).sum() }
        function main() {
            console.log(kinds(4)); console.log(kinds(4));
            console.log(new Host().make() + ":" + new Host().make());
            console.log(inh(4)); console.log(inh(4));
            console.log(loop() + ":" + loop());
            console.log(nest(1) + ":" + nest(40));
        }
        main();"#);
    assert_eq!(
        out,
        [
            "8,5,5,51,150,ctor3:4,50",
            "8,5,5,51,150,ctor3:4,50",
            "host:host",
            "8,true,true,B,true",
            "8,true,true,B,true",
            "0,1,2:0,1,2",
            "3:42",
        ]
    );
}

#[test]
fn a_capture_is_not_a_property_of_the_class_or_its_instances() {
    let out = run(r#"function refl(p: number): any {
            let q = 1;
            class R { own: number = 1; constructor(a?: number, b?: number) {} m(x: number): number { return x + p + q } static st(): number { return q } }
            const r = new R();
            const d: any = Object.getOwnPropertyDescriptor(R.prototype, "m");
            const names: string[] = [];
            for (const key in r) names.push(key);
            return [
                JSON.stringify(Object.getOwnPropertyNames(r)), JSON.stringify(Object.keys(r)),
                JSON.stringify(Object.getOwnPropertyNames(R).sort()), JSON.stringify(Object.keys(R)),
                JSON.stringify(Object.getOwnPropertyNames(R.prototype).sort()), JSON.stringify(Object.keys(R.prototype)),
                "p" in r, "q" in r, "p" in R, "q" in R, "m" in r, "own" in r,
                R.length, R.prototype.m.length, R.name, typeof R, r.constructor === R,
                d.enumerable, d.writable, d.configurable,
                JSON.stringify(Object.getOwnPropertyDescriptor(r, "own")),
                JSON.stringify(r), names.join("+"), Reflect.ownKeys(r).length, Reflect.ownKeys(R.prototype).length,
            ].join(" ")
        }
        function main() { console.log(refl(1)); console.log(refl(2) === refl(3)); }
        main();"#);
    assert_eq!(
        out,
        [
            concat!(
                r#"["own"] ["own"] ["length","name","prototype","st"] [] ["constructor","m"] [] "#,
                "false false false false true true 2 1 R function true false true true ",
                r#"{"value":1,"writable":true,"enumerable":true,"configurable":true} {"own":1} own 1 2"#
            ),
            "true",
        ]
    );
}

#[test]
fn a_closure_written_before_a_capturing_class_reaches_it() {
    // JS binds a class from the top of its block. The lowered binding is
    // pre-declared when an earlier statement mentions it.
    let out = run(r#"function early(p: number): any { const mk = () => new K(); class K { m(): number { return p } } return mk().m() }
        function earlyFn(p: number): any { function mk(): any { return new K() } class K { m(): number { return p } } return mk().m() }
        function sibling(p: number): any { class X { mk(): any { return new A() } } class A { v(): number { return p } } return new X().mk().v() }
        function plain(): any { const mk = () => new K(); class K { m(): number { return 1 } } return mk().m() }
        function main() {
            console.log(early(3) + ":" + early(4));
            console.log(earlyFn(3) + ":" + earlyFn(4));
            console.log(sibling(3) + ":" + sibling(4));
            console.log(plain() + ":" + plain());
        }
        main();"#);
    assert_eq!(out, ["3:4", "3:4", "3:4", "1:1"]);
}

#[test]
fn a_lowered_class_is_not_confused_with_a_static_class_of_the_same_name() {
    // The checker resolves `new Name()`, `instanceof Name` and `: Name`
    // against the program-wide class table first. A lowered class is a
    // binding, so those forms must reach the binding even though a static
    // class of the same name exists in another function or at the top level.
    let out = run(r#"class G { m(): number { return 7 } }
        function plain(): any { class K { m(): number { return 1 } } return new K().m() }
        function direct(p: number): any { class K { m(): number { return p } } const mk = () => new K(); return [new K().m(), mk().m(), new K() instanceof K].join(",") }
        function early(p: number): any { const mk = () => new K(); class K { m(): number { return p } } return mk().m() }
        function shadow(p: number): any { class G { m(): number { return p } } const a: G = new G(); const f = (x: G): number => x.m(); return [a.m(), f(a), a instanceof G, new (G as any)().m()].join(",") }
        function inner(p: number): any { class K { m(): number { return p } } function g(): any { class K { m(): number { return 50 } } return new K().m() } return new K().m() + g() }
        function main() {
            console.log(direct(3) + ":" + direct(4));
            console.log(early(3) + ":" + early(4));
            console.log(shadow(3) + ":" + shadow(4));
            console.log(inner(3) + ":" + inner(4));
            console.log(plain() + ":" + new G().m() + ":" + (new G() instanceof G));
            let sum = 0;
            for (let i = 0; i < 5000; i++) { sum += inner(i) + early(1) + plain(); }
            console.log(sum);
        }
        main();"#);
    assert_eq!(
        out,
        [
            "3,3,true:4,4,true",
            "3:4",
            "3,3,true,3:4,4,true,4",
            "53:54",
            "1:7:true",
            "12757500",
        ]
    );
}

// Private names of a run-time class (`zun_syntax`'s `runtime_private`): a
// private member is per-evaluation state held in the class factory, never on
// the instance, the class or the prototype. Every expected line is node's.

#[test]
fn a_private_field_does_not_stop_a_capture_being_by_reference() {
    let out = run(r##"function t(label: string, f: () => any) { try { console.log(label, f()) } catch (e: any) { console.log(label, "THREW", e && e.constructor && e.constructor.name, e && e.message) } }
function mut(): any { let n = 1; class K { #tag = "k"; read(): string { return this.#tag + n } } const k = new K(); n = 99; return k.read() }
t("mut", mut); t("mut", mut)
function shared(): any[] { let c = 0; class C { #step = 1; inc(): number { c += this.#step; return c } } return [new C(), () => c] }
t("shared", () => { const [o, peek] = shared(); return [o.inc(), o.inc(), peek()].join(",") })
t("shared", () => { const [o, peek] = shared(); return [o.inc(), o.inc(), peek()].join(",") })
"##);
    let expected: &[&str] = &[
        r##"mut k99"##,
        r##"mut k99"##,
        r##"shared 1,2,2"##,
        r##"shared 1,2,2"##,
    ];
    assert_eq!(out, expected);
}

#[test]
fn every_private_member_kind_lowers_with_a_capture() {
    let out = run(r##"function t(label: string, f: () => any) { try { console.log(label, f()) } catch (e: any) { console.log(label, "THREW", e && e.constructor && e.constructor.name, e && e.message) } }
function kinds(seed: number): any {
  class M {
    #f: number = seed;
    #g: number;
    static #count = 0;
    static #made: any[] = [];
    constructor(g: number) { this.#g = g; M.#count++; M.#made.push(g) }
    #helper(x: number): number { return x + this.#f + seed }
    get #acc(): number { return this.#f * 2 }
    set #acc(v: number) { this.#f = v / 2 }
    static #smeth(x: number): number { return x + M.#count }
    static get #sacc(): string { return "s" + M.#count }
    run(): string {
      const a = this.#helper(1);
      this.#f += 10;
      this.#g++;
      const pre = this.#g++;
      this.#g **= 2;
      const b = this.#acc;
      this.#acc = 100;
      this.#f ||= 7;
      this.#f ??= 8;
      const fn = this.#helper;
      return [a, this.#f, this.#g, pre, b, fn.call(this, 0), M.#smeth(1), M.#sacc, M.#made.join("+"), M.#count].join(",")
    }
    static has(o: any): string { return [#f in o, #g in o, #helper in o, #acc in o, #count in o, #smeth in o].join("") }
    static count(): number { return M.#count }
  }
  const m = new M(3); new M(4);
  return m.run() + " " + M.has(m) + " " + M.has({}) + " " + M.has(M) + " " + M.count()
}
t("kinds", () => kinds(5)); t("kinds", () => kinds(5))
"##);
    let expected: &[&str] = &[
        r##"kinds 11,50,25,4,30,55,3,s2,3+4,2 truetruetruetruefalsefalse falsefalsefalsefalsefalsefalse falsefalsefalsefalsetruetrue 2"##,
        r##"kinds 11,50,25,4,30,55,3,s2,3+4,2 truetruetruetruefalsefalse falsefalsefalsefalsefalsefalse falsefalsefalsefalsetruetrue 2"##,
    ];
    assert_eq!(out, expected);
}

#[test]
fn a_private_brand_is_per_evaluation_and_follows_the_object_not_its_prototype() {
    let out = run(r##"function t(label: string, f: () => any) { try { console.log(label, f()) } catch (e: any) { console.log(label, "THREW", e && e.constructor && e.constructor.name, e && e.message) } }
function mk(tag: string): any { return class B { #x = tag; static is(o: any): boolean { return #x in o } static get(o: any): string { return o.#x } getX(): string { return this.#x } } }
t("brand", () => { const A = mk("a"), B = mk("b"); const a = new A(), b = new B(); const moved = new A(); Object.setPrototypeOf(moved, null); const fake = Object.create(A.prototype);
  const out: any[] = [A.is(a), A.is(b), B.is(b), B.is(a), A.is(moved), A.is(fake), A.is({}), A.is(A), A.get(a), B.get(b)];
  for (const f of [() => A.get(b), () => A.get(fake), () => A.is(5), () => A.is(null), () => A.is("s"), () => a.getX.call({}), () => a.getX.call(b), () => A.get(undefined)]) { try { out.push(f()) } catch (e: any) { out.push(e.constructor.name + ":" + e.message) } }
  return out.join(" | ") })
t("brand2", () => { const A = mk("a"); return A.is(new A()) + "," + A.get(new A()) })
"##);
    let expected: &[&str] = &[
        r##"brand true | false | true | false | true | false | false | false | a | b | TypeError:Cannot read private member #x from an object whose class did not declare it | TypeError:Cannot read private member #x from an object whose class did not declare it | TypeError:Cannot use 'in' operator to search for '#x' in 5 | TypeError:Cannot use 'in' operator to search for '#x' in null | TypeError:Cannot use 'in' operator to search for '#x' in s | TypeError:Cannot read private member #x from an object whose class did not declare it | TypeError:Cannot read private member #x from an object whose class did not declare it | TypeError:Cannot read properties of undefined (reading '#x')"##,
        r##"brand2 true,a"##,
    ];
    assert_eq!(out, expected);
}

#[test]
fn a_static_private_does_not_answer_to_a_subclass() {
    let out = run(r##"function t(label: string, f: () => any) { try { console.log(label, f()) } catch (e: any) { console.log(label, "THREW", e && e.constructor && e.constructor.name, e && e.message) } }
function stat(p: number): any {
  class S { static #secret = p; static read(): number { return this.#secret } static viaName(): number { return S.#secret } static #hid(): number { return p + 1 } static invoke(): number { return this.#hid() } static set(v: number): void { this.#secret = v } }
  class T extends S {}
  const out: any[] = [S.read(), S.viaName(), T.viaName(), S.invoke()];
  for (const f of [() => T.read(), () => T.invoke(), () => T.set(1), () => S.read.call({}), () => S.read.call(T)]) { try { out.push(f()) } catch (e: any) { out.push(e.constructor.name + ":" + e.message) } }
  S.set(50); out.push(S.read());
  return out.join(" | ")
}
t("stat", () => stat(7)); t("stat", () => stat(8))
"##);
    let expected: &[&str] = &[
        r##"stat 7 | 7 | 7 | 8 | TypeError:Cannot read private member #secret from an object whose class did not declare it | TypeError:Receiver must be class S | TypeError:Cannot write private member #secret to an object whose class did not declare it | TypeError:Cannot read private member #secret from an object whose class did not declare it | TypeError:Cannot read private member #secret from an object whose class did not declare it | 50"##,
        r##"stat 8 | 8 | 8 | 9 | TypeError:Cannot read private member #secret from an object whose class did not declare it | TypeError:Receiver must be class S | TypeError:Cannot write private member #secret to an object whose class did not declare it | TypeError:Cannot read private member #secret from an object whose class did not declare it | TypeError:Cannot read private member #secret from an object whose class did not declare it | 50"##,
    ];
    assert_eq!(out, expected);
}

#[test]
fn a_private_member_refuses_a_receiver_without_its_brand() {
    let out = run(r##"function t(label: string, f: () => any) { try { console.log(label, f()) } catch (e: any) { console.log(label, "THREW", e && e.constructor && e.constructor.name, e && e.message) } }
function wrong(p: number): any {
  class W { #v = p; #m(): number { return p } get #a(): number { return p } set #s(v: number) { }
    static probe(o: any): string { const out: string[] = []; for (const f of [() => o.#v, () => { o.#v = 1 }, () => o.#m(), () => typeof o.#m, () => { (o as any).#m = 1 }, () => o.#a, () => { o.#a = 1 }, () => o.#s, () => { o.#s = 1 }, () => { o.#v++ }]) { try { out.push(String(f())) } catch (e: any) { out.push(e.constructor.name + ":" + e.message) } } return out.join(" | ") } }
  return W.probe({}) + " || " + W.probe(new W())
}
t("wrong", () => wrong(3)); t("wrong", () => wrong(3))
"##);
    let expected: &[&str] = &[
        r##"wrong TypeError:Cannot read private member #v from an object whose class did not declare it | TypeError:Cannot write private member #v to an object whose class did not declare it | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Cannot read private member #v from an object whose class did not declare it || 3 | undefined | 3 | function | TypeError:Private method '#m' is not writable | 3 | TypeError:'#a' was defined without a setter | TypeError:'#s' was defined without a getter | undefined | undefined"##,
        r##"wrong TypeError:Cannot read private member #v from an object whose class did not declare it | TypeError:Cannot write private member #v to an object whose class did not declare it | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Receiver must be an instance of class W | TypeError:Cannot read private member #v from an object whose class did not declare it || 3 | undefined | 3 | function | TypeError:Private method '#m' is not writable | 3 | TypeError:'#a' was defined without a setter | TypeError:'#s' was defined without a getter | undefined | undefined"##,
    ];
    assert_eq!(out, expected);
}

#[test]
fn a_private_member_is_invisible_to_reflection() {
    let out = run(r##"function t(label: string, f: () => any) { try { console.log(label, f()) } catch (e: any) { console.log(label, "THREW", e && e.constructor && e.constructor.name, e && e.message) } }
function refl(p: number): any {
  class R { #a = p; #b = 2; pub = 1; static #s = 3; static sp = 4; #m(): number { return p } get #g(): number { return 1 } static #sm(): number { return p } use(): number { return this.#a + this.#b + this.#m() + this.#g + R.#s + R.#sm() } }
  const r = new R(); r.use();
  const names: string[] = []; for (const k in r) names.push(k);
  const snames: string[] = []; for (const k in R) snames.push(k);
  return [
    JSON.stringify(Object.getOwnPropertyNames(r)), JSON.stringify(Object.keys(r)), Object.getOwnPropertySymbols(r).length, JSON.stringify(Reflect.ownKeys(r)), JSON.stringify(r), names.join("+"),
    JSON.stringify(Object.getOwnPropertyNames(R).sort()), JSON.stringify(Object.keys(R)), snames.join("+"),
    JSON.stringify(Object.getOwnPropertyNames(R.prototype).sort()), JSON.stringify(Object.keys(R.prototype)), Object.getOwnPropertySymbols(R.prototype).length, JSON.stringify(Reflect.ownKeys(R.prototype)),
    "#a" in r, "a" in r, "#m" in r, "#s" in R, "s" in R, "#m" in R.prototype, "m" in R.prototype,
    Object.getOwnPropertyDescriptor(r, "#a"), Object.getOwnPropertyDescriptor(r, "a"), Object.getOwnPropertyDescriptor(R, "#s"), Object.getOwnPropertyDescriptor(R.prototype, "#m"),
    (r as any)["#a"], (r as any).a, (R as any)["#s"], JSON.stringify(Object.entries(r)), JSON.stringify({ ...r }), JSON.stringify(Object.assign({}, r)), r.use(),
  ].join(" ")
}
t("refl", () => refl(1)); t("refl", () => refl(1))
"##);
    let expected: &[&str] = &[
        r##"refl ["pub"] ["pub"] 0 ["pub"] {"pub":1} pub ["length","name","prototype","sp"] ["sp"] sp ["constructor","use"] [] 0 ["constructor","use"] false false false false false false false        [["pub",1]] {"pub":1} {"pub":1} 9"##,
        r##"refl ["pub"] ["pub"] 0 ["pub"] {"pub":1} pub ["length","name","prototype","sp"] ["sp"] sp ["constructor","use"] [] 0 ["constructor","use"] false false false false false false false        [["pub",1]] {"pub":1} {"pub":1} 9"##,
    ];
    assert_eq!(out, expected);
}

#[test]
fn an_inner_run_time_class_reads_the_outer_static_class_private() {
    let out = run(r##"function t(label: string, f: () => any) { try { console.log(label, f()) } catch (e: any) { console.log(label, "THREW", e && e.constructor && e.constructor.name, e && e.message) } }
class Outer {
  #n: string = "outer";
  makeInner(): any {
    const self = this;
    class Inner {
      #m: string = "inner";
      both(): string { return this.#m + "/" + self.#n }
      static innerBrand(o: any): boolean { return #m in o }
      static outerBrand(o: any): boolean { return #n in o }
    }
    return Inner;
  }
  static outerBrand(o: any): boolean { return #n in o }
}
t("nested", () => { const outer = new Outer(); const I: any = outer.makeInner(); const inner = new I(); const J: any = new Outer().makeInner();
  return [inner.both(), I.innerBrand(inner), I.innerBrand(outer), I.outerBrand(inner), I.outerBrand(outer), Outer.outerBrand(inner), Outer.outerBrand(outer), J.innerBrand(inner), J.innerBrand(new J()), Object.keys(inner).length, Object.keys(outer).length].join(",") })
t("nested", () => { const outer = new Outer(); const I: any = outer.makeInner(); return new I().both() })
"##);
    let expected: &[&str] = &[
        r##"nested inner/outer,true,false,false,true,false,true,false,true,0,0"##,
        r##"nested inner/outer"##,
    ];
    assert_eq!(out, expected);
}

#[test]
fn a_derived_run_time_class_brands_what_super_returns() {
    let out = run(r##"function t(label: string, f: () => any) { try { console.log(label, f()) } catch (e: any) { console.log(label, "THREW", e && e.constructor && e.constructor.name, e && e.message) } }
function derived(Base: any, p: number): any {
  class D extends Base { #d = p; #dm(): number { return this.#d + 1 } constructor(a: number) { super(a); this.#d += a } get d(): number { return this.#dm() } static is(o: any): boolean { return #d in o } }
  return D
}
class PlainBase { a: number; constructor(a: number) { this.a = a } }
t("derived", () => { const D = derived(PlainBase, 10); const d = new D(5); return [d.d, d.a, D.is(d), D.is(new PlainBase(1)), d instanceof PlainBase, Object.keys(d).join("+")].join(",") })
t("derived", () => { const D = derived(PlainBase, 20); const d = new D(5); return [d.d, d.a, D.is(d)].join(",") })
function twice(): any { class E { constructor(o: any) { return o } } class F extends E { #g = 2; constructor(o: any) { super(o) } static is(o: any): boolean { return #g in o } } const o = {}; new F(o); const out: any[] = [F.is(o)]; try { new F(o); out.push("no-throw") } catch (e: any) { out.push(e.constructor.name + ":" + e.message) } return out.join(",") }
t("twice", twice); t("twice", twice)
"##);
    let expected: &[&str] = &[
        r##"derived 16,5,true,false,true,a"##,
        r##"derived 26,5,true"##,
        r##"twice true,TypeError:Cannot initialize #g twice on the same object"##,
        r##"twice true,TypeError:Cannot initialize #g twice on the same object"##,
    ];
    assert_eq!(out, expected);
}

#[test]
fn private_members_hold_in_a_hot_loop() {
    let out = run(r##"function t(label: string, f: () => any) { try { console.log(label, f()) } catch (e: any) { console.log(label, "THREW", e && e.constructor && e.constructor.name, e && e.message) } }
function mut(): any { let n = 1; class K { #tag = "k"; read(): string { return this.#tag + n } } const k = new K(); n = 99; return k.read() }
function mk(tag: string): any { return class B { #x = tag; static is(o: any): boolean { return #x in o } static get(o: any): string { return o.#x } getX(): string { return this.#x } } }
class Outer {
  #n: string = "outer";
  makeInner(): any {
    const self = this;
    class Inner {
      #m: string = "inner";
      both(): string { return this.#m + "/" + self.#n }
      static innerBrand(o: any): boolean { return #m in o }
      static outerBrand(o: any): boolean { return #n in o }
    }
    return Inner;
  }
  static outerBrand(o: any): boolean { return #n in o }
}
function hot(): any { let total = 0; class H { #n = 0; add(x: number): number { this.#n += x; total += x; return this.#n } get n(): number { return this.#n } static is(o: any): boolean { return #n in o } } const h = new H(); for (let i = 0; i < 5000; i++) { h.add(1); if (!H.is(h)) return "lost" } return total + ":" + h.n }
t("hot", hot); t("hot", hot)
let acc = 0; for (let i = 0; i < 5000; i++) { const A = mk("t" + (i % 3)); const a = new A(); if (A.is(a) && !A.is({})) acc += A.get(a).length } console.log("hot-mk", acc)
let acc2 = 0; for (let i = 0; i < 5000; i++) { acc2 += mut().length } console.log("hot-mut", acc2)
let acc3 = 0; for (let i = 0; i < 5000; i++) { const I: any = new Outer().makeInner(); acc3 += new I().both().length } console.log("hot-nested", acc3)
"##);
    let expected: &[&str] = &[
        r##"hot 5000:5000"##,
        r##"hot 5000:5000"##,
        r##"hot-mk 10000"##,
        r##"hot-mut 15000"##,
        r##"hot-nested 55000"##,
    ];
    assert_eq!(out, expected);
}
