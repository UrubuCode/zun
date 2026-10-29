//! Catchable errors raised by native intrinsics.
//!
//! A native reports a failure as `RunError::Native("<Class>: <message>")`. When
//! `<Class>` names one of the prelude `Error` subclasses, the interpreter turns
//! the failure into a real thrown error object so `try`/`catch` (and
//! `e.name` / `e instanceof`) behave as in JS.

use super::value::{self, Value};
use super::Interp;

/// Prelude error classes a native may raise.
const ERROR_CLASSES: [&str; 5] = ["TypeError", "RangeError", "SyntaxError", "ReferenceError", "URIError"];

/// Split a native failure message into `(class, message)` when it names a
/// prelude error class.
pub(super) fn split_native_error(message: &str) -> Option<(&'static str, &str)> {
    for class in ERROR_CLASSES {
        if let Some(rest) = message.strip_prefix(class).and_then(|r| r.strip_prefix(": ")) {
            return Some((class, rest));
        }
    }
    None
}

impl<'a> Interp<'a> {
    /// Allocate an instance of prelude error class `class` carrying `message`.
    pub(super) fn make_error_value(&self, class: &str, message: &str) -> Option<Value> {
        let rid = self
            .module
            .tables
            .type_defs
            .iter()
            .position(|ty| self.module.strings.get(ty.name) == class)?
            + 1;
        let token = zun_metadata::Token::new(zun_metadata::TableKind::TypeDef, rid as u32);
        let (field_start, field_count) = self.type_field_range(token).ok()?;
        let kinds: Vec<u8> = (0..field_count as u32)
            .map(|i| self.field_kind_at(field_start + i))
            .collect::<Result<_, _>>()
            .ok()?;
        // Field order of `Error`: message, stackFrames, cause.
        let values = [
            Value::Str(message.into()),
            Value::Str("".into()),
            Value::Void,
        ];
        let object = zun_gc::with_current_heap_mut(|heap| {
            let object = heap.alloc(rid as u32, field_count);
            for (index, kind) in kinds.iter().enumerate() {
                if let Some(v) = values.get(index) {
                    value::store_field(*kind, object, index, v, heap);
                }
            }
            object
        });
        Some(Value::Object(object))
    }
}
