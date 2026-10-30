//! Strict-mode integrity checks for arrays and dynamic objects.
//!
//! ES modules are strict, so a refused store/delete/mutation throws a
//! `TypeError` instead of failing silently. Natives report the failure as
//! `RunError::Native("TypeError: ...")`, which the call sites turn into a
//! catchable thrown error object.

use super::errors::RunError;
use super::value::Value;
use super::Interp;

/// What a mutator needs from an array.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ArrayOp {
    /// Grows the array (push/unshift/insert): refused once non-extensible.
    Add,
    /// Shrinks the array (pop/shift/delete): refused once sealed.
    Remove,
    /// Overwrites existing elements (sort/reverse/index store): refused once frozen.
    Write,
}

/// Array integrity levels (see `ArrayStorage::integrity`).
pub(crate) const LEVEL_NON_EXTENSIBLE: u8 = 1;
pub(crate) const LEVEL_SEALED: u8 = 2;
pub(crate) const LEVEL_FROZEN: u8 = 3;

/// The `TypeError` message when `level` forbids `op`, or `None` if allowed.
pub(crate) fn array_refusal(level: u8, op: ArrayOp) -> Option<&'static str> {
    match op {
        ArrayOp::Add if level >= LEVEL_NON_EXTENSIBLE => {
            Some("TypeError: Cannot add property, object is not extensible")
        }
        ArrayOp::Remove if level >= LEVEL_SEALED => {
            Some("TypeError: Cannot delete property of [object Array]")
        }
        ArrayOp::Write if level >= LEVEL_FROZEN => {
            Some("TypeError: Cannot assign to read only property of object '[object Array]'")
        }
        _ => None,
    }
}

fn as_object(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Object(object) => Some(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
        _ => None,
    }
}

impl Interp<'_> {
    /// Refuse (with a `TypeError`) a mutation `op` on `array` when its
    /// integrity level forbids it. Non-arrays pass.
    pub(crate) fn guard_array(&self, array: zun_gc::Obj, op: ArrayOp) -> Result<(), RunError> {
        let level = zun_gc::with_current_heap(|heap| heap.array_integrity(array));
        match array_refusal(level, op) {
            Some(message) => Err(RunError::Native(message.into())),
            None => Ok(()),
        }
    }

    /// Pre-dispatch guard for the array mutator natives, keyed by native name.
    pub(crate) fn guard_array_native(&self, name: &str, args: &[Value]) -> Result<(), RunError> {
        let ops: &[ArrayOp] = match name {
            "arrayShift" | "arraySplice" => &[ArrayOp::Remove],
            "arrayUnshift" => &[ArrayOp::Add],
            "arraySpliceInsert" => &[ArrayOp::Add, ArrayOp::Remove],
            "arraySortBy" | "arraySortDefault" | "arrayReverse" => &[ArrayOp::Write],
            _ => return Ok(()),
        };
        let Some(array) = args.first().and_then(as_object) else {
            return Ok(());
        };
        let level = zun_gc::with_current_heap(|heap| heap.array_integrity(array));
        if level == 0 {
            return Ok(());
        }
        let length = zun_gc::with_current_heap(|heap| heap.array_len(array)).unwrap_or(0);
        if name == "arraySpliceInsert" {
            // Only the halves that actually do something are refused.
            let inserts = args
                .get(3)
                .and_then(as_object)
                .and_then(|items| zun_gc::with_current_heap(|heap| heap.array_len(items)))
                .unwrap_or(0);
            let start = match args.get(1) {
                Some(Value::Int(value, _)) => *value as usize,
                _ => 0,
            };
            if inserts > 0 {
                if let Some(message) = array_refusal(level, ArrayOp::Add) {
                    return Err(RunError::Native(message.into()));
                }
            }
            if start < length {
                if let Some(message) = array_refusal(level, ArrayOp::Remove) {
                    return Err(RunError::Native(message.into()));
                }
            }
            return Ok(());
        }
        for op in ops {
            if *op == ArrayOp::Remove && length == 0 {
                continue;
            }
            if let Some(message) = array_refusal(level, *op) {
                return Err(RunError::Native(message.into()));
            }
        }
        Ok(())
    }
}
