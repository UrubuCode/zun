//! `JSON.stringify` (ECMAScript `SerializeJSONProperty`): `toJSON` hooks,
//! replacer functions and property lists, the `space` argument, cycle
//! detection, and the value-dependent skipping rules (`undefined`, functions
//! and symbols are dropped from objects and become `null` in arrays).
//!
//! Objects held while user callbacks (`toJSON`, a replacer) run are registered
//! as native roots so a moving collection inside the callback cannot leave a
//! stale reference behind.

use super::errors::RunError;
use super::value::Value;
use super::Interp;

const MAX_DEPTH: usize = 256;

struct JsonCtx {
    /// Rooted object slots (two per nesting level); rewritten in place by GC.
    roots: Vec<zun_gc::Obj>,
    top: usize,
    replacer: Option<zun_core::FunctionHandle>,
    list: Option<Vec<String>>,
    gap: String,
}

impl JsonCtx {
    fn slot_set(&mut self, slot: usize, obj: zun_gc::Obj) {
        // SAFETY: `slot` is within the pre-sized vector; GC reads/writes the
        // same memory through the registered root scope.
        unsafe { std::ptr::write_volatile(self.roots.as_mut_ptr().add(slot), obj) };
    }
    fn slot_get(&self, slot: usize) -> zun_gc::Obj {
        unsafe { std::ptr::read_volatile(self.roots.as_ptr().add(slot)) }
    }
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for unit in text.chars() {
        match unit {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn as_obj(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Object(object) => Some(*object),
        _ => None,
    }
}

fn is_undefined(value: &Value) -> bool {
    matches!(value, Value::Void)
        || matches!(value, Value::Any(any) if any.tag() == zun_core::AnyTag::Void)
}

impl Interp<'_> {
    /// `JSON.stringify(value, replacer?, space?)`: a string, or `undefined`
    /// when the top-level value is not serialisable.
    pub(crate) fn json_stringify(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let concrete = |value: Option<&Value>| -> Value {
            match value {
                Some(Value::Any(any)) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                    .unwrap_or(Value::Void),
                Some(other) => other.clone(),
                None => Value::Void,
            }
        };
        let value = concrete(args.first());
        let replacer_arg = concrete(args.get(1));
        let space_arg = concrete(args.get(2));
        let mut replacer = None;
        let mut list = None;
        match &replacer_arg {
            Value::Function(handle) => replacer = Some(*handle),
            Value::Object(array) => {
                if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(*array)) {
                    let mut names: Vec<String> = Vec::new();
                    for index in 0..len {
                        let item = zun_gc::with_current_heap(|heap| {
                            heap.array_get(*array, index).and_then(|any| Value::unbox_any(any, heap))
                        });
                        let name = match item {
                            Some(Value::Str(text)) => Some(text.to_string()),
                            Some(Value::Int(n, _)) => Some(n.to_string()),
                            Some(Value::Float(f, _)) => Some(zun_natives::format_float(f)),
                            _ => None,
                        };
                        if let Some(name) = name {
                            if !names.contains(&name) {
                                names.push(name);
                            }
                        }
                    }
                    list = Some(names);
                }
            }
            _ => {}
        }
        let gap = match &space_arg {
            Value::Int(n, _) => " ".repeat((*n).clamp(0, 10) as usize),
            Value::Float(f, _) => " ".repeat(if f.is_nan() { 0 } else { f.clamp(0.0, 10.0) as usize }),
            Value::Str(text) => text.chars().take(10).collect(),
            _ => String::new(),
        };
        // Root slots: two per level plus the wrapper holder.
        let holder = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
        let mut ctx = JsonCtx {
            roots: vec![holder; MAX_DEPTH * 2 + 4],
            top: 1,
            replacer,
            list,
            gap,
        };
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&ctx.roots) };
        ctx.slot_set(0, holder);
        match self.json_property(&mut ctx, "", 0, value, "", 0)? {
            Some(text) => {
                let boxed = zun_gc::with_current_heap_mut(|heap| Value::Str(text.into()).box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                Ok(Value::Any(boxed))
            }
            None => Ok(Value::Any(zun_core::Any::void())),
        }
    }

    /// Whether the function handle expects the receiver as a hidden first arg
    /// (an object-literal method that uses `this`).
    fn json_takes_this(&self, handle: zun_core::FunctionHandle) -> bool {
        if handle.is_bound() {
            return false;
        }
        self.module
            .tables
            .method_defs
            .get((handle.method_rid() as usize).wrapping_sub(1))
            .map(|method| self.module.strings.get(method.name).ends_with("@this>"))
            .unwrap_or(false)
    }

    fn json_call_with_this(
        &mut self,
        handle: zun_core::FunctionHandle,
        this: Value,
        mut call_args: Vec<Value>,
    ) -> Result<Value, RunError> {
        // Callback parameters are dynamic: hand them over boxed.
        let call_args: Vec<Value> = call_args
            .into_iter()
            .map(|arg| {
                zun_gc::with_current_heap_mut(|heap| arg.box_any(heap))
                    .map(Value::Any)
                    .unwrap_or(Value::Void)
            })
            .collect();
        let mut call_args = call_args;
        if self.json_takes_this(handle) {
            call_args.insert(0, this);
        }
        let result = self.invoke_callback(handle, call_args)?;
        Ok(match result {
            Value::Any(any) => {
                zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)).unwrap_or(Value::Void)
            }
            other => other,
        })
    }

    /// Read an own enumerable property of an object (running a getter).
    fn json_get(&mut self, obj: zun_gc::Obj, key: &str) -> Result<Option<Value>, RunError> {
        if zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj)) {
            let (enumerable, getter, accessor, data, function) = zun_gc::with_current_heap(|heap| {
                (
                    heap.dynamic_is_enumerable(obj, key),
                    heap.dynamic_accessor_getter(obj, key),
                    heap.dynamic_is_accessor(obj, key),
                    heap.dynamic_get(obj, key),
                    heap.dynamic_get_function(obj, key),
                )
            });
            if function.is_some() {
                return Ok(Some(Value::Function(function.unwrap())));
            }
            if !enumerable && data.is_none() && !accessor {
                return Ok(None);
            }
            if let Some(getter) = getter {
                return self.invoke_function_value(getter, Vec::new()).map(Some);
            }
            if accessor {
                return Ok(Some(Value::Void));
            }
            return Ok(data.and_then(|any| zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))));
        }
        self.instance_member_get(obj, key)
    }

    /// The class name of a typed instance (`""` for a dynamic object).
    fn json_class_name(&self, obj: zun_gc::Obj) -> String {
        let rid = obj.type_rid();
        if rid == 0 {
            return String::new();
        }
        self.module
            .tables
            .type_defs
            .get(rid as usize - 1)
            .map(|ty| self.module.strings.get(ty.name).to_string())
            .unwrap_or_default()
    }

    /// Serialise one property. `holder_slot` indexes the rooted holder in
    /// `ctx.roots`. Returns `None` for "undefined" (skip).
    fn json_property(
        &mut self,
        ctx: &mut JsonCtx,
        key: &str,
        holder_slot: usize,
        value: Value,
        indent: &str,
        depth: usize,
    ) -> Result<Option<String>, RunError> {
        if depth > MAX_DEPTH - 2 {
            return Err(RunError::Native("TypeError: Converting circular structure to JSON".into()));
        }
        let slot = ctx.top;
        ctx.top += 1;
        let result = self.json_property_at(ctx, key, holder_slot, slot, value, indent, depth);
        ctx.top = slot;
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn json_property_at(
        &mut self,
        ctx: &mut JsonCtx,
        key: &str,
        holder_slot: usize,
        slot: usize,
        mut value: Value,
        indent: &str,
        depth: usize,
    ) -> Result<Option<String>, RunError> {
        // A non-object value leaves the slot pointing at the wrapper holder so
        // a stale entry can never look like an ancestor.
        let wrapper = ctx.slot_get(0);
        ctx.slot_set(slot, wrapper);
        if let Some(obj) = as_obj(&value) {
            ctx.slot_set(slot, obj);
        }
        let key_value = Value::Str(key.into());
        // 1. toJSON
        if let Some(obj) = as_obj(&value) {
            let is_bigint = zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some());
            if is_bigint {
                // BigInt has no toJSON here; serialisation fails below.
            } else if let Some(millis) = zun_gc::with_current_heap(|heap| heap.date_millis(obj)) {
                let _ = millis;
                let iso = zun_gc::with_current_heap(|heap| {
                    if heap.date_is_valid(obj) == Some(true) { heap.date_iso_datetime(obj) } else { None }
                });
                value = match iso {
                    Some(text) => Value::Str(text.into()),
                    None => Value::Null,
                };
            } else if let Some(handle) = zun_gc::with_current_heap(|heap| heap.dynamic_get_function(obj, "toJSON")) {
                let this = Value::Object(ctx.slot_get(slot));
                value = self.json_call_with_this(handle, this, vec![key_value.clone()])?;
                if let Some(next) = as_obj(&value) {
                    ctx.slot_set(slot, next);
                }
            } else if !zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj)) {
                // A typed class instance (e.g. `Date`) with a `toJSON` method.
                if let Some(rid) = self.instance_method_rid(obj, "toJSON") {
                    let this = Value::Object(ctx.slot_get(slot));
                    value = self.call(rid, vec![this], None)?;
                    value = match value {
                        Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                            .unwrap_or(Value::Void),
                        other => other,
                    };
                    if let Some(next) = as_obj(&value) {
                        ctx.slot_set(slot, next);
                    }
                }
            }
        }
        // 2. replacer function
        if let Some(replacer) = ctx.replacer {
            let holder = Value::Object(ctx.slot_get(holder_slot));
            let arg = match as_obj(&value) {
                Some(_) => Value::Object(ctx.slot_get(slot)),
                None => value.clone(),
            };
            value = self.json_call_with_this(replacer, holder, vec![key_value, arg])?;
            if let Some(next) = as_obj(&value) {
                ctx.slot_set(slot, next);
            }
        }
        if let Some(_) = as_obj(&value) {
            value = Value::Object(ctx.slot_get(slot));
        }
        // 3. unwrap primitive wrapper objects
        if let Some(obj) = as_obj(&value) {
            let class = self.json_class_name(obj);
            if matches!(class.as_str(), "NumberObject" | "BooleanObject" | "StringObject") {
                if let Some(inner) = self.instance_member_get(obj, "value")? {
                    value = match inner {
                        Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                            .unwrap_or(Value::Void),
                        other => other,
                    };
                }
            }
        }
        // 4. serialise by kind
        match &value {
            Value::Null => Ok(Some("null".into())),
            Value::Bool(flag) => Ok(Some(flag.to_string())),
            Value::Str(text) => Ok(Some(quote(text))),
            Value::Int(n, _) => Ok(Some(n.to_string())),
            Value::Float(f, _) => Ok(Some(if f.is_finite() {
                zun_natives::format_float(*f)
            } else {
                "null".into()
            })),
            Value::Void | Value::Function(_) => Ok(None),
            Value::Any(any) => {
                // Symbols and class descriptors are not serialisable.
                if is_undefined(&Value::Any(*any)) {
                    return Ok(None);
                }
                Ok(None)
            }
            Value::Object(_) => {
                let obj = ctx.slot_get(slot);
                // A boxed function value (functions held in arrays/`any`).
                if zun_gc::with_current_heap(|heap| heap.dynamic_get_function(obj, "__boxedFn").is_some()) {
                    return Ok(None);
                }
                if zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some()) {
                    return Err(RunError::Native(
                        "TypeError: Do not know how to serialize a BigInt".into(),
                    ));
                }
                // Cycle detection against every ancestor currently on the path.
                for ancestor in 1..slot {
                    if ancestor != slot && ctx.slot_get(ancestor).addr() == obj.addr() {
                        return Err(RunError::Native(
                            "TypeError: Converting circular structure to JSON".into(),
                        ));
                    }
                }
                if zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()) {
                    self.json_array(ctx, slot, indent, depth)
                } else {
                    self.json_object(ctx, slot, indent, depth)
                }
            }
        }
    }

    fn json_array(
        &mut self,
        ctx: &mut JsonCtx,
        slot: usize,
        indent: &str,
        depth: usize,
    ) -> Result<Option<String>, RunError> {
        let len = zun_gc::with_current_heap(|heap| heap.array_len(ctx.slot_get(slot))).unwrap_or(0);
        if len == 0 {
            return Ok(Some("[]".into()));
        }
        let inner = format!("{indent}{}", ctx.gap);
        let mut parts: Vec<String> = Vec::with_capacity(len);
        for index in 0..len {
            let item = zun_gc::with_current_heap(|heap| {
                heap.array_get(ctx.slot_get(slot), index)
                    .and_then(|any| Value::unbox_any(any, heap))
            })
            .unwrap_or(Value::Void);
            let text = self.json_property(ctx, &index.to_string(), slot, item, &inner, depth + 1)?;
            parts.push(text.unwrap_or_else(|| "null".into()));
        }
        Ok(Some(if ctx.gap.is_empty() {
            format!("[{}]", parts.join(","))
        } else {
            format!("[\n{inner}{}\n{indent}]", parts.join(&format!(",\n{inner}")))
        }))
    }

    fn json_object(
        &mut self,
        ctx: &mut JsonCtx,
        slot: usize,
        indent: &str,
        depth: usize,
    ) -> Result<Option<String>, RunError> {
        let obj = ctx.slot_get(slot);
        let keys: Vec<String> = match &ctx.list {
            Some(list) => list.clone(),
            None => {
                if zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj)) {
                    zun_gc::with_current_heap(|heap| {
                        heap.dynamic_keys(obj)
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|key| heap.dynamic_is_enumerable(obj, key))
                            .collect()
                    })
                } else {
                    self.object_own_entries(obj)?.into_iter().map(|(key, _)| key).collect()
                }
            }
        };
        let inner = format!("{indent}{}", ctx.gap);
        let mut parts: Vec<String> = Vec::new();
        for key in keys {
            let current = ctx.slot_get(slot);
            let Some(item) = self.json_get(current, &key)? else { continue };
            let Some(text) = self.json_property(ctx, &key, slot, item, &inner, depth + 1)? else {
                continue;
            };
            let separator = if ctx.gap.is_empty() { ":" } else { ": " };
            parts.push(format!("{}{separator}{text}", quote(&key)));
        }
        if parts.is_empty() {
            return Ok(Some("{}".into()));
        }
        Ok(Some(if ctx.gap.is_empty() {
            format!("{{{}}}", parts.join(","))
        } else {
            format!("{{\n{inner}{}\n{indent}}}", parts.join(&format!(",\n{inner}")))
        }))
    }
}
