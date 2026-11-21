//! `Get(obj, @@toStringTag)` for `Object.prototype.toString` and `String(obj)`.
//!
//! The tag is an ordinary property read: an own or inherited data property, or
//! an accessor whose getter runs with the original object as receiver. The first
//! level that carries the key decides; a non-string result means "no custom
//! tag" and the caller falls back to the built-in tag.

use super::*;

impl<'a> Interp<'a> {
    /// The string `@@toStringTag` of `start`, read through its prototype chain
    /// (class prototype objects for a typed instance, the dynamic prototype
    /// link otherwise). A user getter is invoked and may throw. `Ok(None)` when
    /// the chain has no string tag, or when the tag is a built-in getter (the
    /// typed-array one), which the caller's built-in table already answers.
    pub(crate) fn string_tag_of(&mut self, start: zun_gc::Obj) -> Result<Option<String>, RunError> {
        enum Found {
            Getter(zun_core::FunctionHandle),
            Data(zun_core::Any),
            Absent,
        }
        let symbol = zun_core::WellKnownSymbol::ToStringTag.id();
        // A proxy answers through its `get` trap, which can invent the tag, and
        // falls back to its *target* -- a proxy over an array is
        // `[object Array]`. Reading the proxy object's own symbol store found
        // neither.
        if let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(start)) {
            let read = self.proxy_dynamic_get_symbol(start, symbol)?;
            if let Some(text) = read.and_then(|value| match value {
                Value::Str(text) => Some(text.to_string()),
                _ => None,
            }) {
                return Ok(Some(text));
            }
            return self.string_tag_of(target);
        }
        // The receiver outlives the getter call and the allocations that build
        // lazy class prototype objects.
        zun_gc::root!(receiver = start);
        let mut current = start;
        for _ in 0..64 {
            zun_gc::root!(held = current);
            let found = zun_gc::with_current_heap(|heap| {
                let at = held.get();
                if let Some(getter) = heap.dynamic_symbol_getter(at, symbol) {
                    Found::Getter(getter)
                } else if let Some(any) = heap.dynamic_get_symbol(at, symbol) {
                    Found::Data(any)
                } else {
                    Found::Absent
                }
            });
            match found {
                Found::Getter(getter) => {
                    if getter.builtin_method().is_some() {
                        return Ok(None);
                    }
                    let args = self.with_this_arg(getter, receiver.get(), Vec::new());
                    let value = self.invoke_function_value(getter, args)?;
                    return Ok(match unbox_arg(&value)? {
                        Value::Str(text) => Some(text.to_string()),
                        _ => None,
                    });
                }
                Found::Data(any) => {
                    return Ok(zun_gc::with_current_heap(|heap| {
                        match Value::unbox_any(any, heap) {
                            Some(Value::Str(text)) => Some(text.to_string()),
                            _ => None,
                        }
                    }));
                }
                Found::Absent => {}
            }
            let at = held.get();
            let next = self.instance_class_prototype(at).or_else(|| {
                zun_gc::with_current_heap(|heap| heap.dynamic_prototype(at))
                    .and_then(|proto| proto.as_object())
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
            });
            match next {
                Some(next) => current = next,
                None => return Ok(None),
            }
        }
        Ok(None)
    }
}

impl<'a> Interp<'a> {
    /// The `Object.prototype.toString` spelling a plain object stringifies to
    /// when it has no `toString` / `valueOf` / `[Symbol.toPrimitive]` of its own:
    /// `[object <tag>]` for a string `@@toStringTag`, else `[object Object]`.
    pub(crate) fn default_object_string(&mut self, obj: zun_gc::Obj) -> Result<String, RunError> {
        if let Some(tag) = self.string_tag_of(obj)? {
            return Ok(format!("[object {tag}]"));
        }
        // Then the builtin brand. Without it `String(new WeakMap())` answered
        // "[object Object]" while `Object.prototype.toString.call` on the same
        // object answered "[object WeakMap]": two spellings of one question.
        let brand = zun_gc::with_current_heap(|heap| {
            crate::collections::builtin_constructor_name(heap, obj).map(str::to_owned)
        });
        Ok(format!("[object {}]", brand.as_deref().unwrap_or("Object")))
    }
}
