//! Classes extending a native built-in (`Array`, `ArrayBuffer`, a typed array,
//! `RegExp`). An instance is the native value itself carrying the class
//! prototype, so every case below checks both directions: what reflection
//! lists (`Object.keys`, `JSON.stringify`, `instanceof`, `getPrototypeOf`) and
//! what the value does (index, call, override, species).

mod common;

use common::run_oxc;

fn run(src: &str) -> Vec<String> {
    run_oxc(src)
}

#[test]
fn array_subclass_instance_is_a_real_array() {
    let out = run(r#"class Stack extends Array<number> {
            label: string = "stack";
            constructor(...items: number[]) { super(...items); }
            top(): number { return this[this.length - 1]; }
            get size(): number { return this.length; }
            set size(n: number) { this.length = n; }
        }
        function main() {
            const s: any = new Stack(1, 2, 3);
            console.log(s.length + ":" + s[0] + ":" + s.top() + ":" + s.label);
            console.log((s instanceof Stack) + ":" + (s instanceof Array) + ":" + Array.isArray(s));
            console.log(JSON.stringify(s) + "|" + Object.keys(s).join(","));
            console.log((Object.getPrototypeOf(s) === Stack.prototype) + ":"
                + (Object.getPrototypeOf(Stack.prototype) === Array.prototype) + ":"
                + (Object.getPrototypeOf(Stack) === Array));
            console.log(s.constructor.name + ":" + (s.constructor === Stack));
            console.log(s.hasOwnProperty("label") + ":" + s.hasOwnProperty("top") + ":" + ("top" in s));
            s.push(9);
            s.size = 2;
            console.log(s.size + ":" + s.join("/"));
            delete s.label;
            console.log(String(s.label) + ":" + s.hasOwnProperty("label"));
        }
        main();"#);
    assert_eq!(
        out,
        [
            "3:1:3:stack",
            "true:true:true",
            "[1,2,3]|0,1,2,label",
            "true:true:true",
            "Stack:true",
            "true:false:true",
            "2:1/2",
            "undefined:false",
        ]
    );
}

#[test]
fn array_subclass_override_reaches_the_builtin_through_super() {
    let out = run(r#"class Loud extends Array<number> {
            pushed = 0;
            push(...items: number[]): number { this.pushed += items.length; return super.push(...items); }
            join(sep?: string): string { return "<" + super.join(sep === undefined ? "," : sep) + ">"; }
        }
        function main() {
            const l: any = new Loud();
            console.log(l.push(1, 2) + ":" + l.pushed + ":" + l.length);
            console.log(l.join("-") + "|" + String(l) + "|" + Array.prototype.join.call(l, "+"));
        }
        main();"#);
    assert_eq!(out, ["2:2:2", "<1-2>|<1,2>|1+2"]);
}

#[test]
fn array_species_decides_the_constructor_of_derived_arrays() {
    let out = run(r#"class Sub extends Array {}
        class Plain extends Array { static get [Symbol.species]() { return Array; } }
        class Watch extends Array {
            static seen: string[] = [];
            constructor(...args: any[]) { Watch.seen.push(args.length + ":" + String(args[0])); super(...args); }
        }
        function main() {
            const s: any = Sub.from([1, 2, 3]);
            console.log((s instanceof Sub) + ":" + (Sub.of(1, 2) instanceof Sub));
            console.log((s.map((x: number) => x) instanceof Sub) + ":" + (s.filter(() => true) instanceof Sub)
                + ":" + (s.slice() instanceof Sub) + ":" + (s.concat([4]) instanceof Sub));
            const p: any = Plain.from([1, 2]);
            console.log((p.map((x: number) => x) instanceof Plain) + ":" + Array.isArray(p.map((x: number) => x)));
            const w: any = new Watch();
            w.push(1, 2, 3);
            w.map((x: number) => x);
            w.filter(() => true);
            console.log(Watch.seen.join(" "));
            const plain: any = [1, 2];
            console.log((plain.map((x: number) => x * 2) instanceof Sub) + ":" + plain.map((x: number) => x * 2).join(","));
        }
        main();"#);
    assert_eq!(
        out,
        ["true:true", "true:true:true:true", "false:true", "0:undefined 1:3 1:0", "false:2,4"]
    );
}

#[test]
fn buffer_and_typed_array_subclasses_keep_their_native_behaviour() {
    let out = run(r#"class Buf extends ArrayBuffer { half(): number { return this.byteLength / 2; } }
        class PlainBuf extends ArrayBuffer { static get [Symbol.species]() { return ArrayBuffer; } }
        class U8 extends Uint8Array { total(): number { let s = 0; for (const v of this) s += v; return s; } }
        class Wide extends Uint8Array { static get [Symbol.species]() { return Uint16Array; } }
        function main() {
            const b: any = new Buf(8);
            console.log(b.byteLength + ":" + b.half() + ":" + (b instanceof Buf) + ":" + (b instanceof ArrayBuffer));
            console.log((b.slice(0, 4) instanceof Buf) + ":" + b.slice(0, 4).byteLength + ":" + b.slice(0).constructor.name);
            console.log((new PlainBuf(8).slice(0, 4) instanceof PlainBuf) + ":" + new PlainBuf(4).slice(0).constructor.name);
            console.log(Object.prototype.toString.call(b) + ":" + (Object.getPrototypeOf(Buf.prototype) === ArrayBuffer.prototype));
            const u: any = new U8([1, 2, 3]);
            console.log(u.length + ":" + u[1] + ":" + u.total() + ":" + (u instanceof U8) + ":" + (u instanceof Uint8Array));
            console.log((u.subarray(1) instanceof U8) + ":" + (u.slice(1) instanceof U8) + ":" + u.map((x: number) => x * 2).join(","));
            const w: any = new Wide([1, 2, 3, 4]).slice(1, 3);
            console.log(w.constructor.name + ":" + w.BYTES_PER_ELEMENT + ":" + Array.from(w).join(","));
        }
        main();"#);
    assert_eq!(
        out,
        [
            "8:4:true:true",
            "true:4:Buf",
            "false:ArrayBuffer",
            "[object ArrayBuffer]:true",
            "3:2:6:true:true",
            "true:true:2,4,6",
            "Uint16Array:2:2,3",
        ]
    );
}

#[test]
fn class_value_in_a_dynamic_local_constructs() {
    let out = run(r#"class B { hi(): string { return "hi"; } }
        class S { static make(): any { const C: any = this; return new C(); } }
        class S2 extends S {}
        function mk(C: any): any { return new C(); }
        function main() {
            const C: any = B;
            const made: any = new C();
            console.log((made instanceof B) + ":" + made.hi() + ":" + (mk(B) instanceof B) + ":" + (S2.make() instanceof S2));
        }
        main();"#);
    assert_eq!(out, ["true:hi:true:true"]);
}
