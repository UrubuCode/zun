//! Class construction from descriptors, proxy trap invocation, and promise settler records.
//!
//! Methods moved verbatim out of the collection dispatch module.

use super::*;

impl<'a> Interp<'a> {
    /// Invoke a proxy handler trap, threading its closure environment (if any)
    /// as the leading positional slot and appending `leading` (the target and
    /// the trap-specific operands). Mirrors the dispatch convention of the
    /// `get`/`set`/`has`/`deleteProperty` traps.
    /// Construct an instance of a user class from a runtime class descriptor
    /// token (`USER_CLASS_FLAG | TypeDef rid`) and positional field values: the
    /// runtime twin of `NewObj` (which `new C(...)` lowers to). The value count
    /// may be shorter than the class's field count: trailing slots take the
    /// literal initializer default recorded by `classSetFieldDefault` (non-literal
    /// initializers are not reachable here and make the call fail).
    /// Design boundary: only typed classes are constructible (no
    /// `function.prototype` + `this` model for arbitrary function constructors),
    /// and `newTarget` is approximated by the target class.
    /// Returns the instance boxed as an `Any`.
    pub(crate) fn construct_class_from_descriptor(
        &mut self,
        token: u32,
        mut values: Vec<Value>,
    ) -> Result<Value, RunError> {
        if token & zun_natives::USER_CLASS_FLAG == 0 {
            return Err(RunError::Native("TypeError: value is not a constructor".into()));
        }
        let token = zun_metadata::Token::new(
            zun_metadata::TableKind::TypeDef,
            token & !zun_natives::USER_CLASS_FLAG,
        );
        let (field_start, field_count) = self.type_field_range(token)?;
        // A class with a `constructor` (own or inherited): allocate the instance
        // with its recorded defaults and run the constructor on it.
        if let Some(ctor) = self.class_constructor(token.rid()) {
            let kinds: Vec<u8> = (0..field_count as u32)
                .map(|i| self.field_kind_at(field_start + i))
                .collect::<Result<_, _>>()?;
            // Slots without a recorded default stay zeroed (0, false, null, undefined).
            let defaults: Vec<Option<Value>> = (0..field_count)
                .map(|slot| {
                    self.class_field_defaults
                        .get(&(token.rid(), slot as u32))
                        .cloned()
                })
                .collect();
            let obj = zun_gc::with_current_heap_mut(|h| {
                let obj = h.alloc(token.rid(), field_count);
                for (i, value) in defaults.iter().enumerate() {
                    if let Some(value) = value.as_ref().filter(|v| {
                        !matches!(v, Value::Void) || kinds[i] == crate::value::FIELD_ANY
                    }) {
                        crate::value::store_field(kinds[i], obj, i, value, h);
                    }
                }
                obj
            });
            self.init_builtin_collection_field(obj);
            let mut call_args = Vec::with_capacity(values.len() + 1);
            call_args.push(Value::Object(obj));
            call_args.extend(values);
            let call_args = self.pack_rest_call_args(ctor, call_args);
            let mut call_args = call_args;
            // Omitted trailing parameters carry the null sentinel that the
            // callee's default-parameter prologue consumes.
            let declared = self
                .module
                .tables
                .method_defs
                .get(ctor as usize - 1)
                .and_then(|method| read_body(&self.module.code, method.body_offset).ok())
                .map_or(call_args.len(), |body| body.param_count as usize);
            while call_args.len() < declared {
                call_args.push(Value::Null);
            }
            self.call(ctor, call_args, None)?;
            let obj = self.run_instance_hooks(obj)?;
            let boxed = zun_gc::with_current_heap_mut(|heap| Value::Object(obj).box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            return Ok(Value::Any(boxed));
        }
        if values.len() > field_count {
            return Err(RunError::BadToken);
        }
        // Trailing absent args take the class's literal field-initializer
        // default (registered by `classSetFieldDefault`); an absent arg with no
        // recorded default is an error.
        for slot in values.len()..field_count {
            let default = self
                .class_field_defaults
                .get(&(token.rid(), slot as u32))
                .cloned()
                .ok_or(RunError::BadToken)?;
            values.push(default);
        }
        let kinds: Vec<u8> = (0..field_count as u32)
            .map(|i| self.field_kind_at(field_start + i))
            .collect::<Result<_, _>>()?;
        let obj = zun_gc::with_current_heap_mut(|h| {
            let obj = h.alloc(token.rid(), field_count);
            for (i, value) in values.iter().enumerate() {
                crate::value::store_field(kinds[i], obj, i, value, h);
            }
            obj
        });
        let obj = self.run_instance_hooks(obj)?;
        let boxed = zun_gc::with_current_heap_mut(|heap| Value::Object(obj).box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        Ok(Value::Any(boxed))
    }

    pub(crate) fn invoke_proxy_trap(
        &mut self,
        callback: zun_core::FunctionHandle,
        leading: Vec<Value>,
    ) -> Result<Value, RunError> {
        let mut call_args = Vec::with_capacity(leading.len() + 1);
        if let Some(address) = callback.environment() {
            call_args.push(Value::Object(unsafe {
                zun_gc::Obj::from_addr(address as *mut u64)
            }));
        }
        call_args.extend(leading);
        self.call(callback.method_rid(), call_args, None)
    }

    /// Read every element of a managed array, unboxing each `Any` slot to its
    /// concrete interpreter value.
    pub(crate) fn read_array_values(&self, array: zun_gc::Obj) -> Result<Vec<Value>, RunError> {
        zun_gc::with_current_heap(|heap| {
            let length = heap.array_len(array).ok_or(RunError::BadToken)?;
            let mut values = Vec::with_capacity(length);
            for index in 0..length {
                let any = heap.array_get(array, index).ok_or(RunError::BadToken)?;
                values.push(Value::unbox_any(any, heap).ok_or(RunError::BadToken)?);
            }
            Ok(values)
        })
    }

    /// Resolve a function value and positional arguments to the concrete target
    /// method RID and the final argument vector, peeling a `bind` chain (each
    /// bound record prepends its recorded `[thisArg, ...partial]`) and threading
    /// the target closure environment (if any) as the leading slot. Shared by
    /// `apply`, `bind`-chain calls, and the bound-function path of `CallIndirect`.
    /// Build a `Promise.withResolvers` settler: a bound-function handle over a
    /// record naming the promise and the settle mode. It is callable like any
    /// function value, but is dispatched natively (see [`Self::settler_of`] /
    /// the interception in [`Self::invoke_function_value`] and `CallIndirect`),
    /// not through a method body.
    pub(crate) fn make_promise_settler(
        &mut self,
        promise: zun_gc::Obj,
        mode: f64,
    ) -> zun_core::FunctionHandle {
        let roots = [promise];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let record = zun_gc::with_current_heap_mut(|heap| {
            let record = heap.alloc_dynamic();
            heap.dynamic_set(record, "__settleMode", zun_core::Any::float(mode));
            heap.dynamic_set(
                record,
                "__settlePromise",
                zun_core::Any::object(roots[0].addr() as usize),
            );
            record
        });
        zun_core::FunctionHandle::new_bound(record.addr() as usize)
    }

    /// If `handle` is a `withResolvers` settler, return the promise it settles
    /// and its mode (`SETTLE_RESOLVE` / `SETTLE_REJECT`). A settler is a bound
    /// handle whose record carries `__settleMode`; an ordinary `bind` record
    /// carries `__target` instead, so the two never collide.
    pub(crate) fn settler_of(
        &self,
        handle: zun_core::FunctionHandle,
    ) -> Option<(zun_gc::Obj, f64)> {
        if !handle.is_bound() {
            return None;
        }
        let record = unsafe { zun_gc::Obj::from_addr(handle.environment()? as *mut u64) };
        zun_gc::with_current_heap(|heap| {
            let mode = heap.dynamic_get(record, "__settleMode")?.as_float()?;
            let promise = heap.dynamic_get(record, "__settlePromise")?.as_object()?;
            let promise = unsafe { zun_gc::Obj::from_addr(promise as *mut u64) };
            Some((promise, mode))
        })
    }

    /// Run a `withResolvers` settler: `resolve(v)` fulfils the promise with `v`;
    /// `reject(e)` rejects it with `e` (a first-class reason when `e` is an
    /// object/boxed value, else its string message). Returns `undefined`, like
    /// the JS resolver functions.
    pub(crate) fn invoke_promise_settler(
        &mut self,
        promise: zun_gc::Obj,
        mode: f64,
        mut args: Vec<Value>,
    ) -> Result<Value, RunError> {
        if mode >= 100.0 {
            // An `Object.prototype` method value: `valueOf` answers the receiver
            // object, the predicates answer false, the string forms the tag.
            return Ok(match mode as u32 - 100 {
                2 => Value::Object(promise),
                3..=5 => Value::Bool(false),
                _ => Value::Str("[object Object]".into()),
            });
        }
        let value = if args.is_empty() {
            Value::Void
        } else {
            args.remove(0)
        };
        if mode == SETTLE_REJECT {
            match &value {
                // An object/boxed reason is carried first-class; a plain string
                // keeps the string surface.
                Value::Str(message) => {
                    self.settle_promise_reject(promise, message.to_string());
                }
                other => {
                    let reason = zun_gc::with_current_heap_mut(|heap| other.clone().box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    let message = value_to_error_string(other);
                    self.settle_promise_reject_value(promise, reason, message);
                }
            }
        } else {
            // Resolving with a Promise adopts it (chaining, not wrapping): the
            // settled promise tracks the adopted one. Any other value fulfils
            // directly. Generic (non-Promise) thenables are not yet detected —
            // only managed Promise adoption is handled here.
            self.settle_target_with(promise, value);
        }
        Ok(Value::Void)
    }

    /// Whether a function value receives the `this` argument of `apply` /
    /// `bind` as its leading parameter. Receivers are positional for an
    /// unbound `@this` closure, a class instance method and a receiver-taking
    /// built-in. A plain function keeps the positional receiver only when the
    /// full argument list (`total_args`, receiver included) still fits its
    /// declared parameters; otherwise the receiver is dropped, as in
    /// ECMAScript. A bound function never takes a receiver.
    pub(crate) fn handle_takes_this(
        &self,
        target: zun_core::FunctionHandle,
        total_args: usize,
    ) -> bool {
        if target.is_bound() {
            return false;
        }
        if target.builtin_method().is_some_and(|kind| kind >= 16) {
            return true;
        }
        let rid = target.method_rid();
        let Some(method) = self
            .module
            .tables
            .method_defs
            .get((rid as usize).wrapping_sub(1))
        else {
            return false;
        };
        let name = self.module.strings.get(method.name);
        let class_member = self
            .module
            .tables
            .type_defs
            .iter()
            .any(|def| (def.method_start..def.method_end).contains(&rid));
        if name.contains("@this") || (class_member && !method.is_static && !name.starts_with('<')) {
            return true;
        }
        let Some(body) = read_body(&self.module.code, method.body_offset).ok() else {
            return false;
        };
        let declared =
            (body.param_count as usize).saturating_sub(usize::from(target.environment().is_some()));
        let rest = name.ends_with("@rest>")
            || (method.signature.0 != 0 && self.module.blobs.get(method.signature) == b"R");
        if rest {
            // A leading fixed parameter before the rest can hold the receiver.
            return declared >= 2;
        }
        total_args <= declared
    }

    pub(crate) fn resolve_call_target(
        &self,
        mut handle: zun_core::FunctionHandle,
        mut call_args: Vec<Value>,
    ) -> Result<(u32, Vec<Value>), RunError> {
        while handle.is_bound() {
            let record = unsafe {
                zun_gc::Obj::from_addr(handle.environment().ok_or(RunError::BadToken)? as *mut u64)
            };
            let (target, prepended) = zun_gc::with_current_heap(|heap| {
                let target = heap
                    .dynamic_get_function(record, "__target")
                    .ok_or(RunError::BadToken)?;
                let args_any = heap
                    .dynamic_get(record, "__args")
                    .ok_or(RunError::BadToken)?;
                let array = args_any.as_object().ok_or(RunError::BadToken)?;
                let array = unsafe { zun_gc::Obj::from_addr(array as *mut u64) };
                let length = heap.array_len(array).ok_or(RunError::BadToken)?;
                let mut prepended = Vec::with_capacity(length);
                for index in 0..length {
                    let any = heap.array_get(array, index).ok_or(RunError::BadToken)?;
                    let value = Value::unbox_any(any, heap).ok_or(RunError::BadToken)?;
                    // A function-typed partial argument was stored as a
                    // `functionBox` wrapper (functions do not box into `Any`);
                    // restore the callable before it is threaded as an argument.
                    prepended.push(unbox_function_box(value, heap));
                }
                Ok::<_, RunError>((target, prepended))
            })?;
            // The first bound argument is `thisArg`; a target that has no
            // receiver parameter (a plain function, or another bound
            // function) ignores it.
            let mut prepended = prepended;
            let takes_this =
                self.handle_takes_this(target, prepended.len() + call_args.len());
            if !takes_this && !prepended.is_empty() {
                prepended.remove(0);
            }
            let mut combined = prepended;
            combined.append(&mut call_args);
            call_args = combined;
            handle = target;
        }
        let final_args = if let Some(address) = handle.environment() {
            let environment = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
            let mut args = Vec::with_capacity(call_args.len() + 1);
            args.push(Value::Object(environment));
            args.extend(call_args);
            args
        } else {
            call_args
        };
        Ok((handle.method_rid(), final_args))
    }
}
