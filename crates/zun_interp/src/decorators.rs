//! Interpreter floor for the NUCLEUS #25 decorator residuals: method
//! replacement through the `class_named_methods` side table, per-instance
//! initializers and field transforms, and `context.access` get/set.
//!
//! Every function value kept in these side tables may carry a closure
//! environment (a managed object), so the tables are rooted and rewritten by
//! [`Interp::append_decorator_roots`] / [`Interp::rewrite_decorator_tables`].

use super::{Interp, RunError, Value};
use zun_core::FunctionHandle;
use zun_gc::Obj;

/// Field slot sentinel meaning "the decorated element is a method".
const METHOD_SLOT: i64 = -1;

fn number(value: &Value) -> Result<i64, RunError> {
    match value {
        Value::Int(value, _) => Ok(*value),
        Value::Float(value, _) => Ok(*value as i64),
        Value::Any(any) => {
            match zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)) {
                Some(Value::Int(value, _)) => Ok(value),
                Some(Value::Float(value, _)) => Ok(value as i64),
                _ => Err(RunError::BadToken),
            }
        }
        _ => Err(RunError::BadToken),
    }
}

fn object(value: &Value) -> Result<Obj, RunError> {
    match value {
        Value::Object(object) => Ok(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { Obj::from_addr(address as *mut u64) })
            .ok_or(RunError::BadToken),
        _ => Err(RunError::BadToken),
    }
}

fn string(value: &Value) -> Result<String, RunError> {
    match value {
        Value::Str(name) => Ok(name.to_string()),
        _ => Err(RunError::BadToken),
    }
}

fn rewrite_handle(handle: &mut FunctionHandle, relocation: &zun_gc::RelocationMap) -> usize {
    let Some(old) = handle.environment() else { return 0 };
    let new = relocation.relocate_address(old);
    if old == new {
        return 0;
    }
    *handle = handle.relocate_environment(Some(new));
    1
}

fn root_handle(handle: FunctionHandle, roots: &mut Vec<Obj>) {
    super::roots::collect_frame_roots(&[Value::Function(handle)], &[], roots);
}

impl Interp<'_> {
    /// Root every function value held by a decorator side table (queued class
    /// initializers, replaced/registered methods, instance initializers, field
    /// transforms), so a collection cannot free a closure environment.
    pub(super) fn append_decorator_roots(&self, roots: &mut Vec<Obj>) {
        for handle in &self.decorator_initializers {
            root_handle(*handle, roots);
        }
        for handle in self.class_named_methods.values() {
            root_handle(*handle, roots);
        }
        for (_, handle) in &self.instance_initializers {
            root_handle(*handle, roots);
        }
        for (_, _, handle) in &self.field_transforms {
            root_handle(*handle, roots);
        }
    }

    /// Rewrite the closure environments held by the decorator side tables
    /// after a moving collection.
    pub(super) fn rewrite_decorator_tables(&mut self, relocation: &zun_gc::RelocationMap) -> usize {
        rewrite_decorator_fields(
            &mut self.decorator_initializers,
            &mut self.class_named_methods,
            &mut self.instance_initializers,
            &mut self.field_transforms,
            relocation,
        )
    }

    /// The entry for `name` on `type_rid`, walking base classes and finally the
    /// declaring class `decl`.
    fn method_entry(&self, type_rid: u32, decl: u32, name: &str) -> Option<FunctionHandle> {
        let mut current = Some(type_rid);
        while let Some(rid) = current {
            if let Some(handle) = self.class_named_methods.get(&(rid, name.to_string())) {
                return Some(*handle);
            }
            current = self.types.get(rid).and_then(|descriptor| {
                if descriptor.base.is_null() {
                    None
                } else {
                    Some(descriptor.base.rid())
                }
            });
        }
        self.class_named_methods.get(&(decl, name.to_string())).copied()
    }

    /// Bind `this` as the leading argument of `target` (a bound-function record
    /// identical to the one `functionBind` builds).
    fn bind_this(target: FunctionHandle, this: Obj) -> FunctionHandle {
        let record = zun_gc::with_current_heap_mut(|heap| {
            let array = heap.alloc_array(1);
            heap.array_set(array, 0, zun_core::Any::object(this.addr() as usize));
            let record = heap.alloc_dynamic();
            heap.dynamic_set_function(record, "__target", target);
            heap.dynamic_set(record, "__args", zun_core::Any::object(array.addr() as usize));
            record
        });
        FunctionHandle::new_bound(record.addr() as usize)
    }

    fn function_box(handle: FunctionHandle) -> Value {
        zun_gc::with_current_heap_mut(|heap| {
            let record = heap.alloc_dynamic();
            heap.dynamic_set_function(record, "__boxedFn", handle);
            Value::Any(zun_core::Any::object(record.addr() as usize))
        })
    }

    /// Run the field transforms and instance initializers registered for the
    /// class of `obj` (and its bases) on a freshly constructed instance.
    pub(super) fn run_instance_hooks(&mut self, start: Obj) -> Result<Obj, RunError> {
        // The instance is rooted (and rewritten on a moving collection) for the
        // whole run, because transforms and initializers re-enter the interpreter.
        let roots = [start];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let current = || unsafe { std::ptr::read_volatile(&roots[0]) };
        let type_rid = start.type_rid();
        let transforms: Vec<(u32, u32, FunctionHandle)> = self
            .field_transforms
            .iter()
            .filter(|(class, _, _)| self.types.is_assignable(type_rid, *class))
            .copied()
            .collect();
        for (_, slot, transform) in transforms {
            let (field_start, count) =
                self.type_field_range(zun_metadata::Token::new(zun_metadata::TableKind::TypeDef, type_rid))?;
            if slot as usize >= count {
                return Err(RunError::BadToken);
            }
            let kind = self.field_kind_at(field_start + slot)?;
            let initial = zun_gc::with_current_heap(|heap| {
                crate::value::load_field(kind, current(), slot as usize, heap)
            });
            let result = self.invoke_function_value(transform, vec![initial])?;
            let result = match result {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                    .ok_or(RunError::BadToken)?,
                other => other,
            };
            zun_gc::with_current_heap_mut(|heap| {
                crate::value::store_field(kind, current(), slot as usize, &result, heap);
            });
        }
        let initializers: Vec<FunctionHandle> = self
            .instance_initializers
            .iter()
            .filter(|(class, _)| self.types.is_assignable(type_rid, *class))
            .map(|(_, handle)| *handle)
            .collect();
        for initializer in initializers {
            self.invoke_function_value(initializer, vec![Value::Object(current())])?;
        }
        Ok(current())
    }

    pub(super) fn call_decorator_native(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        Some(match name {
            "classReplaceMethod" => self.native_class_replace_method(args),
            "classBoundMethod" => self.native_class_bound_method(args),
            "classMethodBox" => self.native_class_method_box(args),
            "classSetFieldTransform" => self.native_class_set_field_transform(args),
            "classInstanceInit" => self.native_class_instance_init(args),
            "decoratorAddInstanceInitializer" => self.native_add_instance_initializer(args),
            "decoratorAccessGet" => self.native_access_get(args),
            "decoratorAccessSet" => self.native_access_set(args),
            _ => return None,
        })
    }

    fn arg<'v>(args: &'v [Value], index: usize) -> Result<&'v Value, RunError> {
        args.get(index).ok_or(RunError::BadToken)
    }

    fn native_class_replace_method(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let class = number(Self::arg(args, 0)?)? as u32;
        let name = string(Self::arg(args, 1)?)?;
        // A decorator that returned no function leaves the method untouched.
        if let Value::Function(handle) = Self::arg(args, 2)? {
            self.class_named_methods.insert((class, name), *handle);
        }
        Ok(Value::Void)
    }

    fn native_class_bound_method(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let receiver = object(Self::arg(args, 0)?)?;
        let decl = number(Self::arg(args, 1)?)? as u32;
        let name = string(Self::arg(args, 2)?)?;
        let target = self
            .method_entry(receiver.type_rid(), decl, &name)
            .ok_or(RunError::BadToken)?;
        Ok(Value::Function(Self::bind_this(target, receiver)))
    }

    fn native_class_method_box(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let class = number(Self::arg(args, 0)?)? as u32;
        let name = string(Self::arg(args, 1)?)?;
        match self.class_named_methods.get(&(class, name)).copied() {
            Some(handle) => Ok(Self::function_box(handle)),
            None => Ok(Value::Any(zun_core::Any::null())),
        }
    }

    fn native_class_set_field_transform(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let class = number(Self::arg(args, 0)?)? as u32;
        let slot = number(Self::arg(args, 1)?)? as u32;
        if let Value::Function(handle) = Self::arg(args, 2)? {
            self.field_transforms.push((class, slot, *handle));
        }
        Ok(Value::Void)
    }

    fn native_class_instance_init(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let obj = object(Self::arg(args, 0)?)?;
        let obj = self.run_instance_hooks(obj)?;
        Ok(Value::Any(zun_core::Any::object(obj.addr() as usize)))
    }

    fn native_add_instance_initializer(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let class = number(Self::arg(args, 0)?)? as u32;
        match Self::arg(args, 1)? {
            Value::Function(handle) => self.instance_initializers.push((class, *handle)),
            _ => return Err(RunError::BadToken),
        }
        Ok(Value::Void)
    }

    fn native_access_get(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let target = object(Self::arg(args, 0)?)?;
        let class = number(Self::arg(args, 1)?)? as u32;
        let slot = number(Self::arg(args, 2)?)?;
        let name = string(Self::arg(args, 3)?)?;
        if slot == METHOD_SLOT {
            let method = self
                .method_entry(target.type_rid(), class, &name)
                .ok_or(RunError::BadToken)?;
            return Ok(Self::function_box(Self::bind_this(method, target)));
        }
        let type_rid = target.type_rid();
        let (field_start, count) = self
            .type_field_range(zun_metadata::Token::new(zun_metadata::TableKind::TypeDef, type_rid))?;
        if slot < 0 || slot as usize >= count {
            return Err(RunError::BadToken);
        }
        let kind = self.field_kind_at(field_start + slot as u32)?;
        let value = zun_gc::with_current_heap(|heap| {
            crate::value::load_field(kind, target, slot as usize, heap)
        });
        zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
            .map(Value::Any)
            .ok_or(RunError::BadToken)
    }

    fn native_access_set(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let target = object(Self::arg(args, 0)?)?;
        let slot = number(Self::arg(args, 2)?)?;
        let value = match Self::arg(args, 4)? {
            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                .ok_or(RunError::BadToken)?,
            other => other.clone(),
        };
        let type_rid = target.type_rid();
        let (field_start, count) = self
            .type_field_range(zun_metadata::Token::new(zun_metadata::TableKind::TypeDef, type_rid))?;
        if slot < 0 || slot as usize >= count {
            return Err(RunError::BadToken);
        }
        let kind = self.field_kind_at(field_start + slot as u32)?;
        zun_gc::with_current_heap_mut(|heap| {
            crate::value::store_field(kind, target, slot as usize, &value, heap);
        });
        Ok(Value::Void)
    }
}

/// Field-level form of [`Interp::rewrite_decorator_tables`], usable from
/// closures that already borrow other `Interp` fields.
pub(super) fn rewrite_decorator_fields(
    initializers: &mut [FunctionHandle],
    named_methods: &mut std::collections::HashMap<(u32, String), FunctionHandle>,
    instance_initializers: &mut [(u32, FunctionHandle)],
    field_transforms: &mut [(u32, u32, FunctionHandle)],
    relocation: &zun_gc::RelocationMap,
) -> usize {
    let mut rewritten = 0;
    for handle in initializers.iter_mut() {
        rewritten += rewrite_handle(handle, relocation);
    }
    for handle in named_methods.values_mut() {
        rewritten += rewrite_handle(handle, relocation);
    }
    for (_, handle) in instance_initializers.iter_mut() {
        rewritten += rewrite_handle(handle, relocation);
    }
    for (_, _, handle) in field_transforms.iter_mut() {
        rewritten += rewrite_handle(handle, relocation);
    }
    rewritten
}
