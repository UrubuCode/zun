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
