//! `Reflect.construct` on the realm built-in constructors, and a class
//! prototype's symbol-keyed accessor (`get [Symbol.toStringTag]()`).

mod common;

use common::run_oxc;

fn run(body: &str) -> Vec<String> {
    run_oxc(&format!("const log: any[] = [];\n{body}\nconsole.log(log.join('|'));"))
}

#[test]
fn reflect_construct_array_has_both_specified_shapes() {
    let out = run(
        "log.push(JSON.stringify(Reflect.construct(Array, [1, 2, 3])));\
         log.push((Reflect.construct(Array, [3]) as any[]).length);\
         log.push((Reflect.construct(Array, ['3']) as any[]).length);\
         log.push(JSON.stringify(Reflect.construct(Array, { length: 2, 0: 'a', 1: 'b' } as any)));",
    );
    assert_eq!(out, ["[1,2,3]|3|1|[\"a\",\"b\"]"]);
}

#[test]
fn reflect_construct_map_set_object() {
    let out = run(
        "log.push((Reflect.construct(Map, [[['k', 1]]]) as Map<string, number>).get('k'));\
         log.push((Reflect.construct(Set, [[1, 2, 2]]) as Set<number>).size);\
         log.push((Reflect.construct(Object, [5]) as any).valueOf());\
         log.push(Object.keys(Reflect.construct(Object, []) as any).length);",
    );
    assert_eq!(out, ["1|2|5|0"]);
}

#[test]
fn reflect_construct_rejects_non_constructors_and_primitive_argument_lists() {
    let out = run(
        "const t = (f: () => any) => { try { f(); log.push('ok'); } catch (e: any) { log.push(e.name); } };\
         t(() => Reflect.construct(Symbol as any, []));\
         t(() => Reflect.construct(BigInt as any, [1]));\
         t(() => Reflect.construct(Array, 'ab' as any));\
         t(() => Reflect.construct(Array, [-1]));",
    );
    assert_eq!(out, ["TypeError|TypeError|TypeError|RangeError"]);
}

#[test]
fn reflect_construct_honours_new_target_for_collections() {
    let out = run(
        "function G(this: any) {}\
         const m: any = Reflect.construct(Map, [], G as any);\
         log.push(Object.getPrototypeOf(m) === G.prototype);\
         log.push(m instanceof G);",
    );
    assert_eq!(out, ["true|true"]);
}

#[test]
fn class_symbol_accessor_is_a_prototype_accessor_and_drives_to_string() {
    let out = run(
        "class Tagged { get [Symbol.toStringTag](): string { return 'MyClass'; } }\
         const t = new Tagged();\
         const d: any = Object.getOwnPropertyDescriptor(Tagged.prototype, Symbol.toStringTag);\
         log.push(Object.prototype.toString.call(t));\
         log.push(String(t));\
         log.push(typeof d.get, d.set, d.enumerable, d.configurable);\
         log.push((t as any)[Symbol.toStringTag]);\
         log.push(Symbol.toStringTag in t);\
         log.push(Object.keys(t).length, Object.getOwnPropertySymbols(Tagged.prototype).length);\
         log.push(Object.getOwnPropertySymbols(t).length);",
    );
    assert_eq!(
        out,
        ["[object MyClass]|[object MyClass]|function||false|true|MyClass|true|0|1|0"]
    );
}

#[test]
fn class_symbol_accessor_is_inherited_overridable_and_receiver_bound() {
    let out = run(
        "class Base { n = 0; get [Symbol.toStringTag](): string { this.n++; return 'B' + this.n; } }\
         class Derived extends Base {}\
         class Over extends Base { get [Symbol.toStringTag](): string { return 'Over'; } }\
         const b = new Base();\
         log.push(Object.prototype.toString.call(b), Object.prototype.toString.call(b), b.n);\
         log.push(Object.prototype.toString.call(new Derived()));\
         log.push(Object.prototype.toString.call(new Over()));\
         class Thrower { get [Symbol.toStringTag](): string { throw new RangeError('t'); } }\
         try { Object.prototype.toString.call(new Thrower()); } catch (e: any) { log.push(e.name); }",
    );
    assert_eq!(out, ["[object B1]|[object B2]|2|[object B1]|[object Over]|RangeError"]);
}
