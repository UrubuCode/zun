//! Lookups that follow the explicit links of the dynamic object model: a
//! function's `[[Prototype]]` (statics inherited by a constructor function
//! chained to its base with `Object.setPrototypeOf`) and an ordinary object's
//! prototype chain (`constructor`, inherited setters).
//!
//! These are what a run-time class stands on: the front end lowers a class
//! that is a value to a constructor function plus a prototype object (see
//! `zun_syntax`'s `runtime_class`), so its statics, its `constructor` and its
//! accessors are found by walking links, not by reading a TypeDef.

use super::*;

/// Where a function value finds a static member it does not own.
pub(crate) enum InheritedStatic {
    /// On another function further up its `[[Prototype]]` chain.
    Function(zun_core::FunctionHandle),
    /// On a class the chain reaches (the payload is its class descriptor):
    /// the class's own lookup rules take over from there.
    Class(zun_core::Any),
}

impl<'a> Interp<'a> {
    /// Resolve `name` along a function's `[[Prototype]]` chain. `None` when
    /// the function owns `name` itself, was never relinked, or no link holds
    /// it -- the ordinary own-property and `Function.prototype` paths apply.
    pub(crate) fn function_inherited_static(
        &self,
        handle: zun_core::FunctionHandle,
        name: &str,
    ) -> Option<InheritedStatic> {
        // Every function owns these; they never come from the chain.
        if matches!(name, "prototype" | "name" | "length") {
            return None;
        }
        let owns = |function: zun_core::FunctionHandle| {
            let bag = crate::function_expando_bag(function);
            zun_gc::with_current_heap(|heap| {
                heap.dynamic_has(bag, name)
                    || heap.dynamic_get_function(bag, name).is_some()
                    || heap.dynamic_is_accessor(bag, name)
            })
        };
        if owns(handle) {
            return None;
        }
        let mut current = handle;
        for _ in 0..64 {
            let link = crate::function_prototype_override(current)?;
            if link.as_class_descriptor().is_some() {
                return Some(InheritedStatic::Class(link));
            }
            let parent = zun_gc::with_current_heap(|heap| Value::unbox_any(link, heap))?;
            let Value::Function(parent) = parent else {
                return None;
            };
            if owns(parent) {
                return Some(InheritedStatic::Function(parent));
            }
            current = parent;
        }
        None
    }

    /// The `constructor` an ordinary object reads through explicit links: its
    /// own property, else the first holder on a prototype chain a script (or
    /// `new` on a function) installed. `None` leaves the answer to the
    /// class-derived constructor record -- an object that was never linked,
    /// and every typed instance still on its class's own prototype.
    pub(crate) fn explicit_constructor(
        &mut self,
        object: zun_gc::Obj,
    ) -> Result<Option<Value>, RunError> {
        let (dynamic, linked) = zun_gc::with_current_heap(|heap| {
            (
                heap.is_dynamic_object(object) && heap.proxy_target(object).is_none(),
                heap.dynamic_prototype(object)
                    .is_some_and(|link| link.as_object().is_some()),
            )
        });
        if !dynamic {
            return Ok(None);
        }
        let own = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(object, "constructor")
                .map(Value::Any)
                .or_else(|| heap.dynamic_get_function(object, "constructor").map(Value::Function))
        });
        if own.is_some() {
            return Ok(own);
        }
        // `Function.prototype` owns no `constructor`, so the walk below reached
        // `Object.prototype`'s and answered `Object`: `f.constructor.name` was
        // `"Object"` and `Function.prototype.constructor === Function` was
        // false, where node answers `Function` and true. The `Function` global
        // is the class descriptor the registry owns, so read it off the global
        // object rather than rebuilding it here.
        if object == Self::function_prototype_singleton() {
            if let Some(found) = zun_gc::with_current_heap_mut(|heap| {
                let global = heap.global_object();
                heap.dynamic_get(global, "Function")
            }) {
                return Ok(Some(Value::Any(found)));
            }
        }
        if !linked {
            return Ok(None);
        }
        self.prototype_chain_get(object, "constructor")
    }

    /// An accessor `name` an ordinary object inherits through its explicit
    /// prototype chain: `Some(setter)` when found (the setter is `None` for a
    /// getter-only accessor). `None` when the object owns `name`, is not an
    /// ordinary dynamic object, or a data property shadows the accessor first.
    pub(crate) fn dynamic_chain_accessor_setter(
        object: zun_gc::Obj,
        name: &str,
    ) -> Option<Option<zun_core::FunctionHandle>> {
        zun_gc::with_current_heap(|heap| {
            if !heap.is_dynamic_object(object)
                || heap.is_instance_bag(object)
                || heap.proxy_target(object).is_some()
                || heap.dynamic_has(object, name)
            {
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
}

impl<'a> Interp<'a> {
    /// A function-valued member an object inherits through the prototype
    /// chain it was explicitly linked to (`lookup` reads one holder). `None`
    /// for an object that was never linked -- a plain literal, and every
    /// typed instance still on its class's own prototype, whose methods are
    /// resolved from its TypeDef instead.
    pub(crate) fn linked_chain_function(
        object: zun_gc::Obj,
        lookup: impl Fn(&zun_gc::Heap, zun_gc::Obj) -> Option<zun_core::FunctionHandle>,
    ) -> Option<zun_core::FunctionHandle> {
        zun_gc::with_current_heap(|heap| {
            if !heap.is_dynamic_object(object) || heap.proxy_target(object).is_some() {
                return None;
            }
            let mut holder = heap.dynamic_prototype(object)?.as_object()?;
            for _ in 0..256 {
                let current = unsafe { zun_gc::Obj::from_addr(holder as *mut u64) };
                if let Some(found) = lookup(heap, current) {
                    return Some(found);
                }
                holder = heap.dynamic_prototype(current)?.as_object()?;
            }
            None
        })
    }

    /// The argument list for calling a method found on a prototype chain with
    /// `receiver` as `this`. A class's own prototype object holds its instance
    /// methods, which take the receiver as their first parameter by
    /// construction; every other function value goes through the shared
    /// receiver rule.
    pub(crate) fn inherited_method_args(
        &self,
        method: zun_core::FunctionHandle,
        receiver: zun_gc::Obj,
        mut args: Vec<Value>,
    ) -> Vec<Value> {
        if !method.is_bound()
            && method.builtin_method().is_none()
            && method.generator_method().is_none()
            && self.method_rid_is_instance_method(method.method_rid())
        {
            args.insert(0, Value::Object(receiver));
            return args;
        }
        self.with_this_arg(method, receiver, args)
    }
}

impl Interp<'_> {
    /// Like [`Self::linked_chain_function`] but for a non-function value: used
    /// where the specification's GetMethod has to tell "absent" from "present
    /// but not callable".
    pub(crate) fn linked_chain_value(
        object: zun_gc::Obj,
        lookup: impl Fn(&zun_gc::Heap, zun_gc::Obj) -> Option<zun_core::Any>,
    ) -> Option<zun_core::Any> {
        zun_gc::with_current_heap(|heap| {
            if !heap.is_dynamic_object(object) || heap.proxy_target(object).is_some() {
                return None;
            }
            let mut holder = heap.dynamic_prototype(object)?.as_object()?;
            for _ in 0..256 {
                let current = unsafe { zun_gc::Obj::from_addr(holder as *mut u64) };
                if let Some(found) = lookup(heap, current) {
                    return Some(found);
                }
                holder = heap.dynamic_prototype(current)?.as_object()?;
            }
            None
        })
    }
}
