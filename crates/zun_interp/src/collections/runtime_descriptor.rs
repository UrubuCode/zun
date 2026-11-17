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

    /// The `TypeError` every operation on a revoked proxy throws.
    pub(crate) fn revoked_proxy_error(&mut self, op: &str) -> RunError {
        let message = format!("Cannot perform '{op}' on a proxy that has been revoked");
        match self.new_error_object("TypeError", &message) {
            Some(object) => RunError::Thrown(Value::Object(object)),
            None => RunError::Native(format!("TypeError: {message}")),
        }
    }
}
