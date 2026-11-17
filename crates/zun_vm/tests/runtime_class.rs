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
