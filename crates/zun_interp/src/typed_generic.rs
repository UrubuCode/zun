//! `Array.prototype` methods borrowed on a typed-array receiver.
//!
//! A typed array is an array-like with a fixed `length` and integer-indexed
//! elements, so every generic `Array.prototype` algorithm accepts one
//! (`Array.prototype.sort.call(new Uint8Array([3, 1, 2]))`). The read-only
//! methods run on a plain-array snapshot of the elements. A method that writes
//! (`sort`, `reverse`, `fill`, `copyWithin`) runs on the snapshot too, and the
//! result is then stored back **into the typed array** element by element
//! through the ordinary element conversion, so the write is never lost in the
//! temporary. A method that must change `length` or delete an index (`push`,
//! `pop`, `shift`, `unshift`, `splice`) performs the element moves the
//! specification orders before that step and then throws the `TypeError` the
//! step raises on a typed array.

use super::value::Value;
use super::{Interp, RunError};

/// The message of the `Set(O, "length", ...)` step on a typed array, whose
/// `length` is an accessor without a setter.
const LENGTH_HAS_ONLY_A_GETTER: &str =
    "Cannot set property length of [object Object] which has only a getter";

/// Copies the raw element at `from` over the one at `to`; an index outside the
/// view is ignored, like an integer-indexed store.
fn move_element(view: zun_gc::Obj, from: usize, to: usize) {
    if let Some(value) = crate::typed_arrays::read_element(view, from) {
        let _ = crate::typed_arrays::write_element(view, to, &value);
    }
}

impl Interp<'_> {
    /// A fresh plain array holding the current elements of `view`.
    pub(crate) fn typed_array_snapshot(&mut self, view: zun_gc::Obj) -> zun_gc::Obj {
        // Reading a BigInt element and growing the array both allocate.
        zun_gc::root!(view = view);
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
        zun_gc::root!(target = target);
        let length = crate::typed_arrays::element_len(view.get()).unwrap_or(0);
        for index in 0..length {
            let value = self.view_read(view.get(), index);
            zun_gc::with_current_heap_mut(|heap| {
                let boxed = value.box_any(heap).unwrap_or_else(zun_core::Any::void);
                heap.array_push(target.get(), boxed);
            });
        }
        target.get()
    }

    /// Stores the elements of the plain array `source` into `view`, converting
    /// each to the element type (which can run a user `valueOf`).
    fn typed_array_write_back(
        &mut self,
        view: zun_gc::Obj,
        source: zun_gc::Obj,
    ) -> Result<(), RunError> {
        zun_gc::root!(view = view);
        zun_gc::root!(source = source);
        let length = crate::typed_arrays::element_len(view.get()).unwrap_or(0);
        for index in 0..length {
            let element = zun_gc::with_current_heap(|heap| {
                heap.array_get(source.get(), index)
                    .map(|any| Value::unbox_any(any, heap).unwrap_or(Value::Void))
            });
            let Some(element) = element else {
                break;
            };
            self.view_write(view.get(), index, &element)?;
        }
        Ok(())
    }

    /// `Array.prototype.<name>.call(view, ...args)` for a typed-array `view`.
    pub(crate) fn array_method_on_typed_array(
        &mut self,
        name: &str,
        view: zun_gc::Obj,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        zun_gc::root!(view = view);
        // The arguments ride in a rooted array across the snapshot allocation.
        let holder = zun_gc::with_current_heap_mut(|heap| {
            let list = heap.alloc_array(args.len());
            for (index, value) in args.iter().enumerate() {
                let any = value.box_any_cell(heap).unwrap_or_else(zun_core::Any::void);
                heap.array_set(list, index, any);
            }
            list
        });
        let count = args.len();
        drop(args);
        zun_gc::root!(holder = holder);
        let held = |holder: zun_gc::Obj| -> Vec<Value> {
            (0..count)
                .map(|index| {
                    let any = zun_gc::with_current_heap(|heap| heap.array_get(holder, index))
                        .unwrap_or_else(zun_core::Any::void);
                    crate::proto_methods::proto_concrete(Value::Any(any))
                })
                .collect()
        };
        match name {
            "push" | "pop" | "shift" | "unshift" | "splice" => {
                let args = held(holder.get());
                self.typed_array_resize_attempt(name, view.get(), &args)
            }
            // Generic `toString` calls the receiver's own `join`.
            "toString" => self.proto_dynamic_call(Value::Object(view.get()), "join", Vec::new()),
            "sort" | "reverse" | "fill" | "copyWithin" => {
                let temp = self.typed_array_snapshot(view.get());
                zun_gc::root!(temp = temp);
                let args = held(holder.get());
                self.proto_dynamic_call(Value::Object(temp.get()), name, args)?;
                self.typed_array_write_back(view.get(), temp.get())?;
                Ok(Value::Object(view.get()))
            }
            _ => {
                let temp = self.typed_array_snapshot(view.get());
                let args = held(holder.get());
                self.proto_dynamic_call(Value::Object(temp), name, args)
            }
        }
    }

    /// `push` / `pop` / `shift` / `unshift` / `splice` on a typed array: the
    /// in-range element moves happen, then the step that would resize throws.
    fn typed_array_resize_attempt(
        &mut self,
        name: &str,
        view: zun_gc::Obj,
        args: &[Value],
    ) -> Result<Value, RunError> {
        zun_gc::root!(view = view);
        let length = crate::typed_arrays::element_len(view.get()).unwrap_or(0);
        let kind = crate::typed_arrays::element_of(view.get()).map_or("Object", |kind| kind.js_name());
        let cannot_delete = format!(
            "Cannot delete property '{}' of [object {kind}]",
            length.saturating_sub(1)
        );
        let message = match name {
            "push" => LENGTH_HAS_ONLY_A_GETTER.to_owned(),
            "pop" if length == 0 => LENGTH_HAS_ONLY_A_GETTER.to_owned(),
            "pop" => cannot_delete,
            "shift" if length == 0 => LENGTH_HAS_ONLY_A_GETTER.to_owned(),
            "shift" => {
                for index in 1..length {
                    move_element(view.get(), index, index - 1);
                }
                cannot_delete
            }
            "unshift" => {
                let count = args.len();
                if count > 0 {
                    for index in (0..length.saturating_sub(count)).rev() {
                        move_element(view.get(), index, index + count);
                    }
                    for (index, item) in args.iter().enumerate().take(length) {
                        self.view_write(view.get(), index, item)?;
                    }
                }
                LENGTH_HAS_ONLY_A_GETTER.to_owned()
            }
            _ => {
                // `splice(start, deleteCount, ...items)`.
                let relative = |interp: &mut Self, value: &Value| -> Result<f64, RunError> {
                    let number = interp.dyn_to_number(value)?;
                    Ok(if number.is_nan() { 0.0 } else { number.trunc() })
                };
                let len = length as f64;
                let start = match args.first() {
                    Some(value) => {
                        let start = relative(self, value)?;
                        if start < 0.0 { (len + start).max(0.0) } else { start.min(len) }
                    }
                    None => 0.0,
                } as usize;
                let remaining = length - start;
                let delete = match args.len() {
                    0 => 0,
                    1 => remaining,
                    _ => relative(self, &args[1])?.clamp(0.0, remaining as f64) as usize,
                };
                let items = args.get(2..).unwrap_or(&[]);
                let count = items.len();
                if count < delete {
                    for index in start..length - delete {
                        move_element(view.get(), index + delete, index + count);
                    }
                    cannot_delete
                } else {
                    if count > delete {
                        for index in (start + 1..=length - delete).rev() {
                            let target = index + count - 1;
                            if target < length {
                                move_element(view.get(), index + delete - 1, target);
                            }
                        }
                    }
                    for (offset, item) in items.iter().enumerate() {
                        if start + offset < length {
                            self.view_write(view.get(), start + offset, item)?;
                        }
                    }
                    LENGTH_HAS_ONLY_A_GETTER.to_owned()
                }
            }
        };
        Err(self.proto_type_error(&message))
    }
}
