mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;

#[test]
fn aot_object_contains_the_eligible_entry_point() {
    let module = compile("function main(): int { return 7; }");
    let object = zun_codegen::compile_object(&module).expect("emit object");
    assert!(!object.is_empty());
    assert!(object.windows(2).any(|bytes| bytes == b"m1"));
}

#[test]
fn aot_object_can_select_the_host_target_explicitly() {
    let module = compile("function main(): int { return 7; }");
    let object = zun_codegen::compile_object_for_target(&module, target_lexicon::Triple::host())
        .expect("emit host-targeted object");
    assert!(!object.is_empty());
}

#[test]
fn aot_object_can_select_aarch64_target_isa() {
    let module = compile("function main(): int { return 7; }");
    let target = "aarch64-unknown-linux-gnu"
        .parse::<target_lexicon::Triple>()
        .expect("parse AArch64 target");
    let object = zun_codegen::compile_object_for_target(&module, target)
        .expect("emit AArch64-targeted object");
    assert!(!object.is_empty());
}

#[test]
fn aot_metadata_object_can_select_aarch64_target_format() {
    use object::Object as _;
    let target = "aarch64-unknown-linux-gnu"
        .parse::<target_lexicon::Triple>()
        .expect("parse AArch64 target");
    let object = zun_codegen::compile_metadata_object_for_target(b"ZUN-IL", target)
        .expect("emit AArch64 metadata object");
    let file = object::File::parse(object.as_slice()).expect("parse metadata object");
    assert_eq!(file.architecture(), object::Architecture::Aarch64);
    assert_eq!(file.format(), object::BinaryFormat::Elf);
}

#[test]
fn aot_rejects_an_ineligible_entry_before_linking() {
    let module = compile(
        "function main(): string { if (true) { return \"managed\"; } return \"fallback\"; }",
    );
    let error = zun_codegen::compile_object(&module).expect_err("string entry is not AOT-eligible");
    assert!(matches!(error, zun_codegen::JitError::Ineligible(1)));
}

#[test]
fn aot_object_contains_a_relocatable_string_literal() {
    let module = compile("function main(): string { return \"managed\"; }");
    let object = zun_codegen::compile_object(&module).expect("emit string object");
    assert!(object
        .windows(b"managed".len())
        .any(|bytes| bytes == b"managed"));
}

#[test]
fn overridden_virtual_call_is_jit_eligible_and_dispatches_dynamically() {
    let src = "class Base { speak(): int { return 1; } }\n\
               class Derived extends Base { speak(): int { return 2; } }\n\
               function read(b: Base): int { return b.speak(); }\n\
               function main(): int { return read(new Derived()); }";
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 2));
}

#[test]
fn object_array_store_is_jit_eligible_and_preserves_pointer_abi() {
    let src = "function write(values: Array, value: Array): Array { values[0] = value; return values; }\n\
               function main(): boolean { return true; }";
    let module = compile(src);
    let write_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles object array store");
    assert!(
        jit.is_compiled(write_rid),
        "object array writer should be eligible"
    );

    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    let value = heap.alloc_array(0);
    let result = jit
        .call_packed_with_heap(
            write_rid,
            &[array.addr() as u64, value.addr() as u64],
            &mut heap,
        )
        .expect("jit object array store");
    assert_eq!(result as usize, array.addr() as usize);
    assert_eq!(
        heap.array_get(array, 0).and_then(|any| any.as_object()),
        Some(value.addr() as usize)
    );
}

#[test]
fn boolean_object_field_is_jit_equivalent() {
    let src = "class Flag { value: boolean; }\n\
               function read(flag: Flag): boolean { return flag.value; }\n\
               function main(): boolean { return read(new Flag(true)); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles boolean field reader");
    assert!(
        jit.is_compiled(read_rid),
        "boolean field reader should be eligible"
    );

    let mut heap = zun_gc::Heap::new();
    let flag = heap.alloc(1, 1);
    flag.set_field(0, 1);
    let jit_value = jit
        .call_packed_with_heap(read_rid, &[flag.addr() as u64], &mut heap)
        .expect("jit boolean field read");
    assert_ne!(jit_value, 0);
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn interned_string_field_read_is_jit_equivalent() {
    let src = "class Named { tag: string; }\n\
               function read(n: Named): string { return n.tag; }\n\
               function main(): int { return 1; }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(
        jit.is_compiled(read_rid),
        "interned string field read should be eligible"
    );
    let mut heap = zun_gc::Heap::new();
    let named = heap.alloc(1, 1);
    let handle = heap.intern_string(std::rc::Rc::from("hello"));
    named.set_field(0, handle as u64);
    assert_eq!(
        jit.call_packed_with_heap(read_rid, &[named.addr() as u64], &mut heap)
            .expect("jit string field read"),
        handle as u64
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn string_field_length_uses_the_typed_jit_builtin() {
    let src = "import { length } from \"zun:core\";\n\
               class Named { tag: string; }\n\
               function measure(n: Named): u64 { return length(n.tag); }\n\
               function main(): u64 { return 1; }";
    let module = compile(src);
    let measure_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .map(|index| index as u32 + 1)
        .expect("measure method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(jit.is_compiled(measure_rid));
    let mut heap = zun_gc::Heap::new();
    let named = heap.alloc(1, 1);
    let handle = heap.intern_string(std::rc::Rc::from("field value"));
    named.set_field(0, handle as u64);
    assert_eq!(
        jit.call_packed_with_heap(measure_rid, &[named.addr() as u64], &mut heap)
            .expect("jit field string length"),
        11
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn interned_string_field_constructor_is_jit_equivalent() {
    let src = "class Named { tag: string; }\n\
               function make(): Named { return new Named(\"hello\"); }\n\
               function read(n: Named): string { return n.tag; }\n\
               function main(): int { return 1; }";
    let module = compile(src);
    let make_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "make")
        .map(|index| index as u32 + 1)
        .expect("make method");
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(
        jit.is_compiled(make_rid),
        "string-field constructor should be eligible"
    );
    assert!(
        jit.is_compiled(read_rid),
        "string-field read should be eligible"
    );
    let mut heap = zun_gc::Heap::new();
    let object = jit
        .call_packed_with_heap(make_rid, &[], &mut heap)
        .expect("jit string-field constructor");
    let object = unsafe { zun_gc::Obj::from_addr(object as *mut u64) };
    let handle = object.field(0);
    assert_eq!(heap.try_string(handle as u32).map(|s| &**s), Some("hello"));
    let result = jit
        .call_packed_with_heap(read_rid, &[object.addr() as u64], &mut heap)
        .expect("jit string-field read after construction");
    assert_eq!(result, handle);
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn interned_string_field_store_is_jit_equivalent() {
    let src = "class Named { tag: string; }\n\
               function set(n: Named, value: string): string { n.tag = value; return n.tag; }\n\
               function main(): int { return 1; }";
    let module = compile(src);
    let set_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "set")
        .map(|index| index as u32 + 1)
        .expect("set method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(
        jit.is_compiled(set_rid),
        "interned string field store should be eligible"
    );
    let mut heap = zun_gc::Heap::new();
    let named = heap.alloc(1, 1);
    let handle = heap.intern_string(std::rc::Rc::from("updated"));
    let result = jit
        .call_packed_with_heap(set_rid, &[named.addr() as u64, handle as u64], &mut heap)
        .expect("jit string field store");
    assert_eq!(result, handle as u64);
    assert_eq!(named.field(0), handle as u64);
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn ineligible_methods_are_not_compiled() {
    let module = compile(r#"function main() { print("hi"); }"#);
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
}

#[test]
fn exception_regions_fall_back_to_the_interpreter() {
    let module =
        compile("function main(): int { try { return 1; } finally { let cleanup: int = 2; } }");
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn nested_exception_control_flow_keeps_interpreter_result_under_jit_fallback() {
    let module = compile(
        "class Error {} function main(): int { try { try { throw new Error(); } finally { print(1); } } catch (outer: Error) { return 42; } }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(42, _)));
}

#[test]
fn template_literals_match_interpreter_and_jit_fallback() {
    // A string-only template literal lowers to `ConstStr`/`Concat`, which the
    // typed JIT now compiles (the ConstStr gate admits literal-fed `Concat`). The
    // JIT result must match the interpreter's.
    let module =
        compile("function main(): string { let name: string = \"Zun\"; return `hello ${name}!`; }");
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "hello Zun!"));
    let mut heap = zun_gc::Heap::new();
    let handle = jit
        .call_packed_with_heap(entry, &[], &mut heap)
        .expect("template literal call");
    assert_eq!(heap.string(handle as u32).as_ref(), "hello Zun!");
}

#[test]
fn template_literals_convert_numbers_through_the_native_floor() {
    let module =
        compile("function main(): string { let value: number = 12.5; return `value=${value}`; }");
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "value=12.5"));
}

#[test]
fn template_literals_parse_expression_interpolations() {
    let module = compile(
        "function main(): string { let value: number = 1.0; return `sum=${value + 2.0}`; }",
    );
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "sum=3"));
}

#[test]
fn bigint_operations_preserve_the_tier0_fallback_boundary() {
    let module = compile(
        r#"function main(): int {
            let left: BigInt = 10000000000000000000n;
            let right: BigInt = 2n;
            let product: BigInt = left * right;
            let negated: BigInt = -left;
            let quotient: BigInt = product / right;
            let remainder: BigInt = product % right;
            if (product > left) {
                if (negated < left) {
                    if (quotient == left) {
                        if (remainder.toString() == "0") { return 1; }
                    }
                }
            }
            return 0;
        }"#,
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with BigInt fallback");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

// ---- Regex literals (`/pattern/flags`) lower to `new RegExp(pattern, flags)` ----

#[test]
fn regex_literal_test_method() {
    assert_regex_program("if (/abc/.test(\"xabcy\")) { return 1; } return 0;", 1);
    assert_regex_program("if (/abc/.test(\"xyz\")) { return 1; } return 0;", 0);
}

#[test]
fn regex_literal_flags_are_forwarded() {
    assert_regex_program("if (/ABC/i.test(\"abc\")) { return 1; } return 0;", 1);
    assert_regex_program("if (/ABC/.test(\"abc\")) { return 1; } return 0;", 0);
}

#[test]
fn regex_literal_backreference_uses_engine_b() {
    assert_regex_program(r#"if (/(\w)\1/.test("hello")) { return 1; } return 0;"#, 1);
    assert_regex_program(r#"if (/(\w)\1/.test("abc")) { return 1; } return 0;"#, 0);
}

#[test]
fn regex_literal_slash_in_class_and_escape() {
    assert_regex_program("if (/[/]x/.test(\"a/x\")) { return 1; } return 0;", 1);
    assert_regex_program(r#"if (/a\/b/.test("a/b")) { return 1; } return 0;"#, 1);
}

#[test]
fn regex_literal_after_assign_paren_comma_and_return() {
    assert_regex_program(
        "let r = /b+/; if (r.test(\"abbc\")) { return 1; } return 0;",
        1,
    );
    assert_regex_program(
        "let r = /z/; let s = \"zz\"; if (s.search(r) == 0) { return 1; } return 0;",
        1,
    );
}

#[test]
fn division_is_not_mistaken_for_regex() {
    assert_regex_program(
        "let a = 8; let b = 2; if (a / b == 4) { return 1; } return 0;",
        1,
    );
    assert_regex_program(
        "let x = 9; let y = 3; if ((x) / y == 3) { return 1; } return 0;",
        1,
    );
    assert_regex_program(
        "let a = 100; let b = 5; let c = 2; if (a / b / c == 10) { return 1; } return 0;",
        1,
    );
    assert_regex_program("if (12 / 4 == 3) { return 1; } return 0;", 1);
}

// ---------------------------------------------------------------------------
// Operator tokens: typeof / in / instanceof / delete (NUCLEUS operator gap).
// ---------------------------------------------------------------------------

#[test]
fn typeof_folds_to_static_type_strings() {
    let cases = [
        ("function main(): string { return typeof 1; }", "number"),
        ("function main(): string { return typeof 3.5; }", "number"),
        ("function main(): string { return typeof \"a\"; }", "string"),
        ("function main(): string { return typeof true; }", "boolean"),
        ("function main(): string { return typeof 1n; }", "bigint"),
        ("function main(): string { return typeof {}; }", "object"),
    ];
    for (src, expected) in cases {
        let module = compile(src);
        assert!(
            matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == expected),
            "typeof mismatch for `{src}`, expected {expected}"
        );
    }
}

#[test]
fn typeof_symbol_and_function() {
    let sym = compile("function main(): string { return typeof Symbol(); }");
    assert!(matches!(interp_entry(&sym), Value::Str(value) if value.as_ref() == "symbol"));
    let func =
        compile("function helper(): void { } function main(): string { return typeof helper; }");
    assert!(matches!(interp_entry(&func), Value::Str(value) if value.as_ref() == "function"));
}

#[test]
fn in_operator_on_plain_object() {
    let present = compile(
        "function main(): boolean { let obj = dynamic_new(); dynamic_set(obj, \"k\", box_any(1)); return \"k\" in obj; }",
    );
    assert!(matches!(interp_entry(&present), Value::Bool(true)));
    let absent = compile(
        "function main(): boolean { let obj = dynamic_new(); dynamic_set(obj, \"k\", box_any(1)); return \"missing\" in obj; }",
    );
    assert!(matches!(interp_entry(&absent), Value::Bool(false)));
}

#[test]
fn in_operator_routes_through_proxy_has_trap() {
    let module = compile(
        "import { dynamicSetFunction } from \"zun:core\"; \
         function hasTrap(target: any, key: string): boolean { return true; } \
         function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); \
         dynamicSetFunction(handler, \"has\", hasTrap); let proxy = new Proxy(target, handler); \
         return \"anything\" in proxy; }",
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn delete_operator_removes_plain_property() {
    let module = compile(
        "function main(): boolean { let obj = dynamic_new(); dynamic_set(obj, \"k\", box_any(1)); \
         let removed = delete obj.k; return removed && !(\"k\" in obj); }",
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn delete_operator_routes_through_proxy_delete_trap() {
    let module = compile(
        "import { dynamicSetFunction } from \"zun:core\"; \
         function delTrap(target: any, key: string): boolean { return true; } \
         function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); \
         dynamicSetFunction(handler, \"deleteProperty\", delTrap); let proxy = new Proxy(target, handler); \
         return delete proxy.k; }",
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn instanceof_checks_class_hierarchy() {
    let yes = compile(
        "class Foo {} function main(): boolean { let f = new Foo(); return f instanceof Foo; }",
    );
    assert!(matches!(interp_entry(&yes), Value::Bool(true)));
    let subclass = compile(
        "class Base {} class Derived extends Base {} \
         function main(): boolean { let d = new Derived(); return d instanceof Base; }",
    );
    assert!(matches!(interp_entry(&subclass), Value::Bool(true)));
    let no = compile(
        "class Foo {} class Bar {} function main(): boolean { let b = new Bar(); return b instanceof Foo; }",
    );
    assert!(matches!(interp_entry(&no), Value::Bool(false)));
}

#[test]
fn float_remainder_uses_the_fmod_libcall() {
    // `%` on `number` has no Cranelift instruction; it lowers to the
    // `zun_jit_math_fmod` libcall. Locals keep the operation out of any
    // compile-time constant fold so the opcode is actually exercised.
    assert_equivalent(
        "function main(): number { let a: number = 7.5; let b: number = 2.0; return a % b; }",
    );
    // Truncated-remainder sign: JS and Rust `%` both take the dividend's sign.
    assert_equivalent(
        "function main(): number { let a: number = -7.5; let b: number = 2.0; return a % b; }",
    );
    // A remainder appearing inside a larger JIT-eligible expression.
    assert_equivalent(
        "function main(): number { let a: number = 10.0; let b: number = 3.0; return (a % b) + 1.0; }",
    );
}

#[test]
fn string_plus_uses_the_managed_concat_abi() {
    // `a + b` on `string` emits the `Concat` opcode, which lowers to the same
    // `zun_jit_string_concat` helper as the `.concat` method. String parameters
    // keep the body free of `ConstStr`, so eligibility is exercised directly.
    let module = compile(
        "function measure(value: string, other: string): string { return value + other; } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(
        jit.is_compiled(rid),
        "string `+` method should be JIT-eligible"
    );

    let mut heap = zun_gc::Heap::new();
    let left = heap.intern_string(std::rc::Rc::from("zun "));
    let right = heap.intern_string(std::rc::Rc::from("runtime"));
    let handle = jit
        .call_packed_with_heap(rid, &[left as u64, right as u64], &mut heap)
        .expect("concat call");
    assert_eq!(heap.string(handle as u32).as_ref(), "zun runtime");
}

#[test]
fn static_field_round_trips_through_the_jit_store() {
    // `LoadStatic`/`StoreStatic` lower to the shared heap statics helpers. A
    // method that writes, reads, updates, and reads back a static must be
    // JIT-eligible and compute the same value as the interpreter would.
    let module = compile(
        "class Counter { static total: int; } function measure(): int { Counter.total = 5; Counter.total = Counter.total + 3; return Counter.total; } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(
        jit.is_compiled(rid),
        "static-field method should be JIT-eligible"
    );

    let mut heap = zun_gc::Heap::new();
    let result = jit
        .call_packed_with_heap(rid, &[], &mut heap)
        .expect("static field call");
    assert_eq!(result, 8);
}

#[test]
fn static_float_field_round_trips_through_the_jit_store() {
    // A floating-point static exercises the `Any::float` tag and the f64
    // encode/decode path of the statics helpers.
    let module = compile(
        "class Gauge { static value: number; } function measure(): number { Gauge.value = 2.5; Gauge.value = Gauge.value + 1.0; return Gauge.value; } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(
        jit.is_compiled(rid),
        "float static method should be JIT-eligible"
    );

    let mut heap = zun_gc::Heap::new();
    let bits = jit
        .call_packed_with_heap(rid, &[], &mut heap)
        .expect("float static call");
    assert_eq!(f64::from_bits(bits), 3.5);
}

#[test]
fn data_view_extended_scalar_widths_use_the_typed_view_abi() {
    // The signed/8-bit/float32 DataView accessors lower to their own width-
    // specific helpers, just like the u16/u32/f64 fast paths. Writes are verified
    // against the heap; reads are verified through the JIT return value.
    let module = compile(
        "import { dataViewSetInt8, dataViewSetInt16, dataViewSetInt32, dataViewSetFloat32, dataViewGetInt32, dataViewGetFloat32 } from \"zun:core\"; \
         function setI8(view: DataView, offset: u64, value: i8): boolean { return dataViewSetInt8(view, offset, value); } \
         function setI16(view: DataView, offset: u64, value: i16, little: boolean): boolean { return dataViewSetInt16(view, offset, value, little); } \
         function setI32(view: DataView, offset: u64, value: i32, little: boolean): boolean { return dataViewSetInt32(view, offset, value, little); } \
         function setF32(view: DataView, offset: u64, value: f32, little: boolean): boolean { return dataViewSetFloat32(view, offset, value, little); } \
         function getI32(view: DataView, offset: u64, little: boolean): i32 { return dataViewGetInt32(view, offset, little); } \
         function getF32(view: DataView, offset: u64, little: boolean): f32 { return dataViewGetFloat32(view, offset, little); } \
         function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("method") as u32
            + 1
    };
    let (set_i8, set_i16, set_i32, set_f32, get_i32, get_f32) = (
        find("setI8"),
        find("setI16"),
        find("setI32"),
        find("setF32"),
        find("getI32"),
        find("getF32"),
    );
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    for rid in [set_i8, set_i16, set_i32, set_f32, get_i32, get_f32] {
        assert!(
            jit.is_compiled(rid),
            "extended DataView method should be JIT-eligible"
        );
    }
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(16);
    let view = heap.alloc_data_view(buffer, 0, 16).expect("data view");

    // -5 as an unsigned i8 byte is 251; it must round-trip as signed -5.
    assert!(matches!(
        jit.call_packed_with_heap(set_i8, &[view.addr() as u64, 0, 251], &mut heap),
        Ok(1)
    ));
    assert_eq!(heap.data_view_get_i8(view, 0), Some(-5));
    // -1000 as a u16 is 64536.
    assert!(matches!(
        jit.call_packed_with_heap(set_i16, &[view.addr() as u64, 2, 64536, 1], &mut heap),
        Ok(1)
    ));
    assert_eq!(heap.data_view_get_i16(view, 2, true), Some(-1000));
    assert!(matches!(
        jit.call_packed_with_heap(set_i32, &[view.addr() as u64, 4, 0x1234_5678, 1], &mut heap),
        Ok(1)
    ));
    assert_eq!(heap.data_view_get_i32(view, 4, true), Some(0x1234_5678));
    assert!(matches!(
        jit.call_packed_with_heap(
            set_f32,
            &[view.addr() as u64, 8, 2.5f32.to_bits() as u64, 1],
            &mut heap
        ),
        Ok(1)
    ));
    assert_eq!(heap.data_view_get_f32(view, 8, true), Some(2.5));

    // Reads back through the JIT.
    assert!(matches!(
        jit.call_packed_with_heap(get_i32, &[view.addr() as u64, 4, 1], &mut heap),
        Ok(0x1234_5678)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(get_f32, &[view.addr() as u64, 8, 1], &mut heap),
        Ok(value) if f32::from_bits(value as u32) == 2.5
    ));
}

#[test]
fn string_literal_plus_variable_is_jit_eligible() {
    // `"literal" + value` mixes a `ConstStr` with `Concat`. Both already lower;
    // this checks the ConstStr eligibility gate now admits the pattern (runtime
    // JIT), since literal-prefixed concatenation is extremely common.
    let module = compile(
        "function greet(name: string): string { return \"Hi, \" + name; } function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "greet")
        .expect("greet method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(
        jit.is_compiled(rid),
        "literal + variable concat should be JIT-eligible"
    );

    let mut heap = zun_gc::Heap::new();
    let name = heap.intern_string(std::rc::Rc::from("world"));
    let handle = jit
        .call_packed_with_heap(rid, &[name as u64], &mut heap)
        .expect("greet call");
    assert_eq!(heap.string(handle as u32).as_ref(), "Hi, world");
}

#[test]
fn same_frame_try_catch_falls_back_to_the_interpreter() {
    // `catch (e: Error)` no longer filters by type: a TypeScript catch annotation
    // is documentation only, so the handler is a catch-all (JS semantics). The
    // typed same-frame JIT slice requires a typed catch, so the method stays on
    // the interpreter fallback and both paths of the try/catch run there.
    let module = compile(
        "class Error {} function classify(n: int): int { let result: int = 0; try { if (n < 0) { throw new Error(); } result = 1; } catch (e: Error) { result = 2; } return result; } function main(): int { return classify(5) * 10 + classify(-1); }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "classify")
        .expect("classify method") as u32
        + 1;
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(rid), "a catch-all handler requires Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(12, _)));
}
#[test]
fn same_frame_try_catch_handles_subclass_and_multiple_throws() {
    // Two throw points (one of a subclass of the annotated type), caught by one
    // catch-all handler. The typed same-frame JIT slice no longer applies (see
    // above), so the method runs on the interpreter fallback.
    let module = compile(
        "class Error {} class TypeError extends Error {} \
         function classify(n: int): int { let result: int = 0; \
           try { if (n < 0) { throw new TypeError(); } if (n == 0) { throw new Error(); } result = 1; } \
           catch (e: Error) { result = 2; } \
           return result; } \
         function main(): int { return classify(5) * 100 + classify(0) * 10 + classify(-1); }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "classify")
        .expect("classify method") as u32
        + 1;
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(rid), "a catch-all handler requires Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(122, _)));
}
