//! Native-backed subclass instances.
//!
//! `class X extends Array` (or `ArrayBuffer`, or a typed array) produces
//! instances that are the native value itself with `X.prototype` installed as
//! its `[[Prototype]]` (see `builtin_bases.zun`). The native keeps every
//! built-in behaviour; what changes is that reads which a plain native answers
//! from its built-in class (`constructor`, species) must first consult the
//! value itself and the prototype chain it carries.

use super::*;

/// Whether `object` is one of the native kinds a class can be backed by.
pub(crate) fn is_native_carrier_value(heap: &zun_gc::Heap, object: zun_gc::Obj) -> bool {
    heap.array_len(object).is_some()
        || heap.typed_array_element(object).is_some()
        || heap.buffer_len(object).is_some()
        || heap.regex_source(object).is_some()
}

impl<'a> Interp<'a> {
    /// Whether the class `rid` derives from a native carrier root
    /// (`__BuiltinArray`, `__BuiltinArrayBuffer`, a typed array root).
    pub(crate) fn class_is_native_backed(&self, rid: u32) -> bool {
        zun_natives::NATIVE_CARRIER_BASES
            .iter()
            .any(|base| self.class_chain_has_name(rid, &format!("__Builtin{base}")))
    }

    /// The run-time counterpart of the adoption the checker emits after `new`:
    /// for a construction shell of a native-backed class, the native value its
    /// carrier root created (with the declared fields of the shell installed).
    /// `Ok(None)` for every other object.
    pub(crate) fn adopt_native_shell(
        &mut self,
        shell: zun_gc::Obj,
    ) -> Result<Option<Value>, RunError> {
        let rid = shell.type_rid();
        let typed = zun_gc::with_current_heap(|heap| {
            !heap.is_dynamic_object(shell) || heap.is_instance_bag(shell)
        });
        if rid < 1 || !typed || !self.class_is_native_backed(rid) {
            return Ok(None);
        }
        let Some(adopt) = self.free_function_rid("__zunNativeAdopt") else {
            return Ok(None);
        };
        let boxed = zun_gc::with_current_heap_mut(|heap| Value::Object(shell).box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        self.call(adopt, vec![Value::Any(boxed)], None).map(Some)
    }

    /// Route the plain native `result` of a species-sensitive method through
    /// the species constructor of `receiver` (ECMAScript `ArraySpeciesCreate` /
    /// `SpeciesConstructor`). A plain native, which owns and inherits no
    /// `constructor`, returns `result` untouched and pays one table lookup.
    pub(crate) fn apply_species(
        &mut self,
        receiver: zun_gc::Obj,
        member: &str,
        result: Value,
    ) -> Result<Value, RunError> {
        let helper = zun_gc::with_current_heap(|heap| {
            if heap.array_len(receiver).is_some() {
                matches!(
                    member,
                    "map" | "filter" | "slice" | "splice" | "concat" | "flat" | "flatMap"
                )
                .then_some("__zunArraySpeciesResult")
            } else if heap.typed_array_element(receiver).is_some() {
                matches!(member, "slice" | "map" | "filter" | "subarray")
                    .then_some("__zunTypedSpeciesResult")
            } else if heap.buffer_len(receiver).is_some() {
                (member == "slice").then_some("__zunBufferSpeciesResult")
            } else {
                None
            }
        });
        let Some(helper) = helper else {
            return Ok(result);
        };
        if Self::native_chain_constructor(receiver).is_none() {
            return Ok(result);
        }
        let Some(rid) = self.free_function_rid(helper) else {
            return Ok(result);
        };
        let result_any = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
            .unwrap_or_else(zun_core::Any::void);
        self.call(
            rid,
            vec![
                Value::Any(zun_core::Any::object(receiver.addr() as usize)),
                Value::Str(member.into()),
                Value::Any(result_any),
            ],
            None,
        )
    }

    /// Whether a native value inherits `name` through an installed prototype
    /// (the `in` operator on a native-backed subclass instance). Own members
    /// are the business of the caller.
    pub(crate) fn native_chain_has(object: zun_gc::Obj, name: &str) -> bool {
        zun_gc::with_current_heap(|heap| {
            if !is_native_carrier_value(heap, object) {
                return false;
            }
            let Some(mut holder) = heap.dynamic_prototype(object).and_then(|any| any.as_object())
            else {
                return false;
            };
            for _ in 0..256 {
                let current = unsafe { zun_gc::Obj::from_addr(holder as *mut u64) };
                if heap.dynamic_has(current, name)
                    || heap.dynamic_is_accessor(current, name)
                    || heap.dynamic_get_function(current, name).is_some()
                {
                    return true;
                }
                match heap.dynamic_prototype(current).and_then(|any| any.as_object()) {
                    Some(next) => holder = next,
                    None => return false,
                }
            }
            false
        })
    }

    /// An accessor `name` inherited by a native value through an installed
    /// prototype: `Some(setter)` when found (the setter is `None` for a
    /// getter-only accessor). `None` when the native owns `name`, carries no
    /// prototype, or a data property shadows the accessor first.
    pub(crate) fn native_chain_accessor_setter(
        object: zun_gc::Obj,
        name: &str,
    ) -> Option<Option<zun_core::FunctionHandle>> {
        zun_gc::with_current_heap(|heap| {
            if !is_native_carrier_value(heap, object) || heap.dynamic_has(object, name) {
                return None;
            }
            let mut holder = heap.dynamic_prototype(object)?.as_object()?;
            for _ in 0..256 {
                let current = unsafe { zun_gc::Obj::from_addr(holder as *mut u64) };
                if heap.dynamic_is_accessor(current, name) {
                    return Some(heap.dynamic_accessor_setter(current, name));
                }
                if heap.dynamic_has(current, name) {
                    return None;
                }
                holder = heap.dynamic_prototype(current)?.as_object()?;
            }
            None
        })
    }

    /// The `constructor` a native value owns or inherits through an installed
    /// prototype: an own data property first (`arr.constructor = ...`), then
    /// the first holder on the chain. `None` for a plain native, which answers
    /// from its built-in class.
    pub(crate) fn native_chain_constructor(object: zun_gc::Obj) -> Option<zun_core::Any> {
        zun_gc::with_current_heap(|heap| {
            if !is_native_carrier_value(heap, object) {
                return None;
            }
            let mut holder = object;
            for _ in 0..256 {
                if let Some(found) = heap.dynamic_get(holder, "constructor") {
                    return Some(found);
                }
                let next = heap.dynamic_prototype(holder)?.as_object()?;
                holder = unsafe { zun_gc::Obj::from_addr(next as *mut u64) };
            }
            None
        })
    }
}
