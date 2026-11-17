//! Regression tests for engine gaps fixed as a cleanup pass: numeric computed
//! keys on `any` values, and the length-only typed-array constructor.

use zun_interp::CaptureOutput;

fn out(user: &str) -> Vec<String> {
    let src = zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::oxc::parse_subset(&src, true)
        .unwrap_or_else(|errors| panic!("parse errors: {errors:?}"));
    assert!(diags.is_empty(), "parse: {diags:#?}");
    let hir = zun_sema::check(&src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:#?}");
    let bytes = zun_ilgen::lower(&hir).write();
    let mut o = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut o).expect("runs");
    o.lines
}

#[test]
fn any_numeric_index_read_write() {
    // Reading and writing a numeric computed key on an `any` holding an array,
    // and a numeric property on an `any` holding a plain object, used to throw
    // BadToken.
    assert_eq!(
        out(r#"function main(): void {
            let a: any = [10.0, 20.0, 30.0];
            print(a[1] as number);
            a[1] = 99.0;
            print(a[1] as number);
            let o: any = {};
            o[2] = 7.0;
            print(o[2] as number);
            print(typeof a[5]);
        }"#),
        // a[1], a[1] after write, o[2], and an out-of-range array read is undefined.
        vec!["20", "99", "7", "undefined"],
    );
}

#[test]
fn typed_array_length_constructor() {
    assert_eq!(
        out(r#"function main(): void {
            let a = new Uint8Array(3);
            a.set(0, 5);
            a.set(1, 255);
            print(a.length);
            print(a.get(1));
            let b = new Uint32Array(4);
            print(b.length);
            print(b.byteLength());
            let c = new Uint8Array(new ArrayBuffer(8), 0, 8);
            print(c.length);
        }"#),
        vec!["3", "255", "4", "16", "8"],
    );
}

#[test]
fn function_values_compare_by_identity() {
    // `==`/`!=` on function values is reference identity: the same function is
    // equal to itself; a different function is not. (Functions are JIT-ineligible,
    // so this is an interpreter path.)
    assert_eq!(
        out(r#"
        function helper(): number { return 1.0; }
        function other(): number { return 2.0; }
        function main(): void {
            let a = helper; let b = helper; let c = other;
            print(a == b);
            print(a != b);
            print(a == c);
        }"#),
        vec!["true", "false", "false"],
    );
}

#[test]
fn static_generator_method_owns_a_prototype() {
    // A static generator method read as a value rides a function box; its
    // `.prototype` used to read `undefined` where a free generator function
    // answered an object. Ordinary static methods still own none.
    assert_eq!(
        out(r#"class Bag {
            static *range(): any { yield 2; }
            static plainStatic(): number { return 1; }
        }
        function* plain(): any { yield 1; }
        const range: any = Bag.range;
        console.log(typeof (plain as any).prototype, typeof range.prototype, typeof range["prototype"]);
        console.log(typeof (Bag.plainStatic as any).prototype);
        console.log(range.prototype === range.prototype, range.prototype !== (plain as any).prototype);
        console.log(Object.getPrototypeOf(range.prototype) === Object.getPrototypeOf((plain as any).prototype));
        console.log([...Bag.range()].join());"#),
        ["object object object", "undefined", "true true", "true", "2"]
    );
}

#[test]
fn a_bigint_compares_with_other_primitives_exactly() {
    // Relational operators accept a BigInt against a number, a string, a
    // boolean or a nullish value (only arithmetic refuses the mix), and the
    // comparison is exact: it never rounds the BigInt through a `number`.
    assert_eq!(
        out(r#"const big: any = 5n;
        // @ts-ignore
        console.log(1n < 2, 2n >= 3, 1n < 1.5, 1n < NaN, 1n >= NaN, 1n < "2", 1n < "abc", 1n >= "abc");
        // @ts-ignore
        console.log("2.5" < 3n, "" < 1n, "0x10" > 15n, true < 2n, 0n >= null, 1n < undefined);
        console.log((2n ** 1024n) < Infinity, (-(2n ** 1024n)) > -Infinity);
        console.log((2n ** 53n + 1n) > 9007199254740992, 9007199254740992 < (2n ** 53n + 1n));
        console.log(big < 6, big > 4.5, big < ({ valueOf() { return 6; } } as any));
        try { console.log(big < (Symbol("s") as any)); } catch (e: any) { console.log(e.name + ": " + e.message); }
        try { console.log(big + 1); } catch (e: any) { console.log(e.name + ": " + e.message); }"#),
        [
            "true false true false false true false false",
            "false true true true true false",
            "true true",
            "true true",
            "true true true",
            "TypeError: Cannot convert a Symbol value to a number",
            "TypeError: Cannot mix BigInt and other types, use explicit conversions",
        ]
    );
}

#[test]
fn buffer_concat_takes_a_total_length_and_validates_its_list() {
    assert_eq!(
        out(r#"const show = (f: () => string): string => {
            try { return f(); } catch (e: any) { return e.name + ": " + e.message; }
        };
        console.log(show(() => Buffer.concat([Buffer.from("abc"), Buffer.from("de")], 4).toString("hex")));
        console.log(show(() => Buffer.concat([Buffer.from("a")], 3).toString("hex")));
        console.log(show(() => Buffer.concat([Buffer.from("abc")], 0).toString("hex")) + "|" + show(() => Buffer.concat([Buffer.from("abc")], undefined).toString("hex")));
        console.log(show(() => Buffer.concat([new Uint8Array([1, 2]), Buffer.from("a")]).toString("hex")));
        console.log(show(() => Buffer.concat(["a"] as any).toString("hex")));
        console.log(show(() => Buffer.concat("abc" as any).toString("hex")));
        console.log(show(() => Buffer.concat([Buffer.from("a")], -1).toString("hex")));
        console.log(show(() => Buffer.concat([Buffer.from("a")], 1.5).toString("hex")));
        console.log(show(() => Buffer.concat([Buffer.from("a")], "2" as any).toString("hex")));"#),
        [
            "61626364",
            "610000",
            "|616263",
            "010261",
            "TypeError: The \"list[0]\" argument must be an instance of Buffer or Uint8Array. Received type string ('a')",
            "TypeError: The \"list\" argument must be an instance of Array. Received type string ('abc')",
            "RangeError: The value of \"length\" is out of range. It must be >= 0 && <= 9007199254740991. Received -1",
            "RangeError: The value of \"length\" is out of range. It must be an integer. Received 1.5",
            "TypeError: The \"length\" argument must be of type number. Received type string ('2')",
        ]
    );
}

#[test]
fn string_legacy_html_methods_wrap_and_escape_like_create_html() {
    // Annex B: only `"` in the attribute value is escaped (as `&quot;`); `&`,
    // `<` and the receiver text pass through. The receiver is converted before
    // the attribute value.
    assert_eq!(
        out(r#"const s: any = "x";
        const p: any = String.prototype;
        console.log("x".big(), "x".bold(), "x".fixed(), "x".italics(), "x".blink());
        console.log("x".anchor("a&b"), 'a"b'.anchor('n"m'), "x".link(undefined as any), s.fontsize(12), s.fontcolor("red"));
        console.log(s.small(), s.strike(), s.sub(), s.sup(), s.link());
        console.log(String.prototype.big.call(12), p.link.apply("r", ["u"]), p.anchor.bind("r", "n")());
        console.log(typeof p.big, p.big.name, p.big.length, p.anchor.length, Object.prototype.hasOwnProperty.call(String.prototype, "sup"));
        const d: any = Object.getOwnPropertyDescriptor(String.prototype, "anchor");
        console.log(d.writable, d.enumerable, d.configurable, d.value === p.anchor, Object.keys(String.prototype).length);
        const order: string[] = [];
        console.log(p.link.call({ toString: () => { order.push("this"); return "b"; } }, { toString: () => { order.push("value"); return "v"; } }), order.join(","));
        try { s.link(Symbol("q")); } catch (e: any) { console.log(e.name + ": " + e.message); }
        try { String.prototype.sub.call(Symbol("q") as any); } catch (e: any) { console.log(e.name + ": " + e.message); }"#),
        [
            "<big>x</big> <b>x</b> <tt>x</tt> <i>x</i> <blink>x</blink>",
            "<a name=\"a&b\">x</a> <a name=\"n&quot;m\">a\"b</a> <a href=\"undefined\">x</a> <font size=\"12\">x</font> <font color=\"red\">x</font>",
            "<small>x</small> <strike>x</strike> <sub>x</sub> <sup>x</sup> <a href=\"undefined\">x</a>",
            "<big>12</big> <a href=\"u\">r</a> <a name=\"n\">r</a>",
            "function big 0 1 true",
            "true false true true 0",
            "<a href=\"v\">b</a> this,value",
            "TypeError: Cannot convert a Symbol value to a string",
            "TypeError: Cannot convert a Symbol value to a string",
        ]
    );
}

#[test]
fn symbol_is_callable_as_a_value_but_never_constructible() {
    assert_eq!(
        out(r#"const S: any = Symbol;
        const attempt = (f: () => any): string => {
            try { return String(f()); } catch (e: any) { return e.name + ": " + e.message; }
        };
        console.log(S("d").toString(), S().toString(), S(42).description, S(null).description, String(S(undefined).description));
        console.log((Symbol as any).call(undefined, "d").toString(), (Symbol as any).apply(null, ["viaApply"]).description, (Symbol as any).call().toString());
        console.log(S.call({}, "x").toString(), S.apply(undefined, ["y"]).toString(), S.bind(null, "b")().toString());
        console.log(typeof S("a"), S("a") === S("a"), typeof S, S.name, S.length);
        console.log(attempt(() => S(Symbol("x"))));
        console.log(attempt(() => new S()), "|", attempt(() => new (Symbol as any)("x")), "|", attempt(() => Reflect.construct(S, [])));
        class Sub extends (Symbol as any) {}
        class Big extends (BigInt as any) {}
        console.log(attempt(() => new (Sub as any)()), "|", attempt(() => new Big(5)));"#),
        [
            "Symbol(d) Symbol() 42 null undefined",
            "Symbol(d) viaApply Symbol()",
            "Symbol(x) Symbol(y) Symbol(b)",
            "symbol false function Symbol 0",
            "TypeError: Cannot convert a Symbol value to a string",
            "TypeError: Symbol is not a constructor | TypeError: Symbol is not a constructor | TypeError: Symbol is not a constructor",
            "TypeError: Symbol is not a constructor | TypeError: BigInt is not a constructor",
        ]
    );
}

#[test]
fn a_class_method_borrowed_onto_a_primitive_keeps_the_primitive_receiver() {
    // Class code is strict: `call` binds the value itself, never a wrapper.
    assert_eq!(
        out(r#"class Widget {
            tag = "widget";
            kindOfThis(): string { return this === undefined ? "undefined" : typeof this; }
            isSame(other: any): boolean { return (this as any) === other; }
            read(): string { return (this as any).tag; }
        }
        const w = new Widget();
        const other: any = { tag: "other" };
        console.log(w.kindOfThis.call(undefined), w.kindOfThis.call(null), w.kindOfThis.call(42), w.kindOfThis.call("txt"));
        console.log(w.kindOfThis.call(true), w.kindOfThis.call(10n), w.kindOfThis.call({}), w.kindOfThis.call(w), w.kindOfThis());
        console.log(w.isSame.call(7, 7), w.isSame.call(other, other), w.isSame.call("a", "b"), w.read.call(other), w.read.call(w));"#),
        [
            "undefined object number string",
            "boolean bigint object object object",
            "true true false other widget",
        ]
    );
}

#[test]
fn array_buffer_slice_coerces_its_bounds() {
    // An absent or `undefined` end means "to the end"; every other operand
    // goes through ToIntegerOrInfinity, in typed code as through `any`.
    assert_eq!(
        out(r#"const buf = new ArrayBuffer(8);
        new Uint8Array(buf).set([10, 11, 12, 13, 14, 15, 16, 17]);
        const d: any = buf;
        const u: any = undefined;
        const show = (f: () => any): string => {
            try { return Array.from(new Uint8Array(f())).join(","); } catch (e: any) { return e.name + ": " + e.message; }
        };
        console.log(show(() => buf.slice(2, undefined)), "|", show(() => buf.slice(undefined, 3)), "|", show(() => buf.slice(6, u)));
        console.log(show(() => buf.slice(2, 5)), "|", show(() => buf.slice(1, -5)), "|", show(() => buf.slice(1.9, 3.9)), "|", show(() => buf.slice(2, null as any)));
        console.log(show(() => buf.slice("2" as any, "4" as any)), "|", show(() => buf.slice(NaN as any, 2)), "|", show(() => buf.slice({ valueOf() { return 6; } } as any, 99)));
        console.log(show(() => d.slice(6, undefined)), "|", show(() => d.slice(-2)), "|", show(() => d.slice()).length);
        console.log(show(() => buf.slice(2, Symbol() as any)), "|", show(() => d.slice(1n)));"#),
        [
            "12,13,14,15,16,17 | 10,11,12 | 16,17",
            "12,13,14 | 11,12 | 11,12 | ",
            "12,13 | 10,11 | 16,17",
            "16,17 | 16,17 | 23",
            "TypeError: Cannot convert a Symbol value to a number | TypeError: Cannot convert a BigInt value to a number",
        ]
    );
}
