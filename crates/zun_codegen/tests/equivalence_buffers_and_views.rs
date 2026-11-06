mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;

#[test]
fn array_buffer_length_uses_the_managed_object_abi() {
    let module = compile(
        "import { bufferLength } from \"zun:core\"; function bufferSize(value: ArrayBuffer): u64 { return bufferLength(value); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "bufferSize")
        .expect("bufferSize method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(17);
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[buffer.addr() as u64], &mut heap),
        Ok(17)
    ));
}

#[test]
fn managed_view_lengths_use_typed_object_abis() {
    let module = compile(
        "import { dataViewLength, dataViewByteLength, u8ArrayLength, u8ArrayByteLength, u16ArrayLength, u16ArrayByteLength, u32ArrayLength, u32ArrayByteLength, f64ArrayLength, f64ArrayByteLength } from \"zun:core\"; function data(view: DataView): u64 { return dataViewLength(view); } function dataBytes(view: DataView): u64 { return dataViewByteLength(view); } function u8(view: Uint8Array): u64 { return u8ArrayLength(view); } function u8Bytes(view: Uint8Array): u64 { return u8ArrayByteLength(view); } function u16(view: Uint16Array): u64 { return u16ArrayLength(view); } function u16Bytes(view: Uint16Array): u64 { return u16ArrayByteLength(view); } function u32(view: Uint32Array): u64 { return u32ArrayLength(view); } function u32Bytes(view: Uint32Array): u64 { return u32ArrayByteLength(view); } function f64(view: Float64Array): u64 { return f64ArrayLength(view); } function f64Bytes(view: Float64Array): u64 { return f64ArrayByteLength(view); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("view length method") as u32
            + 1
    };
    let data = find("data");
    let data_bytes = find("dataBytes");
    let u8 = find("u8");
    let u8_bytes = find("u8Bytes");
    let u16 = find("u16");
    let u16_bytes = find("u16Bytes");
    let u32 = find("u32");
    let u32_bytes = find("u32Bytes");
    let f64 = find("f64");
    let f64_bytes = find("f64Bytes");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(data));
    assert!(jit.is_compiled(data_bytes));
    assert!(jit.is_compiled(u8));
    assert!(jit.is_compiled(u8_bytes));
    assert!(jit.is_compiled(u16));
    assert!(jit.is_compiled(u16_bytes));
    assert!(jit.is_compiled(u32));
    assert!(jit.is_compiled(u32_bytes));
    assert!(jit.is_compiled(f64));
    assert!(jit.is_compiled(f64_bytes));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(32);
    let data_view = heap.alloc_data_view(buffer, 0, 12).expect("data view");
    let u8_view = heap.alloc_typed_array(buffer, 0, 5, zun_core::TypedElement::U8).expect("u8 view");
    let u16_view = heap.alloc_typed_array(buffer, 0, 4, zun_core::TypedElement::U16).expect("u16 view");
    let u32_view = heap.alloc_typed_array(buffer, 0, 2, zun_core::TypedElement::U32).expect("u32 view");
    let f64_view = heap.alloc_typed_array(buffer, 0, 2, zun_core::TypedElement::F64).expect("f64 view");
    assert!(matches!(jit.call_packed_with_heap(data, &[data_view.addr() as u64], &mut heap), Ok(12)));
    assert!(matches!(jit.call_packed_with_heap(data_bytes, &[data_view.addr() as u64], &mut heap), Ok(12)));
    assert!(matches!(jit.call_packed_with_heap(u8, &[u8_view.addr() as u64], &mut heap), Ok(5)));
    assert!(matches!(jit.call_packed_with_heap(u8_bytes, &[u8_view.addr() as u64], &mut heap), Ok(5)));
    assert!(matches!(jit.call_packed_with_heap(u16, &[u16_view.addr() as u64], &mut heap), Ok(4)));
    assert!(matches!(jit.call_packed_with_heap(u16_bytes, &[u16_view.addr() as u64], &mut heap), Ok(8)));
    assert!(matches!(jit.call_packed_with_heap(u32, &[u32_view.addr() as u64], &mut heap), Ok(2)));
    assert!(matches!(jit.call_packed_with_heap(u32_bytes, &[u32_view.addr() as u64], &mut heap), Ok(8)));
    assert!(matches!(jit.call_packed_with_heap(f64, &[f64_view.addr() as u64], &mut heap), Ok(2)));
    assert!(matches!(jit.call_packed_with_heap(f64_bytes, &[f64_view.addr() as u64], &mut heap), Ok(16)));
}

#[test]
fn array_buffer_u8_read_uses_the_scalar_managed_abi() {
    let module = compile(
        "import { bufferGetU8 } from \"zun:core\"; function read(buffer: ArrayBuffer, index: u64): u8 { return bufferGetU8(buffer, index); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .expect("read method") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(2);
    heap.buffer_set(buffer, 1, 0xab);
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[buffer.addr() as u64, 1], &mut heap),
        Ok(0xab)
    ));
}

#[test]
fn array_buffer_numeric_reads_use_width_specific_managed_abis() {
    let module = compile(
        "import { bufferReadU32, bufferReadF64 } from \"zun:core\"; function readU32(buffer: ArrayBuffer, offset: u64): u32 { return bufferReadU32(buffer, offset); } function readF64(buffer: ArrayBuffer, offset: u64): number { return bufferReadF64(buffer, offset); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("buffer reader method") as u32
            + 1
    };
    let u32_rid = find("readU32");
    let f64_rid = find("readF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(u32_rid));
    assert!(jit.is_compiled(f64_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(16);
    heap.buffer_write_u32(buffer, 0, 0xfeed_beef);
    heap.buffer_write_f64(buffer, 8, 2.5);
    assert!(matches!(
        jit.call_packed_with_heap(u32_rid, &[buffer.addr() as u64, 0], &mut heap),
        Ok(0xfeed_beef)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(f64_rid, &[buffer.addr() as u64, 8], &mut heap),
        Ok(value) if f64::from_bits(value) == 2.5
    ));
}

#[test]
fn array_buffer_numeric_writes_use_scalar_managed_abis() {
    let module = compile(
        "import { bufferSetU8, bufferWriteU32, bufferWriteF64 } from \"zun:core\"; function setU8(buffer: ArrayBuffer, index: u64, value: u8): boolean { return bufferSetU8(buffer, index, value); } function writeU32(buffer: ArrayBuffer, offset: u64, value: u32): boolean { return bufferWriteU32(buffer, offset, value); } function writeF64(buffer: ArrayBuffer, offset: u64, value: number): boolean { return bufferWriteF64(buffer, offset, value); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("buffer writer method") as u32
            + 1
    };
    let u8_rid = find("setU8");
    let u32_rid = find("writeU32");
    let f64_rid = find("writeF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(u8_rid));
    assert!(jit.is_compiled(u32_rid));
    assert!(jit.is_compiled(f64_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(16);
    assert!(matches!(
        jit.call_packed_with_heap(u8_rid, &[buffer.addr() as u64, 1, 0xab], &mut heap),
        Ok(1)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(u32_rid, &[buffer.addr() as u64, 4, 0xfeed_beef], &mut heap),
        Ok(1)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(f64_rid, &[buffer.addr() as u64, 8, 2.5f64.to_bits()], &mut heap),
        Ok(1)
    ));
    assert_eq!(heap.buffer_get(buffer, 1), Some(0xab));
    assert_eq!(heap.buffer_read_u32(buffer, 4), Some(0xfeed_beef));
    assert_eq!(heap.buffer_read_f64(buffer, 8), Some(2.5));
}

#[test]
fn data_view_u32_read_uses_the_typed_view_abi() {
    let module = compile(
        "import { dataViewGetU32 } from \"zun:core\"; function read(view: DataView, offset: u64, little: boolean): u32 { return dataViewGetU32(view, offset, little); } function main(): u64 { return 0; }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .expect("data view reader") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(4);
    heap.buffer_write_u32(buffer, 0, 0xfeed_beef);
    let view = heap.alloc_data_view(buffer, 0, 4).expect("data view");
    assert!(matches!(
        jit.call_packed_with_heap(rid, &[view.addr() as u64, 0, 1], &mut heap),
        Ok(0xfeed_beef)
    ));
}

#[test]
fn data_view_u16_and_f64_reads_use_width_specific_abis() {
    let module = compile(
        "import { dataViewGetU16, dataViewGetF64 } from \"zun:core\"; function readU16(view: DataView, offset: u64, little: boolean): u16 { return dataViewGetU16(view, offset, little); } function readF64(view: DataView, offset: u64, little: boolean): number { return dataViewGetF64(view, offset, little); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("data view reader") as u32
            + 1
    };
    let u16_rid = find("readU16");
    let f64_rid = find("readF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(u16_rid));
    assert!(jit.is_compiled(f64_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(16);
    heap.buffer_write_u32(buffer, 0, 0xfeed_beef);
    heap.buffer_write_f64(buffer, 8, 2.5);
    let view = heap.alloc_data_view(buffer, 0, 16).expect("data view");
    assert!(matches!(
        jit.call_packed_with_heap(u16_rid, &[view.addr() as u64, 0, 1], &mut heap),
        Ok(0xbeef)
    ));
    assert!(matches!(
        jit.call_packed_with_heap(f64_rid, &[view.addr() as u64, 8, 1], &mut heap),
        Ok(value) if f64::from_bits(value) == 2.5
    ));
}

#[test]
fn data_view_scalar_writes_use_width_specific_abis() {
    let module = compile(
        "import { dataViewSetU32, dataViewSetU16, dataViewSetF64 } from \"zun:core\"; function setU32(view: DataView, offset: u64, value: u32, little: boolean): boolean { return dataViewSetU32(view, offset, value, little); } function setU16(view: DataView, offset: u64, value: u16, little: boolean): boolean { return dataViewSetU16(view, offset, value, little); } function setF64(view: DataView, offset: u64, value: number, little: boolean): boolean { return dataViewSetF64(view, offset, value, little); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == name)
            .expect("data view writer") as u32
            + 1
    };
    let u32_rid = find("setU32");
    let u16_rid = find("setU16");
    let f64_rid = find("setF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(u32_rid));
    assert!(jit.is_compiled(u16_rid));
    assert!(jit.is_compiled(f64_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(16);
    let view = heap.alloc_data_view(buffer, 0, 16).expect("data view");
    assert!(matches!(jit.call_packed_with_heap(u32_rid, &[view.addr() as u64, 0, 0xfeed_beef, 1], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(u16_rid, &[view.addr() as u64, 4, 0xbeef, 1], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(f64_rid, &[view.addr() as u64, 8, 2.5f64.to_bits(), 1], &mut heap), Ok(1)));
    assert_eq!(heap.data_view_get_u32(view, 0, true), Some(0xfeed_beef));
    assert_eq!(heap.data_view_get_u16(view, 4, true), Some(0xbeef));
    assert_eq!(heap.data_view_get_f64(view, 8, true), Some(2.5));
}

#[test]
fn u8_array_access_uses_the_typed_view_abi() {
    let module = compile(
        "import { u8ArrayGet, u8ArraySet } from \"zun:core\"; function read(view: Uint8Array, index: u64): u8 { return u8ArrayGet(view, index); } function write(view: Uint8Array, index: u64, value: u8): boolean { return u8ArraySet(view, index, value); } function main(): u64 { return 0; }",
    );
    let read_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "read")
        .expect("u8 reader") as u32
        + 1;
    let write_rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "write")
        .expect("u8 writer") as u32
        + 1;
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(read_rid));
    assert!(jit.is_compiled(write_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(2);
    let view = heap.alloc_typed_array(buffer, 0, 2, zun_core::TypedElement::U8).expect("u8 view");
    assert!(matches!(jit.call_packed_with_heap(write_rid, &[view.addr() as u64, 1, 0xcd], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(read_rid, &[view.addr() as u64, 1], &mut heap), Ok(0xcd)));
}

#[test]
fn u16_array_access_uses_the_typed_view_abi() {
    let module = compile(
        "import { u16ArrayGet, u16ArraySet } from \"zun:core\"; function read(view: Uint16Array, index: u64): u16 { return u16ArrayGet(view, index); } function write(view: Uint16Array, index: u64, value: u16): boolean { return u16ArraySet(view, index, value); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == name).expect("u16 method") as u32 + 1;
    let read_rid = find("read");
    let write_rid = find("write");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(read_rid));
    assert!(jit.is_compiled(write_rid));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(4);
    let view = heap.alloc_typed_array(buffer, 0, 2, zun_core::TypedElement::U16).expect("u16 view");
    let write_result = jit.call_packed_with_heap(write_rid, &[view.addr() as u64, 1, 0xbeef], &mut heap);
    assert!(matches!(write_result, Ok(1)), "write result: {write_result:?}");
    assert!(matches!(jit.call_packed_with_heap(read_rid, &[view.addr() as u64, 1], &mut heap), Ok(0xbeef)));
}

#[test]
fn u32_and_f64_array_access_use_typed_view_abis() {
    let module = compile(
        "import { u32ArrayGet, u32ArraySet, f64ArrayGet, f64ArraySet } from \"zun:core\"; function readU32(view: Uint32Array, index: u64): u32 { return u32ArrayGet(view, index); } function writeU32(view: Uint32Array, index: u64, value: u32): boolean { return u32ArraySet(view, index, value); } function readF64(view: Float64Array, index: u64): number { return f64ArrayGet(view, index); } function writeF64(view: Float64Array, index: u64, value: number): boolean { return f64ArraySet(view, index, value); } function main(): u64 { return 0; }",
    );
    let find = |name: &str| module.tables.method_defs.iter().position(|method| module.strings.get(method.name) == name).expect("typed array method") as u32 + 1;
    let read_u32 = find("readU32");
    let write_u32 = find("writeU32");
    let read_f64 = find("readF64");
    let write_f64 = find("writeF64");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(read_u32));
    assert!(jit.is_compiled(write_u32));
    assert!(jit.is_compiled(read_f64));
    assert!(jit.is_compiled(write_f64));
    let mut heap = zun_gc::Heap::new();
    let buffer = heap.alloc_buffer(32);
    let u32_view = heap.alloc_typed_array(buffer, 0, 2, zun_core::TypedElement::U32).expect("u32 view");
    let f64_view = heap.alloc_typed_array(buffer, 16, 2, zun_core::TypedElement::F64).expect("f64 view");
    assert!(matches!(jit.call_packed_with_heap(write_u32, &[u32_view.addr() as u64, 1, 0xdead_beef], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(write_f64, &[f64_view.addr() as u64, 1, 3.25f64.to_bits()], &mut heap), Ok(1)));
    assert!(matches!(jit.call_packed_with_heap(read_u32, &[u32_view.addr() as u64, 1], &mut heap), Ok(0xdead_beef)));
    assert!(matches!(jit.call_packed_with_heap(read_f64, &[f64_view.addr() as u64, 1], &mut heap), Ok(value) if f64::from_bits(value) == 3.25));
}

