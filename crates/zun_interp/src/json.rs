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
    /// Rooted object slots, two per nesting level. The collector's registry
    /// owns the storage, so a slot read after a `toJSON` or replacer call
    /// answers the relocated address instead of a cached copy.
    roots: zun_gc::RootArray,
    top: usize,
    replacer: Option<zun_core::FunctionHandle>,
    list: Option<Vec<String>>,
    gap: String,
}

impl JsonCtx {
    fn slot_set(&mut self, slot: usize, obj: zun_gc::Obj) {
        self.roots.set(slot, obj);
    }
    fn slot_get(&self, slot: usize) -> zun_gc::Obj {
        self.roots
            .get(slot)
            .expect("every JSON slot is filled when the context is built")
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
            c => match zun_core::wtf16::lone_unit(c) {
                Some(unit) => out.push_str(&format!("\\u{:04x}", unit)),
                None => out.push(c),
            },
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
                Some(Value::Any(any)) => {
                    zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .unwrap_or(Value::Void)
                }
                Some(other) => other.clone(),
                None => Value::Void,
            }
        };
        let value = concrete(args.first());
        let replacer_arg = concrete(args.get(1));
        let space_arg = concrete(args.get(2));
        let mut replacer = None;
        let mut list = None;
        // A class value is callable, so it is *called* as a replacer -- and a
        // class called without `new` throws.
        if matches!(&replacer_arg, Value::Any(any) if any.as_class_descriptor().is_some()) {
            return Err(RunError::Native(
                "TypeError: Class constructor cannot be invoked without 'new'".into(),
            ));
        }
        // A proxy stands for what it wraps: over a callable it is the replacer
        // function, over an array it is the property list, and a revoked one
        // refuses both (IsArray throws on it).
        let replacer_arg = match as_obj(&replacer_arg) {
            Some(proxy) if zun_gc::with_current_heap(|heap| heap.proxy_target(proxy).is_some()) => {
                if zun_gc::with_current_heap(|heap| heap.proxy_is_revoked(proxy)) {
                    return Err(self.revoked_proxy_error("ownKeys"));
                }
                let target = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy))
                    .ok_or(RunError::BadToken)?;
                // An `apply` trap on a replacer is not honoured: the target's
                // own callable answers.
                match zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_function(target, "__boxedFn")
                        .or_else(|| heap.function_of_cell(target))
                }) {
                    Some(handle) => Value::Function(handle),
                    None => {
                        let list = self.call_managed_collection(
                            "arrayFromArrayLike",
                            &[Value::Object(proxy)],
                        )?;
                        concrete(Some(&list))
                    }
                }
            }
            _ => replacer_arg,
        };
        match &replacer_arg {
            Value::Function(handle) => replacer = Some(*handle),
            Value::Object(array) => {
                // Only a real Array is a property list. An `arguments` object
                // is array-backed but not one, and was taken for a list, which
                // dropped every key it did not name.
                let is_list = zun_gc::with_current_heap(|heap| {
                    heap.array_len(*array).is_some() && !heap.array_is_arguments(*array)
                });
                if let Some(len) =
                    zun_gc::with_current_heap(|heap| heap.array_len(*array)).filter(|_| is_list)
                {
                    let mut names: Vec<String> = Vec::new();
                    let mut seen: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    for index in 0..len {
                        let item = zun_gc::with_current_heap(|heap| {
                            heap.array_get(*array, index)
                                .and_then(|any| Value::unbox_any(any, heap))
                        });
                        let name = match item {
                            Some(Value::Str(text)) => Some(text.to_string()),
                            Some(Value::Int(n, _)) => Some(n.to_string()),
                            Some(Value::Float(f, _)) => Some(zun_natives::format_float(f)),
                            // A String or Number *wrapper* entry counts as its
                            // primitive, which the primitive-only match skipped.
                            Some(Value::Object(entry))
                                if matches!(
                                    self.json_class_name(entry).as_str(),
                                    "NumberObject" | "StringObject"
                                ) =>
                            {
                                self.json_wrapper_primitive_text(entry)
                            }
                            _ => None,
                        };
                        if let Some(name) = name {
                            // First-seen order drives serialization; the set
                            // only makes the duplicate test O(1).
                            if seen.insert(name.clone()) {
                                names.push(name);
                            }
                        }
                    }
                    list = Some(names);
                }
            }
            _ => {}
        }
        // A Number/String wrapper object as `space` counts as its primitive.
        let space_arg = match as_obj(&space_arg) {
            Some(obj)
                if matches!(
                    self.json_class_name(obj).as_str(),
                    "NumberObject" | "StringObject"
                ) =>
            {
                match self.instance_member_get(obj, "value")? {
                    Some(Value::Any(any)) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                            .unwrap_or(Value::Void)
                    }
                    Some(other) => other,
                    None => space_arg,
                }
            }
            _ => space_arg,
        };
        let gap = match &space_arg {
            Value::Int(n, _) => " ".repeat((*n).clamp(0, 10) as usize),
            Value::Float(f, _) => " ".repeat(if f.is_nan() {
                0
            } else {
                f.clamp(0.0, 10.0) as usize
            }),
            Value::Str(text) => text.chars().take(10).collect(),
            _ => String::new(),
        };
        // Root slots: two per level plus the wrapper holder.
        let holder = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
        // Every slot starts at the holder, so reading one before it is written
        // answers a live object rather than nothing.
        let mut ctx = JsonCtx {
            roots: zun_gc::RootArray::from_objects(
                std::iter::repeat(holder).take(MAX_DEPTH * 2 + 4),
            ),
            top: 1,
            replacer,
            list,
            gap,
        };
        ctx.slot_set(0, holder);
        match self.json_property(&mut ctx, "", 0, value, "", 0)? {
            Some(text) => {
                let boxed =
                    zun_gc::with_current_heap_mut(|heap| Value::Str(text.into()).box_any(heap))
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
            .map(|method| {
                self.module.strings.get(method.name).ends_with("@this>")
                    || self
                        .module
                        .strings
                        .get(method.name)
                        .ends_with("@this@rest>")
            })
            .unwrap_or(false)
    }

    fn json_call_with_this(
        &mut self,
        handle: zun_core::FunctionHandle,
        this: Value,
        call_args: Vec<Value>,
    ) -> Result<Value, RunError> {
        // Callback parameters are dynamic: hand them over boxed.
        let call_args: Vec<Value> = call_args
            .into_iter()
            .map(|arg| {
                zun_gc::with_current_heap_mut(|heap| match &arg {
                    // A function rides an `Any` lane inside a `__boxedFn` record.
                    Value::Function(function) => {
                        let record = heap.alloc_dynamic();
                        heap.dynamic_set_function(record, "__boxedFn", *function);
                        Value::Object(record).box_any(heap)
                    }
                    other => other.box_any(heap),
                })
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
    #[allow(clippy::unnecessary_unwrap)] // guarded unwrap reads clearer than if-let here
    fn json_get(&mut self, obj: zun_gc::Obj, key: &str) -> Result<Option<Value>, RunError> {
        // A proxy with a `get` trap: the trap supplies the value (and may throw).
        if zun_gc::with_current_heap(|heap| {
            heap.proxy_target(obj).is_some() && heap.proxy_trap(obj, "get").is_some()
        }) {
            let got = self.proxy_dynamic_get(obj, key)?;
            let got = match got {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                    .unwrap_or(Value::Void),
                other => other,
            };
            return Ok(Some(got));
        }
        // A proxy **without** a `get` trap reads through to its target, where
        // the properties live. Without this, `JSON.stringify(new Proxy({b: 2},
        // {}))` answered `{}` even after the keys were enumerated correctly,
        // and a cycle through such a proxy went undetected.
        if let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(obj)) {
            return self.json_get(target, key);
        }
        // Elements of an array-like (an `arguments` object, a typed array).
        if let Some(index) = key.parse::<usize>().ok().filter(|i| i.to_string() == key) {
            if let Some(len) = crate::typed_arrays::element_len(obj) {
                return Ok(if index < len {
                    crate::typed_arrays::read_element(obj, index)
                } else {
                    None
                });
            }
            if zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()) {
                return Ok(zun_gc::with_current_heap(|heap| {
                    heap.array_get(obj, index)
                        .filter(|any| !any.is_hole())
                        .and_then(|any| Value::unbox_any(any, heap))
                }));
            }
        }
        // A typed instance carrying an expando bag reads its declared fields from
        // the fixed layout; only a key held by the bag takes the dynamic route.
        if zun_gc::with_current_heap(|heap| {
            heap.is_dynamic_object(obj)
                && (!heap.is_instance_bag(obj)
                    || heap.dynamic_descriptor(obj, key).is_some()
                    || heap.dynamic_get_function(obj, key).is_some())
        }) {
            let (enumerable, getter, accessor, data, function) =
                zun_gc::with_current_heap(|heap| {
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
                return self
                    .invoke_function_value(getter, self.with_this_arg(getter, obj, Vec::new()))
                    .map(Some);
            }
            if accessor {
                return Ok(Some(Value::Void));
            }
            return Ok(
                data.and_then(|any| zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)))
            );
        }
        self.instance_member_get(obj, key)
    }

    /// The class name of a typed instance (`""` for a dynamic object).
    pub(crate) fn json_class_name(&self, obj: zun_gc::Obj) -> String {
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
            return Err(RunError::Native(
                "TypeError: Converting circular structure to JSON".into(),
            ));
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
            // A proxy's `get` trap observes the `toJSON` lookup (and may
            // throw) -- and when it answers a callable, that callable **is**
            // the hook and has to be called. Discarding the result meant
            // `JSON.stringify(proxyWithToJSON)` ignored it: node answers
            // `"PROXY_JSON"` where zun answered `{}`, and the trap log then
            // showed an extra `get` of the first key once the walk started
            // reading through the target.
            if zun_gc::with_current_heap(|heap| {
                heap.proxy_target(obj).is_some() && heap.proxy_trap(obj, "get").is_some()
            }) {
                let hook = self.proxy_dynamic_get(obj, "toJSON")?;
                let hook = match hook {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                            .unwrap_or(Value::Void)
                    }
                    other => other,
                };
                let hook = zun_gc::with_current_heap(|heap| {
                    crate::collections::unbox_function_box(hook, heap)
                });
                if let Value::Function(handle) = hook {
                    let this = Value::Object(ctx.slot_get(slot));
                    value = self.json_call_with_this(handle, this, vec![key_value.clone()])?;
                    if let Some(next) = as_obj(&value) {
                        ctx.slot_set(slot, next);
                    }
                }
            }
            if let Some(millis) = zun_gc::with_current_heap(|heap| heap.date_millis(obj)) {
                let _ = millis;
                let iso = zun_gc::with_current_heap(|heap| {
                    if heap.date_is_valid(obj) == Some(true) {
                        heap.date_iso_datetime(obj)
                    } else {
                        None
                    }
                });
                value = match iso {
                    Some(text) => Value::Str(text.into()),
                    None => Value::Null,
                };
            } else if zun_gc::with_current_heap(|heap| {
                heap.typed_array_element(obj) == Some(zun_core::TypedElement::U8)
                    && heap.is_node_buffer(obj)
                    && inherited_function(heap, obj, "toJSON").is_none()
            }) {
                // A Node `Buffer` serialises as `{ type: 'Buffer', data: [..] }`.
                let json = self.nb_to_json(obj, &[])?;
                value = match json {
                    Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                        .unwrap_or(Value::Void),
                    other => other,
                };
                if let Some(next) = as_obj(&value) {
                    ctx.slot_set(slot, next);
                }
            } else if let Some(handle) =
                zun_gc::with_current_heap(|heap| inherited_function(heap, obj, "toJSON"))
            {
                let this = Value::Object(ctx.slot_get(slot));
                value = self.json_call_with_this(handle, this, vec![key_value.clone()])?;
                if let Some(next) = as_obj(&value) {
                    ctx.slot_set(slot, next);
                }
            } else if let Some(handle) = {
                // A BigInt *object* inherits `BigInt.prototype.toJSON` too.
                let bigint_wrapper = self.is_bigint_wrapper(obj);
                let name = zun_gc::with_current_heap(|heap| {
                    if bigint_wrapper {
                        Some("BigInt")
                    } else if heap.map_entries(obj).is_some() {
                        Some("Map")
                    } else if heap.set_values(obj).is_some() {
                        Some("Set")
                    } else if heap.bigint_value(obj).is_some() {
                        // A BigInt is not serialisable by itself, but a
                        // `toJSON` installed on `BigInt.prototype` applies to
                        // it like any other prototype hook. Without this the
                        // unserialisable-BigInt error below wins over the
                        // hook the program installed.
                        Some("BigInt")
                    } else {
                        None
                    }
                });
                name.and_then(|n| self.builtin_prototype_by_name(n)).and_then(|proto| {
                    zun_gc::with_current_heap(|heap| heap.dynamic_get_function(proto, "toJSON"))
                })
            } {
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
                        Value::Any(any) => {
                            zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                                .unwrap_or(Value::Void)
                        }
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
        if as_obj(&value).is_some() {
            value = Value::Object(ctx.slot_get(slot));
        }
        // 3. unwrap primitive wrapper objects
        if let Some(obj) = as_obj(&value) {
            let class = self.json_class_name(obj);
            if matches!(
                class.as_str(),
                "NumberObject" | "BooleanObject" | "StringObject"
            ) {
                if let Some(inner) = self.instance_member_get(obj, "value")? {
                    value = match inner {
                        Value::Any(any) => {
                            zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                                .unwrap_or(Value::Void)
                        }
                        other => other,
                    };
                }
            } else if let Some(ctor) = self.wrapper_kind(obj) {
                // `Boolean.prototype` and friends are wrapper objects too.
                value = self.wrapper_primitive(Value::Object(obj), ctor)?;
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
                if zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_function(obj, "__boxedFn").is_some()
                }) {
                    return Ok(None);
                }
                // A BigInt, or a BigInt object (which holds one): neither is
                // serialisable without a `toJSON` hook.
                if zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some())
                    || self.is_bigint_wrapper(obj)
                {
                    return Err(RunError::Native(
                        "TypeError: Do not know how to serialize a BigInt".into(),
                    ));
                }
                // A `JSON.rawJSON` box embeds its text verbatim.
                if zun_gc::with_current_heap(|heap| heap.dynamic_is_raw_json(obj)) {
                    let text = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get(obj, "rawJSON")
                            .and_then(|any| Value::unbox_any(any, heap))
                    });
                    if let Some(Value::Str(text)) = text {
                        return Ok(Some(text.to_string()));
                    }
                }
                // Cycle detection against every ancestor currently on the path.
                for ancestor in 1..slot {
                    if ancestor != slot && ctx.slot_get(ancestor).addr() == obj.addr() {
                        return Err(RunError::Native(
                            "TypeError: Converting circular structure to JSON".into(),
                        ));
                    }
                }
                if zun_gc::with_current_heap(|heap| {
                    heap.array_len(obj).is_some() && !heap.array_is_arguments(obj)
                }) {
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
            // An accessor element (an array index defined with a getter) reads
            // through its getter.
            let item = match &item {
                Value::Object(record)
                    if zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get(*record, "__elemAcc").is_some()
                    }) =>
                {
                    match zun_gc::with_current_heap(|heap| {
                        heap.dynamic_accessor_getter(*record, "v")
                    }) {
                        Some(getter) => {
                            let this = ctx.slot_get(slot);
                            self.invoke_function_value(
                                getter,
                                self.with_this_arg(getter, this, Vec::new()),
                            )?
                        }
                        None => Value::Void,
                    }
                }
                _ => item,
            };
            let text =
                self.json_property(ctx, &index.to_string(), slot, item, &inner, depth + 1)?;
            parts.push(text.unwrap_or_else(|| "null".into()));
        }
        Ok(Some(if ctx.gap.is_empty() {
            format!("[{}]", parts.join(","))
        } else {
            format!(
                "[\n{inner}{}\n{indent}]",
                parts.join(&format!(",\n{inner}"))
            )
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
                // A proxy without the trap defers to its target, where the
                // properties live: `JSON.stringify(new Proxy({b: 2}, {}))`
                // answered `{}`, and a cycle through a proxy went undetected
                // because the walk never entered it. The same resolution
                // `Object.keys` needed.
                let holder = self.enumerability_holder(obj);
                if zun_gc::with_current_heap(|heap| {
                    heap.is_dynamic_object(holder) && !heap.is_instance_bag(holder)
                }) {
                    zun_gc::with_current_heap(|heap| {
                        heap.dynamic_keys(holder)
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|key| heap.dynamic_is_enumerable(holder, key))
                            .collect()
                    })
                } else {
                    self.object_own_entries(holder)?
                        .into_iter()
                        .map(|(key, _)| key)
                        .collect()
                }
            }
        };
        let inner = format!("{indent}{}", ctx.gap);
        let mut parts: Vec<String> = Vec::new();
        for key in keys {
            let current = ctx.slot_get(slot);
            let Some(item) = self.json_get(current, &key)? else {
                continue;
            };
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
            format!(
                "{{\n{inner}{}\n{indent}}}",
                parts.join(&format!(",\n{inner}"))
            )
        }))
    }
}

/// A function-valued property looked up on `obj` and then along its dynamic
/// prototype chain (a `toJSON` inherited from a constructor's `prototype`).
fn inherited_function(
    heap: &zun_gc::Heap,
    obj: zun_gc::Obj,
    name: &str,
) -> Option<zun_core::FunctionHandle> {
    let mut current = obj;
    for _ in 0..MAX_DEPTH {
        if let Some(handle) = heap.dynamic_get_function(current, name) {
            return Some(handle);
        }
        let next = heap.dynamic_prototype(current)?.as_object()?;
        current = unsafe { zun_gc::Obj::from_addr(next as *mut u64) };
    }
    None
}

impl Interp<'_> {
    /// The text a String or Number wrapper object names as a property key.
    fn json_wrapper_primitive_text(&mut self, wrapper: zun_gc::Obj) -> Option<String> {
        let ctor = match self.json_class_name(wrapper).as_str() {
            "NumberObject" => "Number",
            "StringObject" => "String",
            _ => return None,
        };
        match self.wrapper_primitive(Value::Object(wrapper), ctor).ok()? {
            Value::Str(text) => Some(text.to_string()),
            Value::Int(n, _) => Some(n.to_string()),
            Value::Float(f, _) => Some(zun_natives::format_float(f)),
            _ => None,
        }
    }
}
