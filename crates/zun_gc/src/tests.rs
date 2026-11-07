use super::*;

#[test]
fn alloc_sets_type_and_zeroes_fields() {
    let mut heap = Heap::new();
    let o = heap.alloc(7, 3);
    assert_eq!(o.type_rid(), 7);
    assert_ne!(o.identity_hash(), 0);
    assert_eq!(o.field(0), 0);
    assert_eq!(o.field(1), 0);
    assert_eq!(o.field(2), 0);
}

#[test]
fn function_environment_is_kept_as_a_gc_root() {
    let mut heap = Heap::new();
    let environment = heap.alloc(3, 0);
    let garbage = heap.alloc(4, 0);
    let function = zun_core::FunctionHandle::new(11, Some(environment.addr() as usize));

    let freed = unsafe { heap.collect_with_function_roots(&[], &[function], &|_| Vec::new()) };
    assert_eq!(freed, 1);
    assert_eq!(heap.live_count(), 1);
    assert!(heap.by_addr.contains_key(&(environment.addr() as usize)));
    assert!(!heap.by_addr.contains_key(&(garbage.addr() as usize)));
}

#[test]
fn promise_result_keeps_managed_object_alive() {
    let mut heap = Heap::new();
    let promise = heap.alloc_promise();
    let result = heap.alloc(9, 0);
    assert_eq!(
        heap.promise_resolve(promise, zun_core::Any::object(result.addr() as usize)),
        Some(true)
    );
    assert_eq!(unsafe { heap.collect(&[promise], &|_| Vec::new()) }, 0);
    assert_eq!(
        heap.promise_state(promise),
        Some(zun_core::ManagedPromiseState::Resolved)
    );
    assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 2);
}

#[test]
fn map_uses_same_value_zero_and_preserves_entries() {
    let mut heap = Heap::new();
    let map = heap.alloc_map();
    let one = heap.intern_string(Rc::from("one"));
    assert_eq!(
        heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::string(one)),
        None
    );
    assert_eq!(
        heap.map_insert(
            map,
            zun_core::Any::float(f64::NAN),
            zun_core::Any::bool(true)
        ),
        None
    );
    assert_eq!(
        heap.map_get(map, zun_core::Any::int(1))
            .and_then(|v| v.as_string())
            .and_then(|h| heap.try_string(h))
            .map(|s| s.as_ref()),
        Some("one")
    );
    assert_eq!(
        heap.map_get(map, zun_core::Any::float(f64::NAN))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(heap.map_len(map), Some(2));
    assert_eq!(
        heap.map_entries(map)
            .unwrap()
            .iter()
            .filter_map(|(key, _)| key.as_int())
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(heap.map_has(map, zun_core::Any::int(1)), Some(true));
    assert_eq!(heap.map_clear(map), Some(()));
    assert_eq!(heap.map_len(map), Some(0));
    assert_eq!(heap.map_has(map, zun_core::Any::int(1)), Some(false));
    let one = heap.intern_string(Rc::from("one-again"));
    heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::string(one));
    assert_eq!(
        heap.map_remove(map, zun_core::Any::int(1))
            .and_then(|(_, value)| value.as_string()),
        Some(one)
    );
    assert_eq!(heap.map_len(map), Some(0));
}

#[test]
fn map_traces_object_keys_and_values() {
    let mut heap = Heap::new();
    let map = heap.alloc_map();
    let key = heap.alloc(1, 0);
    let value = heap.alloc(2, 0);
    heap.map_insert(
        map,
        zun_core::Any::object(key.addr() as usize),
        zun_core::Any::object(value.addr() as usize),
    );
    let no_refs = |_: u32| Vec::<usize>::new();
    let freed = unsafe { heap.collect(&[map], &no_refs) };
    assert_eq!(freed, 0);
    assert_eq!(heap.live_count(), 3);
}

#[test]
fn set_has_same_value_zero_and_traces_object_keys() {
    let mut heap = Heap::new();
    let set = heap.alloc_set();
    let key = heap.alloc(1, 0);
    assert_eq!(
        heap.set_add(set, zun_core::Any::object(key.addr() as usize)),
        Some(true)
    );
    assert_eq!(
        heap.set_add(set, zun_core::Any::object(key.addr() as usize)),
        Some(false)
    );
    assert_eq!(
        heap.set_has(set, zun_core::Any::object(key.addr() as usize)),
        Some(true)
    );
    assert_eq!(heap.set_len(set), Some(1));
    assert_eq!(
        heap.set_values(set)
            .unwrap()
            .iter()
            .filter_map(|value| value.as_object())
            .count(),
        1
    );
    assert_eq!(heap.set_clear(set), Some(()));
    assert_eq!(heap.set_len(set), Some(0));
    assert_eq!(
        heap.set_remove(set, zun_core::Any::object(key.addr() as usize)),
        Some(false)
    );
    assert_eq!(
        heap.set_has(set, zun_core::Any::object(key.addr() as usize)),
        Some(false)
    );
}

#[test]
fn symbols_are_isolate_local_heap_state() {
    let mut first = Heap::new();
    let mut second = Heap::new();
    // Each isolate seeds the well-known symbols first; a `Symbol.for` key
    // then resolves to the same stable id in every isolate.
    let well_known = first.symbol_count();
    assert_eq!(second.symbol_count(), well_known);
    let first_symbol = first.symbol_for("iterator");
    let second_symbol = second.symbol_for("iterator");
    assert_eq!(first_symbol, second_symbol);
    assert_eq!(first.symbol_description(first_symbol), Some("iterator"));
    assert_eq!(first.symbol_key_for(first_symbol), Some("iterator"));
    assert_eq!(first.symbol_count(), well_known + 1);
    assert_eq!(second.symbol_count(), well_known + 1);
    let unique = first.symbol_create(Some("iterator"));
    assert_ne!(unique, first_symbol);
    assert_eq!(first.symbol_key_for(unique), None);
}

#[test]
fn weak_reference_does_not_keep_target_alive() {
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    let weak = heap.weak_ref_new(target);
    assert_eq!(heap.weak_ref_get(weak), Some(target));
    let freed = unsafe { heap.collect(&[], &|_| Vec::new()) };
    assert_eq!(freed, 1);
    assert_eq!(heap.weak_ref_get(weak), None);
}

#[test]
fn weak_ref_object_clears_once_its_target_is_collected() {
    // A managed WeakRef object dereferences to its target while the target
    // is rooted, and to nothing once a collection reclaims it.
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    let weak_ref = heap.alloc_weak_ref(target);
    assert_eq!(
        unsafe { heap.collect(&[weak_ref, target], &|_| Vec::new()) },
        0
    );
    assert_eq!(heap.weak_ref_get_object(weak_ref), Some(target));
    // Drop the target root; only the WeakRef object stays rooted.
    unsafe { heap.collect(&[weak_ref], &|_| Vec::new()) };
    assert_eq!(heap.weak_ref_get_object(weak_ref), None);
}

#[test]
fn weak_reference_observes_a_live_root() {
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    let weak = heap.weak_ref_new(target);
    assert_eq!(unsafe { heap.collect(&[target], &|_| Vec::new()) }, 0);
    assert_eq!(heap.weak_ref_get(weak), Some(target));
}

#[test]
fn weak_map_promotes_value_only_when_key_is_live() {
    let mut heap = Heap::new();
    let weak_map = heap.weak_map_new();
    let key = heap.alloc(1, 0);
    let value = heap.alloc(2, 0);
    assert!(heap.weak_map_set(weak_map, key, value));
    assert_eq!(unsafe { heap.collect(&[key], &|_| Vec::new()) }, 0);
    assert_eq!(heap.weak_map_get(weak_map), Some((key, value)));

    let key2 = heap.alloc(3, 0);
    let value2 = heap.alloc(4, 0);
    assert!(heap.weak_map_set(weak_map, key2, value2));
    assert_eq!(unsafe { heap.collect(&[key], &|_| Vec::new()) }, 2);
    assert_eq!(heap.weak_map_get(weak_map), Some((key, value)));
}

#[test]
fn weak_map_promotion_traces_dynamic_value_children() {
    let mut heap = Heap::new();
    let weak_map = heap.weak_map_new();
    let key = heap.alloc(1, 0);
    let value = heap.alloc_dynamic();
    let child = heap.alloc(2, 0);
    heap.dynamic_set(value, "child", zun_core::Any::object(child.addr() as usize));
    heap.weak_map_set(weak_map, key, value);
    assert_eq!(unsafe { heap.collect(&[key], &|_| Vec::new()) }, 0);
    assert_eq!(heap.live_count(), 3);
}

#[test]
fn weak_map_ephemerons_survive_repeated_collections_only_for_live_keys() {
    let mut heap = Heap::new();
    let mut handles = Vec::new();
    let mut live_keys = Vec::new();
    for index in 0..64u32 {
        let weak_map = heap.weak_map_new();
        let key = heap.alloc(index + 1, 0);
        let value = heap.alloc(index + 100, 0);
        assert!(heap.weak_map_set(weak_map, key, value));
        if index % 2 == 0 {
            live_keys.push(key);
        }
        handles.push((weak_map, key, value));
    }

    let no_refs = |_: u32| Vec::<usize>::new();
    for _ in 0..4 {
        unsafe { heap.collect(&live_keys, &no_refs) };
    }
    for (index, (weak_map, key, value)) in handles.into_iter().enumerate() {
        if index % 2 == 0 {
            assert_eq!(heap.weak_map_get(weak_map), Some((key, value)));
        } else {
            assert_eq!(heap.weak_map_get(weak_map), None);
        }
    }
}

#[test]
fn weak_map_stores_multiple_entries_until_collection() {
    let mut heap = Heap::new();
    let weak_map = heap.weak_map_new();
    let key_a = heap.alloc(1, 0);
    let value_a = heap.alloc(2, 0);
    let key_b = heap.alloc(3, 0);
    let value_b = heap.alloc(4, 0);
    assert!(heap.weak_map_set(weak_map, key_a, value_a));
    assert!(heap.weak_map_set(weak_map, key_b, value_b));
    let entries = heap.weak_map_entries(weak_map);
    assert_eq!(entries.len(), 2);
    assert!(entries.contains(&(key_a, value_a)));
    assert!(entries.contains(&(key_b, value_b)));
}

#[test]
fn weak_map_stress_cycles_with_collection_at_each_safepoint() {
    let mut heap = Heap::new();
    let weak_map = heap.weak_map_new();
    let mut live_keys = Vec::new();
    let mut entries = Vec::new();
    let ref_slots = |type_rid: u32| {
        if type_rid == 0 {
            Vec::new()
        } else {
            vec![0usize]
        }
    };

    for index in 0..32u32 {
        let key = heap.alloc(index + 1, 1);
        let value = heap.alloc(index + 100, 1);
        key.set_field(0, value.addr() as u64);
        value.set_field(0, key.addr() as u64);
        assert!(heap.weak_map_set(weak_map, key, value));
        if index % 2 == 0 {
            live_keys.push(key);
        }
        entries.push((key, value));
        unsafe { heap.collect(&live_keys, &ref_slots) };
    }

    let remaining = heap.weak_map_entries(weak_map);
    for (index, (key, value)) in entries.into_iter().enumerate() {
        if index % 2 == 0 {
            assert!(remaining.contains(&(key, value)));
        } else {
            assert!(!remaining.iter().any(|(candidate, _)| *candidate == key));
        }
    }
}

#[test]
fn weak_map_stress_cycles_survive_evacuating_minor_collection() {
    let mut heap = Heap::new();
    let weak_map = heap.alloc_weak_map();
    unsafe { heap.collect(&[weak_map], &|_| Vec::new()) };
    let weak_map_id = heap
        .weak_map_objects
        .get(&(weak_map.addr() as usize))
        .copied()
        .unwrap();

    let mut roots = vec![weak_map];
    let mut live_key_indices = Vec::new();
    let mut key_values = Vec::new();
    let ref_slots = |type_rid: u32| {
        if type_rid == 0 {
            Vec::new()
        } else {
            vec![0usize]
        }
    };

    for index in 0..32u32 {
        let key = heap.alloc(index + 1, 1);
        let value = heap.alloc(index + 100, 1);
        key.set_field(0, value.addr() as u64);
        value.set_field(0, key.addr() as u64);
        assert!(heap.weak_map_set(weak_map_id, key, value));
        if index % 2 == 0 {
            live_key_indices.push(index as usize);
            roots.push(key);
        }
        key_values.push((key, value));
    }

    unsafe {
        heap.collect_young_evacuating(&mut roots, &ref_slots, &mut |_| {});
    }

    let entries = heap.weak_map_entries(weak_map_id);
    assert_eq!(entries.len(), live_key_indices.len());
    for index in live_key_indices {
        let key = roots[1 + index / 2];
        assert!(entries.iter().any(|(candidate, _)| *candidate == key));
    }
    assert!(key_values.iter().enumerate().any(|(index, (key, _))| {
        index % 2 == 1 && !heap.by_addr.contains_key(&(key.addr() as usize))
    }));
}

#[test]
fn finalization_queues_token_after_target_is_swept() {
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    heap.finalization_register(target, 99);
    assert!(heap.finalization_drain().is_empty());
    assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 1);
    assert_eq!(heap.finalization_drain(), vec![99]);
    assert!(heap.finalization_drain().is_empty());
}

#[test]
fn finalization_queue_preserves_callback_routing_metadata() {
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    heap.finalization_register_with_callback(target, 99, 7);
    assert!(heap.finalization_drain_records().is_empty());
    unsafe { heap.collect(&[], &|_| Vec::new()) };
    assert_eq!(
        heap.finalization_drain_records(),
        vec![FinalizationRecord {
            held: zun_core::Any::int(99),
            callback_id: 7,
            callback: None
        }]
    );
}

#[test]
fn finalization_queue_preserves_closure_callback_identity() {
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    let environment = heap.alloc(2, 0);
    let callback = zun_core::FunctionHandle::new(17, Some(environment.addr() as usize));
    heap.finalization_register_with_function(target, 100, callback);
    unsafe { heap.collect(&[], &|_| Vec::new()) };
    let records = heap.finalization_drain_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].callback, Some(callback));
}

#[test]
fn finalization_delivers_an_object_held_value() {
    // An arbitrary (object) held value survives until delivery and is
    // carried on the drained record (NUCLEUS #16).
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    let held = heap.alloc(2, 0);
    let callback = zun_core::FunctionHandle::new(5, None);
    heap.finalization_register_with_held(
        target,
        zun_core::Any::object(held.addr() as usize),
        callback,
        0,
    );
    // The target has no root, so it is swept and the registration queued;
    // the held object must still be alive for the pending cleanup.
    unsafe { heap.collect(&[], &|_| Vec::new()) };
    assert!(heap.by_addr.contains_key(&(held.addr() as usize)));
    let records = heap.finalization_drain_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].held.as_object(), Some(held.addr() as usize));
}

#[test]
fn minor_collection_keeps_registered_callback_environment_alive() {
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    let environment = heap.alloc(2, 0);
    let callback = zun_core::FunctionHandle::new(17, Some(environment.addr() as usize));
    heap.finalization_register_with_function(target, 100, callback);
    unsafe { heap.collect(&[target], &|_| Vec::new()) };
    let garbage = heap.alloc(3, 0);

    assert_eq!(unsafe { heap.collect_young(&[target], &|_| Vec::new()) }, 1);
    assert!(heap.by_addr.contains_key(&(environment.addr() as usize)));
    assert!(!heap.by_addr.contains_key(&(garbage.addr() as usize)));
}

#[test]
fn finalization_unregister_cancels_before_target_is_swept() {
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    let id = heap.finalization_register(target, 99);
    assert!(heap.finalization_unregister(id));
    assert!(!heap.finalization_unregister(id));
    unsafe { heap.collect(&[], &|_| Vec::new()) };
    assert!(heap.finalization_drain().is_empty());
}

#[test]
fn finalization_unregister_rejects_already_queued_registration() {
    let mut heap = Heap::new();
    let target = heap.alloc(1, 0);
    let id = heap.finalization_register(target, 101);
    unsafe { heap.collect(&[], &|_| Vec::new()) };
    assert!(!heap.finalization_unregister(id));
    assert_eq!(heap.finalization_drain(), vec![101]);
}

#[test]
fn byte_buffers_roundtrip_through_the_heap() {
    let mut heap = Heap::new();
    let buffer = heap.alloc_buffer(4);
    assert_eq!(heap.buffer_len(buffer), Some(4));
    assert_eq!(heap.buffer_set(buffer, 2, 0xfe), Some(true));
    assert_eq!(heap.buffer_get(buffer, 2), Some(0xfe));
    assert_eq!(heap.buffer_set(buffer, 4, 1), Some(false));
    assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 1);
    assert_eq!(heap.buffer_len(buffer), None);
}

#[test]
fn typed_buffer_views_roundtrip_through_the_heap() {
    let mut heap = Heap::new();
    let buffer = heap.alloc_buffer(16);
    assert_eq!(heap.buffer_write_u32(buffer, 0, 0xfeed_beef), Some(true));
    assert_eq!(heap.buffer_read_u32(buffer, 0), Some(0xfeed_beef));
    assert_eq!(heap.buffer_write_f64(buffer, 8, 2.5), Some(true));
    assert_eq!(heap.buffer_read_f64(buffer, 8), Some(2.5));
    assert_eq!(heap.buffer_write_u32(buffer, 14, 1), Some(false));
    let view = heap.buffer_view(buffer, 4, 8).unwrap();
    assert_eq!(
        heap.buffer_view_write_u32(buffer, view, 0, 0x1234),
        Some(true)
    );
    assert_eq!(heap.buffer_view_read_u32(buffer, view, 0), Some(0x1234));
    assert_eq!(heap.buffer_view_write_u32(buffer, view, 5, 1), Some(false));
    let bytes = zun_core::TypedArrayView::new(16, 0, 4, zun_core::TypedElement::U8).unwrap();
    assert_eq!(heap.buffer_view_write_u8(buffer, bytes, 0, 7), Some(true));
    assert_eq!(heap.buffer_view_read_u8(buffer, bytes, 0), Some(7));
    let shorts = zun_core::TypedArrayView::new(16, 0, 4, zun_core::TypedElement::U16).unwrap();
    assert_eq!(
        heap.buffer_view_write_u16(buffer, shorts, 1, 0xbeef),
        Some(true)
    );
    assert_eq!(heap.buffer_view_read_u16(buffer, shorts, 1), Some(0xbeef));
    let floats = zun_core::TypedArrayView::new(16, 8, 1, zun_core::TypedElement::F64).unwrap();
    assert_eq!(
        heap.buffer_view_write_f64(buffer, floats, 0, 1.5),
        Some(true)
    );
    assert_eq!(heap.buffer_view_read_f64(buffer, floats, 0), Some(1.5));
    let data = heap.data_view(buffer, 1, 10).unwrap();
    assert_eq!(
        heap.data_view_write_u16(buffer, data, 1, 0x1234, true),
        Some(true)
    );
    assert_eq!(heap.data_view_read_u16(buffer, data, 1, true), Some(0x1234));
    assert_eq!(
        heap.data_view_read_u16(buffer, data, 1, false),
        Some(0x3412)
    );
    assert_eq!(
        heap.data_view_write_u32(buffer, data, 3, 0xdead_beef, false),
        Some(true)
    );
    assert_eq!(
        heap.data_view_read_u32(buffer, data, 3, false),
        Some(0xdead_beef)
    );
    assert_eq!(
        heap.data_view_write_f64(buffer, data, 2, -3.5, true),
        Some(true)
    );
    assert_eq!(heap.data_view_read_f64(buffer, data, 2, true), Some(-3.5));
    assert_eq!(
        heap.data_view_write_u32(buffer, data, 7, 1, true),
        Some(false)
    );
}

#[test]
fn date_values_roundtrip_and_invalid_dates_do_not_compare() {
    let mut heap = Heap::new();
    let earlier = heap.alloc_date(Some(100));
    let later = heap.alloc_date(Some(200));
    let invalid = heap.alloc_date(None);
    assert_eq!(heap.date_millis(earlier), Some(100));
    assert_eq!(
        heap.date_compare(earlier, later),
        Some(std::cmp::Ordering::Less)
    );
    assert_eq!(heap.date_utc_date(earlier), Some((1970, 1, 1)));
    assert_eq!(heap.date_utc_time(earlier), Some((0, 0, 0, 100)));
    assert_eq!(heap.date_weekday(earlier), Some(4));
    assert_eq!(heap.date_ordinal(earlier), Some(1));
    assert_eq!(heap.date_iso(earlier).as_deref(), Some("1970-01-01"));
    assert_eq!(
        heap.date_iso_datetime(earlier).as_deref(),
        Some("1970-01-01T00:00:00.100Z")
    );
    assert_eq!(heap.date_set_millis(earlier, 200), Some(()));
    assert_eq!(heap.date_millis(earlier), Some(200));
    assert_eq!(heap.date_invalidate(earlier), Some(()));
    assert_eq!(heap.date_millis(earlier), None);
    assert_eq!(heap.date_millis(invalid), None);
    assert_eq!(heap.date_is_valid(invalid), Some(false));
    let now = heap.alloc_date_now();
    assert_eq!(heap.date_is_valid(now), Some(true));
    assert!(heap.date_millis(now).is_some());
    assert_eq!(heap.date_compare(earlier, invalid), None);
}

#[test]
fn error_values_roundtrip_through_the_heap() {
    let mut heap = Heap::new();
    let error = heap.alloc_error("TypeError", "bad value");
    assert_eq!(heap.error_name(error), Some("TypeError"));
    assert_eq!(heap.error_message(error), Some("bad value"));
    assert_eq!(heap.error_set_name(error, "RangeError"), Some(()));
    assert_eq!(heap.error_name(error), Some("RangeError"));
    assert_eq!(heap.error_set_message(error, "fixed"), Some(()));
    assert_eq!(heap.error_message(error), Some("fixed"));
    assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 1);
    assert_eq!(heap.error_message(error), None);
}

#[test]
fn fields_read_back_what_was_written() {
    let mut heap = Heap::new();
    let o = heap.alloc(1, 2);
    o.set_field(0, 42);
    o.set_field(1, f64::to_bits(3.5));
    assert_eq!(o.field(0), 42);
    assert_eq!(f64::from_bits(o.field(1)), 3.5);
}

#[test]
fn addresses_stay_stable_across_many_allocations() {
    let mut heap = Heap::new();
    let first = heap.alloc(1, 4);
    first.set_field(0, 0xDEAD_BEEF);
    // Allocating many more objects (spanning several chunks) must not move
    // or corrupt the first object.
    let mut last = first;
    for i in 0..200_000u64 {
        last = heap.alloc(2, 4);
        last.set_field(0, i);
    }
    assert_eq!(first.field(0), 0xDEAD_BEEF);
    assert_eq!(first.type_rid(), 1);
    assert!(
        heap.chunk_count() > 1,
        "should have spanned multiple chunks"
    );
    assert_eq!(last.field(0), 199_999);
}

#[test]
fn nested_object_pointer_in_a_field() {
    let mut heap = Heap::new();
    let inner = heap.alloc(5, 1);
    inner.set_field(0, 99);
    let outer = heap.alloc(6, 1);
    outer.set_field(0, inner.addr() as u64);
    // Recover the inner object from the raw pointer stored in outer's field.
    let recovered = unsafe { Obj::from_addr(outer.field(0) as *mut u64) };
    assert_eq!(recovered.field(0), 99);
    assert_eq!(recovered.type_rid(), 5);
}

#[test]
fn dynamic_properties_roundtrip_through_the_heap() {
    let mut heap = Heap::new();
    let object = heap.alloc_dynamic();
    assert_eq!(object.type_rid(), 0);
    assert_eq!(
        heap.dynamic_set(object, "answer", zun_core::Any::int(42)),
        None
    );
    assert_eq!(
        heap.dynamic_get(object, "answer")
            .and_then(zun_core::Any::as_int),
        Some(42)
    );
    assert_eq!(
        heap.dynamic_remove(object, "answer")
            .and_then(zun_core::Any::as_int),
        Some(42)
    );
}

#[test]
fn symbol_properties_roundtrip_through_the_heap() {
    let mut heap = Heap::new();
    let object = heap.alloc_dynamic();
    let iterator = heap.symbol_for("Symbol.iterator");
    assert_eq!(
        heap.dynamic_set_symbol(object, iterator, zun_core::Any::int(7)),
        None
    );
    assert_eq!(
        heap.dynamic_get_symbol(object, iterator)
            .and_then(zun_core::Any::as_int),
        Some(7)
    );
}

#[test]
fn callable_symbol_properties_keep_closure_environment_live() {
    let mut heap = Heap::new();
    let object = heap.alloc_dynamic();
    let iterator = heap.symbol_for("Symbol.iterator");
    let function = zun_core::FunctionHandle::new(17, Some(0x2000));
    assert_eq!(
        heap.dynamic_set_symbol_function(object, iterator, function),
        None
    );
    assert_eq!(
        heap.dynamic_get_symbol_function(object, iterator),
        Some(function)
    );
}

#[test]
fn dynamic_object_references_are_traced_by_collection() {
    let mut heap = Heap::new();
    let child = heap.alloc(7, 0);
    let dynamic = heap.alloc_dynamic();
    heap.dynamic_set(
        dynamic,
        "child",
        zun_core::Any::object(child.addr() as usize),
    );
    let unreachable = heap.alloc(8, 0);
    let no_refs = |_: u32| Vec::<usize>::new();

    let freed = unsafe { heap.collect(&[dynamic], &no_refs) };
    assert_eq!(freed, 1);
    assert_eq!(heap.live_count(), 2);
    assert!(heap.dynamic_get(dynamic, "child").is_some());
    let _ = unreachable;
}

#[test]
fn array_storage_roundtrips_and_traces_object_elements() {
    let mut heap = Heap::new();
    let child = heap.alloc(7, 0);
    let array = heap.alloc_array(1);
    assert_eq!(heap.array_len(array), Some(1));
    assert_eq!(
        heap.array_set(array, 0, zun_core::Any::object(child.addr() as usize)),
        // A fresh array slot is a hole (sparse-array semantics), not an integer zero.
        Some(zun_core::Any::hole())
    );
    assert_eq!(
        heap.array_get(array, 0).and_then(zun_core::Any::as_object),
        Some(child.addr() as usize)
    );
    assert!(heap.array_insert(array, 1, zun_core::Any::int(9)));
    assert_eq!(
        heap.array_remove(array, 1).and_then(zun_core::Any::as_int),
        Some(9)
    );
    let unreachable = heap.alloc(8, 0);
    let no_refs = |_: u32| Vec::<usize>::new();
    let freed = unsafe { heap.collect(&[array], &no_refs) };
    assert_eq!(freed, 1);
    assert_eq!(heap.live_count(), 2);
    let _ = unreachable;
}

#[test]
fn collect_frees_unreachable_and_keeps_reachable() {
    let mut heap = Heap::new();
    // a has one reference field (slot 0) pointing at b; c is unreferenced.
    let b = heap.alloc(2, 0);
    let a = heap.alloc(1, 1);
    a.set_field(0, b.addr() as u64);
    let _c = heap.alloc(3, 0);
    assert_eq!(heap.live_count(), 3);

    // Type 1 has a reference in slot 0; types 2 and 3 have none.
    let ref_slots = |type_rid: u32| if type_rid == 1 { vec![0usize] } else { vec![] };
    // Root set is just `a`; `a` keeps `b` alive; `c` is garbage.
    let freed = unsafe { heap.collect(&[a], &ref_slots) };
    assert_eq!(freed, 1);
    assert_eq!(heap.live_count(), 2);
}

#[test]
fn full_collection_promotes_surviving_objects() {
    let mut heap = Heap::new();
    let keep = heap.alloc(7, 0);
    let garbage = heap.alloc(8, 0);
    assert_eq!(heap.generation_counts(), (2, 0));

    let freed = unsafe { heap.collect(&[keep], &|_| Vec::new()) };
    assert_eq!(freed, 1);
    assert_eq!(heap.generation_counts(), (0, 1));
    assert!(heap.by_addr.contains_key(&(keep.addr() as usize)));
    assert!(!heap.by_addr.contains_key(&(garbage.addr() as usize)));
}

#[test]
fn nursery_budget_tracks_bump_allocations_and_resets_after_minor_collection() {
    let mut heap = Heap::new();
    assert!(!heap.nursery_collection_due());
    let _object = heap.alloc(7, 2);
    assert!(!heap.nursery_collection_due());

    heap.nursery_budget = 32 * 1024;
    heap.nursery_bytes = heap.nursery_budget;
    assert!(heap.nursery_collection_due());
    unsafe { heap.collect_young(&[], &|_| Vec::new()) };
    assert!(!heap.nursery_collection_due());
}

#[test]
fn write_barrier_remembers_old_objects_pointing_to_young_objects() {
    let mut heap = Heap::new();
    let old = heap.alloc(7, 1);
    unsafe { heap.collect(&[old], &|_| Vec::new()) };
    let young = heap.alloc(8, 0);

    heap.store_field(old, 0, young.addr() as u64, true);
    assert_eq!(heap.remembered_set_len(), 1);

    heap.store_field(old, 0, 0, true);
    assert_eq!(heap.remembered_set_len(), 1);
}

#[test]
fn compaction_plan_reports_reclaimable_storage_without_moving_objects() {
    let mut heap = Heap::new();
    let live = heap.alloc(1, 2);
    let dead = heap.alloc(2, 3);
    let live_addr = live.addr();
    unsafe { heap.collect(&[live], &|_| Vec::new()) };

    let plan = heap.compaction_plan();
    assert_eq!(plan.live_objects, 1);
    assert_eq!(plan.reclaimable_objects, 1);
    assert_eq!(plan.live_slots, zun_core::OBJECT_HEADER_SLOTS + 2);
    assert_eq!(plan.reclaimable_slots, zun_core::OBJECT_HEADER_SLOTS + 3);
    assert_eq!(live.addr(), live_addr);
    assert!(!heap.by_addr.contains_key(&(dead.addr() as usize)));
}

#[test]
fn relocation_rewrites_reference_fields_without_moving_objects() {
    let mut heap = Heap::new();
    let owner = heap.alloc(1, 1);
    let owner_addr = owner.addr();
    let old_target = heap.alloc(2, 0);
    let new_target = heap.alloc(3, 0);
    heap.store_field(owner, 0, old_target.addr() as u64, true);

    let mut relocation = RelocationMap::new();
    relocation.insert(old_target, new_target);
    let rewritten = heap.rewrite_object_references(&relocation, &|type_rid| {
        if type_rid == 1 {
            vec![0]
        } else {
            vec![]
        }
    });

    assert_eq!(rewritten, 1);
    assert_eq!(owner.field(0), new_target.addr() as u64);
    assert_eq!(owner.addr(), owner_addr);
}

#[test]
fn relocation_coordinator_runs_heap_owned_passes() {
    let mut heap = Heap::new();
    let owner = heap.alloc(1, 1);
    let child = heap.alloc(2, 0);
    let new_owner = unsafe { Obj::from_addr(0x5000 as *mut u64) };
    let new_child = unsafe { Obj::from_addr(0x6000 as *mut u64) };
    heap.store_field(owner, 0, child.addr() as u64, true);
    let mut relocation = RelocationMap::new();
    relocation.insert(owner, new_owner);
    relocation.insert(child, new_child);

    assert_eq!(
        heap.rewrite_all_references(&relocation, &|type_rid| {
            if type_rid == 1 {
                vec![0]
            } else {
                vec![]
            }
        }),
        3
    );
    assert_eq!(heap.by_addr.get(&(new_owner.addr() as usize)), Some(&0));
    assert_eq!(heap.by_addr.get(&(new_child.addr() as usize)), Some(&1));
    assert_eq!(owner.field(0), new_child.addr() as u64);
}

#[test]
fn compaction_moves_live_objects_and_rewrites_interpreter_roots() {
    let mut heap = Heap::new();
    let owner = heap.alloc(1, 1);
    let child = heap.alloc(2, 0);
    let old_owner = owner.addr();
    let old_child = child.addr();
    heap.store_field(owner, 0, child.addr() as u64, true);
    let mut roots = vec![owner];

    let result = unsafe {
        heap.compact(&mut roots, &|type_rid| {
            if type_rid == 1 {
                vec![0]
            } else {
                vec![]
            }
        })
    };

    assert_eq!(result.moved_objects, 2);
    assert_eq!(result.rewritten_references, 1);
    assert_ne!(roots[0].addr(), old_owner);
    let new_child = roots[0].field(0) as *mut u64;
    assert_ne!(new_child, old_child);
    assert!(heap.by_addr.contains_key(&(roots[0].addr() as usize)));
    assert!(heap.by_addr.contains_key(&(new_child as usize)));
    assert_eq!(heap.live_count(), 2);
}

#[test]
fn compaction_callback_exposes_forwarding_map_before_release() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let old_address = object.addr() as usize;
    let mut roots = vec![object];
    let mut relocated = None;

    let result = unsafe {
        heap.compact_with_relocation(&mut roots, &|_| Vec::new(), &mut |map| {
            relocated = Some(map.relocate_address(old_address))
        })
    };

    assert_eq!(result.moved_objects, 1);
    assert_eq!(relocated, Some(roots[0].addr() as usize));
    assert!(relocated.is_some_and(|address| address != old_address));
}

#[test]
fn collect_and_compact_reclaims_dead_objects_before_moving_survivors() {
    let mut heap = Heap::new();
    let owner = heap.alloc(1, 1);
    let child = heap.alloc(2, 0);
    let dead = heap.alloc(3, 0);
    let old_child = child.addr();
    let dead_addr = dead.addr();
    heap.store_field(owner, 0, child.addr() as u64, true);
    let mut roots = vec![owner];

    let result = unsafe {
        heap.collect_and_compact(&mut roots, &|type_rid| {
            if type_rid == 1 {
                vec![0]
            } else {
                vec![]
            }
        })
    };

    assert_eq!(result.reclaimed_objects, 1);
    assert_eq!(result.moved_objects, 2);
    assert_eq!(heap.live_count(), 2);
    assert!(!heap.by_addr.contains_key(&(dead_addr as usize)));
    assert_ne!(roots[0], owner);
    assert_ne!(roots[0].field(0) as *mut u64, old_child);
}

#[test]
fn compaction_rewrites_active_native_roots_before_releasing_chunks() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let old_address = object.addr();
    let native_roots = [object];
    let _scope = unsafe { NativeRootScope::enter(&native_roots) };
    let mut roots = vec![object];

    let result = unsafe { heap.compact(&mut roots, &|_| Vec::new()) };

    assert_eq!(result.moved_objects, 1);
    assert_eq!(result.rewritten_references, 1);
    assert_ne!(roots[0].addr(), old_address);
    assert_eq!(native_roots[0], roots[0]);
}

#[test]
fn compaction_rewrites_a_stopped_native_frame_stack_map() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let mut frame = [0u8; 8];
    unsafe {
        (frame.as_mut_ptr() as *mut u64).write_unaligned(object.addr() as u64);
    }
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 20,
        span: 1,
        reference_slots: vec![0],
    }]);
    let mut roots = vec![object];

    let result = unsafe {
        heap.compact_with_native_frame(
            &mut roots,
            &|_| Vec::new(),
            &registry,
            20,
            frame.as_mut_ptr(),
        )
    };

    assert_eq!(result.moved_objects, 1);
    assert_eq!(result.rewritten_references, 1);
    assert_eq!(
        unsafe { (frame.as_ptr() as *const u64).read_unaligned() },
        roots[0].addr() as u64
    );
}

#[test]
fn compaction_rewrites_multiple_stopped_native_frames() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let mut first = [0u8; 8];
    let mut second = [0u8; 8];
    unsafe {
        (first.as_mut_ptr() as *mut u64).write_unaligned(object.addr() as u64);
        (second.as_mut_ptr() as *mut u64).write_unaligned(object.addr() as u64);
    }
    let mut first_registry = StackMapRegistry::new();
    first_registry.replace([NativeStackMap {
        pc: 30,
        span: 1,
        reference_slots: vec![0],
    }]);
    let mut second_registry = StackMapRegistry::new();
    second_registry.replace([NativeStackMap {
        pc: 40,
        span: 1,
        reference_slots: vec![0],
    }]);
    let frames = [
        StoppedNativeFrame {
            registry: &first_registry,
            pc: 30,
            sp: first.as_mut_ptr(),
        },
        StoppedNativeFrame {
            registry: &second_registry,
            pc: 40,
            sp: second.as_mut_ptr(),
        },
    ];
    let mut roots = vec![object];

    let result = unsafe { heap.compact_with_native_frames(&mut roots, &|_| Vec::new(), &frames) };

    assert_eq!(result.moved_objects, 1);
    assert_eq!(result.rewritten_references, 2);
    assert_eq!(
        unsafe { (first.as_ptr() as *const u64).read_unaligned() },
        roots[0].addr() as u64
    );
    assert_eq!(
        unsafe { (second.as_ptr() as *const u64).read_unaligned() },
        roots[0].addr() as u64
    );
}

#[test]
fn compaction_rewrites_registered_native_frame_stack_map() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let mut frame = [0u8; 8];
    unsafe {
        (frame.as_mut_ptr() as *mut u64).write_unaligned(object.addr() as u64);
    }
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 50,
        span: 1,
        reference_slots: vec![0],
    }]);
    let _scope = unsafe { ActiveNativeFrameScope::enter(&registry, 50, frame.as_mut_ptr()) };
    let mut roots = vec![object];

    let result = unsafe { heap.compact_with_registered_native_frames(&mut roots, &|_| Vec::new()) };

    assert_eq!(result.moved_objects, 1);
    assert_eq!(result.rewritten_references, 1);
    assert_eq!(
        unsafe { (frame.as_ptr() as *const u64).read_unaligned() },
        roots[0].addr() as u64
    );
}

#[test]
fn collection_marks_registered_native_frame_roots() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let mut frame = [0u8; 8];
    unsafe {
        (frame.as_mut_ptr() as *mut u64).write_unaligned(object.addr() as u64);
    }
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 60,
        span: 1,
        reference_slots: vec![0],
    }]);
    let _scope = unsafe { ActiveNativeFrameScope::enter(&registry, 60, frame.as_mut_ptr()) };

    let reclaimed = unsafe { heap.collect_with_registered_native_frames(&[], &|_| Vec::new()) };

    assert_eq!(reclaimed, 0);
    assert_eq!(heap.live_count(), 1);
}

#[test]
fn collect_and_compact_preserves_registered_native_frame_roots() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let old_address = object.addr();
    let mut frame = [0u8; 8];
    unsafe {
        (frame.as_mut_ptr() as *mut u64).write_unaligned(old_address as u64);
    }
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 70,
        span: 1,
        reference_slots: vec![0],
    }]);
    let _scope = unsafe { ActiveNativeFrameScope::enter(&registry, 70, frame.as_mut_ptr()) };
    let mut roots = Vec::new();

    let result = unsafe {
        heap.collect_and_compact_with_registered_native_frames(&mut roots, &|_| Vec::new())
    };

    assert_eq!(result.reclaimed_objects, 0);
    assert_eq!(result.moved_objects, 1);
    assert_eq!(heap.live_count(), 1);
    let moved_address = unsafe { (frame.as_ptr() as *const u64).read_unaligned() } as *mut u64;
    assert_ne!(moved_address, old_address);
    assert_eq!(
        moved_address,
        heap.records
            .iter()
            .find(|record| record.alive)
            .unwrap()
            .addr
    );
}

#[test]
fn collect_and_compact_marks_native_root_scopes() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let old_address = object.addr();
    let roots = [object];
    let _native_roots = unsafe { NativeRootScope::enter(&roots) };
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 75,
        span: 1,
        reference_slots: Vec::new(),
    }]);
    let _scope = unsafe { ActiveNativeFrameScope::enter(&registry, 75, std::ptr::null_mut()) };
    let mut explicit_roots = Vec::new();

    let result = unsafe {
        heap.collect_and_compact_with_registered_native_frames(&mut explicit_roots, &|_| Vec::new())
    };

    assert_eq!(result.reclaimed_objects, 0);
    assert_eq!(result.moved_objects, 1);
    assert_ne!(roots[0].addr(), old_address);
    assert_eq!(heap.live_count(), 1);
}

#[test]
fn collect_and_compact_accepts_explicit_native_frames() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let old_address = object.addr();
    let mut frame = [0u8; 8];
    unsafe {
        (frame.as_mut_ptr() as *mut u64).write_unaligned(old_address as u64);
    }
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 80,
        span: 1,
        reference_slots: vec![0],
    }]);
    let frames = [StoppedNativeFrame {
        registry: &registry,
        pc: 80,
        sp: frame.as_mut_ptr(),
    }];

    let result = unsafe {
        heap.collect_and_compact_with_native_frames(&mut Vec::new(), &|_| Vec::new(), &frames)
    };

    assert_eq!(result.reclaimed_objects, 0);
    assert_eq!(result.moved_objects, 1);
    let moved_address = unsafe { (frame.as_ptr() as *const u64).read_unaligned() } as *mut u64;
    assert_ne!(moved_address, old_address);
}

#[test]
fn young_collection_marks_registered_native_frame_roots() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let mut frame = [0u8; 8];
    unsafe {
        (frame.as_mut_ptr() as *mut u64).write_unaligned(object.addr() as u64);
    }
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 90,
        span: 1,
        reference_slots: vec![0],
    }]);
    let _scope = unsafe { ActiveNativeFrameScope::enter(&registry, 90, frame.as_mut_ptr()) };

    let reclaimed =
        unsafe { heap.collect_young_with_registered_native_frames(&[], &|_| Vec::new()) };

    assert_eq!(reclaimed, 0);
    assert_eq!(heap.live_count(), 1);
}

#[test]
fn compaction_rewrites_data_view_buffer_fields_and_indexes() {
    let mut heap = Heap::new();
    let buffer = heap.alloc_buffer(8);
    let view = heap.alloc_data_view(buffer, 0, 8).unwrap();
    let mut roots = vec![view];

    unsafe {
        heap.compact(&mut roots, &|type_rid| {
            if type_rid == 0 {
                vec![0]
            } else {
                vec![]
            }
        });
    }

    let moved_buffer = roots[0].field(0) as usize;
    assert!(heap.buffers.contains_key(&moved_buffer));
    assert!(heap.data_views.contains_key(&(roots[0].addr() as usize)));
}

#[test]
fn relocation_rewrites_strong_collection_references() {
    let mut heap = Heap::new();
    let array = heap.alloc_array(1);
    let map = heap.alloc_map();
    let old_target = heap.alloc(2, 0);
    let new_target = heap.alloc(3, 0);
    heap.array_set(array, 0, zun_core::Any::object(old_target.addr() as usize));
    heap.map_insert(
        map,
        zun_core::Any::object(old_target.addr() as usize),
        zun_core::Any::object(old_target.addr() as usize),
    );

    let mut relocation = RelocationMap::new();
    relocation.insert(old_target, new_target);
    assert_eq!(heap.rewrite_auxiliary_references(&relocation), 3);
    assert_eq!(
        heap.array_get(array, 0).and_then(|value| value.as_object()),
        Some(new_target.addr() as usize)
    );
    let entries = heap.map_entries(map).unwrap();
    assert_eq!(entries[0].0.as_object(), Some(new_target.addr() as usize));
    assert_eq!(entries[0].1.as_object(), Some(new_target.addr() as usize));
}

#[test]
fn relocation_rewrites_iterator_indexes_and_targets() {
    let mut heap = Heap::new();
    let old_array = heap.alloc_array(0);
    let new_array = heap.alloc_array(0);
    let iterator = heap.alloc_array_iterator(old_array).unwrap();
    let new_iterator = heap.alloc(0, 1);
    let mut relocation = RelocationMap::new();
    relocation.insert(old_array, new_array);
    relocation.insert(iterator, new_iterator);

    assert_eq!(heap.rewrite_iterator_references(&relocation), 2);
    assert_eq!(heap.array_iterator_done(new_iterator), Some(true));
    assert_eq!(new_iterator.field(0), new_array.addr() as u64);
    assert!(heap.array_iterator_done(iterator).is_none());
}

#[test]
fn relocation_rewrites_weak_metadata_without_promoting_it() {
    let mut heap = Heap::new();
    let weak_owner = heap.alloc_weak_map();
    let old_key = heap.alloc(1, 0);
    let old_value = heap.alloc(3, 0);
    let new_owner = unsafe { Obj::from_addr(0x5000 as *mut u64) };
    let new_key = unsafe { Obj::from_addr(0x6000 as *mut u64) };
    let new_value = unsafe { Obj::from_addr(0x7000 as *mut u64) };
    assert!(heap.weak_map_set_object(weak_owner, old_key, old_value));
    let weak_ref = heap.weak_ref_new(old_key);

    let mut relocation = RelocationMap::new();
    relocation.insert(weak_owner, new_owner);
    relocation.insert(old_key, new_key);
    relocation.insert(old_value, new_value);
    assert_eq!(heap.rewrite_weak_references(&relocation), 4);

    assert_eq!(
        heap.weak_refs[weak_ref.0 as usize].target,
        Some(new_key.addr() as usize)
    );
    let weak_map_id = heap
        .weak_map_objects
        .get(&(new_owner.addr() as usize))
        .copied()
        .unwrap();
    assert_eq!(
        heap.weak_maps[weak_map_id.0 as usize].entries,
        vec![(new_key.addr() as usize, new_value.addr() as usize)]
    );
    assert!(heap.weak_map_get_object(weak_owner).is_none());
}

#[test]
fn relocation_rewrites_promise_and_proxy_state() {
    let mut heap = Heap::new();
    let promise = heap.alloc_promise();
    let target = heap.alloc(1, 0);
    let handler = heap.alloc(3, 0);
    let proxy = heap.alloc_proxy(target, handler);
    let new_promise = unsafe { Obj::from_addr(0x5000 as *mut u64) };
    let new_target = unsafe { Obj::from_addr(0x6000 as *mut u64) };
    let new_handler = unsafe { Obj::from_addr(0x7000 as *mut u64) };
    let new_proxy = unsafe { Obj::from_addr(0x8000 as *mut u64) };
    heap.promise_resolve(promise, zun_core::Any::object(target.addr() as usize));

    let mut relocation = RelocationMap::new();
    relocation.insert(promise, new_promise);
    relocation.insert(proxy, new_proxy);
    relocation.insert(target, new_target);
    relocation.insert(handler, new_handler);
    assert_eq!(heap.rewrite_runtime_references(&relocation), 9);

    assert_eq!(
        heap.promise_result(new_promise)
            .and_then(|value| value.as_object()),
        Some(new_target.addr() as usize)
    );
    assert_eq!(heap.proxy_target(new_proxy), Some(new_target));
    assert_eq!(heap.proxy_handler(new_proxy), Some(new_handler));
    assert!(heap.promise_result(promise).is_none());
}

#[test]
fn minor_collection_reclaims_unreachable_young_and_promotes_reachable_young() {
    let mut heap = Heap::new();
    let old = heap.alloc(7, 1);
    unsafe { heap.collect(&[old], &|_| vec![0]) };
    let young = heap.alloc(8, 0);
    let garbage = heap.alloc(9, 0);
    heap.store_field(old, 0, young.addr() as u64, true);

    let freed = unsafe { heap.collect_young(&[], &|_| vec![0]) };
    assert_eq!(freed, 1);
    assert!(heap.by_addr.contains_key(&(old.addr() as usize)));
    assert!(heap.by_addr.contains_key(&(young.addr() as usize)));
    assert!(!heap.by_addr.contains_key(&(garbage.addr() as usize)));
    assert_eq!(heap.generation_counts(), (0, 2));
}

#[test]
fn evacuating_minor_collection_rewrites_roots_and_old_to_young_fields() {
    let mut heap = Heap::new();
    let old = heap.alloc(7, 1);
    unsafe { heap.collect(&[old], &|_| vec![0]) };
    let young = heap.alloc(8, 0);
    let old_young_address = young.addr();
    heap.store_field(old, 0, old_young_address as u64, true);
    let mut roots = vec![old, young];
    let mut relocated = false;

    let freed = unsafe {
        heap.collect_young_evacuating(&mut roots, &|_| vec![0], &mut |map| {
            relocated =
                map.relocate_address(old_young_address as usize) != old_young_address as usize;
        })
    };

    assert_eq!(freed, 0);
    assert!(relocated);
    assert_ne!(roots[1].addr(), old_young_address);
    assert_eq!(old.field(0), roots[1].addr() as u64);
    assert_eq!(heap.generation_counts(), (0, 2));
}

#[test]
fn evacuating_minor_collection_rewrites_addressed_array_side_tables() {
    let mut heap = Heap::new();
    let array = heap.alloc_array(1);
    let old_address = array.addr();
    let mut roots = vec![array];

    unsafe { heap.collect_young_evacuating(&mut roots, &|_| Vec::new(), &mut |_| {}) };

    assert_ne!(roots[0].addr(), old_address);
    assert_eq!(heap.array_len(roots[0]), Some(1));
    assert_eq!(heap.array_len(array), None);
}

#[test]
fn evacuating_minor_collection_rewrites_stopped_native_frame_roots() {
    let mut heap = Heap::new();
    let old = heap.alloc(7, 0);
    unsafe { heap.collect(&[old], &|_| Vec::new()) };
    let young = heap.alloc(8, 0);
    let old_address = young.addr();
    let mut frame = [0u8; 8];
    unsafe {
        (frame.as_mut_ptr() as *mut u64).write_unaligned(old_address as u64);
    }
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 91,
        span: 1,
        reference_slots: vec![0],
    }]);
    let frames = [StoppedNativeFrame {
        registry: &registry,
        pc: 91,
        sp: frame.as_mut_ptr(),
    }];
    let mut roots = vec![old];

    let freed = unsafe {
        heap.collect_young_evacuating_with_native_frames(
            &mut roots,
            &|_| Vec::new(),
            &frames,
            &mut |_| {},
        )
    };

    assert_eq!(freed, 0);
    let moved = unsafe { (frame.as_ptr() as *const u64).read_unaligned() } as *mut u64;
    assert_ne!(moved, old_address);
    assert!(heap.by_addr.contains_key(&(moved as usize)));
}

#[test]
fn minor_collection_clears_dead_young_weak_references() {
    let mut heap = Heap::new();
    let old = heap.alloc(7, 0);
    unsafe { heap.collect(&[old], &|_| Vec::new()) };
    let young = heap.alloc(8, 0);
    let weak = heap.weak_ref_new(young);

    assert_eq!(unsafe { heap.collect_young(&[old], &|_| Vec::new()) }, 1);
    assert_eq!(heap.weak_ref_get(weak), None);
    assert!(heap.by_addr.contains_key(&(old.addr() as usize)));
}

#[test]
fn side_table_reference_writes_enter_the_remembered_set() {
    let mut heap = Heap::new();
    let array = heap.alloc_array(1);
    let map = heap.alloc_map();
    let set = heap.alloc_set();
    let dynamic = heap.alloc_dynamic();
    let promise = heap.alloc_promise();
    unsafe { heap.collect(&[array, map, set, dynamic, promise], &|_| Vec::new()) };
    let child = heap.alloc(8, 0);

    assert!(heap
        .array_set(array, 0, zun_core::Any::object(child.addr() as usize))
        .is_some());
    assert_eq!(heap.remembered_set_len(), 1);
    heap.map_insert(
        map,
        zun_core::Any::object(child.addr() as usize),
        zun_core::Any::int(1),
    );
    assert_eq!(heap.remembered_set_len(), 2);
    heap.set_add(set, zun_core::Any::object(child.addr() as usize));
    assert_eq!(heap.remembered_set_len(), 3);
    heap.dynamic_set(
        dynamic,
        "child",
        zun_core::Any::object(child.addr() as usize),
    );
    assert_eq!(heap.remembered_set_len(), 4);
    heap.promise_resolve(promise, zun_core::Any::object(child.addr() as usize));

    assert_eq!(heap.remembered_set_len(), 5);
}

#[test]
fn weak_map_writes_remember_an_old_owner_for_minor_ephemeron_scanning() {
    let mut heap = Heap::new();
    let weak_map = heap.alloc_weak_map();
    unsafe { heap.collect(&[weak_map], &|_| Vec::new()) };
    let key = heap.alloc(8, 0);
    let value = heap.alloc(9, 0);

    assert!(heap.weak_map_set_object(weak_map, key, value));
    assert_eq!(heap.remembered_set_len(), 1);
}

#[test]
fn collection_reuses_freed_blocks() {
    let mut heap = Heap::new();
    let keep = heap.alloc(1, 2);
    let garbage = heap.alloc(1, 2);
    let garbage_addr = garbage.addr();
    let garbage_hash = garbage.identity_hash();
    let no_refs = |_: u32| Vec::<usize>::new();

    let freed = unsafe { heap.collect(&[keep], &no_refs) };
    assert_eq!(freed, 1);
    // A new allocation of the same size reuses the reclaimed block.
    let reused = heap.alloc(9, 2);
    assert_eq!(reused.addr(), garbage_addr);
    assert_ne!(reused.identity_hash(), garbage_hash);
    assert_eq!(reused.type_rid(), 9);
    assert_eq!(reused.field(0), 0); // reused block was zeroed
}

#[test]
fn transitively_reachable_objects_survive() {
    let mut heap = Heap::new();
    // Chain a -> b -> c, all type 1 with a single reference in slot 0.
    let c = heap.alloc(1, 1);
    let b = heap.alloc(1, 1);
    let a = heap.alloc(1, 1);
    b.set_field(0, c.addr() as u64);
    a.set_field(0, b.addr() as u64);
    let unreachable = heap.alloc(1, 1);
    let _ = unreachable;

    let ref_slots = |_: u32| vec![0usize];
    let freed = unsafe { heap.collect(&[a], &ref_slots) };
    assert_eq!(freed, 1); // only the detached object
    assert_eq!(heap.live_count(), 3);
}

#[test]
fn string_table_roundtrip() {
    let mut heap = Heap::new();
    let h = heap.intern_string("hello".into());
    assert_eq!(&**heap.string(h), "hello");
}

#[test]
fn extern_alloc_matches_method_alloc() {
    let mut heap = Heap::new();
    let ptr = unsafe { zun_gc_alloc(&mut heap as *mut Heap, 3, 2) };
    let o = unsafe { Obj::from_addr(ptr) };
    assert_eq!(o.type_rid(), 3);
    o.set_field(1, 7);
    assert_eq!(o.field(1), 7);
}

#[test]
fn native_root_scope_contributes_to_root_set() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let registered = [object];
    let _scope = unsafe { NativeRootScope::enter(&registered) };
    let mut roots = Vec::new();
    append_native_roots(&mut roots);
    assert_eq!(roots, registered);
}

#[test]
fn collection_keeps_native_roots_alive() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let registered = [object];
    let _scope = unsafe { NativeRootScope::enter(&registered) };
    let freed = unsafe { heap.collect_with_native_roots(&[], &|_| Vec::new()) };
    assert_eq!(freed, 0);
    assert_eq!(heap.live_count(), 1);
}

#[test]
fn nested_native_root_scopes_restore_the_outer_frame() {
    let mut heap = Heap::new();
    let outer_object = heap.alloc(1, 0);
    let inner_object = heap.alloc(2, 0);
    let outer = [outer_object];
    let inner = [inner_object];
    let outer_scope = unsafe { NativeRootScope::enter(&outer) };

    let mut roots = Vec::new();
    append_native_roots(&mut roots);
    assert_eq!(roots, outer);

    {
        let _inner_scope = unsafe { NativeRootScope::enter(&inner) };
        roots.clear();
        append_native_roots(&mut roots);
        assert_eq!(roots, vec![outer_object, inner_object]);
        let freed = unsafe { heap.collect_with_native_roots(&[], &|_| Vec::new()) };
        assert_eq!(freed, 0);
        assert_eq!(heap.live_count(), 2);
    }

    roots.clear();
    append_native_roots(&mut roots);
    assert_eq!(roots, outer);
    let freed = unsafe { heap.collect_with_native_roots(&[], &|_| Vec::new()) };
    assert_eq!(freed, 1);
    assert_eq!(heap.live_count(), 1);
    drop(outer_scope);
    roots.clear();
    append_native_roots(&mut roots);
    assert!(roots.is_empty());
}

#[test]
fn native_root_scopes_can_unwind_out_of_order() {
    let mut heap = Heap::new();
    let outer_object = heap.alloc(1, 0);
    let outer = [outer_object];
    let outer_scope = unsafe { NativeRootScope::enter(&outer) };
    let inner_scope = unsafe { NativeRootScope::enter(&outer) };

    drop(outer_scope);
    let mut roots = Vec::new();
    append_native_roots(&mut roots);
    assert_eq!(roots, outer);

    drop(inner_scope);
    roots.clear();
    append_native_roots(&mut roots);
    assert!(roots.is_empty());
}

#[test]
fn stack_map_lookup_matches_the_registered_span() {
    let mut registry = StackMapRegistry::new();
    registry.replace([
        NativeStackMap {
            pc: 200,
            span: 8,
            reference_slots: Vec::new(),
        },
        NativeStackMap {
            pc: 100,
            span: 16,
            reference_slots: Vec::new(),
        },
    ]);
    assert_eq!(registry.lookup(100).unwrap().pc, 100);
    assert_eq!(registry.lookup(115).unwrap().pc, 100);
    assert!(registry.lookup(116).is_none());
    assert!(registry.lookup(99).is_none());
    assert_eq!(registry.lookup(203).unwrap().pc, 200);
    assert!(registry.lookup(208).is_none());
}

#[test]
fn active_stack_map_scopes_restore_the_previous_registry() {
    let outer = StackMapRegistry::new();
    let inner = StackMapRegistry::new();
    let outer_scope = unsafe { ActiveStackMapScope::enter(&outer) };
    assert!(with_active_stack_map_registry(|registry| {
        registry.is_some_and(|registry| std::ptr::eq(registry, &outer))
    }));

    let inner_scope = unsafe { ActiveStackMapScope::enter(&inner) };
    assert!(with_active_stack_map_registry(|registry| {
        registry.is_some_and(|registry| std::ptr::eq(registry, &inner))
    }));

    drop(outer_scope);
    assert!(with_active_stack_map_registry(|registry| {
        registry.is_some_and(|registry| std::ptr::eq(registry, &inner))
    }));

    drop(inner_scope);
    assert!(with_active_stack_map_registry(|registry| registry.is_none()));
}

#[test]
fn active_stack_map_collection_consumes_the_thread_registry() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let mut frame = [0u64; 2];
    frame[1] = object.addr() as u64;
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 100,
        span: 4,
        reference_slots: vec![8],
    }]);
    let _scope = unsafe { ActiveStackMapScope::enter(&registry) };

    let freed = unsafe {
        heap.collect_with_active_native_frame(&[], 101, frame.as_ptr() as *const u8, &|_| {
            Vec::new()
        })
    };
    assert_eq!(freed, 0);
    assert_eq!(heap.live_count(), 1);
}

#[test]
fn active_stack_map_collection_includes_native_root_scopes() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let roots = [object];
    let _native_roots = unsafe { NativeRootScope::enter(&roots) };
    let frame = [0u8; 8];
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 105,
        span: 1,
        reference_slots: Vec::new(),
    }]);
    let _scope = unsafe { ActiveStackMapScope::enter(&registry) };

    let reclaimed = unsafe {
        heap.collect_with_active_native_frame(&[], 105, frame.as_ptr() as *const u8, &|_| {
            Vec::new()
        })
    };

    assert_eq!(reclaimed, 0);
    assert_eq!(heap.live_count(), 1);
}

#[test]
fn active_stack_map_collection_falls_back_to_explicit_roots() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let freed = unsafe {
        heap.collect_with_active_native_frame(&[object], 100, std::ptr::null(), &|_| Vec::new())
    };
    assert_eq!(freed, 0);
    assert_eq!(heap.live_count(), 1);
}

#[test]
fn stack_map_reads_reference_slots_from_a_stopped_frame() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let mut frame = [0u64; 2];
    frame[1] = object.addr() as u64;
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 100,
        span: 4,
        reference_slots: vec![8],
    }]);

    let roots = unsafe { registry.roots_at(101, frame.as_ptr() as *const u8) };
    assert_eq!(roots, vec![object]);
}

#[test]
fn native_frame_stack_map_roots_participate_in_collection() {
    let mut heap = Heap::new();
    let object = heap.alloc(1, 0);
    let mut frame = [0u64; 2];
    frame[1] = object.addr() as u64;
    let mut registry = StackMapRegistry::new();
    registry.replace([NativeStackMap {
        pc: 100,
        span: 4,
        reference_slots: vec![8],
    }]);

    let freed = unsafe {
        heap.collect_with_native_frame(&[], &registry, 101, frame.as_ptr() as *const u8, &|_| {
            Vec::new()
        })
    };
    assert_eq!(freed, 0);
    assert_eq!(heap.live_count(), 1);
}

#[test]
fn generator_state_keeps_argument_object_alive() {
    let mut heap = Heap::new();
    let generator = heap.alloc_generator(0, 17, Vec::new());
    let child = heap.alloc(2, 0);
    heap.generator_state_mut(generator)
        .expect("generator state")
        .args
        .push(zun_core::Any::object(child.addr() as usize));
    let freed = unsafe { heap.collect(&[generator], &|_| Vec::new()) };
    assert_eq!(freed, 0);
    assert_eq!(heap.live_count(), 2);
}

#[test]
fn bigint_payload_is_traced_by_its_managed_owner_and_relocated() {
    let mut heap = Heap::new();
    let _dead = heap.alloc(1, 0);
    let bigint =
        heap.alloc_bigint(zun_core::BigInt::parse_decimal("18446744073709551616").unwrap());
    let mut roots = vec![bigint];
    let result = unsafe { heap.collect_and_compact(&mut roots, &|_| Vec::new()) };
    assert_eq!(result.reclaimed_objects, 1);
    assert_eq!(
        heap.bigint_value(roots[0]).unwrap().to_string(),
        "18446744073709551616"
    );
    assert_eq!(
        roots[0].type_rid(),
        zun_core::CoreTypeId::BigInt.get() as u32
    );
}

#[test]
fn realm_global_object_is_a_heap_singleton_and_relocates() {
    let mut heap = Heap::new();
    let global = heap.global_object();
    assert_eq!(heap.global_object(), global);
    let mut roots = vec![global];
    unsafe { heap.collect_young_evacuating(&mut roots, &|_| Vec::new(), &mut |_| {}) };
    assert_eq!(heap.global_object(), roots[0]);
    assert_ne!(roots[0], global);
}
