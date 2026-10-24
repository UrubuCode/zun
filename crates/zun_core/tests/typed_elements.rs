//! The typed-array element kinds, at the level below the language.
//!
//! These check the three things the generated accessors are responsible for:
//! each kind stores the bit pattern JS stores, a view refuses to be read as
//! another kind, and the window bounds are the view's and not the buffer's.

use zun_core::{ByteBuffer, TypedArrayView, TypedElement};

fn window(element: TypedElement, elements: usize) -> (ByteBuffer, TypedArrayView) {
    let buffer = ByteBuffer::new(64);
    let view = TypedArrayView::new(64, 0, elements, element).expect("view fits");
    (buffer, view)
}

#[test]
fn the_table_has_every_kind_javascript_has() {
    // The independent oracle: the eleven constructors ECMAScript defines,
    // written out rather than read back from the table under test.
    let mut names: Vec<&str> = TypedElement::ALL.iter().map(|kind| kind.js_name()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "BigInt64Array",
            "BigUint64Array",
            "Float32Array",
            "Float64Array",
            "Int16Array",
            "Int32Array",
            "Int8Array",
            "Uint16Array",
            "Uint32Array",
            "Uint8Array",
            "Uint8ClampedArray",
        ]
    );
}

#[test]
fn element_widths_match_the_javascript_ones() {
    for kind in TypedElement::ALL {
        let expected = match kind.js_name() {
            "Int8Array" | "Uint8Array" | "Uint8ClampedArray" => 1,
            "Int16Array" | "Uint16Array" => 2,
            "Int32Array" | "Uint32Array" | "Float32Array" => 4,
            "Float64Array" | "BigInt64Array" | "BigUint64Array" => 8,
            other => panic!("unlisted kind {other}"),
        };
        assert_eq!(kind.size(), expected, "{}", kind.js_name());
    }
}

#[test]
fn signed_kinds_keep_their_sign() {
    // The whole point of the signed kinds: 0xFF is -1, not 255.
    let (mut buffer, view) = window(TypedElement::I8, 4);
    assert!(view.write_i8(&mut buffer, 0, -1));
    assert_eq!(view.read_i8(&buffer, 0), Some(-1));
    assert_eq!(buffer.get(0), Some(0xFF));

    let (mut buffer, view) = window(TypedElement::I16, 4);
    assert!(view.write_i16(&mut buffer, 1, -2));
    assert_eq!(view.read_i16(&buffer, 1), Some(-2));
    // Little-endian, second element: bytes 2 and 3.
    assert_eq!((buffer.get(2), buffer.get(3)), (Some(0xFE), Some(0xFF)));

    let (mut buffer, view) = window(TypedElement::I32, 4);
    assert!(view.write_i32(&mut buffer, 0, i32::MIN));
    assert_eq!(view.read_i32(&buffer, 0), Some(i32::MIN));

    let (mut buffer, view) = window(TypedElement::I64, 4);
    assert!(view.write_i64(&mut buffer, 3, i64::MIN));
    assert_eq!(view.read_i64(&buffer, 3), Some(i64::MIN));
}

#[test]
fn unsigned_kinds_hold_their_whole_range() {
    let (mut buffer, view) = window(TypedElement::U64, 4);
    assert!(view.write_u64(&mut buffer, 0, u64::MAX));
    assert_eq!(view.read_u64(&buffer, 0), Some(u64::MAX));

    let (mut buffer, view) = window(TypedElement::U32, 4);
    assert!(view.write_u32(&mut buffer, 0, u32::MAX));
    assert_eq!(view.read_u32(&buffer, 0), Some(u32::MAX));
}

#[test]
fn float32_stores_single_precision_not_double() {
    let (mut buffer, view) = window(TypedElement::F32, 4);
    // 0.1 is not representable in either width; as `f32` it comes back as
    // the `f32` approximation, which differs from the `f64` one.
    assert!(view.write_f32(&mut buffer, 0, 0.1));
    let read = view.read_f32(&buffer, 0).expect("in range");
    assert_eq!(read, 0.1f32);
    assert_ne!(f64::from(read), 0.1f64);
    // And the stored bytes are the four of `0.1f32`.
    for (index, byte) in 0.1f32.to_le_bytes().into_iter().enumerate() {
        assert_eq!(buffer.get(index), Some(byte));
    }
}

#[test]
fn the_clamped_kind_clamps_where_the_plain_one_wraps() {
    let (mut clamped_buffer, clamped) = window(TypedElement::U8Clamped, 8);
    let (mut plain_buffer, plain) = window(TypedElement::U8, 8);

    // Above the range: clamped saturates, plain wraps (300 - 256 = 44).
    assert!(clamped.write_u8_clamped(&mut clamped_buffer, 0, 300.0));
    assert_eq!(clamped.read_u8_clamped(&clamped_buffer, 0), Some(255));
    assert!(plain.write_u8(&mut plain_buffer, 0, 300u32 as u8));
    assert_eq!(plain.read_u8(&plain_buffer, 0), Some(44));

    // Below the range, and NaN.
    assert!(clamped.write_u8_clamped(&mut clamped_buffer, 1, -5.0));
    assert_eq!(clamped.read_u8_clamped(&clamped_buffer, 1), Some(0));
    assert!(clamped.write_u8_clamped(&mut clamped_buffer, 2, f64::NAN));
    assert_eq!(clamped.read_u8_clamped(&clamped_buffer, 2), Some(0));
}

#[test]
fn the_clamped_kind_rounds_halves_to_even() {
    // ToUint8Clamp rounds .5 to the nearest even integer, so 0.5 is 0 and
    // 1.5 is 2 — not what `as u8` or a naive `round()` produce.
    let (mut buffer, view) = window(TypedElement::U8Clamped, 8);
    for (index, (written, expected)) in
        [(0.5, 0u8), (1.5, 2), (2.5, 2), (3.5, 4), (1.4, 1), (1.6, 2)]
            .into_iter()
            .enumerate()
    {
        assert!(view.write_u8_clamped(&mut buffer, index, written));
        assert_eq!(
            view.read_u8_clamped(&buffer, index),
            Some(expected),
            "{written} should clamp to {expected}"
        );
    }
}

#[test]
fn a_view_refuses_to_be_read_as_another_kind() {
    let (mut buffer, view) = window(TypedElement::I32, 4);
    assert!(view.write_i32(&mut buffer, 0, 7));
    assert_eq!(view.read_u32(&buffer, 0), None);
    assert_eq!(view.read_f32(&buffer, 0), None);
    assert!(!view.write_u32(&mut buffer, 0, 7));
    // And the refused write left the element alone.
    assert_eq!(view.read_i32(&buffer, 0), Some(7));
}

#[test]
fn the_window_bounds_are_the_views_not_the_buffers() {
    // Four `i32`s at offset 16 of a 64-byte buffer: index 4 is still inside
    // the buffer but outside the view, and must not be reachable.
    let mut buffer = ByteBuffer::new(64);
    let view = TypedArrayView::new(64, 16, 4, TypedElement::I32).expect("view fits");
    assert_eq!(view.len(), 4);
    assert_eq!(view.byte_len(), 16);
    assert_eq!(view.read_i32(&buffer, 3), Some(0));
    assert_eq!(view.read_i32(&buffer, 4), None);
    assert!(!view.write_i32(&mut buffer, 4, 1));
}

#[test]
fn every_kind_is_readable_through_exactly_its_own_accessor() {
    // A sweep, so a kind added to the table without working accessors fails
    // here rather than in whatever program first uses it.
    for kind in TypedElement::ALL {
        let buffer = ByteBuffer::new(64);
        let view = TypedArrayView::new(64, 0, 2, *kind).expect("view fits");
        assert_eq!(view.len(), 2, "{}", kind.js_name());
        assert_eq!(view.byte_len(), 2 * kind.size(), "{}", kind.js_name());
        let readable = [
            view.read_i8(&buffer, 0).is_some(),
            view.read_u8(&buffer, 0).is_some(),
            view.read_u8_clamped(&buffer, 0).is_some(),
            view.read_i16(&buffer, 0).is_some(),
            view.read_u16(&buffer, 0).is_some(),
            view.read_i32(&buffer, 0).is_some(),
            view.read_u32(&buffer, 0).is_some(),
            view.read_f32(&buffer, 0).is_some(),
            view.read_f64(&buffer, 0).is_some(),
            view.read_i64(&buffer, 0).is_some(),
            view.read_u64(&buffer, 0).is_some(),
        ];
        assert_eq!(
            readable.iter().filter(|readable| **readable).count(),
            1,
            "{} is readable through the wrong number of accessors",
            kind.js_name()
        );
    }
}
