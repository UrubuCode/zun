mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;

#[test]
fn realm_global_binding_remains_on_the_tier0_fallback() {
    let module = compile(
        "function main(): boolean { return globalThis == global; }",
    );
    let entry = module.entry_point.expect("entry point");
    let jit = zun_codegen::Jit::compile(&module).expect("JIT eligibility analysis");
    assert!(!jit.is_compiled(entry.rid()), "realm object access requires Tier 0");
}

#[test]
fn typed_string_receiver_call_is_jit_equivalent() {
    let module = zun_compiler::compile(
        r#"import { length } from "zun:core";
        class String {
            extern length(): u64;
            isShort(): boolean { return this.length() < 4; }
        }
        function measure(value: string): boolean { return value.isShort(); }
        function main(): boolean { return true; }"#,
    )
    .expect("compile typed string receiver call");
    let measure = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .map(|index| index as u32 + 1)
        .expect("measure method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(jit.is_compiled(measure));
    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from("zun"));
    assert_eq!(jit.call_packed_with_heap(measure, &[value as u64], &mut heap).unwrap(), 1);
}

#[test]
fn typed_string_receiver_chain_preserves_managed_result() {
    let module = zun_compiler::compile(
        r#"import { trim, length } from "zun:core";
        class String {
            extern trim(): string;
            extern length(): u64;
            isShort(): boolean { return this.length() < 4; }
        }
        function measure(value: string): boolean { return value.trim().isShort(); }
        function main(): boolean { return true; }"#,
    )
    .expect("compile typed string receiver chain");
    let measure = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .map(|index| index as u32 + 1)
        .expect("measure method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(jit.is_compiled(measure));
    let mut heap = zun_gc::Heap::new();
    let value = heap.intern_string(std::rc::Rc::from(" zun "));
    assert_eq!(jit.call_packed_with_heap(measure, &[value as u64], &mut heap).unwrap(), 1);
}

#[test]
fn collection_allocation_ids_only_apply_to_call_builtins() {
    let (map_new, _) = zun_natives::resolve("collections", "mapNew").expect("mapNew intrinsic");
    let source = format!(
        "function main(): i32 {{ return {map_new} as i32; }}"
    );
    let module = compile(&source);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("constant does not look like allocation");
    assert!(jit.is_compiled(entry));
}


#[test]
fn static_class_method_uses_direct_call_without_receiver() {
    assert_equivalent(
        "class Math { static add(left: number, right: number): number { return left + right; } } function main(): number { return Math.add(19.0, 23.0); }",
    );
}

#[test]
fn array_for_of_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): number { let values = new Array(2); values[0] = 20.0; values[1] = 22.0; for (let value of values) { print(value); } return 42.0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "array iteration requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(42.0, _)));
}

#[test]
fn generic_set_has_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "import { setHas } from \"zun:core\";\n\
         function contains(set: Set, value: any): boolean { return setHas(set, value); }\n\
         function main(): boolean { let set = new Set(); set.add(7); return contains(set, 7); }",
    );
    let contains = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "contains")
        .expect("contains method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with fallback");
    assert!(!jit.is_compiled(contains), "generic Set.has must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn generic_map_has_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "import { mapHas } from \"zun:core\";\n\
         function contains(map: Map, key: any): boolean { return mapHas(map, key); }\n\
         function main(): boolean { let map = new Map(); map.set(7, 42); return contains(map, 7); }",
    );
    let contains = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "contains")
        .expect("contains method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with fallback");
    assert!(!jit.is_compiled(contains), "generic Map.has must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn checked_string_array_unbox_preserves_tier0_fallback_boundary() {
    let module = compile(
        "function read(values: Array): string { return values[0] as string; } function main(): int { let values = new Array(1); values[0] = \"hello\"; return 1; }",
    );
    let read = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .expect("read method") as u32
        + 1;
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(read), "checked string unbox must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn interned_string_length_uses_the_typed_jit_abi() {
    let module = compile(
        "import { length } from \"zun:core\"; function measure(value: string): u64 { return length(value); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid), "measure RID {rid}, methods: {:?}", module.tables.method_defs);

    let mut heap = zun_gc::Heap::new();
    let handle = heap.intern_string(std::rc::Rc::from("hé"));
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[handle as u64], &mut heap),
        Ok(2)
    ));
}

#[test]
fn aliased_intrinsic_import_uses_the_local_binding() {
    let module = compile(
        "import { length as stringLength } from \"zun:core\"; function measure(value: string): u64 { return stringLength(value); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "measure")
        .expect("measure method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let handle = heap.intern_string(std::rc::Rc::from("alias"));
    assert!(matches!(jit.call_packed_with_heap(rid, &[handle as u64], &mut heap), Ok(5)));
}

