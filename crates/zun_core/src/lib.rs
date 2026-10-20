//! Frozen contracts shared by the Zun runtime and its standard-library intrinsics.
//!
//! This crate deliberately has no dependency on the interpreter, compiler, or
//! heap implementation. It is the small Rust equivalent of the runtime-owned
//! portion of `System.Private.CoreLib`.

#[path = "views.rs"]
mod views;
mod ordered_map;
mod collection_iterators;
mod arrays;
mod byte_buffer;
mod dynamic_object;
mod error_value;
mod date_value;
mod ids;
mod any_value;
mod function_handle;
mod symbols;

pub use ordered_map::OrderedMap;
pub use collection_iterators::{MapIterator, SetIterator};
pub use arrays::{ArrayIterator, ArrayStorage};
pub use byte_buffer::ByteBuffer;
pub use dynamic_object::DynamicObject;
pub use error_value::ErrorValue;
pub use date_value::DateValue;
pub use ids::{CoreTypeId, IntrinsicId};
pub use any_value::{Any, AnyTag};
pub use function_handle::FunctionHandle;
pub use symbols::{SymbolId, SymbolRegistry};
pub use views::ByteView;
pub use views::DataView;
pub use views::TypedElement;
pub use views::TypedArrayView;

/// Size of one raw object slot in bytes.
pub const SLOT_SIZE: usize = 8;

/// Number of slots reserved before an object's first field.
pub const OBJECT_HEADER_SLOTS: usize = 2;

/// Convert an object field slot to its byte offset in the frozen layout.
pub const fn field_offset(slot: usize) -> usize {
    (OBJECT_HEADER_SLOTS + slot) * SLOT_SIZE
}

/// Stable identifier for a runtime intrinsic.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_layout_offsets_are_stable() {
        assert_eq!(field_offset(0), 16);
        assert_eq!(field_offset(3), 40);
    }

    #[test]
    fn function_handle_keeps_code_and_environment_identity() {
        let plain = FunctionHandle::new(7, None);
        assert_eq!(plain.method_rid(), 7);
        assert_eq!(plain.environment(), None);

        let closure = FunctionHandle::new(9, Some(0x1000));
        assert_eq!(closure.method_rid(), 9);
        assert_eq!(closure.environment(), Some(0x1000));
        assert_eq!(FunctionHandle::new(9, Some(0x1000)), closure);
    }

    #[test]
    fn intrinsic_ids_are_positional() {
        assert_eq!(IntrinsicId::new(7).get(), 7);
    }

    #[test]
    fn core_type_ids_are_stable() {
        assert_eq!(CoreTypeId::Object.get(), 0);
        assert_eq!(CoreTypeId::Function.get(), 5);
        assert_eq!(CoreTypeId::new(2), Some(CoreTypeId::Number));
        assert_eq!(CoreTypeId::new(6), None);
    }

    #[test]
    fn any_roundtrips_each_dynamic_tag() {
        assert_eq!(Any::int(-7).as_int(), Some(-7));
        assert_eq!(Any::float(3.5).as_float(), Some(3.5));
        assert_eq!(Any::bool(true).as_bool(), Some(true));
        assert_eq!(Any::object(0x1234).as_object(), Some(0x1234));
        assert_eq!(Any::string(9).as_string(), Some(9));
        assert_eq!(Any::int(1).as_float(), None);
    }

    #[test]
    fn symbols_are_unique_but_global_keys_are_canonical() {
        let mut symbols = SymbolRegistry::new();
        let first = symbols.create(Some("iterator"));
        let second = symbols.create(Some("iterator"));
        let global_a = symbols.for_key("iterator");
        let global_b = symbols.for_key("iterator");
        assert_ne!(first, second);
        assert_eq!(global_a, global_b);
        assert_eq!(symbols.description(first), Some("iterator"));
        assert_eq!(symbols.len(), 3);
    }

    #[test]
    fn iterator_symbol_is_canonical_within_an_isolate() {
        let mut symbols = SymbolRegistry::new();
        let first = symbols.iterator();
        let second = symbols.iterator();
        assert_eq!(first, second);
        assert_eq!(symbols.description(first), Some("Symbol.iterator"));
    }

    #[test]
    fn dynamic_object_stores_and_replaces_properties() {
        let mut object = DynamicObject::new();
        assert!(object.is_empty());
        assert_eq!(object.set("answer", Any::int(41)), None);
        assert_eq!(object.get("answer").and_then(Any::as_int), Some(41));
        assert_eq!(
            object.set("answer", Any::int(42)).and_then(Any::as_int),
            Some(41)
        );
        assert!(object.contains("answer"));
        assert_eq!(object.remove("answer").and_then(Any::as_int), Some(42));
        assert_eq!(object.len(), 0);
    }

    #[test]
    fn dynamic_object_keeps_symbol_properties_separate_from_string_properties() {
        let mut symbols = SymbolRegistry::new();
        let iterator = symbols.iterator();
        let mut object = DynamicObject::new();
        object.set("Symbol.iterator", Any::int(1));
        object.set_symbol(iterator, Any::int(2));

        assert_eq!(object.get("Symbol.iterator").and_then(Any::as_int), Some(1));
        assert_eq!(object.get_symbol(iterator).and_then(Any::as_int), Some(2));
        assert_eq!(object.len(), 1);
    }

    #[test]
    fn dynamic_object_stores_callable_symbol_properties() {
        let mut symbols = SymbolRegistry::new();
        let iterator = symbols.iterator();
        let mut object = DynamicObject::new();
        let function = FunctionHandle::new(17, Some(0x2000));

        assert_eq!(object.set_symbol_function(iterator, function), None);
        assert_eq!(object.get_symbol_function(iterator), Some(function));
        assert_eq!(object.object_references().collect::<Vec<_>>(), vec![0x2000]);
    }

    #[test]
    fn array_storage_is_contiguous_and_bounds_checked() {
        let mut array = ArrayStorage::with_length(2);
        assert_eq!(array.len(), 2);
        assert_eq!(array.get(0).and_then(Any::as_int), Some(0));
        assert_eq!(array.set(1, Any::int(7)).and_then(Any::as_int), Some(0));
        assert_eq!(array.get(1).and_then(Any::as_int), Some(7));
        assert_eq!(array.get(2), None);
        assert_eq!(array.set(2, Any::int(9)), None);
        assert_eq!(array.push(Any::int(11)), 2);
        assert_eq!(array.pop().and_then(Any::as_int), Some(11));
        array.resize(4);
        assert_eq!(array.len(), 4);
        assert_eq!(array.iter().count(), 4);
        array.resize(1);
        assert_eq!(array.len(), 1);
        assert!(array.insert(1, Any::int(13)));
        assert_eq!(array.remove(1).and_then(Any::as_int), Some(13));
        assert!(!array.insert(3, Any::int(14)));
    }

    #[test]
    fn array_iterator_preserves_index_order_and_end_state() {
        let mut array = ArrayStorage::with_length(3);
        array.set(0, Any::int(10));
        array.set(1, Any::int(20));
        array.set(2, Any::int(30));
        let mut iterator = ArrayIterator::new();
        assert_eq!(iterator.next(&array).and_then(Any::as_int), Some(10));
        assert_eq!(iterator.next(&array).and_then(Any::as_int), Some(20));
        assert_eq!(iterator.next(&array).and_then(Any::as_int), Some(30));
        assert!(iterator.is_done(&array));
        assert_eq!(iterator.next(&array), None);
    }

    #[test]
    fn byte_buffer_is_contiguous_and_bounds_checked() {
        let mut buffer = ByteBuffer::new(3);
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.get(0), Some(0));
        assert!(buffer.set(1, 0xab));
        assert_eq!(buffer.get(1), Some(0xab));
        assert!(!buffer.set(3, 1));
        assert_eq!(buffer.get(3), None);
        assert_eq!(buffer.as_slice(), &[0, 0xab, 0]);
        let mut typed = ByteBuffer::new(16);
        assert!(typed.write_u16(0, 0x1234));
        assert_eq!(typed.read_u16(0), Some(0x1234));
        assert!(typed.write_u32(2, 0xdead_beef));
        assert_eq!(typed.read_u32(2), Some(0xdead_beef));
        assert!(typed.write_f64(8, 3.5));
        assert_eq!(typed.read_f64(8), Some(3.5));
        assert!(!typed.write_u32(14, 1));
        let date = DateValue::from_millis(1_700_000_000_123);
        assert!(date.is_valid());
        assert_eq!(date.millis(), Some(1_700_000_000_123));
        assert_eq!(
            date.compare(DateValue::from_millis(1_700_000_000_124)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(DateValue::from_millis(0).utc_date(), Some((1970, 1, 1)));
        assert_eq!(DateValue::from_millis(0).weekday(), Some(4));
        assert_eq!(DateValue::from_millis(0).ordinal(), Some(1));
        assert_eq!(
            DateValue::from_millis(1_700_000_000_000).utc_date(),
            Some((2023, 11, 14))
        );
        assert_eq!(
            DateValue::from_millis(1_700_000_000_000).ordinal(),
            Some(318)
        );
        assert_eq!(
            DateValue::from_millis(3_723_004).utc_time(),
            Some((1, 2, 3, 4))
        );
        assert_eq!(
            DateValue::from_millis(-1).utc_time(),
            Some((23, 59, 59, 999))
        );
        assert_eq!(
            DateValue::from_millis(3_723_004).iso_datetime().as_deref(),
            Some("1970-01-01T01:02:03.004Z")
        );
        assert_eq!(
            DateValue::parse_iso_datetime("1970-01-01T01:02:03.004Z")
                .and_then(DateValue::iso_datetime)
                .as_deref(),
            Some("1970-01-01T01:02:03.004Z")
        );
        assert!(DateValue::parse_iso_datetime("1970-01-01T24:00:00.000Z").is_none());
        assert_eq!(
            DateValue::from_millis(0).iso_date().as_deref(),
            Some("1970-01-01")
        );
        assert_eq!(
            DateValue::parse_iso_date("2023-11-14")
                .and_then(DateValue::iso_date)
                .as_deref(),
            Some("2023-11-14")
        );
        assert!(DateValue::parse_iso_date("2023-02-29").is_none());
        assert!(DateValue::parse_iso_date("bad-date").is_none());
        assert_eq!(DateValue::invalid().compare(date), None);
        let view = ByteView::new(typed.len(), 2, 8).unwrap();
        assert_eq!(view.read_u32(&typed, 0), Some(0xdead_beef));
        assert!(view.write_u32(&mut typed, 4, 7));
        assert_eq!(typed.read_u32(6), Some(7));
        assert!(!view.write_u32(&mut typed, 5, 8));
        assert!(ByteView::new(typed.len(), 15, 2).is_none());
        let array = TypedArrayView::new(typed.len(), 4, 8, TypedElement::U32).unwrap();
        assert_eq!(array.len(), 2);
        assert!(array.write_u32(&mut typed, 1, 0xcafe));
        assert_eq!(array.read_u32(&typed, 1), Some(0xcafe));
        assert!(!array.write_u32(&mut typed, 2, 1));
        assert!(TypedArrayView::new(typed.len(), 1, 4, TypedElement::U32).is_none());
        assert!(TypedArrayView::new(typed.len(), 0, 6, TypedElement::U32).is_none());
        let bytes = TypedArrayView::new(typed.len(), 0, 4, TypedElement::U8).unwrap();
        assert!(bytes.write_u8(&mut typed, 0, 0x7f));
        assert_eq!(bytes.read_u8(&typed, 0), Some(0x7f));
        let shorts = TypedArrayView::new(typed.len(), 0, 4, TypedElement::U16).unwrap();
        assert!(shorts.write_u16(&mut typed, 1, 0xbeef));
        assert_eq!(shorts.read_u16(&typed, 1), Some(0xbeef));
        let floats = TypedArrayView::new(typed.len(), 8, 8, TypedElement::F64).unwrap();
        assert!(floats.write_f64(&mut typed, 0, 1.25));
        assert_eq!(floats.read_f64(&typed, 0), Some(1.25));
        let data = DataView::new(typed.len(), 1, 10).unwrap();
        assert!(data.set_u16(&mut typed, 1, 0x1234, true));
        assert_eq!(data.get_u16(&typed, 1, true), Some(0x1234));
        assert_eq!(data.get_u16(&typed, 1, false), Some(0x3412));
        assert!(data.set_u32(&mut typed, 3, 0xdead_beef, false));
        assert_eq!(data.get_u32(&typed, 3, false), Some(0xdead_beef));
        assert!(data.set_f64(&mut typed, 2, -3.5, true));
        assert_eq!(data.get_f64(&typed, 2, true), Some(-3.5));
        assert!(!data.set_u32(&mut typed, 7, 1, true));
    }

    #[test]
    fn ordered_map_preserves_insertion_order_and_updates_in_place() {
        let mut map = OrderedMap::new();
        let key = |candidate: Any, wanted: i64| candidate.as_int() == Some(wanted);
        assert_eq!(
            map.insert(Any::int(2), Any::int(20), 2, |k| key(k, 2)),
            None
        );
        assert_eq!(
            map.insert(Any::int(1), Any::int(10), 1, |k| key(k, 1)),
            None
        );
        assert_eq!(
            map.insert(Any::int(2), Any::int(22), 2, |k| key(k, 2))
                .and_then(Any::as_int),
            Some(20)
        );
        assert_eq!(map.get(2, |k| key(k, 2)).and_then(Any::as_int), Some(22));
        let values: Vec<i64> = map.iter().filter_map(|(_, value)| value.as_int()).collect();
        assert_eq!(values, vec![22, 10]);
        assert_eq!(
            map.remove(1, |k| key(k, 1))
                .map(|(_, value)| value.as_int()),
            Some(Some(10))
        );
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn ordered_map_compacts_tombstones_without_changing_order() {
        let mut map = OrderedMap::new();
        for key in 0..12 {
            map.insert(Any::int(key), Any::int(key * 10), key as u64, |candidate| {
                candidate.as_int() == Some(key)
            });
        }
        for key in 0..10 {
            assert!(map
                .remove(key as u64, |candidate| candidate.as_int() == Some(key))
                .is_some());
        }
        map.insert(Any::int(12), Any::int(120), 12, |candidate| {
            candidate.as_int() == Some(12)
        });
        let keys: Vec<i64> = map.iter().filter_map(|(key, _)| key.as_int()).collect();
        assert_eq!(keys, vec![10, 11, 12]);
    }

    #[test]
    fn map_and_set_iterators_follow_live_insertion_order() {
        let mut map = OrderedMap::new();
        for value in 1..=3 {
            map.insert(Any::int(value), Any::int(value * 10), value as u64, |key| {
                key.as_int() == Some(value)
            });
        }
        let mut map_iterator = MapIterator::new();
        assert_eq!(
            map_iterator.next(&map).map(|(_, value)| value.as_int()),
            Some(Some(10))
        );
        assert_eq!(
            map_iterator.next(&map).map(|(_, value)| value.as_int()),
            Some(Some(20))
        );

        let mut set_iterator = SetIterator::new();
        assert_eq!(
            set_iterator.next(&map).and_then(|value| value.as_int()),
            Some(1)
        );
        assert_eq!(
            set_iterator.next(&map).and_then(|value| value.as_int()),
            Some(2)
        );
        assert_eq!(
            set_iterator.next(&map).and_then(|value| value.as_int()),
            Some(3)
        );
        assert_eq!(set_iterator.next(&map), None);
    }
}
