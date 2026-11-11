//! Function apply/call/bind/construct native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_function(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "functionApply" => {
                // `f.apply(thisArg, argsArray)`: invoke `f` with `thisArg` as
                // the leading positional argument, spread with the array.
                // A callee carried as `any` (a decorator's method `value`) arrives
                // as a `functionBox` record; restore the callable.
                let callee = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let callee = zun_gc::with_current_heap(|heap| unbox_function_box(callee, heap));
                let target = match callee {
                    Value::Function(handle) => handle,
                    _ => return Err(RunError::BadToken),
                };
                let this_arg = args.get(1).ok_or(RunError::BadToken)?.clone();
                let array =
                    object(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let spread = self.read_array_values(array)?;
                // Argument-marshalling site: restore function values that rode
                // the `Any` lane as `functionBox` wrappers.
                let spread = zun_gc::with_current_heap(|heap| {
                    spread
                        .into_iter()
                        .map(|value| unbox_function_box(value, heap))
                        .collect::<Vec<_>>()
                });
                let mut call_args = Vec::with_capacity(1 + spread.len());
                if self.handle_takes_this(target, 1 + spread.len()) {
                    call_args.push(this_arg);
                }
                call_args.extend(spread);
                self.invoke_function_value(target, call_args)
            }
            "functionCallSpread" => {
                // `f(...xs)` / `f(a, ...xs, b)`: invoke `f` with the array's
                // elements as the positional arguments.
                let callee = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let callee = zun_gc::with_current_heap(|heap| unbox_function_box(callee, heap));
                let target = match callee {
                    Value::Function(handle) => handle,
                    _ => return Err(RunError::BadToken),
                };
                let array =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let spread = self.read_array_values(array)?;
                let call_args = zun_gc::with_current_heap(|heap| {
                    spread
                        .into_iter()
                        .map(|value| unbox_function_box(value, heap))
                        .collect::<Vec<_>>()
                });
                self.invoke_function_value(target, call_args)
            }
            "functionBind" => {
                // `f.bind(thisArg, ...partial)`: allocate a bound function whose
                // prepended arguments are `[thisArg, ...partial]` (the array
                // argument), carrying the target function in a GC-traced record.
                let target = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let array =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let record = zun_gc::with_current_heap_mut(|heap| {
                    let record = heap.alloc_dynamic();
                    heap.dynamic_set_function(record, "__target", target);
                    heap.dynamic_set(
                        record,
                        "__args",
                        zun_core::Any::object(array.addr() as usize),
                    );
                    record
                });
                Ok(Value::Function(zun_core::FunctionHandle::new_bound(
                    record.addr() as usize,
                )))
            }
            "functionConstruct" => self.function_construct(args),
            "functionBoxIs" => {
                // Handle identity between a `functionBox` record and a function.
                let boxed =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let same =
                    zun_gc::with_current_heap(|heap| heap.dynamic_get_function(boxed, "__boxedFn"))
                        .is_some_and(|stored| stored == target);
                Ok(Value::Bool(same))
            }
            "functionBox" => {
                // Wrap a callable in a GC-traced object so it can ride through
                // an `Any`-boxing argument lane. The function is stored in a
                // function slot (its captured environment stays traced); the
                // `__boxedFn` key is the sole marker `unbox_function_box` keys on.
                let target = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let record = zun_gc::with_current_heap_mut(|heap| {
                    let record = heap.alloc_dynamic();
                    heap.dynamic_set_function(record, "__boxedFn", target);
                    record
                });
                Ok(Value::Object(record))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
