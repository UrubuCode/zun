//! ECMAScript property-definition semantics shared by `Object.defineProperty`,
//! `Reflect.defineProperty`, `Reflect.set` and the strict `delete` operator,
//! including the integrity levels of arrays and typed arrays.

use super::collections::same_value;
use super::errors::RunError;
use super::integrity::{array_refusal, ArrayOp};
use super::value::Value;
use super::Interp;

/// An own property key: a string or a symbol.
#[derive(Clone, Debug)]
pub(crate) enum PropKey {
    Str(String),
    Sym(zun_core::SymbolId),
}

fn as_object(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Object(object) => Some(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
        _ => None,
    }
}

/// Decode a property-key argument (string, number or symbol).
pub(crate) fn prop_key(value: &Value) -> Option<PropKey> {
    match value {
        Value::Str(text) => Some(PropKey::Str(text.to_string())),
        Value::Int(n, _) => Some(PropKey::Str(n.to_string())),
        Value::Float(f, _) => Some(PropKey::Str(if f.fract() == 0.0 && f.abs() < 1e15 {
            (*f as i64).to_string()
        } else {
            f.to_string()
        })),
        Value::Any(any) => {
            if let Some(symbol) = any.as_symbol() {
                return Some(PropKey::Sym(symbol));
            }
            if let Some(handle) = any.as_string() {
                return zun_gc::with_current_heap(|heap| {
                    heap.try_string(handle).map(|s| PropKey::Str(s.to_string()))
                });
            }
            if let Some(n) = any.as_int() {
                return Some(PropKey::Str(n.to_string()));
            }
            if let Some(f) = any.as_float() {
                return Some(PropKey::Str(if f.fract() == 0.0 && f.abs() < 1e15 {
                    (f as i64).to_string()
                } else {
                    f.to_string()
                }));
            }
            None
        }
        _ => None,
    }
}

fn number_of(value: &Value) -> Option<f64> {
    match value {
        Value::Int(n, _) => Some(*n as f64),
        Value::Float(f, _) => Some(*f),
        Value::Any(any) => any.as_int().map(|n| n as f64).or_else(|| any.as_float()),
        _ => None,
    }
}

/// A parsed property descriptor object.
struct Desc {
    value: Option<zun_core::Any>,
    writable: Option<bool>,
    enumerable: Option<bool>,
    configurable: Option<bool>,
    has_get: bool,
    get: Option<zun_core::FunctionHandle>,
    has_set: bool,
    set: Option<zun_core::FunctionHandle>,
}

impl Desc {
    fn parse(desc: zun_gc::Obj) -> Desc {
        zun_gc::with_current_heap(|heap| {
            let flag = |name: &str| {
                heap.dynamic_get(desc, name)
                    .map(|any| super::collections::is_truthy(&Value::Any(any), heap))
            };
            let get = heap.dynamic_get_function(desc, "get");
            let set = heap.dynamic_get_function(desc, "set");
            Desc {
                value: heap.dynamic_get(desc, "value"),
                writable: flag("writable"),
                enumerable: flag("enumerable"),
                configurable: flag("configurable"),
                has_get: get.is_some() || heap.dynamic_has(desc, "get"),
                get,
                has_set: set.is_some() || heap.dynamic_has(desc, "set"),
                set,
            }
        })
    }
    fn is_accessor(&self) -> bool {
        self.has_get || self.has_set
    }
    fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }
}

const OBJECT_PROTOTYPE_KEY: &str = "$Object.prototype";

impl Interp<'_> {
    /// The realm's shared `Object.prototype` object: created on first use and
    /// anchored on the realm global object (so it is rooted and relocated with
    /// it).
    pub(crate) fn object_prototype_singleton() -> zun_gc::Obj {
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(existing) = heap.dynamic_get(global, OBJECT_PROTOTYPE_KEY).and_then(|a| a.as_object()) {
                return unsafe { zun_gc::Obj::from_addr(existing as *mut u64) };
            }
            let root = heap.alloc_dynamic();
            let any = zun_core::Any::object(root.addr() as usize);
            heap.dynamic_define(global, OBJECT_PROTOTYPE_KEY, any, false);
            root
        })
    }

    /// Map a prototype argument to the stored link: an explicit `null` is kept
    /// as a `null` link (no prototype at all); the shared `Object.prototype`
    /// is the default link (stored as none).
    pub(crate) fn normalize_prototype(
        prototype: Option<zun_core::Any>,
        arg: &Value,
    ) -> Option<zun_core::Any> {
        let is_null = matches!(arg, Value::Null | Value::Void)
            || matches!(arg, Value::Any(a) if matches!(a.tag(), zun_core::AnyTag::Null | zun_core::AnyTag::Void));
        if is_null {
            return Some(zun_core::Any::null());
        }
        let root = Self::object_prototype_singleton();
        match prototype {
            Some(link) if link.as_object() == Some(root.addr() as usize) => None,
            other => other,
        }
    }

    /// ValidateAndApplyPropertyDescriptor for a dynamic object. Returns whether
    /// the definition was accepted.
    pub(crate) fn define_own_property(
        &self,
        obj: zun_gc::Obj,
        key: &PropKey,
        desc_obj: zun_gc::Obj,
    ) -> bool {
        let desc = Desc::parse(desc_obj);
        if desc.is_accessor() && desc.is_data() {
            return false;
        }
        let name = match key {
            PropKey::Sym(symbol) => {
                // Symbol-keyed data properties carry only an enumerable flag.
                let value = desc.value.unwrap_or_else(zun_core::Any::void);
                let enumerable = desc.enumerable.unwrap_or_else(|| {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_symbol_enumerable(obj, *symbol))
                });
                return zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_define_symbol(obj, *symbol, value, enumerable)
                });
            }
            PropKey::Str(name) => name.as_str(),
        };
        if zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()) {
            return self.define_array_property(obj, name, &desc);
        }
        let current = zun_gc::with_current_heap(|heap| heap.dynamic_descriptor(obj, name));
        let Some(current) = current else {
            if !zun_gc::with_current_heap(|heap| heap.dynamic_is_extensible(obj)) {
                return false;
            }
            let enumerable = desc.enumerable.unwrap_or(false);
            let configurable = desc.configurable.unwrap_or(false);
            return zun_gc::with_current_heap_mut(|heap| {
                if desc.is_accessor() {
                    heap.dynamic_define_accessor(obj, name.to_owned(), desc.get, desc.set, enumerable, configurable)
                } else {
                    heap.dynamic_define_data(
                        obj,
                        name.to_owned(),
                        desc.value.unwrap_or_else(zun_core::Any::void),
                        desc.writable.unwrap_or(false),
                        enumerable,
                        configurable,
                    )
                }
            });
        };
        let (cur_enumerable, cur_configurable) = match &current {
            zun_core::PropertyDescriptor::Data { enumerable, configurable, .. } => (*enumerable, *configurable),
            zun_core::PropertyDescriptor::Accessor { enumerable, configurable, .. } => (*enumerable, *configurable),
        };
        if !cur_configurable {
            if desc.configurable == Some(true) {
                return false;
            }
            if let Some(enumerable) = desc.enumerable {
                if enumerable != cur_enumerable {
                    return false;
                }
            }
            match &current {
                zun_core::PropertyDescriptor::Data { value, writable, .. } => {
                    if desc.is_accessor() {
                        return false;
                    }
                    if !writable {
                        if desc.writable == Some(true) {
                            return false;
                        }
                        if let Some(new_value) = desc.value {
                            let same = zun_gc::with_current_heap(|heap| {
                                match (Value::unbox_any(new_value, heap), Value::unbox_any(*value, heap)) {
                                    (Some(a), Some(b)) => same_value(&a, &b),
                                    _ => new_value == *value,
                                }
                            });
                            if !same {
                                return false;
                            }
                        }
                    }
                }
                zun_core::PropertyDescriptor::Accessor { get, set, .. } => {
                    if desc.is_data() {
                        return false;
                    }
                    if desc.has_get && desc.get != *get {
                        return false;
                    }
                    if desc.has_set && desc.set != *set {
                        return false;
                    }
                }
            }
        }
        let enumerable = desc.enumerable.unwrap_or(cur_enumerable);
        let configurable = desc.configurable.unwrap_or(cur_configurable);
        zun_gc::with_current_heap_mut(|heap| match current {
            zun_core::PropertyDescriptor::Data { value, writable, .. } => {
                if desc.is_accessor() {
                    heap.dynamic_force_define_accessor(obj, name.to_owned(), desc.get, desc.set, enumerable, configurable)
                } else {
                    heap.dynamic_force_define_data(
                        obj,
                        name.to_owned(),
                        desc.value.unwrap_or(value),
                        desc.writable.unwrap_or(writable),
                        enumerable,
                        configurable,
                    )
                }
            }
            zun_core::PropertyDescriptor::Accessor { get, set, .. } => {
                if desc.is_data() {
                    heap.dynamic_force_define_data(
                        obj,
                        name.to_owned(),
                        desc.value.unwrap_or_else(zun_core::Any::void),
                        desc.writable.unwrap_or(false),
                        enumerable,
                        configurable,
                    )
                } else {
                    heap.dynamic_force_define_accessor(
                        obj,
                        name.to_owned(),
                        if desc.has_get { desc.get } else { get },
                        if desc.has_set { desc.set } else { set },
                        enumerable,
                        configurable,
                    )
                }
            }
        })
    }

    /// `Object.defineProperty` on an array: only data descriptors that keep the
    /// element/`length` shape (value updates) are supported.
    fn define_array_property(&self, array: zun_gc::Obj, name: &str, desc: &Desc) -> bool {
        let (len, level) = zun_gc::with_current_heap(|heap| {
            (heap.array_len(array).unwrap_or(0), heap.array_integrity(array))
        });
        if desc.is_accessor() {
            return false;
        }
        if name == "length" {
            if let Some(value) = desc.value {
                let wanted = value.as_int().map(|n| n as f64).or_else(|| value.as_float());
                if let Some(wanted) = wanted {
                    let wanted = wanted as usize;
                    if wanted != len {
                        let op = if wanted < len { ArrayOp::Remove } else { ArrayOp::Write };
                        if array_refusal(level, op).is_some() {
                            return false;
                        }
                        zun_gc::with_current_heap_mut(|heap| heap.array_resize(array, wanted));
                    }
                }
            }
            if desc.writable == Some(false) {
                zun_gc::with_current_heap_mut(|heap| heap.array_raise_integrity(array, 3));
            }
            return true;
        }
        match name.parse::<usize>() {
            Ok(index) if index < len => {
                if let Some(value) = desc.value {
                    if array_refusal(level, ArrayOp::Write).is_some() {
                        return false;
                    }
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(array, index, value));
                }
                true
            }
            Ok(index) => {
                if array_refusal(level, ArrayOp::Add).is_some() {
                    return false;
                }
                zun_gc::with_current_heap_mut(|heap| {
                    heap.array_resize(array, index + 1);
                    heap.array_set(array, index, desc.value.unwrap_or_else(zun_core::Any::void));
                });
                true
            }
            Err(_) => false,
        }
    }

    /// `Reflect.set(target, key, value)`: a boolean success. Honors
    /// writability, extensibility, array integrity, and runs accessor setters.
    pub(crate) fn reflect_set(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let Some(target) = args.first().and_then(as_object) else {
            return Ok(Value::Bool(false));
        };
        let Some(key) = args.get(1).and_then(prop_key) else {
            return Ok(Value::Bool(false));
        };
        let value_arg = args.get(2).ok_or(RunError::BadToken)?;
        // Arrays: elements and `length`.
        let array_info = zun_gc::with_current_heap(|heap| {
            heap.array_len(target).map(|len| (len, heap.array_integrity(target)))
        });
        if let Some((len, level)) = array_info {
            let PropKey::Str(name) = &key else {
                return Ok(Value::Bool(false));
            };
            let any = zun_gc::with_current_heap_mut(|heap| value_arg.box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            if name == "length" {
                let Some(wanted) = number_of(value_arg) else {
                    return Ok(Value::Bool(false));
                };
                let wanted = wanted as usize;
                if wanted == len {
                    return Ok(Value::Bool(true));
                }
                let op = if wanted < len { ArrayOp::Remove } else { ArrayOp::Write };
                if array_refusal(level, op).is_some() {
                    return Ok(Value::Bool(false));
                }
                zun_gc::with_current_heap_mut(|heap| heap.array_resize(target, wanted));
                return Ok(Value::Bool(true));
            }
            let Ok(index) = name.parse::<usize>() else {
                return Ok(Value::Bool(false));
            };
            let op = if index < len { ArrayOp::Write } else { ArrayOp::Add };
            if array_refusal(level, op).is_some() {
                return Ok(Value::Bool(false));
            }
            zun_gc::with_current_heap_mut(|heap| {
                if index >= len {
                    heap.array_resize(target, index + 1);
                }
                heap.array_set(target, index, any);
            });
            return Ok(Value::Bool(true));
        }
        match key {
            PropKey::Sym(symbol) => {
                let any = zun_gc::with_current_heap_mut(|heap| value_arg.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                let ok = zun_gc::with_current_heap_mut(|heap| {
                    let exists = heap.dynamic_get_symbol(target, symbol).is_some();
                    if exists || heap.dynamic_is_extensible(target) {
                        heap.dynamic_set_symbol(target, symbol, any);
                        true
                    } else {
                        false
                    }
                });
                Ok(Value::Bool(ok))
            }
            PropKey::Str(name) => {
                let (is_accessor, setter) = zun_gc::with_current_heap(|heap| {
                    (
                        heap.dynamic_is_accessor(target, &name),
                        heap.dynamic_accessor_setter(target, &name),
                    )
                });
                if is_accessor {
                    let Some(setter) = setter else {
                        return Ok(Value::Bool(false));
                    };
                    let any = zun_gc::with_current_heap_mut(|heap| value_arg.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    let arg = zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                        .unwrap_or(Value::Any(any));
                    self.invoke_function_value(setter, vec![arg])?;
                    return Ok(Value::Bool(true));
                }
                if !zun_gc::with_current_heap(|heap| heap.dynamic_can_set(target, &name)) {
                    return Ok(Value::Bool(false));
                }
                if let Value::Function(handle) = value_arg {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set_function(target, name, *handle);
                    });
                    return Ok(Value::Bool(true));
                }
                let any = zun_gc::with_current_heap_mut(|heap| value_arg.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set(target, name, any);
                });
                Ok(Value::Bool(true))
            }
        }
    }

    /// `Object.defineProperty(obj, key, desc)` / `Reflect.defineProperty`.
    /// `strict` throws a `TypeError` on refusal and returns the object;
    /// otherwise a boolean.
    pub(crate) fn define_property_native(
        &mut self,
        args: &[Value],
        strict: bool,
    ) -> Result<Value, RunError> {
        let Some(target) = args.first().and_then(as_object) else {
            return Err(RunError::Native(
                "TypeError: Object.defineProperty called on non-object".into(),
            ));
        };
        let Some(key) = args.get(1).and_then(prop_key) else {
            return Err(RunError::BadToken);
        };
        let Some(desc) = args.get(2).and_then(as_object) else {
            return Err(RunError::Native(
                "TypeError: Property description must be an object".into(),
            ));
        };
        let ok = self.define_own_property(target, &key, desc);
        if strict {
            if !ok {
                let shown = match &key {
                    PropKey::Str(name) => name.clone(),
                    PropKey::Sym(_) => "Symbol()".to_owned(),
                };
                return Err(RunError::Native(format!(
                    "TypeError: Cannot redefine property: {shown}"
                )));
            }
            Ok(Value::Object(target))
        } else {
            Ok(Value::Bool(ok))
        }
    }

    /// The strict-mode `delete obj.key` operator: a `TypeError` when the
    /// property is non-configurable, otherwise `true`.
    pub(crate) fn delete_strict(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let Some(target) = args.first().and_then(as_object) else {
            return Ok(Value::Bool(true));
        };
        let Some(PropKey::Str(name)) = args.get(1).and_then(prop_key) else {
            return Ok(Value::Bool(true));
        };
        if zun_gc::with_current_heap(|heap| heap.array_len(target).is_some()) {
            return Ok(Value::Bool(true));
        }
        let allowed = zun_gc::with_current_heap(|heap| heap.dynamic_can_delete(target, &name));
        if !allowed {
            return Err(RunError::Native(format!(
                "TypeError: Cannot delete property '{name}' of #<Object>"
            )));
        }
        zun_gc::with_current_heap_mut(|heap| heap.dynamic_delete(target, &name));
        Ok(Value::Bool(true))
    }

    /// Apply an integrity `level` (1 non-extensible, 2 seal, 3 freeze) to a
    /// non-dynamic object: arrays carry a level, a non-empty typed array cannot
    /// be frozen. Returns whether the object was handled.
    pub(crate) fn apply_nondynamic_integrity(
        &self,
        obj: zun_gc::Obj,
        level: u8,
    ) -> Result<bool, RunError> {
        if zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()) {
            zun_gc::with_current_heap_mut(|heap| heap.array_raise_integrity(obj, level));
            return Ok(true);
        }
        if let Some(len) = crate::typed_arrays::element_len(obj) {
            if level == 3 && len > 0 {
                return Err(RunError::Native(
                    "TypeError: Cannot freeze array buffer views with elements".into(),
                ));
            }
            return Ok(true);
        }
        Ok(false)
    }
}
