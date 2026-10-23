//! `Heap::buffer_slice` — the floor under `ArrayBuffer.prototype.slice`.
//!
//! The property that matters is that it *copies*: JS slices an ArrayBuffer
//! into an independent buffer, so a write to one must not be visible in the
//! other. An implementation that aliased would pass any length-only test.

use zun_gc::Heap;

/// A buffer of `len` bytes holding 0, 1, 2, ...
fn filled(heap: &mut Heap, len: usize) -> zun_gc::Obj {
    let buffer = heap.alloc_buffer(len);
    for index in 0..len {
        heap.buffer_set(buffer, index, index as u8);
    }
    buffer
}

fn bytes(heap: &Heap, buffer: zun_gc::Obj) -> Vec<u8> {
    (0..heap.buffer_len(buffer).unwrap())
        .map(|index| heap.buffer_get(buffer, index).unwrap())
        .collect()
}

#[test]
fn slice_copies_the_requested_range() {
    let mut heap = Heap::new();
    let source = filled(&mut heap, 8);
    let copy = heap.buffer_slice(source, 2, 6).expect("slice");
    assert_eq!(bytes(&heap, copy), vec![2, 3, 4, 5]);
}

#[test]
fn the_slice_is_independent_of_its_source() {
    // The point of copying rather than aliasing.
    let mut heap = Heap::new();
    let source = filled(&mut heap, 4);
    let copy = heap.buffer_slice(source, 0, 4).expect("slice");

    heap.buffer_set(copy, 0, 99);
    assert_eq!(heap.buffer_get(source, 0), Some(0), "source was modified");

    heap.buffer_set(source, 1, 77);
    assert_eq!(heap.buffer_get(copy, 1), Some(1), "copy was modified");
}

#[test]
fn a_whole_slice_equals_the_source() {
    let mut heap = Heap::new();
    let source = filled(&mut heap, 5);
    let copy = heap.buffer_slice(source, 0, 5).expect("slice");
    assert_eq!(bytes(&heap, copy), bytes(&heap, source));
}

#[test]
fn an_inverted_range_is_empty_rather_than_an_error() {
    let mut heap = Heap::new();
    let source = filled(&mut heap, 8);
    let copy = heap.buffer_slice(source, 6, 2).expect("slice");
    assert_eq!(heap.buffer_len(copy), Some(0));
}

#[test]
fn a_range_past_the_end_clamps() {
    let mut heap = Heap::new();
    let source = filled(&mut heap, 8);
    let copy = heap.buffer_slice(source, 4, 99).expect("slice");
    assert_eq!(bytes(&heap, copy), vec![4, 5, 6, 7]);

    let wholly_past = heap.buffer_slice(source, 50, 99).expect("slice");
    assert_eq!(heap.buffer_len(wholly_past), Some(0));
}

#[test]
fn slicing_an_empty_buffer_yields_an_empty_buffer() {
    let mut heap = Heap::new();
    let source = heap.alloc_buffer(0);
    let copy = heap.buffer_slice(source, 0, 4).expect("slice");
    assert_eq!(heap.buffer_len(copy), Some(0));
}

#[test]
fn the_source_survives_the_allocation_the_slice_performs() {
    // `buffer_slice` allocates, and allocation can move objects. Reading the
    // source afterwards catches an implementation that held an address across
    // that call.
    let mut heap = Heap::new();
    let source = filled(&mut heap, 16);
    let mut copies = Vec::new();
    for _ in 0..32 {
        copies.push(heap.buffer_slice(source, 0, 16).expect("slice"));
    }
    assert_eq!(bytes(&heap, source), (0..16).collect::<Vec<u8>>());
    for copy in copies {
        assert_eq!(bytes(&heap, copy), (0..16).collect::<Vec<u8>>());
    }
}
