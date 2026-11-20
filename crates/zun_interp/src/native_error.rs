//! Catchable errors raised by native intrinsics.
//!
//! A native reports a failure as `RunError::Native("<Class>: <message>")`. When
//! `<Class>` names one of the prelude `Error` subclasses, the interpreter turns
//! the failure into a real thrown error object so `try`/`catch` (and
//! `e.name` / `e instanceof`) behave as in JS.

use super::value::{self, Value};
use super::Interp;

/// Prelude error classes a native may raise.
const ERROR_CLASSES: [&str; 6] = [
    "DOMException",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "ReferenceError",
    "URIError",
];

/// Split a native failure message into `(class, message)` when it names a
/// prelude error class.
pub(super) fn split_native_error(message: &str) -> Option<(&'static str, &str)> {
    for class in ERROR_CLASSES {
        if let Some(rest) = message
            .strip_prefix(class)
            .and_then(|r| r.strip_prefix(": "))
        {
            return Some((class, rest));
        }
    }
    None
}

impl<'a> Interp<'a> {
    /// Allocate an instance of prelude error class `class` carrying `message`.
    pub(super) fn make_error_value(&self, class: &str, message: &str) -> Option<Value> {
        // A `DOMException` failure reads `<name>: <message>`; the name is stored
        // in the record's `nameOverride` slot (see `builtin/error.zun`).
        let (name, message) = match class {
            "DOMException" => message.split_once(": ").unwrap_or(("Error", message)),
            _ => ("", message),
        };
        self.alloc_error_record(class, message, name).map(Value::Object)
    }

    /// Allocate an instance of the prelude error class `class` with every slot
    /// initialised. The record is filled **by field name** from the class's
    /// own metadata: `message` and `nameOverride` take the given text and
    /// every other slot takes the zero value of its kind.
    ///
    /// It used to be filled from a positional array of seven values, which
    /// froze the prelude class's field list: one more declared field, even at
    /// the end, left a slot unset here and made `new_error_object` refuse the
    /// class outright (measured: 13 fixtures lost).
    pub(crate) fn alloc_error_record(
        &self,
        class: &str,
        message: &str,
        name: &str,
    ) -> Option<zun_gc::Obj> {
        use zun_il::JitKind;
        let rid = self.type_rid_of(class)?;
        let token = zun_metadata::Token::new(zun_metadata::TableKind::TypeDef, rid);
        let (field_start, field_count) = self.type_field_range(token).ok()?;
        let mut slots: Vec<(u8, Value)> = Vec::with_capacity(field_count);
        for index in 0..field_count as u32 {
            let field = self
                .module
                .tables
                .field_defs
                .get((field_start + index) as usize - 1)?;
            let value = match self.module.strings.get(field.name) {
                "message" => Value::Str(message.into()),
                "nameOverride" => Value::Str(name.into()),
                _ => match field.kind {
                    value::FIELD_STRING => Value::Str("".into()),
                    value::FIELD_ANY | value::FIELD_OBJECT | value::FIELD_ARRAY => Value::Void,
                    kind => match JitKind::from_byte(kind) {
                        JitKind::Num(_) => Value::Float(0.0, zun_il::NumKind::F64),
                        JitKind::Bool => Value::Bool(false),
                        JitKind::String => Value::Str("".into()),
                        _ => Value::Void,
                    },
                },
            };
            slots.push((field.kind, value));
        }
        Some(zun_gc::with_current_heap_mut(|heap| {
            let object = heap.alloc(rid as u32, field_count);
            for (index, (kind, value)) in slots.iter().enumerate() {
                value::store_field(*kind, object, index, value, heap);
            }
            object
        }))
    }
}
