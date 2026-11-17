//! `Reflect.construct` on a realm built-in constructor descriptor.
//!
//! `new Array(...)`, `new Map(...)` and friends are lowered statically by sema;
//! `Reflect.construct(Array, [...])` only has the descriptor and an argument
//! list at run time. This module routes each descriptor to the construction
//! `new <Name>(...)` already performs (the same natives and prelude helpers),
//! and answers a `TypeError` for the built-ins that are not constructors.

use super::*;

/// The realm constructors this module recognises, in lookup order.
const BUILTIN_CTORS: &[&str] = &[
    "Array",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "WeakRef",
    "FinalizationRegistry",
    "Promise",
    "RegExp",
    "Object",
    "Symbol",
    "BigInt",
    "Function",
];

impl<'a> Interp<'a> {
    /// The realm global name of the built-in constructor descriptor `token`
    /// among [`BUILTIN_CTORS`], if it is one of them.
    pub(crate) fn builtin_ctor_global_name(token: u32) -> Option<&'static str> {
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            BUILTIN_CTORS.iter().copied().find(|name| {
                heap.dynamic_get(global, name).and_then(|any| any.as_class_descriptor())
                    == Some(token)
            })
        })
    }

    /// `[[Construct]]` of the built-in constructor descriptor `token` with the
    /// argument list `values` (`array` is the same list as a managed array).
    /// `None` when `token` is not one of the built-ins this module knows, so the
    /// caller reports its own "not a constructor".
    pub(super) fn construct_builtin_descriptor(
        &mut self,
        token: u32,
        array: zun_gc::Obj,
        values: Vec<Value>,
    ) -> Option<Result<Value, RunError>> {
        // Buffers and typed arrays have their own dynamic construction.
        if let Some(result) = self.construct_builtin_buffer(token, &values) {
            return Some(result.map(|built| {
                let boxed = zun_gc::with_current_heap_mut(|heap| built.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                Value::Any(boxed)
            }));
        }
        let name = Self::builtin_ctor_global_name(token)?;
        Some(self.construct_named_builtin(name, array, values))
    }

    fn construct_named_builtin(
        &mut self,
        name: &str,
        array: zun_gc::Obj,
        values: Vec<Value>,
    ) -> Result<Value, RunError> {
        // `new Function(p1, ..., body)` through the constructor value. The result
        // is a function, which is returned as is: it does not box into `any`.
        if name == "Function" {
            return self.function_from_strings(values).map(Value::Function);
        }
        let first = values.first().map(unbox_arg).transpose()?;
        let built = match name {
            // The same `__zunArrayCtor` helper `new Array(...)` calls: a single
            // numeric argument is a length, anything else is the element list.
            "Array" => {
                let rid = self.free_function_rid("__zunArrayCtor").ok_or_else(|| {
                    RunError::Native("TypeError: Array is not a constructor".into())
                })?;
                // `call` roots its arguments for the duration of the call.
                self.call(rid, vec![Value::Object(array)], None)?
            }
            "Map" => {
                self.call_managed_collection("mapFromIterable", &[first.unwrap_or(Value::Void)])?
            }
            "Set" => {
                self.call_managed_collection("setFromIterable", &[first.unwrap_or(Value::Void)])?
            }
            "WeakMap" | "WeakSet" => {
                let (empty, from) = if name == "WeakMap" {
                    ("weakMapNew", "__zunWeakMapFrom")
                } else {
                    ("weakSetNew", "__zunWeakSetFrom")
                };
                match first {
                    None | Some(Value::Void) | Some(Value::Null) => {
                        self.call_managed_collection(empty, &[])?
                    }
                    Some(source) => {
                        let rid = self.free_function_rid(from).ok_or_else(|| {
                            RunError::Native(format!("TypeError: {name} is not a constructor"))
                        })?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| source.box_any(heap))
                            .unwrap_or_else(zun_core::Any::null);
                        self.call(rid, vec![Value::Any(boxed)], None)?
                    }
                }
            }
            "WeakRef" => match first {
                Some(target @ (Value::Object(_) | Value::Function(_))) => {
                    self.call_managed_collection("weakRefNew", &[target])?
                }
                _ => return Err(RunError::Native("TypeError: WeakRef: invalid target".into())),
            },
            "FinalizationRegistry" => match first.as_ref().and_then(Self::function_of) {
                Some(callback) => self.call_managed_collection(
                    "finalizationRegistryNew",
                    &[Value::Function(callback)],
                )?,
                None => {
                    return Err(RunError::Native(
                        "TypeError: FinalizationRegistry: cleanup must be callable".into(),
                    ))
                }
            },
            "Promise" => match first.as_ref().and_then(Self::function_of) {
                Some(executor) => self
                    .call_managed_collection("promiseNewExecutor", &[Value::Function(executor)])?,
                None => {
                    let other = first;
                    let shown = other.map_or_else(|| "undefined".to_string(), |v| v.display());
                    return Err(RunError::Native(format!(
                        "TypeError: Promise resolver {shown} is not a function"
                    )));
                }
            },
            "RegExp" => {
                if matches!(first, Some(Value::Object(_) | Value::Function(_))) {
                    return Err(RunError::Native(
                        "TypeError: Reflect.construct(RegExp, [object]) is not supported".into(),
                    ));
                }
                let spell = |value: Option<Value>| -> Option<zun_core::ZStr> {
                    match value {
                        Some(Value::Str(text)) => Some(text),
                        Some(Value::Void) | None => None,
                        Some(other) => Some(other.display().as_str().into()),
                    }
                };
                let source = spell(first).unwrap_or_else(|| "(?:)".into());
                let flags = spell(values.get(1).map(unbox_arg).transpose()?)
                    .unwrap_or_else(|| "".into());
                self.call_managed_collection("regexpNew", &[Value::Str(source), Value::Str(flags)])?
            }
            // `new Object(value)`: ToObject, a fresh ordinary object for
            // `undefined` / `null`, and an object argument returned as is.
            "Object" => match first {
                None | Some(Value::Void) | Some(Value::Null) => {
                    Value::Object(zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic()))
                }
                Some(value) => match self.primitive_wrapper_object(&value)? {
                    Some(wrapper) => Value::Object(wrapper),
                    None => value,
                },
            },
            // `Symbol` and `BigInt` are not constructors.
            _ => {
                return Err(RunError::Native(format!(
                    "TypeError: {name} is not a constructor"
                )))
            }
        };
        let boxed = zun_gc::with_current_heap_mut(|heap| built.box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        Ok(Value::Any(boxed))
    }
}

impl<'a> Interp<'a> {
    /// The function behind a callable value: a function handle itself, or the
    /// managed cell a function takes once it is stored in an `any` slot.
    fn function_of(value: &Value) -> Option<zun_core::FunctionHandle> {
        match value {
            Value::Function(handle) => Some(*handle),
            Value::Object(object) => zun_gc::with_current_heap(|heap| {
                heap.function_of_cell(*object)
                    .or_else(|| heap.dynamic_get_function(*object, "__boxedFn"))
            }),
            _ => None,
        }
    }
}
