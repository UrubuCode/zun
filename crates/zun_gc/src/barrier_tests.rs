//! Write-barrier coverage.
//!
//! A young collection never traces an old object, so a reference stored into a
//! tenured object is kept alive only if the store passed the write barrier. Each
//! case below tenures an owner, stores a young object into it through one public
//! store path, and requires that
//!
//! * the verifier ([`Heap::unremembered_old_to_young_edges`]) finds no
//!   unremembered old-to-young edge, and
//! * the young object survives (and is found again, relocated) after a young
//!   evacuating collection that sees the owner only as an old object.
//!
//! A new store path that forgets its barrier fails here by adding a case; the
//! verifier also runs before every young collection of every other test and of
//! the interpreter and VM suites when `ZUN_GC_VERIFY=1`.

use super::*;
use zun_core::Any;

type Layout = std::rc::Rc<[usize]>;

/// Type RID 7 has one reference field (slot 0); every other type has none.
fn layout(rid: u32) -> Layout {
    if rid == 7 {
        std::rc::Rc::from(vec![0usize])
    } else {
        std::rc::Rc::from(Vec::new())
    }
}

/// The type RID of the young object every case stores.
const CHILD: u32 = 8;

struct Case {
    name: &'static str,
    /// Allocate the (still young) owner.
    make: fn(&mut Heap) -> Obj,
    /// Store `child` into the (by now old) owner through one store path.
    store: fn(&mut Heap, Obj, Obj),
    /// Read the stored reference back out of the owner.
    read: fn(&Heap, Obj) -> Option<usize>,
}

fn object_any(child: Obj) -> Any {
    Any::object(child.addr() as usize)
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "field slot (store_field)",
            make: |heap| heap.alloc(7, 1),
            store: |heap, owner, child| heap.store_field(owner, 0, child.addr() as u64, true),
            read: |_, owner| Some(owner.field(0) as usize),
        },
        Case {
            name: "any-typed field (store_any_field)",
            make: |heap| heap.alloc(1, 1),
            store: |heap, owner, child| heap.store_any_field(owner, 0, object_any(child)),
            read: |heap, owner| heap.load_any_field(owner, 0).and_then(|any| any.as_object()),
        },
        Case {
            name: "array element (array_set)",
            make: |heap| heap.alloc_array(1),
            store: |heap, owner, child| {
                heap.array_set(owner, 0, object_any(child));
            },
            read: |heap, owner| heap.array_get(owner, 0).and_then(|any| any.as_object()),
        },
        Case {
            name: "array element (array_push)",
            make: |heap| heap.alloc_array(0),
            store: |heap, owner, child| {
                heap.array_push(owner, object_any(child));
            },
            read: |heap, owner| heap.array_get(owner, 0).and_then(|any| any.as_object()),
        },
        Case {
            name: "array element (array_insert)",
            make: |heap| heap.alloc_array(0),
            store: |heap, owner, child| {
                heap.array_insert(owner, 0, object_any(child));
            },
            read: |heap, owner| heap.array_get(owner, 0).and_then(|any| any.as_object()),
        },
        Case {
            name: "map value (map_insert)",
            make: |heap| heap.alloc_map(),
            store: |heap, owner, child| {
                heap.map_insert(owner, Any::int(1), object_any(child));
            },
            read: |heap, owner| heap.map_get(owner, Any::int(1)).and_then(|any| any.as_object()),
        },
        Case {
            name: "map key (map_insert)",
            make: |heap| heap.alloc_map(),
            store: |heap, owner, child| {
                heap.map_insert(owner, object_any(child), Any::int(1));
            },
            read: |heap, owner| {
                heap.map_entries(owner)
                    .and_then(|entries| entries.first().and_then(|(key, _)| key.as_object()))
            },
        },
        Case {
            name: "set element (set_add)",
            make: |heap| heap.alloc_set(),
            store: |heap, owner, child| {
                heap.set_add(owner, object_any(child));
            },
            read: |heap, owner| {
                heap.set_values(owner)
                    .and_then(|values| values.first().and_then(|value| value.as_object()))
            },
        },
        Case {
            name: "dynamic property (dynamic_set)",
            make: |heap| heap.alloc_dynamic(),
            store: |heap, owner, child| {
                heap.dynamic_set(owner, "p", object_any(child));
            },
            read: |heap, owner| heap.dynamic_get(owner, "p").and_then(|any| any.as_object()),
        },
        Case {
            name: "dynamic property (dynamic_define)",
            make: |heap| heap.alloc_dynamic(),
            store: |heap, owner, child| {
                heap.dynamic_define(owner, "p", object_any(child), true);
            },
            read: |heap, owner| heap.dynamic_get(owner, "p").and_then(|any| any.as_object()),
        },
        Case {
            name: "dynamic property (dynamic_define_data)",
            make: |heap| heap.alloc_dynamic(),
            store: |heap, owner, child| {
                heap.dynamic_define_data(owner, "p", object_any(child), true, true, true);
            },
            read: |heap, owner| heap.dynamic_get(owner, "p").and_then(|any| any.as_object()),
        },
        Case {
            name: "dynamic symbol property (dynamic_set_symbol)",
            make: |heap| heap.alloc_dynamic(),
            store: |heap, owner, child| {
                let symbol = heap.symbol_for("k");
                heap.dynamic_set_symbol(owner, symbol, object_any(child));
            },
            read: |heap, owner| {
                let symbol = *heap.dynamic_symbol_keys(owner).first()?;
                heap.dynamic_get_symbol(owner, symbol)
                    .and_then(|any| any.as_object())
            },
        },
        Case {
            name: "dynamic function property (dynamic_set_function)",
            make: |heap| heap.alloc_dynamic(),
            store: |heap, owner, child| {
                let function = zun_core::FunctionHandle::new(5, Some(child.addr() as usize));
                heap.dynamic_set_function(owner, "f", function);
            },
            read: |heap, owner| {
                heap.dynamic_get_function(owner, "f")
                    .and_then(|function| function.environment())
            },
        },
        Case {
            name: "dynamic accessor (dynamic_define_accessor)",
            make: |heap| heap.alloc_dynamic(),
            store: |heap, owner, child| {
                let getter = zun_core::FunctionHandle::new(5, Some(child.addr() as usize));
                heap.dynamic_define_accessor(owner, "a", Some(getter), None, true, true);
            },
            read: |heap, owner| {
                heap.dynamic_accessor_getter(owner, "a")
                    .and_then(|function| function.environment())
            },
        },
        Case {
            name: "prototype (dynamic_set_prototype)",
            make: |heap| heap.alloc_dynamic(),
            store: |heap, owner, child| {
                heap.dynamic_set_prototype(owner, Some(object_any(child)));
            },
            read: |heap, owner| {
                heap.dynamic_prototype(owner).and_then(|any| any.as_object())
            },
        },
        Case {
            name: "promise result (promise_resolve)",
            make: |heap| heap.alloc_promise(),
            store: |heap, owner, child| {
                heap.promise_resolve(owner, object_any(child));
            },
            read: |heap, owner| heap.promise_result(owner).and_then(|any| any.as_object()),
        },
        Case {
            name: "promise reason (promise_reject_value)",
            make: |heap| heap.alloc_promise(),
            store: |heap, owner, child| {
                heap.promise_reject_value(owner, object_any(child), "boom");
            },
            read: |heap, owner| heap.promise_reason(owner).and_then(|any| any.as_object()),
        },
        Case {
            name: "generator state (generator_state_mut)",
            make: |heap| heap.alloc_generator(1, 3, Vec::new()),
            store: |heap, owner, child| {
                heap.generator_state_mut(owner).unwrap().current = object_any(child);
            },
            read: |heap, owner| {
                heap.generator_state(owner)
                    .and_then(|state| state.current.as_object())
            },
        },
        Case {
            name: "generator state (generator_state_set)",
            make: |heap| heap.alloc_generator(1, 3, Vec::new()),
            store: |heap, owner, child| {
                let mut state = heap.generator_state(owner).unwrap().clone();
                state.args = vec![object_any(child)];
                heap.generator_state_set(owner, state);
            },
            read: |heap, owner| {
                heap.generator_state(owner)
                    .and_then(|state| state.args.first().and_then(|any| any.as_object()))
            },
        },
        Case {
            name: "generator frame (generator_frame_set)",
            make: |heap| heap.alloc_generator(1, 3, Vec::new()),
            store: |heap, owner, child| {
                heap.generator_frame_set(
                    owner,
                    GeneratorFrame {
                        method_rid: 3,
                        pc: 0,
                        locals: vec![object_any(child)],
                        eval_stack: Vec::new(),
                    },
                );
            },
            read: |heap, owner| {
                heap.generator_frame(owner)
                    .and_then(|frame| frame.locals.first().and_then(|any| any.as_object()))
            },
        },
        Case {
            name: "generator frame (generator_frame_mut)",
            make: |heap| {
                let generator = heap.alloc_generator(1, 3, Vec::new());
                heap.generator_frame_set(
                    generator,
                    GeneratorFrame {
                        method_rid: 3,
                        pc: 0,
                        locals: vec![Any::int(0)],
                        eval_stack: Vec::new(),
                    },
                );
                generator
            },
            store: |heap, owner, child| {
                heap.generator_frame_mut(owner).unwrap().locals[0] = object_any(child);
            },
            read: |heap, owner| {
                heap.generator_frame(owner)
                    .and_then(|frame| frame.locals.first().and_then(|any| any.as_object()))
            },
        },
        Case {
            name: "proxy target (proxy_revoke)",
            make: |heap| {
                let target = heap.alloc(1, 0);
                let handler = heap.alloc(2, 0);
                heap.alloc_proxy(target, handler)
            },
            // `proxy_revoke` allocates its own young target; the case checks that
            // one, so the pre-allocated `child` is simply unused here.
            store: |heap, owner, _child| {
                heap.proxy_revoke(owner);
            },
            read: |heap, owner| heap.proxy_target(owner).map(|target| target.addr() as usize),
        },
    ]
}

fn tenure(heap: &mut Heap, owner: Obj) -> Obj {
    let mut roots = [owner];
    unsafe { heap.collect_young_evacuating(&mut roots, &layout, &mut |_| {}) };
    let owner = roots[0];
    let index = heap.by_addr[&(owner.addr() as usize)];
    assert_eq!(heap.records[index].generation, records::Generation::Old);
    owner
}

#[test]
fn every_store_path_remembers_an_old_owner_and_keeps_the_young_object_alive() {
    for case in cases() {
        let mut heap = Heap::new();
        heap.set_verify_barriers(true);
        heap.set_poison_released_memory(true);
        let owner = (case.make)(&mut heap);
        let owner = tenure(&mut heap, owner);
        let child = heap.alloc(CHILD, 0);
        (case.store)(&mut heap, owner, child);

        let violations = heap.unremembered_old_to_young_edges(&layout);
        assert!(violations.is_empty(), "{}: {violations:?}", case.name);
        assert!(
            heap.remembered_set.contains(&(owner.addr() as usize)),
            "{}: the owner was not remembered",
            case.name
        );

        // The owner is old and the only reference to the child: a young
        // collection must keep the child through the remembered set alone.
        let mut roots = [owner];
        unsafe { heap.collect_young_evacuating(&mut roots, &layout, &mut |_| {}) };
        let stored = (case.read)(&heap, roots[0])
            .unwrap_or_else(|| panic!("{}: the stored reference is gone", case.name));
        let survivor = unsafe { Obj::from_addr(stored as *mut u64) };
        assert!(
            heap.by_addr.contains_key(&stored),
            "{}: the stored reference dangles",
            case.name
        );
        assert_eq!(survivor.type_rid(), if case.name.starts_with("proxy") { 0 } else { CHILD });
    }
}

#[test]
fn every_store_path_also_survives_a_non_moving_young_collection() {
    for case in cases() {
        let mut heap = Heap::new();
        heap.set_verify_barriers(true);
        let owner = (case.make)(&mut heap);
        let owner = tenure(&mut heap, owner);
        let child = heap.alloc(CHILD, 0);
        (case.store)(&mut heap, owner, child);
        unsafe { heap.collect_young(&[owner], &layout) };
        let stored = (case.read)(&heap, owner)
            .unwrap_or_else(|| panic!("{}: the stored reference is gone", case.name));
        assert!(heap.by_addr.contains_key(&stored), "{}: dangling", case.name);
    }
}

#[test]
fn the_verifier_reports_a_store_that_skipped_the_barrier() {
    let mut heap = Heap::new();
    let owner = heap.alloc(7, 1);
    let owner = tenure(&mut heap, owner);
    let child = heap.alloc(CHILD, 0);
    // A raw slot write: exactly the store a missing barrier call would leave.
    owner.set_field(0, child.addr() as u64);
    let violations = heap.unremembered_old_to_young_edges(&layout);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].contains("without a write barrier"));
}

#[test]
#[should_panic(expected = "remembered-set invariant broken")]
fn a_young_collection_refuses_to_run_over_an_unbarriered_store() {
    let mut heap = Heap::new();
    heap.set_verify_barriers(true);
    let owner = heap.alloc(7, 1);
    let owner = tenure(&mut heap, owner);
    let child = heap.alloc(CHILD, 0);
    owner.set_field(0, child.addr() as u64);
    unsafe { heap.collect_young(&[owner], &layout) };
}

#[test]
fn a_young_collection_does_not_trace_old_objects() {
    // With the verifier off, an unbarriered old-to-young edge is invisible to a
    // young collection: the old object is trusted, not traced. This pins the
    // design -- the cost of a young collection does not include the old
    // generation -- and shows what the barrier is for.
    let mut heap = Heap::new();
    heap.set_verify_barriers(false);
    let owner = heap.alloc(7, 1);
    let owner = tenure(&mut heap, owner);
    let child = heap.alloc(CHILD, 0);
    owner.set_field(0, child.addr() as u64);
    unsafe { heap.collect_young(&[owner], &layout) };
    assert!(!heap.by_addr.contains_key(&(child.addr() as usize)));
}

#[test]
fn old_garbage_is_not_reclaimed_by_a_young_collection_but_by_a_full_one() {
    let mut heap = Heap::new();
    let owner = heap.alloc(7, 1);
    let owner = tenure(&mut heap, owner);
    unsafe { heap.collect_young(&[], &layout) };
    assert!(heap.by_addr.contains_key(&(owner.addr() as usize)));
    unsafe { heap.collect(&[], &layout) };
    assert!(!heap.by_addr.contains_key(&(owner.addr() as usize)));
}

#[test]
fn a_young_value_behind_an_old_weak_map_key_survives() {
    let mut heap = Heap::new();
    heap.set_verify_barriers(true);
    let map = heap.alloc_weak_map();
    let key = heap.alloc(1, 0);
    let mut roots = [map, key];
    unsafe { heap.collect_young_evacuating(&mut roots, &layout, &mut |_| {}) };
    let (map, key) = (roots[0], roots[1]);
    let value = heap.alloc(CHILD, 0);
    assert!(heap.weak_map_set_object(map, key, value));
    let mut roots = [map, key];
    unsafe { heap.collect_young_evacuating(&mut roots, &layout, &mut |_| {}) };
    let value = heap.weak_map_get_value_object(roots[0], roots[1]).expect("value kept");
    assert_eq!(value.type_rid(), CHILD);
    assert!(heap.by_addr.contains_key(&(value.addr() as usize)));
}
