mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;

#[test]
fn numeric_array_load_is_jit_equivalent() {
    let src = "function read(values: Array): number { return values[1] as number; }\n\
               function main(): number { let values = new Array(2); values[0] = box_any(7.0); values[1] = box_any(41.0); return read(values); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array load");
    assert!(jit.is_compiled(read_rid), "numeric array reader should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(2);
    heap.array_set(array, 0, zun_core::Any::float(7.0));
    heap.array_set(array, 1, zun_core::Any::float(41.0));
    let jit_value = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_packed(read_rid, &[array.addr() as u64])
            .expect("jit array load")
    };
    assert_eq!(f64::from_bits(jit_value), 41.0);
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 41.0));
}

#[test]
fn numeric_array_store_is_jit_equivalent() {
    let src = "function write(values: Array, value: number): number { values[1] = value; return value; }\n\
               function main(): number { let values = new Array(2); return write(values, 41.0); }";
    let module = compile(src);
    let write_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array store");
    assert!(jit.is_compiled(write_rid), "numeric array writer should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(2);
    let jit_value = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_packed(write_rid, &[array.addr() as u64, 41.0f64.to_bits()])
            .expect("jit array store")
    };
    assert_eq!(f64::from_bits(jit_value), 41.0);
    assert_eq!(heap.array_get(array, 1).and_then(|value| value.as_float()), Some(41.0));
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 41.0));
}

#[test]
fn u64_array_load_is_jit_equivalent() {
    let src = "function read(values: Array): u64 { return values[1] as u64; }\n\
               function main(): u64 { let values = new Array(2); values[1] = 41; return read(values); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles u64 array load");
    assert!(jit.is_compiled(rid), "u64 array reader should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(2);
    heap.array_set(array, 1, zun_core::Any::int(41));
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64], &mut heap)
        .expect("jit u64 array load");
    assert_eq!(value, 41);
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 41));
}

#[test]
fn u64_array_store_is_jit_equivalent() {
    let src = "function write(values: Array, value: u64): u64 { values[1] = value; return value; }\n\
               function main(): u64 { let values = new Array(2); return write(values, 41); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .map(|index| index as u32 + 1)
        .expect("write method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles u64 array store");
    assert!(jit.is_compiled(rid), "u64 array writer should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(2);
    let value = jit
        .call_packed_with_heap(rid, &[array.addr() as u64, 41], &mut heap)
        .expect("jit u64 array store");
    assert_eq!(value, 41);
    assert_eq!(heap.array_get(array, 1).and_then(|item| item.as_int()), Some(41));
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 41));
}

#[test]
fn numeric_array_pop_is_jit_equivalent() {
    let src = "function pop_value(values: Array): number { return values.pop() as number; }\n\
               function main(): number { let values = new Array(1); values[0] = box_any(41.0); return pop_value(values); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "pop_value")
        .map(|index| index as u32 + 1)
        .expect("pop method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array pop");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::float(41.0));
    let bits = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_packed(rid, &[array.addr() as u64]).expect("jit array pop")
    };
    assert_eq!(f64::from_bits(bits), 41.0);
    assert_eq!(heap.array_len(array), Some(0));
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 41.0));
}

#[test]
fn numeric_array_push_is_jit_equivalent() {
    let src = "function push_value(values: Array, value: number): number { return values.push(value) as number; }\n\
               function main(): number { let values = new Array(0); return push_value(values, 41.0); }";
    let module = compile(src);
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "push_value")
        .map(|index| index as u32 + 1)
        .expect("push method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric array push");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(0);
    let bits = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_packed(rid, &[array.addr() as u64, 41.0f64.to_bits()])
            .expect("jit array push")
    };
    assert_eq!(f64::from_bits(bits), 1.0);
    assert_eq!(heap.array_get(array, 0).and_then(|value| value.as_float()), Some(41.0));
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn object_array_push_and_pop_are_jit_equivalent() {
    let src = "class Item {} function pop_value(values: Array): Item { return values.pop() as Item; }\n\
               function push_value(values: Array, value: Item): u64 { return push(values, value) as u64; }\n\
               function push(values: Array, value: Item): u64 { return values.push(value) as u64; }\n\
               function main(): u64 { return 0; }";
    let module = compile(src);
    let pop_rid = module.tables.method_defs.iter()
        .position(|method| module.strings.get(method.name) == "pop_value")
        .map(|index| index as u32 + 1).expect("object pop method");
    let push_rid = module.tables.method_defs.iter()
        .position(|method| module.strings.get(method.name) == "push_value")
        .map(|index| index as u32 + 1).expect("object push method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles object array methods");
    assert!(!jit.is_compiled(pop_rid), "checked CastClass must remain Tier 0");
    assert!(jit.is_compiled(push_rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(0);
    let item = heap.alloc(1, 0);
    assert_eq!(jit.call_packed_with_heap(push_rid, &[array.addr() as u64, item.addr() as u64], &mut heap).unwrap(), 1);
    assert_eq!(heap.array_len(array), Some(1));
}

#[test]
fn checked_string_array_unbox_keeps_interpreter_fallback() {
    let src = "function read(values: Array): string { return values[0] as string; }\n\
               function main(): string { let values = new Array(1); values[0] = box_any(\"ok\"); return read(values); }";
    let module = compile(src);
    let read_rid = module.tables.method_defs.iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1).expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles with fallback");
    assert!(!jit.is_compiled(read_rid), "checked UnboxAny must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(_)));
}

#[test]
fn proven_numeric_array_unbox_remains_jit_eligible() {
    let src = "function read(values: Array): u64 { return values[0] as u64; }\n\
               function main(): u64 { let values = new Array(1); values[0] = 37; return read(values); }";
    let module = compile(src);
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles numeric unbox");
    assert!(jit.is_compiled(read_rid), "proven numeric UnboxAny should be eligible");
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::int(37));
    let jit_value = jit
        .call_packed_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit numeric unbox");
    assert_eq!(jit_value, 37);
    assert!(matches!(interp_entry(&module), Value::Int(37, _)));
}

#[test]
fn direct_any_array_load_preserves_the_two_word_return_abi() {
    let module = compile(
        "function read(values: Array): any { return values[0]; } function main(): u64 { return 0; }",
    );
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles Any return");
    assert!(jit.is_compiled(read_rid));
    let mut heap = zun_gc::Heap::new();
    let array = heap.alloc_array(1);
    heap.array_set(array, 0, zun_core::Any::int(37));
    let carrier = jit
        .call_packed_any_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit Any return");
    assert_eq!(carrier.into_any(), Some(zun_core::Any::int(37)));

    let object = heap.alloc(1, 0);
    heap.array_set(array, 0, zun_core::Any::object(object.addr() as usize));
    let carrier = jit
        .call_packed_any_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit object Any return");
    assert_eq!(carrier.into_any(), Some(zun_core::Any::object(object.addr() as usize)));

    let string = heap.intern_string(std::rc::Rc::from("carrier"));
    heap.array_set(array, 0, zun_core::Any::string(string));
    let carrier = jit
        .call_packed_any_with_heap(read_rid, &[array.addr() as u64], &mut heap)
        .expect("jit string Any return");
    assert_eq!(carrier.into_any(), Some(zun_core::Any::string(string)));
}

#[test]
fn direct_any_parameter_roundtrips_through_the_two_word_shim() {
    let module = compile(
        "function echo(value: any): any { return value; } function main(): u64 { return 0; }",
    );
    let echo_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "echo")
        .map(|index| index as u32 + 1)
        .expect("echo method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles Any parameter");
    assert!(jit.is_compiled(echo_rid));
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(91));
    let mut heap = zun_gc::Heap::new();
    let result = jit
        .call_packed_any_with_heap(echo_rid, &[carrier.tag, carrier.payload], &mut heap)
        .expect("jit Any parameter");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(91)));
}

#[test]
fn direct_any_call_preserves_the_two_word_carrier() {
    let module = compile(
        "function echo(value: any): any { return value; }\n\
         function forward(value: any): any { return echo(value); }\n\
         function main(): u64 { return 0; }",
    );
    let forward_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "forward")
        .map(|index| index as u32 + 1)
        .expect("forward method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles direct Any call");
    assert!(jit.is_compiled(forward_rid));
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(123));
    let mut heap = zun_gc::Heap::new();
    let result = jit
        .call_packed_any_with_heap(forward_rid, &[carrier.tag, carrier.payload], &mut heap)
        .expect("jit direct Any call");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(123)));

    let object = heap.alloc(1, 0);
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::object(object.addr() as usize));
    let result = jit
        .call_packed_any_with_heap(forward_rid, &[carrier.tag, carrier.payload], &mut heap)
        .expect("jit direct managed Any call");
    assert_eq!(result.into_any(), Some(zun_core::Any::object(object.addr() as usize)));
}

#[test]
fn direct_any_call_expands_multiple_carriers_without_shifting_arguments() {
    let module = compile(
        "function choose(first: any, second: any): any { return second; }\n\
         function forward(first: any, second: any): any { return choose(first, second); }\n\
         function main(): u64 { return 0; }",
    );
    let forward_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "forward")
        .map(|index| index as u32 + 1)
        .expect("forward method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles multi-carrier call");
    assert!(jit.is_compiled(forward_rid));
    let first = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(1));
    let second = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(2));
    let mut heap = zun_gc::Heap::new();
    let result = jit
        .call_packed_any_with_heap(
            forward_rid,
            &[first.tag, first.payload, second.tag, second.payload],
            &mut heap,
        )
        .expect("jit multi-carrier call");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(2)));
}

#[test]
fn direct_any_virtual_call_preserves_the_carrier() {
    let module = compile(
        "class Box { get(value: any): any { return value; } }\n\
         function forward(box: Box, value: any): any { return box.get(value); }\n\
         function main(): u64 { return 0; }",
    );
    let forward_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "forward")
        .map(|index| index as u32 + 1)
        .expect("forward method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles virtual Any call");
    assert!(jit.is_compiled(forward_rid));
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(321));
    let mut heap = zun_gc::Heap::new();
    let receiver = heap.alloc(1, 0);
    let result = jit
        .call_packed_any_with_heap(
            forward_rid,
            &[receiver.addr() as u64, carrier.tag, carrier.payload],
            &mut heap,
        )
        .expect("jit virtual Any call");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(321)));

    let payload = heap.alloc(1, 0);
    let carrier = zun_codegen::JitAnyAbi::from_any(zun_core::Any::object(payload.addr() as usize));
    let result = jit
        .call_packed_any_with_heap(
            forward_rid,
            &[receiver.addr() as u64, carrier.tag, carrier.payload],
            &mut heap,
        )
        .expect("jit virtual managed Any call");
    assert_eq!(result.into_any(), Some(zun_core::Any::object(payload.addr() as usize)));
}

#[test]
fn generic_map_get_preserves_the_two_word_carrier() {
    let module = compile(
        "import { mapGet } from \"zun:core\";\n\
         function read(map: Map, key: any): any { return mapGet(map, key); }\n\
         function main(): u64 { return 0; }",
    );
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .map(|index| index as u32 + 1)
        .expect("read method");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles generic mapGet");
    assert!(jit.is_compiled(read_rid));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    heap.map_insert(map, zun_core::Any::int(7), zun_core::Any::int(42));
    let key = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(7));
    let result = jit
        .call_packed_any_with_heap(read_rid, &[map.addr() as u64, key.tag, key.payload], &mut heap)
        .expect("jit generic mapGet");
    assert_eq!(result.into_any(), Some(zun_core::Any::int(42)));

    let value = heap.alloc(1, 0);
    heap.map_insert(map, zun_core::Any::int(8), zun_core::Any::object(value.addr() as usize));
    let key = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(8));
    let result = jit
        .call_packed_any_with_heap(read_rid, &[map.addr() as u64, key.tag, key.payload], &mut heap)
        .expect("jit generic managed mapGet");
    assert_eq!(result.into_any(), Some(zun_core::Any::object(value.addr() as usize)));

    let missing_key = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(99));
    let result = jit
        .call_packed_any_with_heap(read_rid, &[map.addr() as u64, missing_key.tag, missing_key.payload], &mut heap)
        .expect("jit generic mapGet missing key");
    assert_eq!(result.into_any(), Some(zun_core::Any::void()));

    let string_handle = heap.intern_string(std::rc::Rc::from("mapped"));
    heap.map_insert(
        map,
        zun_core::Any::int(10),
        zun_core::Any::string(string_handle),
    );
    let key = zun_codegen::JitAnyAbi::from_any(zun_core::Any::int(10));
    let result = jit
        .call_packed_any_with_heap(read_rid, &[map.addr() as u64, key.tag, key.payload], &mut heap)
        .expect("jit generic mapGet string");
    assert_eq!(result.into_any(), Some(zun_core::Any::string(string_handle)));

    let string_key = heap.intern_string(std::rc::Rc::from("key"));
    let string_value = heap.intern_string(std::rc::Rc::from("value"));
    heap.map_insert(
        map,
        zun_core::Any::string(string_key),
        zun_core::Any::string(string_value),
    );
    let key = zun_codegen::JitAnyAbi::from_any(zun_core::Any::string(string_key));
    let result = jit
        .call_packed_any_with_heap(read_rid, &[map.addr() as u64, key.tag, key.payload], &mut heap)
        .expect("jit generic mapGet string key");
    assert_eq!(result.into_any(), Some(zun_core::Any::string(string_value)));

    let interpreter_module = compile(
        "import { mapGet } from \"zun:core\";\n\
         function main(): any { let map = new Map(); return mapGet(map, box_any(99)); }",
    );
    assert!(matches!(interp_entry(&interpreter_module), Value::Void));
}

