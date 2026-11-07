mod common;
use common::*;

use zun_interp::value::Value;

#[test]
fn generator_for_of_preserves_tier0_fallback_boundary() {
    let module = compile(
        "function* values(): int { yield 7; return 9; } function main(): any { for (let value of values()) { return value; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "generator suspension requires Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Float(7.0, _)));
}

#[test]
fn generator_arguments_preserve_tier0_fallback_boundary() {
    let module = compile(
        "function* values(seed: number): int { yield seed; yield seed + 1; return 9; } function main(): any { let total: number = 0.0; for (let value of values(2)) { total = total + (value as number); } return total; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "generator suspension requires Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Float(5.0, _)));
}

#[test]
fn jit_can_allocate_an_array_for_a_scalar_length_result_with_heap_scope() {
    let module =
        compile("function main(): u64 { let values = new Array(3); return values.length; }");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let mut heap = zun_gc::Heap::new();
    assert!(matches!(
        jit.call_entry_with_heap(entry, &mut heap),
        Ok(zun_codegen::RetVal::Int(3))
    ));
    assert!(matches!(interp_entry(&module), Value::Int(3, _)));
}

#[test]
fn jit_can_allocate_an_object_and_read_a_numeric_field_with_heap_scope() {
    let module = compile(
        "class Box { value: int; }
         function read(box: Box): int { return box.value; }
         function main(): int { return read(new Box(7)); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let mut heap = zun_gc::Heap::new();
    assert!(matches!(
        jit.call_entry_with_heap(entry, &mut heap),
        Ok(zun_codegen::RetVal::Int(7))
    ));
    assert!(matches!(interp_entry(&module), Value::Int(7, _)));
}

#[test]
fn jit_preserves_object_references_through_managed_fields() {
    let module = compile(
        "class Box { value: int; }
         class Holder { box: Box; }
         function main(): int { let value = new Box(7); let holder = new Holder(value); return holder.box.value; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let mut heap = zun_gc::Heap::new();
    assert!(matches!(
        jit.call_entry_with_heap(entry, &mut heap),
        Ok(zun_codegen::RetVal::Int(7))
    ));
    assert!(matches!(interp_entry(&module), Value::Int(7, _)));
}

#[test]
fn jit_stores_and_reloads_managed_object_references() {
    let module = compile(
        "class Box { value: int; }
         class Holder { box: Box; }
         function main(): int { let first = new Box(1); let second = new Box(9); let holder = new Holder(first); holder.box = second; return holder.box.value; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let mut heap = zun_gc::Heap::new();
    assert!(matches!(
        jit.call_entry_with_heap(entry, &mut heap),
        Ok(zun_codegen::RetVal::Int(9))
    ));
    assert!(matches!(interp_entry(&module), Value::Int(9, _)));
}

#[test]
fn managed_collections_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { mapNew, mapSet, mapHas } from \"zun:core\"; function main(): int { let map = mapNew(); mapSet(map, 7.0, 42.0); if (mapHas(map, 7.0)) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "managed collections require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn managed_map_and_set_size_methods_use_typed_object_receivers() {
    let module = compile(
        "function mapSize(value: Map): u64 { return value.size(); } function setSize(value: Set): u64 { return value.size(); } function main(): u64 { return 0; }",
    );
    let map_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapSize")
        .expect("mapSize method") as u32
        + 1;
    let set_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "setSize")
        .expect("setSize method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(map_rid));
    assert!(jit.is_compiled(set_rid));

    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let set = heap.alloc_set();
    heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::int(2));
    heap.set_add(set, zun_core::Any::int(3));
    assert!(matches!(
        jit.call_packed_with_heap(map_rid, &[map.addr() as u64], &mut heap),
        Ok(1)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(set_rid, &[set.addr() as u64], &mut heap),
        Ok(1)
    ));
}

#[test]
fn numeric_map_and_set_has_use_typed_jit_abis() {
    let module = compile(
        "import { mapHasF64, setHasF64, setAddF64, mapDeleteF64, setDeleteF64, mapSetF64, mapGetF64Or, mapHasI64, setHasI64, setAddI64, mapDeleteI64, setDeleteI64, mapSetI64, mapGetI64Or } from \"zun:core\"; function mapContains(value: Map, key: number): boolean { return mapHasF64(value, key); } function setContains(value: Set, key: number): boolean { return setHasF64(value, key); } function setInsert(value: Set, key: number): boolean { return setAddF64(value, key); } function mapRemove(value: Map, key: number): boolean { return mapDeleteF64(value, key); } function setRemove(value: Set, key: number): boolean { return setDeleteF64(value, key); } function mapInsert(value: Map, key: number, item: number): boolean { return mapSetF64(value, key, item); } function mapRead(value: Map, key: number, fallback: number): number { return mapGetF64Or(value, key, fallback); } function mapContainsI64(value: Map, key: i64): boolean { return mapHasI64(value, key); } function setContainsI64(value: Set, key: i64): boolean { return setHasI64(value, key); } function setInsertI64(value: Set, key: i64): boolean { return setAddI64(value, key); } function mapRemoveI64(value: Map, key: i64): boolean { return mapDeleteI64(value, key); } function setRemoveI64(value: Set, key: i64): boolean { return setDeleteI64(value, key); } function mapInsertI64(value: Map, key: i64, item: i64): boolean { return mapSetI64(value, key, item); } function mapReadI64(value: Map, key: i64, fallback: i64): i64 { return mapGetI64Or(value, key, fallback); } function main(): u64 { return 0; }",
    );
    let map_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapContains")
        .expect("mapContains method") as u32
        + 1;
    let set_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "setContains")
        .expect("setContains method") as u32
        + 1;
    let insert_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "setInsert")
        .expect("setInsert method") as u32
        + 1;
    let map_remove_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapRemove")
        .expect("mapRemove method") as u32
        + 1;
    let set_remove_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "setRemove")
        .expect("setRemove method") as u32
        + 1;
    let map_insert_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapInsert")
        .expect("mapInsert method") as u32
        + 1;
    let map_read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapRead")
        .expect("mapRead method") as u32
        + 1;
    let method_rid = |name: &str| -> u32 {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("typed collection method") as u32
            + 1
    };
    let map_i64_rid = method_rid("mapContainsI64");
    let set_i64_rid = method_rid("setContainsI64");
    let add_i64_rid = method_rid("setInsertI64");
    let map_remove_i64_rid = method_rid("mapRemoveI64");
    let set_remove_i64_rid = method_rid("setRemoveI64");
    let map_insert_i64_rid = method_rid("mapInsertI64");
    let map_read_i64_rid = method_rid("mapReadI64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(map_rid));
    assert!(jit.is_compiled(set_rid));
    assert!(jit.is_compiled(insert_rid));
    assert!(jit.is_compiled(map_remove_rid));
    assert!(jit.is_compiled(set_remove_rid));
    assert!(jit.is_compiled(map_insert_rid));
    assert!(jit.is_compiled(map_read_rid));
    assert!(jit.is_compiled(map_i64_rid));
    assert!(jit.is_compiled(set_i64_rid));
    assert!(jit.is_compiled(add_i64_rid));
    assert!(jit.is_compiled(map_remove_i64_rid));
    assert!(jit.is_compiled(set_remove_i64_rid));
    assert!(jit.is_compiled(map_insert_i64_rid));
    assert!(jit.is_compiled(map_read_i64_rid));

    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let set = heap.alloc_set();
    heap.map_insert(map, zun_core::Any::float(7.5), zun_core::Any::float(42.0));
    heap.set_add(set, zun_core::Any::float(7.5));
    let key = 7.5f64.to_bits();
    assert_eq!(
        jit.call_packed_with_heap(map_rid, &[map.addr() as u64, key], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(set_rid, &[set.addr() as u64, key], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(
            insert_rid,
            &[set.addr() as u64, 8.5f64.to_bits()],
            &mut heap
        )
        .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(set_rid, &[set.addr() as u64, 8.5f64.to_bits()], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(
            map_insert_rid,
            &[map.addr() as u64, 8.5f64.to_bits(), 9.5f64.to_bits()],
            &mut heap
        )
        .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(
            map_read_rid,
            &[map.addr() as u64, 8.5f64.to_bits(), 0.0f64.to_bits()],
            &mut heap
        )
        .unwrap(),
        9.5f64.to_bits()
    );
    assert_eq!(
        jit.call_packed_with_heap(
            map_read_rid,
            &[map.addr() as u64, 99.0f64.to_bits(), 4.5f64.to_bits()],
            &mut heap
        )
        .unwrap(),
        4.5f64.to_bits()
    );
    assert_eq!(
        jit.call_packed_with_heap(map_rid, &[map.addr() as u64, 8.5f64.to_bits()], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(map_remove_rid, &[map.addr() as u64, key], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(set_remove_rid, &[set.addr() as u64, key], &mut heap)
            .unwrap(),
        1
    );
    heap.map_insert(map, zun_core::Any::int(7), zun_core::Any::int(42));
    heap.set_add(set, zun_core::Any::int(7));
    assert_eq!(
        jit.call_packed_with_heap(map_i64_rid, &[map.addr() as u64, 7], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(set_i64_rid, &[set.addr() as u64, 7], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(add_i64_rid, &[set.addr() as u64, 8], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(map_insert_i64_rid, &[map.addr() as u64, 8, 9], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(map_read_i64_rid, &[map.addr() as u64, 8, 0], &mut heap)
            .unwrap(),
        9
    );
    assert_eq!(
        jit.call_packed_with_heap(map_remove_i64_rid, &[map.addr() as u64, 8], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(set_remove_i64_rid, &[set.addr() as u64, 8], &mut heap)
            .unwrap(),
        1
    );
}
