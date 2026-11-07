mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;
use zun_interp::CaptureOutput;

#[test]
fn proven_float_array_unbox_is_jit_equivalent() {
    let src = "function read(values: Array): number { return values[0] as number; }\n\
               function main(): number { let values = new Array(1); values[0] = 2.5; return read(values); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles float unbox");
    assert!(
        jit.is_compiled(read_rid),
        "proven float UnboxAny should be eligible"
    );
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::float(2.5));
    let jit_value = jit
        .call_packed_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit float unbox");
    assert_eq!(f64::from_bits(jit_value), 2.5);
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.5));
}

#[test]
fn proven_bool_array_unbox_is_jit_equivalent() {
    let src = "function read(values: Array): boolean { return values[0] as boolean; }\n\
               function main(): boolean { let values = new Array(1); values[0] = true; return read(values); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles bool unbox");
    assert!(
        jit.is_compiled(read_rid),
        "proven bool UnboxAny should be eligible"
    );
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::bool(true));
    let jit_value = jit
        .call_packed_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit bool unbox");
    assert_eq!(jit_value, 1);
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn proven_bool_array_store_is_jit_equivalent() {
    let src = "function write(values: Array, value: boolean): boolean { values[0] = value; return values[0] as boolean; }\n\
               function main(): boolean { let values = new Array(1); return write(values, true); }";
    let module = compile(src);
    let write_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles bool store");
    assert!(
        jit.is_compiled(write_rid),
        "proven bool array store should be eligible"
    );
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    let jit_value = jit
        .call_packed_with_heap(write_rid, &[array.addr() as u64, 1], &mut heap)
        .expect("jit bool store");
    assert_eq!(jit_value, 1);
    assert_eq!(
        heap.array_get(array, 0).and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn proven_i32_array_load_and_store_are_jit_equivalent() {
    let src = "function write(values: Array, value: i32): i32 { values[0] = value; return values[0] as i32; }\n\
               function main(): i32 { let values = new Array(1); return write(values, 42); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i32 array access");
    assert!(jit.is_compiled(rid), "i32 array access should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    let jit_value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64, 42], &mut heap)
        .expect("jit i32 array access");
    assert_eq!(jit_value as i32, 42);
    assert_eq!(
        heap.array_get(array, 0).and_then(|value| value.as_int()),
        Some(42)
    );
    assert!(matches!(interp_entry(&module), Value::Int(42, _)));
}

#[test]
fn proven_i32_array_pop_and_push_are_jit_equivalent() {
    let src = "function pop_value(values: Array): i32 { return values.pop() as i32; }\n\
               function push_value(values: Array, value: i32): u64 { return values.push(value) as u64; }\n\
               function main(): i32 { return 1; }";
    let module = compile(src);
    let pop_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "pop_value")
        .map(|index| index as u32 + 1)
        .expect("pop method");
    let push_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "push_value")
        .map(|index| index as u32 + 1)
        .expect("push method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i32 stack methods");
    assert!(jit.is_compiled(pop_rid));
    assert!(jit.is_compiled(push_rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::int(7));
    assert_eq!(
        jit.call_packed_with_heap(pop_rid, &[array.addr() as u64], &mut heap)
            .unwrap() as i32,
        7
    );
    assert_eq!(
        jit.call_packed_with_heap(push_rid, &[array.addr() as u64, 9], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        heap.array_get(array, 0).and_then(|value| value.as_int()),
        Some(9)
    );
}

#[test]
fn proven_i64_array_access_uses_the_signed_i64_abi() {
    let src = "function write(values: Array, value: i64): i64 { values[0] = value; return values[0] as i64; }\n\
               function main(): i64 { let values = new Array(1); return write(values, -9); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i64 array access");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64, (-9i64) as u64], &mut heap)
        .unwrap();
    assert_eq!(value as i64, -9);
    assert_eq!(
        heap.array_get(array, 0).and_then(|item| item.as_int()),
        Some(-9)
    );
}

#[test]
fn narrow_i16_array_access_is_jit_equivalent() {
    let src = "function write(values: Array, value: i16): i16 { values[0] = value; return values[0] as i16; }\n\
               function main(): i16 { let values = new Array(1); return write(values, -12); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("narrow array module remains valid");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64, (-12i64) as u64], &mut heap)
        .expect("narrow array JIT call");
    assert_eq!(value as i64, -12);
    assert!(matches!(
        interp_entry(&module),
        Value::Int(-12, zun_il::NumKind::I16)
    ));
}

#[test]
fn narrow_i8_array_access_is_jit_equivalent() {
    let src = "function write(values: Array, value: i8): i8 { values[0] = value; return values[0] as i8; }\n\
               function main(): i8 { let values = new Array(1); return write(values, -7); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("narrow array module remains valid");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64, (-7i64) as u64], &mut heap)
        .expect("narrow array JIT call");
    assert_eq!(value as i64, -7);
    assert!(matches!(
        interp_entry(&module),
        Value::Int(-7, zun_il::NumKind::I8)
    ));
}

#[test]
fn narrow_i16_array_push_pop_is_jit_equivalent() {
    let src = "function roundtrip(values: Array, value: i16): i16 { values.push(value); return values.pop() as i16; }\n\
               function main(): i16 { let values = new Array(0); return roundtrip(values, -13); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "roundtrip")
        .map(|index| index as u32 + 1)
        .expect("roundtrip method");
    let jit = zun_codegen::Jit::compile(&module).expect("narrow push/pop module remains valid");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(0);
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64, (-13i64) as u64], &mut heap)
        .expect("narrow push/pop JIT call");
    assert_eq!(value as i64, -13);
    assert!(matches!(
        interp_entry(&module),
        Value::Int(-13, zun_il::NumKind::I16)
    ));
}

#[test]
fn i32_array_loop_is_jit_equivalent() {
    let src = "function main(): i32 { let values = new Array(1); values[0] = 0; let i: i32 = 0; while (i < 4) { values[0] = (values[0] as i32) + 1; i = i + 1; } return values[0] as i32; }";
    let module = compile(src);
    let body =
        zun_module::read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
    let opcodes: Vec<_> = zun_il::decode_all(body.code)
        .unwrap()
        .iter()
        .map(|instruction| instruction.opcode)
        .collect();
    assert!(
        zun_codegen::eligible_methods(&module).contains(&1),
        "opcodes: {opcodes:?}"
    );
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i32 array loop");
    let mut heap = zun_gc::Heap::new();
    let jit_value = jit
        .call_entry_with_heap(1, &mut heap)
        .expect("jit array loop");
    let RetVal::Int(jit_value) = jit_value else {
        panic!("unexpected JIT result")
    };
    assert_eq!(jit_value, 4);
    assert!(matches!(interp_entry(&module), Value::Int(4, _)));
}

#[test]
fn i64_array_loop_is_jit_equivalent() {
    let src = "function main(): i64 { let values = new Array(1); values[0] = 0; let i: i64 = 0; while (i < 4) { values[0] = (values[0] as i64) + 1; i = i + 1; } return values[0] as i64; }";
    let module = compile(src);
    assert!(zun_codegen::eligible_methods(&module).contains(&1));
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i64 array loop");
    let mut heap = zun_gc::Heap::new();
    let jit_value = jit
        .call_entry_with_heap(1, &mut heap)
        .expect("jit i64 array loop");
    let RetVal::Int(jit_value) = jit_value else {
        panic!("unexpected JIT result")
    };
    assert_eq!(jit_value, 4);
    assert!(matches!(
        interp_entry(&module),
        Value::Int(4, zun_il::NumKind::I64)
    ));
}

#[test]
fn numeric_i32_map_and_set_has_use_typed_jit_abis() {
    let src = "import { mapHasI32, setHasI32 } from \"zun:core\"; function mapContains(value: Map, key: i32): boolean { return mapHasI32(value, key); } function setContains(value: Set, key: i32): boolean { return setHasI32(value, key); } function main(): boolean { return true; }";
    let module = compile(src);
    let map_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "mapContains")
        .map(|index| index as u32 + 1)
        .expect("map method");
    let set_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "setContains")
        .map(|index| index as u32 + 1)
        .expect("set method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i32 collection queries");
    assert!(jit.is_compiled(map_rid));
    assert!(jit.is_compiled(set_rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let set = heap.alloc_set();
    heap.map_insert(map, zun_core::Any::int(7), zun_core::Any::int(1));
    heap.set_add(set, zun_core::Any::int(7));
    assert_eq!(
        jit.call_packed_with_heap(map_rid, &[map.addr() as u64, 7], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(set_rid, &[set.addr() as u64, 7], &mut heap)
            .unwrap(),
        1
    );
}

#[test]
fn numeric_i32_map_get_uses_typed_jit_abi() {
    let src = "import { mapGetI32Or } from \"zun:core\"; function read(value: Map, key: i32, fallback: i32): i32 { return mapGetI32Or(value, key, fallback); } function main(): i32 { return 0; }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i32 map get");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    heap.map_insert(map, zun_core::Any::int(7), zun_core::Any::int(42));
    assert_eq!(
        jit.call_packed_with_heap(rid, &[map.addr() as u64, 7, 0], &mut heap)
            .unwrap() as i32,
        42
    );
    assert_eq!(
        jit.call_packed_with_heap(rid, &[map.addr() as u64, 8, -3i32 as u64], &mut heap)
            .unwrap() as i32,
        -3
    );
}

#[test]
fn numeric_i32_map_and_set_mutations_use_typed_jit_abis() {
    let src = "import { mapSetI32, setAddI32 } from \"zun:core\"; function put(value: Map, key: i32, item: i32): boolean { return mapSetI32(value, key, item); } function add(value: Set, item: i32): boolean { return setAddI32(value, item); } function main(): boolean { return true; }";
    let module = compile(src);
    let put_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "put")
        .map(|index| index as u32 + 1)
        .expect("map put method");
    let add_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "add")
        .map(|index| index as u32 + 1)
        .expect("set add method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i32 collection mutations");
    assert!(jit.is_compiled(put_rid));
    assert!(jit.is_compiled(add_rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let set = heap.alloc_set();
    assert_eq!(
        jit.call_packed_with_heap(put_rid, &[map.addr() as u64, 7, 42], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(put_rid, &[map.addr() as u64, 7, 99], &mut heap)
            .unwrap(),
        0
    );
    assert_eq!(
        heap.map_get(map, zun_core::Any::int(7))
            .and_then(|value| value.as_int()),
        Some(99)
    );
    assert_eq!(
        jit.call_packed_with_heap(add_rid, &[set.addr() as u64, 7], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(add_rid, &[set.addr() as u64, 7], &mut heap)
            .unwrap(),
        0
    );
    assert_eq!(heap.set_has(set, zun_core::Any::int(7)), Some(true));
}

#[test]
fn numeric_i32_map_and_set_deletions_use_typed_jit_abis() {
    let src = "import { mapSetI32, setAddI32, mapDeleteI32, setDeleteI32 } from \"zun:core\"; function removeMap(value: Map, key: i32): boolean { return mapDeleteI32(value, key); } function removeSet(value: Set, item: i32): boolean { return setDeleteI32(value, item); } function main(): boolean { let map = new Map(); let set = new Set(); mapSetI32(map, 7, 1); setAddI32(set, 7); return mapDeleteI32(map, 7) && setDeleteI32(set, 7); }";
    let module = compile(src);
    let map_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "removeMap")
        .map(|index| index as u32 + 1)
        .expect("map delete method");
    let set_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "removeSet")
        .map(|index| index as u32 + 1)
        .expect("set delete method");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i32 collection deletions");
    assert!(jit.is_compiled(map_rid));
    assert!(jit.is_compiled(set_rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let set = heap.alloc_set();
    heap.map_insert(map, zun_core::Any::int(7), zun_core::Any::int(42));
    heap.set_add(set, zun_core::Any::int(7));
    assert_eq!(
        jit.call_packed_with_heap(map_rid, &[map.addr() as u64, 7], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(map_rid, &[map.addr() as u64, 7], &mut heap)
            .unwrap(),
        0
    );
    assert_eq!(
        jit.call_packed_with_heap(set_rid, &[set.addr() as u64, 7], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        jit.call_packed_with_heap(set_rid, &[set.addr() as u64, 7], &mut heap)
            .unwrap(),
        0
    );
}

#[test]
fn numeric_i32_map_mutation_loop_is_jit_equivalent() {
    let src = "import { mapSetI32, mapGetI32Or } from \"zun:core\"; function fill(value: Map): i32 { let index: i32 = 0; while (index < 4) { mapSetI32(value, index, index * 10); index = index + 1; } return mapGetI32Or(value, 3, -1); } function main(): i32 { let value = new Map(); return fill(value); }";
    let module = compile(src);
    let fill_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "fill")
        .map(|index| index as u32 + 1)
        .expect("fill method");
    assert!(matches!(interp_entry(&module), Value::Int(30, _)));
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i32 map loop");
    assert!(jit.is_compiled(fill_rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let jit_value = jit
        .call_packed_with_heap(fill_rid, &[map.addr() as u64], &mut heap)
        .unwrap() as i32;
    assert_eq!(jit_value, 30);
}

#[test]
fn numeric_i32_set_mutation_loop_is_jit_equivalent() {
    let src = "import { setAddI32, setHasI32 } from \"zun:core\"; function fill(value: Set): boolean { let index: i32 = 0; while (index < 4) { setAddI32(value, index); index = index + 1; } return setHasI32(value, 3); } function main(): boolean { let value = new Set(); return fill(value); }";
    let module = compile(src);
    let fill_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "fill")
        .map(|index| index as u32 + 1)
        .expect("fill method");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i32 set loop");
    assert!(jit.is_compiled(fill_rid));
    let mut heap = zun_gc::Heap::new();
    let set = heap.alloc_set();
    assert_eq!(
        jit.call_packed_with_heap(fill_rid, &[set.addr() as u64], &mut heap)
            .unwrap(),
        1
    );
}

#[test]
fn numeric_i64_collection_intrinsics_are_tier0_jit_equivalent() {
    let src = "import { mapSetI64, mapGetI64Or, setAddI64, mapDeleteI64, setDeleteI64 } from \"zun:core\"; function main(): boolean { let map = new Map(); let set = new Set(); mapSetI64(map, 7, 42); setAddI64(set, 7); return mapGetI64Or(map, 7, -1) == 42 && mapDeleteI64(map, 7) && setDeleteI64(set, 7); }";
    let module = compile(src);
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles i64 collection intrinsics");
    let entry = module.entry_point.expect("entry point").rid();
    assert!(
        !jit.is_compiled(entry),
        "constructor-bearing entry keeps the Tier 0 fallback"
    );
}

#[test]
fn numeric_f64_collection_intrinsics_are_tier0_jit_equivalent() {
    let src = "import { mapSetF64, mapGetF64Or, setAddF64, mapDeleteF64, setDeleteF64 } from \"zun:core\"; function main(): boolean { let map = new Map(); let set = new Set(); mapSetF64(map, 7.5, 42.5); setAddF64(set, 7.5); return mapGetF64Or(map, 7.5, -1.0) == 42.5 && mapDeleteF64(map, 7.5) && setDeleteF64(set, 7.5); }";
    let module = compile(src);
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles f64 collection intrinsics");
    let entry = module.entry_point.expect("entry point").rid();
    assert!(
        !jit.is_compiled(entry),
        "constructor-bearing entry keeps the Tier 0 fallback"
    );
}

#[test]
fn managed_collection_clear_is_jit_equivalent() {
    let src = "import { mapClear, mapSize, setClear, setSize } from \"zun:core\"; function clearMap(value: Map): u64 { mapClear(value); return mapSize(value); } function clearSet(value: Set): u64 { setClear(value); return setSize(value); } function main(): boolean { return true; }";
    let interp_module = compile("import { mapSetI32, mapClear, mapSize, setAddI32, setClear, setSize } from \"zun:core\"; function main(): boolean { let map = new Map(); let set = new Set(); mapSetI32(map, 1, 2); setAddI32(set, 1); mapClear(map); setClear(set); if (mapSize(map) == 0) { return setSize(set) == 0; } return false; }");
    let module = compile(src);
    let map_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "clearMap")
        .map(|index| index as u32 + 1)
        .expect("map clear method");
    let set_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "clearSet")
        .map(|index| index as u32 + 1)
        .expect("set clear method");
    assert!(matches!(interp_entry(&interp_module), Value::Bool(true)));
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles collection clear");
    assert!(jit.is_compiled(map_rid));
    assert!(jit.is_compiled(set_rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let set = heap.alloc_set();
    heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::int(2));
    heap.set_add(set, zun_core::Any::int(1));
    assert_eq!(
        jit.call_packed_with_heap(map_rid, &[map.addr() as u64], &mut heap)
            .unwrap(),
        0
    );
    assert_eq!(
        jit.call_packed_with_heap(set_rid, &[set.addr() as u64], &mut heap)
            .unwrap(),
        0
    );
}

#[test]
fn managed_map_values_uses_the_managed_array_abi() {
    let src = "import { mapValues } from \"zun:core\"; function values(value: Map): Array { return mapValues(value); } function main(): boolean { return true; }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "values")
        .map(|index| index as u32 + 1)
        .expect("map values method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles map values");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::int(10));
    heap.map_insert(map, zun_core::Any::int(2), zun_core::Any::int(20));
    let array_addr = jit
        .call_packed_with_heap(rid, &[map.addr() as u64], &mut heap)
        .unwrap();
    let array = unsafe { zun_gc::Obj::from_addr(array_addr as *mut u64) };
    assert_eq!(heap.array_len(array), Some(2));
    assert_eq!(
        heap.array_get(array, 0).and_then(|value| value.as_int()),
        Some(10)
    );
    assert_eq!(
        heap.array_get(array, 1).and_then(|value| value.as_int()),
        Some(20)
    );
}

#[test]
fn managed_map_keys_uses_the_managed_array_abi() {
    let src = "import { mapKeys } from \"zun:core\"; function keys(value: Map): Array { return mapKeys(value); } function main(): boolean { return true; }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "keys")
        .map(|index| index as u32 + 1)
        .expect("map keys method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles map keys");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::int(10));
    heap.map_insert(map, zun_core::Any::int(2), zun_core::Any::int(20));
    let array_addr = jit
        .call_packed_with_heap(rid, &[map.addr() as u64], &mut heap)
        .unwrap();
    let array = unsafe { zun_gc::Obj::from_addr(array_addr as *mut u64) };
    assert_eq!(heap.array_len(array), Some(2));
    assert_eq!(
        heap.array_get(array, 0).and_then(|value| value.as_int()),
        Some(1)
    );
    assert_eq!(
        heap.array_get(array, 1).and_then(|value| value.as_int()),
        Some(2)
    );
}

#[test]
fn managed_map_entries_uses_nested_managed_arrays() {
    let src = "import { mapEntries } from \"zun:core\"; function entries(value: Map): Array { return mapEntries(value); } function main(): boolean { return true; }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "entries")
        .map(|index| index as u32 + 1)
        .expect("map entries method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles map entries");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::int(10));
    let array_addr = jit
        .call_packed_with_heap(rid, &[map.addr() as u64], &mut heap)
        .unwrap();
    let entries = unsafe { zun_gc::Obj::from_addr(array_addr as *mut u64) };
    let pair = heap
        .array_get(entries, 0)
        .and_then(|value| value.as_object())
        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
        .expect("entry pair");
    assert_eq!(heap.array_len(entries), Some(1));
    assert_eq!(
        heap.array_get(pair, 0).and_then(|value| value.as_int()),
        Some(1)
    );
    assert_eq!(
        heap.array_get(pair, 1).and_then(|value| value.as_int()),
        Some(10)
    );
}

#[test]
fn managed_set_values_and_entries_use_managed_arrays() {
    let src = "import { setValues, setEntries } from \"zun:core\"; function values(value: Set): Array { return setValues(value); } function entries(value: Set): Array { return setEntries(value); } function main(): boolean { return true; }";
    let module = compile(src);
    let values_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "values")
        .map(|index| index as u32 + 1)
        .expect("set values method");
    let entries_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "entries")
        .map(|index| index as u32 + 1)
        .expect("set entries method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles set values and entries");
    assert!(jit.is_compiled(values_rid));
    assert!(jit.is_compiled(entries_rid));
    let mut heap = zun_gc::Heap::new();
    let set = heap.alloc_set();
    heap.set_add(set, zun_core::Any::int(7));
    let values_addr = jit
        .call_packed_with_heap(values_rid, &[set.addr() as u64], &mut heap)
        .unwrap();
    let entries_addr = jit
        .call_packed_with_heap(entries_rid, &[set.addr() as u64], &mut heap)
        .unwrap();
    let values = unsafe { zun_gc::Obj::from_addr(values_addr as *mut u64) };
    let entries = unsafe { zun_gc::Obj::from_addr(entries_addr as *mut u64) };
    let pair = heap
        .array_get(entries, 0)
        .and_then(|value| value.as_object())
        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
        .expect("set entry pair");
    assert_eq!(
        heap.array_get(values, 0).and_then(|value| value.as_int()),
        Some(7)
    );
    assert_eq!(
        heap.array_get(pair, 0).and_then(|value| value.as_int()),
        Some(7)
    );
    assert_eq!(
        heap.array_get(pair, 1).and_then(|value| value.as_int()),
        Some(7)
    );
}

#[test]
fn managed_collection_constructors_keep_the_tier0_fallback() {
    let src = "import { mapSetI32, setAddI32 } from \"zun:core\"; function main(): boolean { let map = new Map(); let set = new Set(); mapSetI32(map, 1, 2); setAddI32(set, 1); return true; }";
    let module = compile(src);
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit preserves constructor fallback");
    assert!(!jit.is_compiled(entry));
}

#[test]
fn managed_collection_constructor_return_can_use_object_jit_abi() {
    let src = "import { mapNew, setNew } from \"zun:core\"; function makeMap(): Map { return mapNew(); } function makeSet(): Set { return setNew(); } function main(): boolean { return true; }";
    let module = compile(src);
    let map_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "makeMap")
        .map(|index| index as u32 + 1)
        .expect("map constructor method");
    let set_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "makeSet")
        .map(|index| index as u32 + 1)
        .expect("set constructor method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles object constructors");
    assert!(jit.is_compiled(map_rid));
    assert!(jit.is_compiled(set_rid));
    let mut heap = zun_gc::Heap::new();
    let map_addr = jit.call_packed_with_heap(map_rid, &[], &mut heap).unwrap();
    let set_addr = jit.call_packed_with_heap(set_rid, &[], &mut heap).unwrap();
    let map = unsafe { zun_gc::Obj::from_addr(map_addr as *mut u64) };
    let set = unsafe { zun_gc::Obj::from_addr(set_addr as *mut u64) };
    assert_eq!(heap.map_len(map), Some(0));
    assert_eq!(heap.set_len(set), Some(0));
}

#[test]
fn proven_bool_array_pop_and_push_are_jit_equivalent() {
    let src = "function pop_value(values: Array): boolean { return values.pop() as boolean; }\n\
               function push_value(values: Array, value: boolean): u64 { return values.push(value) as u64; }\n\
               function main(): boolean { return true; }";
    let module = compile(src);
    let pop_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "pop_value")
        .map(|index| index as u32 + 1)
        .expect("pop method");
    let push_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "push_value")
        .map(|index| index as u32 + 1)
        .expect("push method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles bool stack methods");
    assert!(jit.is_compiled(pop_rid));
    assert!(jit.is_compiled(push_rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::bool(true));
    assert_eq!(
        jit.call_packed_with_heap(pop_rid, &[array.addr() as u64], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(heap.array_len(array), Some(0));
    assert_eq!(
        jit.call_packed_with_heap(push_rid, &[array.addr() as u64, 1], &mut heap)
            .unwrap(),
        1
    );
    assert_eq!(
        heap.array_get(array, 0).and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn numeric_array_allocation_is_jit_equivalent() {
    let src = "function make(length: u64): Array { return new Array(length); }\n\
               function main(): Array { return make(3); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "make")
        .map(|index| index as u32 + 1)
        .expect("make method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array allocation");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let bits = jit
        .call_packed_with_heap(rid, &[3], &mut heap)
        .expect("jit array allocation");
    let array = unsafe { zun_gc::Obj::from_addr(bits as *mut u64) };
    assert_eq!(heap.array_len(array), Some(3));
    assert!(matches!(interp_entry(&module), Value::Object(_)));
}

#[test]
fn jit_reads_length_from_an_existing_managed_array() {
    let src = "function length(values: Array): u64 { return values.length; }\n\
               function main(): u64 { let values = new Array(3); return length(values); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "length")
        .map(|index| index as u32 + 1)
        .expect("length method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles array length");
    assert!(
        jit.is_compiled(rid),
        "array length reader should be eligible"
    );
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(3);
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64], &mut heap)
        .expect("jit array length");
    assert_eq!(value, 3);
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 3));
}

#[test]
fn gc_preserves_and_reclaims_a_jit_allocated_object_by_root_contract() {
    let module = compile(
        "class Box { value: int; }
         function make(): Box { return new Box(7); }
         function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "make")
        .map(|index| index as u32 + 1)
        .expect("make method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles object allocation");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let bits = jit
        .call_packed_with_heap(rid, &[], &mut heap)
        .expect("jit object allocation");
    let object = unsafe { zun_gc::Obj::from_addr(bits as *mut u64) };
    assert_eq!(object.field(0), 7);
    assert_eq!(unsafe { heap.collect(&[object], &|_| Vec::new()) }, 0);
    assert_eq!(object.field(0), 7);
    assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 1);
}

#[test]
fn gc_traces_references_inside_a_jit_allocated_object() {
    let module = compile(
        "class Box { value: int; }
         class Holder { box: Box; }
         function make(): Holder { let value = new Box(7); return new Holder(value); }
         function main(): int { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "make")
        .map(|index| index as u32 + 1)
        .expect("make method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles nested allocation");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let bits = jit
        .call_packed_with_heap(rid, &[], &mut heap)
        .expect("jit nested allocation");
    let holder = unsafe { zun_gc::Obj::from_addr(bits as *mut u64) };
    let box_object = unsafe { zun_gc::Obj::from_addr(holder.field(0) as *mut u64) };
    assert_eq!(box_object.field(0), 7);
    let holder_type = holder.type_rid();
    assert_eq!(
        unsafe {
            heap.collect(&[holder], &|type_rid| {
                (type_rid == holder_type)
                    .then_some(vec![0])
                    .unwrap_or_default()
            })
        },
        0
    );
    assert_eq!(box_object.field(0), 7);
    assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 2);
}

#[test]
fn generator_session_yields_and_resumes_in_tier0() {
    let module = zun_compiler::compile("function* count(): int { yield 1; yield 2; return 3; }")
        .expect("compile generator");
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "count")
        .map(|index| index as u32 + 1)
        .expect("generator method");
    let mut output = CaptureOutput::default();
    let mut session = zun_interp::GeneratorSession::new(&module, rid, Vec::new(), &mut output)
        .expect("create generator session");
    assert!(!session.is_done());
    assert!(
        matches!(session.next().expect("first generator next"), zun_interp::GeneratorResult::Yielded(Value::Float(value, _)) if value == 1.0)
    );
    assert!(matches!(session.current(), Some(Value::Float(value, _)) if *value == 1.0));
    assert!(matches!(
        session.next().expect("second generator next"),
        zun_interp::GeneratorResult::Yielded(Value::Float(value, _)) if value == 2.0
    ));
    let complete = session.next().expect("generator completion");
    assert!(
        matches!(complete, zun_interp::GeneratorResult::Complete(Value::Int(value, _)) if value == 3),
        "{complete:?}"
    );
    assert!(session.is_done());
    assert!(session.current().is_none());
    assert!(matches!(
        session.next().expect("sticky generator completion"),
        zun_interp::GeneratorResult::Complete(Value::Int(value, _)) if value == 3
    ));
}

#[test]
fn generator_session_implements_host_iterator_protocol() {
    let module = zun_compiler::compile("function* count(): int { yield 4; yield 5; return 6; }")
        .expect("compile generator");
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "count")
        .map(|index| index as u32 + 1)
        .expect("generator method");
    let mut output = CaptureOutput::default();
    let session = zun_interp::GeneratorSession::new(&module, rid, Vec::new(), &mut output)
        .expect("create generator session");
    let values: Vec<_> = session.collect();
    assert!(matches!(values.as_slice(), [
        Value::Float(first, _),
        Value::Float(second, _)
    ] if *first == 4.0 && *second == 5.0));
}

#[test]
fn generator_session_exposes_iterator_result_shape() {
    let module = zun_compiler::compile("function* count(): int { yield 4; return 6; }")
        .expect("compile generator");
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "count")
        .map(|index| index as u32 + 1)
        .expect("generator method");
    let mut output = CaptureOutput::default();
    let mut session = zun_interp::GeneratorSession::new(&module, rid, Vec::new(), &mut output)
        .expect("create generator session");

    let yielded = session.next_result().expect("yield result");
    assert!(!yielded.done);
    assert!(matches!(yielded.value, Value::Float(value, _) if value == 4.0));

    let completed = session.next_result().expect("completion result");
    assert!(completed.done);
    assert!(matches!(completed.value, Value::Int(value, _) if value == 6));

    let sticky = session.next_result().expect("sticky completion result");
    assert!(sticky.done);
    assert!(matches!(sticky.value, Value::Int(value, _) if value == 6));
}

#[test]
fn generator_session_rejects_a_regular_method() {
    let module = zun_compiler::compile("function plain(): int { return 1; }")
        .expect("compile regular method");
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "plain")
        .map(|index| index as u32 + 1)
        .expect("regular method");
    let mut output = CaptureOutput::default();
    let error = zun_interp::GeneratorSession::new(&module, rid, Vec::new(), &mut output)
        .expect_err("regular method must not create a generator session");
    assert!(
        matches!(error, zun_interp::RunError::Malformed(message) if message.contains("requires a method containing yield"))
    );
}

#[test]
fn non_overridden_virtual_call_is_jit_eligible() {
    let src = "class Counter { n: int; get(): int { return this.n; } }\n\
               function read(c: Counter): int { return c.get(); }\n\
               function main(): int { return read(new Counter(7)); }";
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
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 7));
}
