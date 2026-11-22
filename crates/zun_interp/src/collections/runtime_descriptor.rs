//! Error-stack rendering and object own-entry / property-descriptor builders.
//!
//! Methods moved verbatim out of the collection dispatch module.

use super::*;

impl<'a> Interp<'a> {
    /// Render the live activation chain as the frame lines of an `Error.stack`
    /// string: innermost frame first, one `    at <fn> (<span>)` per frame. See
    /// the `captureStackTrace` arm for the fidelity notes.
    pub(crate) fn render_stack_trace(&self) -> String {
        let mut lines = String::new();
        for (index, frame) in self.call_stack.iter().rev().enumerate() {
            if index > 0 {
                lines.push('\n');
            }
            let (name, span) = match self.module.debug_frame_at(frame.method_rid, frame.pc) {
                Some(debug) => (debug.method_name, debug.source_span),
                None => ("<anonymous>", None),
            };
            let name = if name.is_empty() { "<anonymous>" } else { name };
            match span {
                Some((start, end)) => {
                    lines.push_str(&format!("    at {name} ({start}..{end})"));
                }
                None => lines.push_str(&format!("    at {name}")),
            }
        }
        lines
    }

    /// Enumerate the own enumerable string keys and their values for either a
    /// dynamic object (insertion-ordered dynamic keys) or a fixed-layout typed
    /// object (declared instance fields, base classes first, static and private
    /// fields excluded).
    pub(crate) fn object_own_entries(
        &mut self,
        obj: zun_gc::Obj,
    ) -> Result<Vec<(String, Value)>, RunError> {
        self.object_own_entries_impl(obj, true)
    }

    /// Enumerate own enumerable keys; `read_values` false skips getters (values
    /// come back as `undefined`), as `Object.keys` never reads property values.
    pub(crate) fn object_own_entries_impl(
        &mut self,
        obj: zun_gc::Obj,
        read_values: bool,
    ) -> Result<Vec<(String, Value)>, RunError> {
        // A primitive wrapper object (`new String("hi")`, `new Number(1)`):
        // a string wrapper exposes its characters; number/boolean wrappers
        // have no own enumerable properties.
        // The internal `value` slot of the wrapper is never listed; a property
        // a script stored on the wrapper follows the indices.
        if let Some(kind) = self.wrapper_instance_kind(obj) {
            let mut out: Vec<(String, Value)> = Vec::new();
            if kind == "String" {
                let text = self.string_wrapper_text(obj).unwrap_or_default();
                out.extend(
                    text.chars()
                        .enumerate()
                        .map(|(index, ch)| (index.to_string(), Value::Str(ch.to_string().into()))),
                );
            }
            out.extend(Self::wrapper_expandos(obj, true));
            return Ok(out);
        }
        // A typed array: its elements by index.
        if let Some(len) = crate::typed_arrays::element_len(obj) {
            let mut out: Vec<(String, Value)> = Vec::with_capacity(len);
            for index in 0..len {
                if let Some(element) = crate::typed_arrays::read_element(obj, index) {
                    out.push((index.to_string(), element));
                }
            }
            // Named properties a script stored on the view follow its elements.
            let expandos = zun_gc::with_current_heap(|heap| {
                heap.dynamic_keys(obj)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|key| heap.dynamic_is_enumerable(obj, key))
                    .filter_map(|key| {
                        if let Some(function) = heap.dynamic_get_function(obj, &key) {
                            return Some((key, Value::Function(function)));
                        }
                        heap.dynamic_get(obj, &key)
                            .and_then(|any| Value::unbox_any(any, heap))
                            .map(|value| (key, value))
                    })
                    .collect::<Vec<_>>()
            });
            out.extend(expandos);
            return Ok(out);
        }
        // An array: its present indices (as strings), then enumerable expando
        // properties.
        if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(obj)) {
            let mut out: Vec<(String, Value)> = Vec::new();
            let present = zun_gc::with_current_heap(|heap| heap.array_present_indices(obj))
                .unwrap_or_default();
            let _ = len;
            for index in present {
                let element = zun_gc::with_current_heap(|heap| {
                    heap.array_get(obj, index)
                        .filter(|any| !any.is_hole())
                        .and_then(|any| Value::unbox_any(any, heap))
                });
                if let Some(element) = element {
                    out.push((index.to_string(), element));
                }
            }
            let expandos = zun_gc::with_current_heap(|heap| {
                heap.dynamic_keys(obj)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|key| heap.dynamic_is_enumerable(obj, key))
                    .filter_map(|key| {
                        // A method installed on the array (`arr.join = fn`)
                        // rides in the function slot and is an own property too.
                        if let Some(function) = heap.dynamic_get_function(obj, &key) {
                            return Some((key, Value::Function(function)));
                        }
                        heap.dynamic_get(obj, &key)
                            .and_then(|any| Value::unbox_any(any, heap))
                            .map(|value| (key, value))
                    })
                    .collect::<Vec<_>>()
            });
            out.extend(expandos);
            return Ok(out);
        }
        // A proxy that defines `ownKeys` or `getOwnPropertyDescriptor` answers
        // through those traps.
        if let Some(entries) = self.proxy_own_entries(obj, read_values)? {
            return Ok(entries);
        }
        // A proxy without the relevant trap defers to its target, where the
        // properties actually live. `Object.keys(new Proxy({b: 2}, {}))`
        // answered empty and `JSON.stringify` of any proxy answered `{}`,
        // while `getOwnPropertyNames` already routed correctly.
        let obj = self.enumerability_holder(obj);
        if zun_gc::with_current_heap(|heap| {
            heap.is_dynamic_object(obj) && !heap.is_instance_bag(obj)
        }) {
            let keys = zun_gc::with_current_heap(|heap| heap.dynamic_keys(obj))
                .ok_or(RunError::BadToken)?;
            let mut out = Vec::with_capacity(keys.len());
            for key in keys {
                let (enumerable, getter, data, function) = zun_gc::with_current_heap(|heap| {
                    (
                        heap.dynamic_is_enumerable(obj, &key),
                        heap.dynamic_accessor_getter(obj, &key),
                        heap.dynamic_get(obj, &key),
                        heap.dynamic_get_function(obj, &key),
                    )
                });
                if !enumerable {
                    continue;
                }
                if !read_values {
                    out.push((key, Value::Void));
                    continue;
                }
                // A function-valued own property rides in the function slot;
                // it is an entry like any other, in its insertion position.
                let value = if let Some(function) = function {
                    Value::Function(function)
                } else if let Some(getter) = getter {
                    self.invoke_function_value(getter, self.with_this_arg(getter, obj, Vec::new()))?
                } else if zun_gc::with_current_heap(|heap| heap.dynamic_is_accessor(obj, &key)) {
                    Value::Void
                } else if let Some(data) = data {
                    zun_gc::with_current_heap(|heap| Value::unbox_any(data, heap))
                        .ok_or(RunError::BadToken)?
                } else {
                    continue;
                };
                out.push((key, value));
            }
            return Ok(out);
        }
        // Typed object: walk the declared-field layout from the base-most type
        // down to the concrete type so keys read in definition order.
        let type_rid = obj.type_rid();
        // A platform interface keeps its state in internal slots, reachable
        // only through its accessors: `Object.getOwnPropertyNames(new URL(..))`
        // is empty. The prelude models those slots as declared fields, which
        // listed them all.
        if self.platform_internal_slots(type_rid) {
            return Ok(Self::wrapper_expandos(obj, true));
        }
        let mut chain = Vec::new();
        let mut current = type_rid;
        loop {
            // A zero rid is not a valid TypeDef (1-based); stop walking rather
            // than underflow on `current - 1`.
            if current == 0 {
                break;
            }
            chain.push(current);
            let ty = self
                .module
                .tables
                .type_defs
                .get(current as usize - 1)
                .ok_or(RunError::BadToken)?;
            if ty.base.is_null() || ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                break;
            }
            current = ty.base.rid();
        }
        // Every TypeDef's field range already includes the inherited fields
        // (base-first), so only the concrete type is walked. The leading fields
        // that belong to a runtime-internal base (the `Error` record slots,
        // `AggregateError.errors`, a `__Builtin*` wrapper's native value) are
        // not own enumerable properties and are skipped.
        let mut hidden_prefix = 0usize;
        for &rid in &chain {
            let ty = self
                .module
                .tables
                .type_defs
                .get(rid as usize - 1)
                .ok_or(RunError::BadToken)?;
            let name = self.module.strings.get(ty.name);
            if name == "Error" || name == "AggregateError" || name.starts_with("__Builtin") {
                hidden_prefix = hidden_prefix.max((ty.field_end - ty.field_start) as usize);
            }
        }
        chain.reverse();
        let mut out = Vec::new();
        for type_rid in chain.into_iter().rev().take(1) {
            let (field_start, field_end) = {
                let ty = self
                    .module
                    .tables
                    .type_defs
                    .get(type_rid as usize - 1)
                    .ok_or(RunError::BadToken)?;
                (ty.field_start, ty.field_end)
            };
            for field_rid in field_start..field_end {
                if ((field_rid - field_start) as usize) < hidden_prefix {
                    continue;
                }
                let field = self
                    .module
                    .tables
                    .field_defs
                    .get(field_rid as usize - 1)
                    .ok_or(RunError::BadToken)?;
                if field.is_static || field.is_private {
                    continue;
                }
                let name = self.module.strings.get(field.name).to_string();
                let layout = self.types.get_field(field_rid).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| {
                    crate::value::load_field(layout.kind, obj, layout.slot, heap)
                });
                out.push((name, value));
            }
        }
        // An assigned `Error.name` is an own enumerable data property.
        if let Some(value) = self.error_assigned_name(obj) {
            out.push(("name".to_string(), value));
        }
        // Expando properties of a typed instance follow its declared fields;
        // canonical integer keys sort first, as for any ordinary object.
        if zun_gc::with_current_heap(|heap| heap.is_instance_bag(obj)) {
            let extras: Vec<(String, Value)> = zun_gc::with_current_heap(|heap| {
                heap.dynamic_keys(obj)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|key| heap.dynamic_is_enumerable(obj, key))
                    .filter_map(|key| {
                        // A function-valued expando rides in the function slot.
                        if let Some(function) = heap.dynamic_get_function(obj, &key) {
                            return Some((key, Value::Function(function)));
                        }
                        heap.dynamic_get(obj, &key)
                            .and_then(|any| Value::unbox_any(any, heap))
                            .map(|value| (key, value))
                    })
                    .collect()
            });
            out.extend(extras);
            let is_index = |key: &str| {
                key.parse::<u32>()
                    .is_ok_and(|n| n.to_string() == key && n != u32::MAX)
            };
            let (mut indexed, rest): (Vec<_>, Vec<_>) =
                out.into_iter().partition(|(key, _)| is_index(key));
            indexed.sort_by_key(|(key, _)| key.parse::<u32>().unwrap_or(0));
            indexed.extend(rest);
            out = indexed;
        }
        Ok(out)
    }

    /// The properties a script stored on the primitive-wrapper instance `obj`,
    /// in insertion order; `enumerable_only` drops the non-enumerable ones.
    fn wrapper_expandos(obj: zun_gc::Obj, enumerable_only: bool) -> Vec<(String, Value)> {
        zun_gc::with_current_heap(|heap| {
            heap.dynamic_keys(obj)
                .unwrap_or_default()
                .into_iter()
                .filter(|key| !enumerable_only || heap.dynamic_is_enumerable(obj, key))
                .map(|key| {
                    let value = match heap.dynamic_get_function(obj, &key) {
                        Some(function) => Value::Function(function),
                        None => heap
                            .dynamic_get(obj, &key)
                            .and_then(|any| Value::unbox_any(any, heap))
                            .unwrap_or(Value::Void),
                    };
                    (key, value)
                })
                .collect()
        })
    }

    /// Own string property names of `obj` (dynamic or typed), including
    /// non-enumerable keys (`getOwnPropertyNames`).
    pub(crate) fn object_own_names(&mut self, obj: zun_gc::Obj) -> Result<Vec<String>, RunError> {
        if let Some(kind) = self.wrapper_instance_kind(obj) {
            let mut names: Vec<String> = Vec::new();
            if kind == "String" {
                let text = self.string_wrapper_text(obj).unwrap_or_default();
                names.extend((0..text.chars().count()).map(|index| index.to_string()));
                names.push("length".to_owned());
            }
            names.extend(Self::wrapper_expandos(obj, false).into_iter().map(|(name, _)| name));
            return Ok(names);
        }
        if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(obj)) {
            let mut names: Vec<String> = zun_gc::with_current_heap(|heap| {
                (0..len)
                    .filter(|&index| !heap.array_get(obj, index).is_some_and(|any| any.is_hole()))
                    .map(|index| index.to_string())
                    .collect()
            });
            names.push("length".to_owned());
            let expandos =
                zun_gc::with_current_heap(|heap| heap.dynamic_keys(obj)).unwrap_or_default();
            names.extend(expandos);
            return Ok(names);
        }
        if zun_gc::with_current_heap(|heap| {
            heap.is_dynamic_object(obj) && !heap.is_instance_bag(obj)
        }) {
            zun_gc::with_current_heap(|heap| heap.dynamic_keys(obj)).ok_or(RunError::BadToken)
        } else {
            let mut names: Vec<String> = self
                .object_own_entries(obj)?
                .into_iter()
                .map(|(name, _)| name)
                .collect();
            // A typed instance that took an expando keeps its declared fields;
            // the bag's non-enumerable keys follow them.
            let bag_keys = zun_gc::with_current_heap(|heap| {
                if heap.is_instance_bag(obj) {
                    heap.dynamic_keys(obj).unwrap_or_default()
                } else {
                    Vec::new()
                }
            });
            for key in bag_keys {
                if !names.contains(&key) {
                    names.push(key);
                }
            }
            for (name, _) in self.error_hidden_own(obj) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            // `name` is assigned after construction, so it follows `message`.
            if let Some(position) = names.iter().position(|n| n == "name") {
                if self.error_assigned_name(obj).is_some() {
                    let name = names.remove(position);
                    names.push(name);
                }
            }
            Ok(names)
        }
    }

    /// Build a descriptor object `{ value, writable, enumerable, configurable }`
    /// or `{ get, set, enumerable, configurable }` for `obj[key]`, or `None` if
    /// there is no own property.
    /// Data descriptor of an own symbol-keyed property (`None` when absent).
    pub(crate) fn build_symbol_descriptor(
        &mut self,
        obj: zun_gc::Obj,
        symbol: zun_core::SymbolId,
    ) -> Option<zun_gc::Obj> {
        zun_gc::with_current_heap_mut(|heap| {
            let getter = heap.dynamic_symbol_getter(obj, symbol);
            let setter = heap.dynamic_symbol_setter(obj, symbol);
            if getter.is_some() || setter.is_some() {
                let out = heap.alloc_dynamic();
                match getter {
                    Some(getter) => {
                        heap.dynamic_set_function(out, "get", getter);
                    }
                    None => {
                        heap.dynamic_set(out, "get".to_owned(), zun_core::Any::void());
                    }
                };
                match setter {
                    Some(setter) => {
                        heap.dynamic_set_function(out, "set", setter);
                    }
                    None => {
                        heap.dynamic_set(out, "set".to_owned(), zun_core::Any::void());
                    }
                };
                let enumerable = heap.dynamic_is_symbol_enumerable(obj, symbol);
                let configurable = heap.dynamic_is_symbol_configurable(obj, symbol);
                heap.dynamic_set(
                    out,
                    "enumerable".to_owned(),
                    zun_core::Any::bool(enumerable),
                );
                heap.dynamic_set(
                    out,
                    "configurable".to_owned(),
                    zun_core::Any::bool(configurable),
                );
                return Some(out);
            }
            if let Some(function) = heap.dynamic_get_symbol_function(obj, symbol) {
                let out = heap.alloc_dynamic();
                heap.dynamic_set_function(out, "value", function);
                let writable = heap.dynamic_is_symbol_writable(obj, symbol);
                let enumerable = heap.dynamic_is_symbol_enumerable(obj, symbol);
                let configurable = heap.dynamic_is_symbol_configurable(obj, symbol);
                heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(writable));
                heap.dynamic_set(
                    out,
                    "enumerable".to_owned(),
                    zun_core::Any::bool(enumerable),
                );
                heap.dynamic_set(
                    out,
                    "configurable".to_owned(),
                    zun_core::Any::bool(configurable),
                );
                return Some(out);
            }
            let value = heap.dynamic_get_symbol(obj, symbol)?;
            let enumerable = heap.dynamic_is_symbol_enumerable(obj, symbol);
            let frozen = !heap.dynamic_can_set(obj, " __symbol_probe");
            let writable = heap.dynamic_is_symbol_writable(obj, symbol);
            let out = heap.alloc_dynamic();
            heap.dynamic_set(out, "value".to_owned(), value);
            heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(writable));
            heap.dynamic_set(
                out,
                "enumerable".to_owned(),
                zun_core::Any::bool(enumerable),
            );
            let configurable = !frozen && heap.dynamic_is_symbol_configurable(obj, symbol);
            heap.dynamic_set(
                out,
                "configurable".to_owned(),
                zun_core::Any::bool(configurable),
            );
            Some(out)
        })
    }

    pub(crate) fn build_descriptor(
        &mut self,
        obj: zun_gc::Obj,
        key: &str,
    ) -> Result<Option<zun_gc::Obj>, RunError> {
        // Arrays: indexed elements and the `length` data property.
        if let Some((len, level)) = zun_gc::with_current_heap(|heap| {
            heap.array_len(obj)
                .map(|len| (len, heap.array_integrity(obj)))
        }) {
            let entry = if key == "length" {
                Some((zun_core::Any::int(len as i64), level < 3, false, false))
            } else {
                match key.parse::<usize>() {
                    Ok(index) if index < len => {
                        zun_gc::with_current_heap(|heap| heap.array_get(obj, index))
                            .map(|value| (value, level < 3, true, level < 2))
                    }
                    _ => None,
                }
            };
            // A non-index expando on an array (e.g. a template object's `raw`)
            // reports through the dynamic-property path below.
            let expando = entry.is_none()
                && key.parse::<usize>().is_err()
                && zun_gc::with_current_heap(|heap| heap.dynamic_descriptor(obj, key).is_some());
            if !expando {
                let Some((value, writable, enumerable, configurable)) = entry else {
                    return Ok(None);
                };
                let out = zun_gc::with_current_heap_mut(|heap| {
                    let out = heap.alloc_dynamic();
                    heap.dynamic_set(out, "value".to_owned(), value);
                    heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(writable));
                    heap.dynamic_set(
                        out,
                        "enumerable".to_owned(),
                        zun_core::Any::bool(enumerable),
                    );
                    heap.dynamic_set(
                        out,
                        "configurable".to_owned(),
                        zun_core::Any::bool(configurable),
                    );
                    out
                });
                return Ok(Some(out));
            }
        }
        let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
        // A typed instance with an expando bag is a dynamic object too, but a declared
        // field lives in its fixed layout: only a key present in the bag is read there.
        let in_dynamic_storage = (is_dynamic
            || zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()))
            && (!zun_gc::with_current_heap(|heap| heap.is_instance_bag(obj))
                || zun_gc::with_current_heap(|heap| heap.dynamic_descriptor(obj, key).is_some()));
        if in_dynamic_storage {
            let Some(descriptor) =
                zun_gc::with_current_heap(|heap| heap.dynamic_descriptor(obj, key))
            else {
                return Ok(None);
            };
            let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
            let set_bool = |name: &str, flag: bool| {
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set(out, name.to_owned(), zun_core::Any::bool(flag));
                });
            };
            match descriptor {
                zun_core::PropertyDescriptor::Data {
                    value,
                    writable,
                    enumerable,
                    configurable,
                } => {
                    zun_gc::with_current_heap_mut(|heap| {
                        // A function-slot property reports its function as the value.
                        match heap.dynamic_get_function(obj, key) {
                            Some(handle) => {
                                heap.dynamic_set_function(out, "value", handle);
                            }
                            None => {
                                heap.dynamic_set(out, "value".to_owned(), value);
                            }
                        }
                    });
                    set_bool("writable", writable);
                    set_bool("enumerable", enumerable);
                    set_bool("configurable", configurable);
                }
                zun_core::PropertyDescriptor::Accessor {
                    get,
                    set,
                    enumerable,
                    configurable,
                } => {
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(handle) = get {
                            heap.dynamic_set_function(out, "get", handle);
                        } else {
                            heap.dynamic_set(out, "get".to_owned(), zun_core::Any::void());
                        }
                        if let Some(handle) = set {
                            heap.dynamic_set_function(out, "set", handle);
                        } else {
                            heap.dynamic_set(out, "set".to_owned(), zun_core::Any::void());
                        }
                    });
                    set_bool("enumerable", enumerable);
                    set_bool("configurable", configurable);
                }
            }
            Ok(Some(out))
        } else {
            // Typed object: a declared field is a writable, enumerable data
            // property (the fixed layout is non-configurable).
            let entries = self.object_own_entries(obj)?;
            let assigned_name = key == "name" && self.error_assigned_name(obj).is_some();
            let (value, enumerable) = match entries.into_iter().find(|(name, _)| name == key) {
                Some((_, value)) => (value, true),
                None => match self
                    .error_hidden_own(obj)
                    .into_iter()
                    .find(|(name, _)| name == key)
                {
                    Some((_, value)) => (value, false),
                    None => return Ok(None),
                },
            };
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
            zun_gc::with_current_heap_mut(|heap| {
                heap.dynamic_set(out, "value".to_owned(), boxed);
                heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(true));
                heap.dynamic_set(
                    out,
                    "enumerable".to_owned(),
                    zun_core::Any::bool(enumerable),
                );
                // Error own data properties are configurable; fixed-layout
                // fields are not.
                heap.dynamic_set(
                    out,
                    "configurable".to_owned(),
                    zun_core::Any::bool(!enumerable || assigned_name),
                );
            });
            Ok(Some(out))
        }
    }

    /// Classify how a reflective operation on `obj` resolves for a given trap
    /// name (see [`ProxyRoute`]). A proxy with the trap routes through it; a
    /// proxy without it falls back to its target; a plain object operates on
    /// itself.
    pub(crate) fn proxy_route(
        &mut self,
        obj: zun_gc::Obj,
        trap: &str,
    ) -> Result<ProxyRoute, RunError> {
        if zun_gc::with_current_heap(|heap| heap.proxy_is_revoked(obj)) {
            return Err(self.revoked_proxy_error(trap));
        }
        Ok(match zun_gc::with_current_heap(|heap| heap.proxy_target(obj)) {
            Some(target) => match zun_gc::with_current_heap(|heap| heap.proxy_trap(obj, trap)) {
                Some(callback) => ProxyRoute::Trap(callback, target),
                None => ProxyRoute::Target(target),
            },
            None => ProxyRoute::NotProxy,
        })
    }

    /// A proxy's `ownKeys` trap result, validated as the specification's
    /// `[[OwnPropertyKeys]]` requires: every entry is a string or a symbol, no
    /// key repeats, and no non-configurable own key of the target is dropped.
    /// `None` when `obj` is not a proxy, or has no such trap.
    ///
    /// One trap call answers for all three reflective surfaces
    /// (`Reflect.ownKeys`, `getOwnPropertyNames`, `getOwnPropertySymbols`),
    /// each keeping the entries it reports. `Reflect.ownKeys` used to compose
    /// the other two and so called the trap twice.
    pub(crate) fn proxy_own_keys_list(
        &mut self,
        obj: zun_gc::Obj,
    ) -> Result<Option<zun_gc::Obj>, RunError> {
        let ProxyRoute::Trap(callback, target) = self.proxy_route(obj, "ownKeys")? else {
            return Ok(None);
        };
        let result = self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
        let list = object(&result).and_then(|list| {
            zun_gc::with_current_heap(|heap| heap.array_len(list)).map(|len| (list, len))
        });
        let Some((list, count)) = list else {
            return Err(RunError::Native(
                "TypeError: proxy ownKeys trap result is not a list".to_string(),
            ));
        };
        let mut names: Vec<String> = Vec::new();
        let mut symbols: Vec<zun_core::SymbolId> = Vec::new();
        for index in 0..count {
            let entry = zun_gc::with_current_heap(|heap| heap.array_get(list, index))
                .ok_or(RunError::BadToken)?;
            let duplicate = if let Some(symbol) = entry.as_symbol() {
                let seen = symbols.contains(&symbol);
                symbols.push(symbol);
                seen
            } else if let Ok(Value::Str(text)) = unbox_arg(&Value::Any(entry)) {
                let name = text.to_string();
                let seen = names.contains(&name);
                names.push(name);
                seen
            } else {
                return Err(RunError::Native(
                    "TypeError: proxy ownKeys trap result entry is not a string or symbol"
                        .to_string(),
                ));
            };
            if duplicate {
                return Err(self.proxy_own_keys_error(
                    "'ownKeys' on proxy: trap returned duplicate entries",
                ));
            }
        }
        // A non-configurable own key of the target cannot be hidden.
        for name in self.object_own_names(target)? {
            if !names.contains(&name)
                && !zun_gc::with_current_heap(|heap| heap.dynamic_can_delete(target, &name))
            {
                return Err(self.proxy_own_keys_error(&format!(
                    "'ownKeys' on proxy: trap result did not include '{name}'"
                )));
            }
        }
        for symbol in zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(target)) {
            if !symbols.contains(&symbol)
                && !zun_gc::with_current_heap(|heap| {
                    heap.dynamic_is_symbol_configurable(target, symbol)
                })
            {
                let label = zun_gc::with_current_heap(|heap| {
                    heap.symbol_description(symbol).unwrap_or_default().to_owned()
                });
                return Err(self.proxy_own_keys_error(&format!(
                    "'ownKeys' on proxy: trap result did not include 'Symbol({label})'"
                )));
            }
        }
        Ok(Some(list))
    }

    /// The own enumerable *symbol* keys of a proxy and their values, through
    /// the same traps as [`Self::proxy_own_entries`]: `ownKeys` offers them,
    /// `getOwnPropertyDescriptor` decides enumerability, `get` reads them.
    /// `None` when `obj` is not a proxy with an `ownKeys` trap.
    ///
    /// `Object.assign` and object spread read a source's symbol store
    /// directly, which for a proxy is empty, so a symbol key the trap offered
    /// was dropped from the copy.
    pub(crate) fn proxy_own_symbol_entries(
        &mut self,
        obj: zun_gc::Obj,
    ) -> Result<Option<Vec<(zun_core::SymbolId, Value)>>, RunError> {
        let Some(list) = self.proxy_own_keys_list(obj)? else {
            return Ok(None);
        };
        let count = zun_gc::with_current_heap(|heap| heap.array_len(list)).unwrap_or(0);
        let symbols: Vec<zun_core::SymbolId> = (0..count)
            .filter_map(|index| zun_gc::with_current_heap(|heap| heap.array_get(list, index)))
            .filter_map(|entry| entry.as_symbol())
            .collect();
        let descriptor = match self.proxy_route(obj, "getOwnPropertyDescriptor")? {
            ProxyRoute::Trap(callback, target) => Some((callback, target)),
            _ => None,
        };
        let mut out: Vec<(zun_core::SymbolId, Value)> = Vec::new();
        for symbol in symbols {
            let enumerable = match descriptor {
                Some((callback, target)) => {
                    let got = self.invoke_proxy_trap(
                        callback,
                        vec![
                            Value::Object(target),
                            Value::Any(zun_core::Any::symbol(symbol)),
                        ],
                    )?;
                    object(&got).is_some_and(|got| {
                        zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get(got, "enumerable")
                                .and_then(|any| Value::unbox_any(any, heap))
                        })
                        .is_some_and(|flag| matches!(flag, Value::Bool(true)))
                    })
                }
                None => zun_gc::with_current_heap(|heap| {
                    heap.proxy_target(obj)
                        .is_some_and(|target| heap.dynamic_is_symbol_enumerable(target, symbol))
                }),
            };
            if !enumerable {
                continue;
            }
            // The descriptor says the key is there, so it is copied even when
            // the read finds nothing: a trapless `get` reads the target, which
            // need not hold a key the `ownKeys` trap invented.
            let value = self
                .proxy_dynamic_get_symbol(obj, symbol)?
                .unwrap_or(Value::Void);
            out.push((symbol, value));
        }
        Ok(Some(out))
    }

    /// A thrown `TypeError` for a broken `ownKeys` invariant.
    fn proxy_own_keys_error(&mut self, message: &str) -> RunError {
        match self.new_error_object("TypeError", message) {
            Some(error) => RunError::Thrown(Value::Object(error)),
            None => RunError::Native(format!("TypeError: {message}")),
        }
    }

    /// The own enumerable string keys of a proxy, and optionally their values,
    /// resolved the way `EnumerableOwnPropertyNames` does: the `ownKeys` trap
    /// supplies the candidate keys and the `getOwnPropertyDescriptor` trap
    /// decides which of them are enumerable. Answers `None` for an object that
    /// is not a proxy, and for a proxy with neither trap -- there its target
    /// answers for it, which [`Self::enumerability_holder`] already arranges.
    ///
    /// Without this, a proxy that *did* define the traps reported no own keys
    /// at all: `Object.keys`, `Object.values`, `Object.entries` and
    /// `JSON.stringify` all read the proxy object's own (empty) property table
    /// and answered as if it held nothing.
    pub(crate) fn proxy_own_entries(
        &mut self,
        proxy: zun_gc::Obj,
        read_values: bool,
    ) -> Result<Option<Vec<(String, Value)>>, RunError> {
        let descriptor = match self.proxy_route(proxy, "getOwnPropertyDescriptor")? {
            ProxyRoute::Trap(callback, target) => Some((callback, target)),
            ProxyRoute::Target(_) => None,
            ProxyRoute::NotProxy => return Ok(None),
        };
        let keys: Vec<String> = match self.proxy_route(proxy, "ownKeys")? {
            ProxyRoute::Trap(callback, target) => {
                let result = self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
                let list = object(&result).and_then(|list| {
                    zun_gc::with_current_heap(|heap| heap.array_len(list)).map(|len| (list, len))
                });
                let Some((list, count)) = list else {
                    return Err(RunError::Native(
                        "TypeError: proxy ownKeys trap result is not a list".to_string(),
                    ));
                };
                (0..count)
                    .filter_map(|index| {
                        zun_gc::with_current_heap(|heap| heap.array_get(list, index))
                    })
                    .filter(|entry| entry.as_symbol().is_none())
                    .filter_map(|entry| match unbox_arg(&Value::Any(entry)) {
                        Ok(Value::Str(text)) => Some(text.to_string()),
                        _ => None,
                    })
                    .collect()
            }
            // No `ownKeys` trap: the candidates are the target's own names,
            // which the descriptor trap then filters.
            ProxyRoute::Target(target) => match descriptor {
                Some(_) => self.object_own_names(target)?,
                None => return Ok(None),
            },
            ProxyRoute::NotProxy => return Ok(None),
        };
        // Without a descriptor trap, enumerability is the target's own.
        let inherent: Vec<String> = match descriptor {
            Some(_) => Vec::new(),
            None => {
                let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy))
                else {
                    return Ok(None);
                };
                self.object_own_entries_impl(target, false)?
                    .into_iter()
                    .map(|(key, _)| key)
                    .collect()
            }
        };
        let mut out: Vec<(String, Value)> = Vec::with_capacity(keys.len());
        for key in keys {
            let enumerable = match descriptor {
                Some((callback, target)) => {
                    let got = self.invoke_proxy_trap(
                        callback,
                        vec![Value::Object(target), Value::Str(key.clone().into())],
                    )?;
                    // An absent `enumerable` field completes to `false`, and a
                    // trap that answers `undefined` reports no such property.
                    object(&got).is_some_and(|got| {
                        zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get(got, "enumerable")
                                .and_then(|any| Value::unbox_any(any, heap))
                        })
                        .is_some_and(|flag| matches!(flag, Value::Bool(true)))
                    })
                }
                None => inherent.contains(&key),
            };
            if !enumerable {
                continue;
            }
            let value = if read_values {
                self.proxy_dynamic_get(proxy, &key)?
            } else {
                Value::Void
            };
            out.push((key, value));
        }
        Ok(Some(out))
    }

    /// The `TypeError` every operation on a revoked proxy throws.
    pub(crate) fn revoked_proxy_error(&mut self, op: &str) -> RunError {
        let message = format!("Cannot perform '{op}' on a proxy that has been revoked");
        match self.new_error_object("TypeError", &message) {
            Some(object) => RunError::Thrown(Value::Object(object)),
            None => RunError::Native(format!("TypeError: {message}")),
        }
    }
}
