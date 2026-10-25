//! Managed collection dispatch extracted from the interpreter core.

use super::{Interp, RunError};
use super::value::Value;
use super::state::{Aggregate, CombinatorKind, ExecutionFrame, ExecutionOutcome, Reaction, ReactionKind, TimerEntry};
use zun_sched::{Continuation, ContinuationId, Settlement, TaskId};
use zun_module::read_body;

/// `Promise.withResolvers` settle modes, stored in the settler record's
/// `__settleMode` slot (as a float `Any`).
pub(super) const SETTLE_RESOLVE: f64 = 0.0;
pub(super) const SETTLE_REJECT: f64 = 1.0;

/// How a reflective `Object`/`Reflect` operation on a possible proxy resolves.
/// The descriptor-dependent statics (`getOwnPropertyDescriptor`,
/// `defineProperty`, `getPrototypeOf`, `setPrototypeOf`, `isExtensible`,
/// `preventExtensions`, `ownKeys`) consult this before touching the
/// property-descriptor machinery (NUCLEUS #6): a proxy with the matching trap
/// invokes it; a proxy without it, or any non-proxy object, operates on the
/// underlying target directly.
enum ProxyRoute {
    /// The handler defines the trap; invoke it with the target as the leading
    /// positional argument (closure environment threaded first, as with the
    /// `get`/`set`/`has`/`deleteProperty` traps).
    Trap(zun_core::FunctionHandle, zun_gc::Obj),
    /// A proxy without the trap: the operation falls back to its target.
    Target(zun_gc::Obj),
    /// Not a proxy: the operation runs on the object itself.
    NotProxy,
}

impl<'a> Interp<'a> {
    pub(crate) fn call_managed_collection(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        // The typed-array intrinsics are generated from the master table in
        // `zun_core`, so every kind the checker accepts runs on Tier 0.
        if let Some(result) = crate::typed_arrays::call(name, args) {
            return result;
        }
        if let Some(result) = crate::node_buffer::call(name, args) {
            return result;
        }
        if let Some(result) = self.call_typed_array_ext(name, args) {
            return result;
        }
        let object = |value: &Value| match value {
            Value::Object(object) => Some(*object),
            Value::Any(any) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
            _ => None,
        };
        let any = |value: &Value| zun_gc::with_current_heap_mut(|heap| value.box_any(heap));
        // A Promise callback argument: a function value, or absent (the lowering
        // passes `null`/`undefined` for an omitted `then`/`catch`/`finally` slot).
        let callback_arg = |value: Option<&Value>| match value {
            Some(Value::Function(handle)) => Some(*handle),
            _ => None,
        };
        match name {
            "bufferNew" => {
                let length = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_buffer(length));
                Ok(Value::Object(object))
            }
            "bufferLength" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.buffer_len(object))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "bufferSlice" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index_at = |position: usize| -> Result<usize, RunError> {
                    match args.get(position).ok_or(RunError::BadToken)? {
                        // A negative bound is JS's count-from-the-end; the
                        // caller resolves that, so anything negative here has
                        // already been clamped and reads as a huge unsigned
                        // value, which `buffer_slice` clamps to the length.
                        Value::Int(value, _) => Ok(*value as usize),
                        _ => Err(RunError::BadToken),
                    }
                };
                let begin = index_at(1)?;
                let end = index_at(2)?;
                let copy = zun_gc::with_current_heap_mut(|heap| {
                    heap.buffer_slice(object, begin, end)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(copy))
            }
            "bufferResizableNew" => {
                let int_at = |position: usize| -> Result<usize, RunError> {
                    match args.get(position).ok_or(RunError::BadToken)? {
                        Value::Int(value, _) => Ok(*value as usize),
                        _ => Err(RunError::BadToken),
                    }
                };
                let length = int_at(0)?;
                let max = int_at(1)?;
                // `maxByteLength < byteLength` is a construction-time error.
                let object = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_resizable_buffer(length, max)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(object))
            }
            "bufferResize" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let new_length = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let ok = zun_gc::with_current_heap_mut(|heap| heap.buffer_resize(object, new_length))
                    .ok_or(RunError::BadToken)?;
                // A fixed-length buffer or an out-of-range length is a
                // `RangeError`; surfaced through the project's native-error
                // convention (as with other rejected buffer ops).
                if !ok {
                    return Err(RunError::BadToken);
                }
                Ok(Value::Bool(true))
            }
            "bufferTransfer" | "bufferTransferFixed" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // A negative value is the sentinel the surface synthesizes for
                // an omitted `newLength` ("keep the source length").
                let new_length = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => (*value >= 0).then_some(*value as usize),
                    _ => return Err(RunError::BadToken),
                };
                let fixed = name == "bufferTransferFixed";
                // A detached source, or a destination length that exceeds a
                // resizable cap, is a `TypeError`/`RangeError`, surfaced through
                // the project's native-error convention.
                let transferred =
                    zun_gc::with_current_heap_mut(|heap| heap.buffer_transfer(object, new_length, fixed))
                        .ok_or(RunError::BadToken)?;
                Ok(Value::Object(transferred))
            }
            "bufferDetached" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.buffer_detached(object))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "bufferMaxByteLength" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.buffer_max_byte_length(object))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "bufferResizable" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.buffer_resizable(object))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "bufferGetU8" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.buffer_get(object, index))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U8,
                ))
            }
            "bufferSetU8" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u8,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.buffer_set(object, index, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "bufferReadU32" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.buffer_read_u32(object, offset))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U32,
                ))
            }
            "bufferWriteU32" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.buffer_write_u32(object, offset, value)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "bufferReadF64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Float(
                    zun_gc::with_current_heap(|heap| heap.buffer_read_f64(object, offset))
                        .ok_or(RunError::BadToken)?,
                    zun_il::NumKind::F64,
                ))
            }
            "bufferWriteF64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.buffer_write_f64(object, offset, value)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewNew" => {
                let buffer =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let number = |value: &Value| match value {
                    Value::Int(value, _) => Ok(*value as usize),
                    _ => Err(RunError::BadToken),
                };
                let offset = number(args.get(1).ok_or(RunError::BadToken)?)?;
                let length = number(args.get(2).ok_or(RunError::BadToken)?)?;
                let view = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_data_view(buffer, offset, length)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(view))
            }
            "dataViewTrackingNew" => {
                let buffer =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let view = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_data_view_tracking(buffer, offset)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(view))
            }
            "dataViewLength" => {
                let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(zun_gc::with_current_heap(|heap| heap.data_view_len(view)).ok_or(RunError::BadToken)? as i64, zun_il::NumKind::U64))
            }
            "dataViewByteLength" => {
                let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(zun_gc::with_current_heap(|heap| heap.data_view_byte_len(view)).ok_or(RunError::BadToken)? as i64, zun_il::NumKind::U64))
            }
            "dataViewByteOffset" => {
                let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(zun_gc::with_current_heap(|heap| heap.data_view_byte_offset(view)).ok_or(RunError::BadToken)? as i64, zun_il::NumKind::U64))
            }
            "dataViewBuffer" => {
                let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let buffer = zun_gc::with_current_heap(|heap| heap.data_view_buffer(view)).ok_or(RunError::BadToken)?;
                Ok(Value::Object(buffer))
            }
            "isView" => {
                // Any value is accepted; a non-view object — or a value that
                // is not an object at all — is `false`, never an error.
                let verdict = match object(args.first().ok_or(RunError::BadToken)?) {
                    Some(object) => zun_gc::with_current_heap(|heap| heap.is_view(object)),
                    None => false,
                };
                Ok(Value::Bool(verdict))
            }
            "dataViewGetU32" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_u32(view, relative, little_endian)
                    })
                    .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U32,
                ))
            }
            "dataViewSetU32" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(3).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_u32(view, relative, value, little_endian)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewGetInt8" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_i8(view, relative)
                    })
                    .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::I8,
                ))
            }
            "dataViewSetInt8" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as i8,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_i8(view, relative, value)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewGetUint8" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_u8(view, relative)
                    })
                    .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U8,
                ))
            }
            "dataViewSetUint8" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u8,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_u8(view, relative, value)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewGetInt16" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_i16(view, relative, little_endian)
                    })
                    .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::I16,
                ))
            }
            "dataViewSetInt16" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as i16,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(3).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_i16(view, relative, value, little_endian)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewGetInt32" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_i32(view, relative, little_endian)
                    })
                    .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::I32,
                ))
            }
            "dataViewSetInt32" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as i32,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(3).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_i32(view, relative, value, little_endian)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewGetFloat32" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Float(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_f32(view, relative, little_endian)
                    })
                    .ok_or(RunError::BadToken)? as f64,
                    zun_il::NumKind::F32,
                ))
            }
            "dataViewSetFloat32" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value as f32,
                    Value::Int(value, _) => *value as f32,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(3).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_f32(view, relative, value, little_endian)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewGetInt64" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_i64(view, relative, little_endian)
                    })
                    .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::I64,
                ))
            }
            "dataViewSetInt64" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as i64,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(3).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_i64(view, relative, value, little_endian)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewGetUint64" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_u64(view, relative, little_endian)
                    })
                    .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "dataViewSetUint64" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u64,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(3).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_u64(view, relative, value, little_endian)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewGetU16" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_u16(view, relative, little_endian)
                    })
                    .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U16,
                ))
            }
            "dataViewSetU16" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u16,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(3).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_u16(view, relative, value, little_endian)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "dataViewGetF64" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Float(
                    zun_gc::with_current_heap(|heap| {
                        heap.data_view_get_f64(view, relative, little_endian)
                    })
                    .ok_or(RunError::BadToken)?,
                    zun_il::NumKind::F64,
                ))
            }
            "dataViewSetF64" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let relative = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let little_endian = match args.get(3).ok_or(RunError::BadToken)? {
                    Value::Bool(value) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.data_view_set_f64(view, relative, value, little_endian)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "arrayIteratorNew" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let iterator =
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(array))
                        .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "stringSplit" => {
                // String-separator split. Indices/pieces are UTF-16 based for
                // the empty-separator case (one piece per code unit), matching
                // the rest of the String surface.
                let value = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(value) => value.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let separator = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(separator) => separator.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let limit = match args.get(2) {
                    Some(Value::Int(limit, _)) => *limit,
                    _ => -1,
                };
                let mut pieces: Vec<String> = if separator.is_empty() {
                    value
                        .encode_utf16()
                        .map(|unit| String::from_utf16_lossy(&[unit]))
                        .collect()
                } else {
                    value.split(separator.as_ref()).map(str::to_string).collect()
                };
                if limit >= 0 {
                    pieces.truncate(limit as usize);
                }
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(pieces.len());
                    for (index, piece) in pieces.iter().enumerate() {
                        let value = zun_core::Any::string(heap.intern_string(piece.as_str().into()));
                        heap.array_set(array, index, value);
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "stringIteratorNew" => {
                // Iterate by Unicode code point (`chars()`), not UTF-16 unit.
                let value = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(value) => value.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let points: Vec<String> = value.chars().map(|point| point.to_string()).collect();
                let iterator = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(points.len());
                    for (index, point) in points.iter().enumerate() {
                        let value = zun_core::Any::string(heap.intern_string(point.as_str().into()));
                        heap.array_set(array, index, value);
                    }
                    heap.alloc_array_iterator(array)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "arrayIteratorNext" => {
                let iterator =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(
                    zun_gc::with_current_heap_mut(|heap| heap.array_iterator_next(iterator))
                        .and_then(|value| {
                            zun_gc::with_current_heap_mut(|heap| Value::unbox_any(value, heap))
                        })
                        .unwrap_or(Value::Void),
                )
            }
            "arrayIteratorDone" => {
                let iterator =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.array_iterator_done(iterator))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "setIteratorNew" => {
                let set =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let iterator = zun_gc::with_current_heap_mut(|heap| heap.alloc_set_iterator(set))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "setIteratorNext" => {
                let iterator =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(
                    zun_gc::with_current_heap_mut(|heap| heap.set_iterator_next(iterator))
                        .and_then(|value| {
                            zun_gc::with_current_heap_mut(|heap| Value::unbox_any(value, heap))
                        })
                        .unwrap_or(Value::Void),
                )
            }
            "setIteratorDone" => {
                let iterator =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.set_iterator_done(iterator))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "mapIteratorNew" => {
                let map =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let iterator = zun_gc::with_current_heap_mut(|heap| heap.alloc_map_iterator(map))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "mapIteratorNext" => {
                let iterator =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(
                    zun_gc::with_current_heap_mut(|heap| heap.map_iterator_next(iterator))
                        .map(Value::Object)
                        .unwrap_or(Value::Void),
                )
            }
            "mapIteratorDone" => {
                let iterator =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.map_iterator_done(iterator))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "generatorIteratorNew" => {
                if let Some(method_rid) = args.first().and_then(|value| match value {
                    Value::Int(value, _) => Some(*value as u32),
                    Value::Float(value, _) => Some(*value as u32),
                    _ => None,
                }) {
                    return self.create_generator(method_rid, &[]);
                }
                let source = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                // A generator object drives itself through the existing frame
                // resume machinery; anything else (a built-in iterator object, a
                // raw array/set/map, or a user `[Symbol.iterator]` object) routes
                // through the generic iteration protocol.
                if zun_gc::with_current_heap(|heap| heap.generator_state(source).is_some()) {
                    return Ok(Value::Object(source));
                }
                self.protocol_iterator_new(source)
            }
            "generatorIteratorNewArgs" => {
                let method_rid = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    Value::Float(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                let array = object(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let values = zun_gc::with_current_heap(|heap| {
                    let length = heap.array_len(array)?;
                    (0..length).map(|index| heap.array_get(array, index)).collect::<Option<Vec<_>>>()
                })
                .ok_or(RunError::BadToken)?;
                let generator = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_generator(0, method_rid, values)
                });
                Ok(Value::Object(generator))
            }
            "generatorIteratorDone" => {
                let generator = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.generator_state(generator).is_none()) {
                    return self.protocol_done(generator);
                }
                let needs_prefetch = zun_gc::with_current_heap(|heap| {
                    heap.generator_state(generator)
                        .map(|state| !state.done && !state.has_current)
                })
                .ok_or(RunError::BadToken)?;
                if needs_prefetch {
                    let value = self.call_managed_collection(
                        "generatorIteratorNext",
                        &[Value::Object(generator)],
                    )?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or(zun_core::Any::null());
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(state) = heap.generator_state_mut(generator) {
                            if !state.done {
                                state.current = boxed;
                                state.has_current = true;
                            }
                        }
                    });
                }
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| {
                        heap.generator_state(generator).map(|state| state.done)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "generatorIteratorNext" => {
                let generator = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.generator_state(generator).is_none()) {
                    return self.protocol_next(generator);
                }
                let mut state = zun_gc::with_current_heap_mut(|heap| {
                    heap.generator_state_take(generator)
                })
                .ok_or(RunError::BadToken)?;
                if state.done {
                    zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
                    return Ok(Value::Void);
                }
                if state.has_current {
                    let value = zun_gc::with_current_heap(|heap| {
                        Value::unbox_any(state.current, heap)
                    })
                    .ok_or(RunError::BadToken)?;
                    state.has_current = false;
                    zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
                    return Ok(value);
                }
                let outcome = if let Some(saved) = state.frame.take() {
                    let locals = zun_gc::with_current_heap(|heap| {
                        saved.locals.iter().map(|value| Value::unbox_any(*value, heap)).collect::<Option<Vec<_>>>()
                    })
                    .ok_or(RunError::BadToken)?;
                    let stack = zun_gc::with_current_heap(|heap| {
                        saved.eval_stack.iter().map(|value| Value::unbox_any(*value, heap)).collect::<Option<Vec<_>>>()
                    })
                    .ok_or(RunError::BadToken)?;
                    let mut frame = ExecutionFrame::new(state.method_rid, locals);
                    frame.pc = saved.pc;
                    frame.stack = stack;
                    self.resume_suspended(frame)
                } else {
                    let initial_args = zun_gc::with_current_heap(|heap| {
                        state.args.iter().map(|value| Value::unbox_any(*value, heap)).collect::<Option<Vec<_>>>()
                    })
                    .ok_or(RunError::BadToken)?;
                    self.dispatch_outcome(state.method_rid, initial_args, None)
                };
                let result = match outcome {
                    ExecutionOutcome::Suspended(frame) => {
                        let value = self.yielded.take().ok_or(RunError::Malformed(
                            "generator suspended without yielding a value".into(),
                        ))?;
                        state.current = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                            .ok_or(RunError::BadToken)?;
                        let locals = zun_gc::with_current_heap_mut(|heap| {
                            frame.locals.iter().map(|value| value.box_any(heap)).collect::<Option<Vec<_>>>()
                        })
                        .ok_or(RunError::BadToken)?;
                        let eval_stack = zun_gc::with_current_heap_mut(|heap| {
                            frame.stack.iter().map(|value| value.box_any(heap)).collect::<Option<Vec<_>>>()
                        })
                        .ok_or(RunError::BadToken)?;
                        state.frame = Some(zun_gc::GeneratorFrame {
                            method_rid: frame.method_rid,
                            pc: frame.pc,
                            locals,
                            eval_stack,
                        });
                        value
                    }
                    ExecutionOutcome::Complete(value) => {
                        state.done = true;
                        state.current = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                            .ok_or(RunError::BadToken)?;
                        value
                    }
                    ExecutionOutcome::Thrown(value) => return Err(RunError::Thrown(value)),
                    ExecutionOutcome::Fault(error) => return Err(error),
                };
                zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
                Ok(result)
            }
            "mapNew" => {
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_map());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "mapFrom" => {
                // `new Map(iterable)` where the iterable is an ARRAY of
                // `[key, value]` pairs. Arbitrary iterator-protocol objects are
                // deferred; only the array form is accepted here.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_map());
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                // Root both the source and the target: `map_insert` may grow the
                // table (allocating), relocating either handle.
                let roots = [source, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let entry = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let pair = entry
                        .as_object()
                        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                        .ok_or(RunError::BadToken)?;
                    let key = zun_gc::with_current_heap(|heap| heap.array_get(pair, 0))
                        .ok_or(RunError::BadToken)?;
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(pair, 1))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.map_insert(roots[1], key, value));
                }
                Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
            }
            "mapSet" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.map_insert(object, key, value));
                Ok(Value::Void)
            }
            "mapSetI32" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                let value = match args.get(2).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.map_insert(object, zun_core::Any::int(key), zun_core::Any::int(value))).is_none()))
            }
            "mapSetI64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                let value = match args.get(2).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.map_insert(object, zun_core::Any::int(key), zun_core::Any::int(value))).is_none()))
            }
            "mapSetF64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Float(value, _) => *value, _ => return Err(RunError::BadToken) };
                let value = match args.get(2).ok_or(RunError::BadToken)? { Value::Float(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.map_insert(object, zun_core::Any::float(key), zun_core::Any::float(value))).is_none()))
            }
            "mapHasI32" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| heap.map_has(object, zun_core::Any::int(key))).ok_or(RunError::BadToken)?))
            }
            "mapHasI64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| heap.map_has(object, zun_core::Any::int(key))).ok_or(RunError::BadToken)?))
            }
            "mapHasF64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Float(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| heap.map_has(object, zun_core::Any::float(key))).ok_or(RunError::BadToken)?))
            }
            "mapGetI32Or" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                let fallback = match args.get(2).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                let value = zun_gc::with_current_heap(|heap| heap.map_get(object, zun_core::Any::int(key)).and_then(|value| value.as_int())).unwrap_or(fallback);
                Ok(Value::Int(value, zun_il::NumKind::I32))
            }
            "mapGetI64Or" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                let fallback = match args.get(2).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                let value = zun_gc::with_current_heap(|heap| heap.map_get(object, zun_core::Any::int(key)).and_then(|value| value.as_int())).unwrap_or(fallback);
                Ok(Value::Int(value, zun_il::NumKind::I64))
            }
            "mapGetF64Or" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Float(value, _) => *value, _ => return Err(RunError::BadToken) };
                let fallback = match args.get(2).ok_or(RunError::BadToken)? { Value::Float(value, _) => *value, _ => return Err(RunError::BadToken) };
                let value = zun_gc::with_current_heap(|heap| heap.map_get(object, zun_core::Any::float(key)).and_then(|value| value.as_float())).unwrap_or(fallback);
                Ok(Value::Float(value, zun_il::NumKind::F64))
            }
            "mapDeleteI32" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.map_remove(object, zun_core::Any::int(key))).is_some()))
            }
            "mapDeleteI64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.map_remove(object, zun_core::Any::int(key))).is_some()))
            }
            "mapDeleteF64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? { Value::Float(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.map_remove(object, zun_core::Any::float(key))).is_some()))
            }
            "mapGet" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(zun_gc::with_current_heap(|heap| heap.map_get(object, key))
                    .map(Value::Any)
                    .unwrap_or(Value::Void))
            }
            "mapHas" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.map_has(object, key))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "mapSize" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.map_len(object))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "mapDelete" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.map_remove(object, key)).is_some(),
                ))
            }
            "mapClear" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.map_clear(object))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Void)
            }
            "mapValues" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = zun_gc::with_current_heap(|heap| heap.map_entries(object))
                    .ok_or(RunError::BadToken)?;
                let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(entries.len()));
                for (index, (_, value)) in entries.into_iter().enumerate() {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.array_set(array, index, value);
                    });
                }
                let iterator = zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(array))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "mapKeys" | "mapEntries" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = zun_gc::with_current_heap(|heap| heap.map_entries(object))
                    .ok_or(RunError::BadToken)?;
                let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(entries.len()));
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    let item = if name == "mapKeys" {
                        key
                    } else {
                        zun_gc::with_current_heap_mut(|heap| {
                            let item = heap.alloc_array(2);
                            let _ = heap.array_set(item, 0, key);
                            let _ = heap.array_set(item, 1, value);
                            zun_core::Any::object(item.addr() as usize)
                        })
                    };
                    zun_gc::with_current_heap_mut(|heap| {
                        let _ = heap.array_set(array, index, item);
                    });
                }
                let iterator = zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(array))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "mapForEach" => {
                // `callback(value, key, map)` for each entry, in insertion
                // order. Mirrors the Array callback path: entries are
                // materialized into rooted arrays so a GC during callback
                // re-entry relocates them transparently.
                let map_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let entries = zun_gc::with_current_heap(|heap| heap.map_entries(map_object))
                    .ok_or(RunError::BadToken)?;
                let length = entries.len();
                let (keys, values) = zun_gc::with_current_heap_mut(|heap| {
                    let keys = heap.alloc_array(length);
                    let values = heap.alloc_array(length);
                    for (index, (key, value)) in entries.into_iter().enumerate() {
                        heap.array_set(keys, index, key);
                        heap.array_set(values, index, value);
                    }
                    (keys, values)
                });
                let roots = [keys, values, map_object];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let key = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(value),
                        Value::Any(key),
                        Value::Any(zun_core::Any::object(roots[2].addr() as usize)),
                    ];
                    self.invoke_callback(callback, callback_args)?;
                }
                Ok(Value::Void)
            }
            "setNew" => {
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_set());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "setFrom" => {
                // `new Set(iterable)` where the iterable is an ARRAY of
                // elements. Arbitrary iterator-protocol objects are deferred;
                // only the array form is accepted here.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_set());
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let roots = [source, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.set_add(roots[1], value));
                }
                Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
            }
            "setFromIterable" => {
                // `new Set(iterable)` for a non-array built-in iterable: a
                // string (iterated by Unicode code point), another Set (its
                // elements), or another Map (its `[key, value]` entry pairs).
                // An array argument takes the `setFrom` fast path in sema and
                // does not reach here, but is handled too for `any` receivers.
                // Arbitrary user `[Symbol.iterator]` objects are deferred.
                let arg = args.first().ok_or(RunError::BadToken)?;
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_set());
                // String: intern each code point and add it. Interned strings
                // are referenced by a stable id, so no rooting dance is needed.
                if let Value::Str(value) = arg {
                    let points: Vec<String> =
                        value.chars().map(|point| point.to_string()).collect();
                    let roots = [target];
                    let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                    for point in points {
                        zun_gc::with_current_heap_mut(|heap| {
                            let id = heap.intern_string(point.as_str().into());
                            heap.set_add(roots[0], zun_core::Any::string(id));
                        });
                    }
                    return Ok(Value::Any(zun_core::Any::object(roots[0].addr() as usize)));
                }
                let source = object(arg).ok_or(RunError::BadToken)?;
                // Snapshot the source elements into a rooted array, then drain
                // it into the target set (the mapForEach materialization idiom:
                // a GC during `set_add` relocates the rooted array transparently).
                let values: Vec<zun_core::Any> =
                    if let Some(values) = zun_gc::with_current_heap(|heap| heap.set_values(source)) {
                        values
                    } else if let Some(entries) =
                        zun_gc::with_current_heap(|heap| heap.map_entries(source))
                    {
                        // A Set built from a Map iterates `[key, value]` pairs.
                        zun_gc::with_current_heap_mut(|heap| {
                            entries
                                .into_iter()
                                .map(|(key, value)| {
                                    let pair = heap.alloc_array(2);
                                    let _ = heap.array_set(pair, 0, key);
                                    let _ = heap.array_set(pair, 1, value);
                                    zun_core::Any::object(pair.addr() as usize)
                                })
                                .collect()
                        })
                    } else if let Some(length) =
                        zun_gc::with_current_heap(|heap| heap.array_len(source))
                    {
                        (0..length)
                            .map(|index| {
                                zun_gc::with_current_heap(|heap| heap.array_get(source, index))
                                    .ok_or(RunError::BadToken)
                            })
                            .collect::<Result<_, _>>()?
                    } else {
                        // An arbitrary user `[Symbol.iterator]` iterable: drive
                        // the iteration protocol, adding each produced value.
                        let target = self.drive_iterable(source, target, |heap, set, value| {
                            heap.set_add(set, value);
                        })?;
                        return Ok(Value::Any(zun_core::Any::object(target.addr() as usize)));
                    };
                let length = values.len();
                let staged = zun_gc::with_current_heap_mut(|heap| {
                    let staged = heap.alloc_array(length);
                    for (index, value) in values.into_iter().enumerate() {
                        heap.array_set(staged, index, value);
                    }
                    staged
                });
                let roots = [staged, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.set_add(roots[1], value));
                }
                Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
            }
            "mapFromIterable" => {
                // `new Map(iterable)` for a non-array built-in iterable: another
                // Map (its `[key, value]` entries) or another Set whose elements
                // are `[key, value]` pair arrays. An array argument takes the
                // `mapFrom` fast path in sema; it is handled here too for `any`
                // receivers. A string is not a valid map iterable (its elements
                // are not pairs) and is rejected, matching JS's TypeError.
                let arg = args.first().ok_or(RunError::BadToken)?;
                if matches!(arg, Value::Str(_)) {
                    return Err(RunError::Native(
                        "Map constructor requires an iterable of [key, value] pairs".into(),
                    ));
                }
                let source = object(arg).ok_or(RunError::BadToken)?;
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_map());
                // A Map source contributes its entries directly; snapshot them
                // into rooted key/value arrays (the mapForEach idiom).
                if let Some(entries) = zun_gc::with_current_heap(|heap| heap.map_entries(source)) {
                    let length = entries.len();
                    let (keys, values) = zun_gc::with_current_heap_mut(|heap| {
                        let keys = heap.alloc_array(length);
                        let values = heap.alloc_array(length);
                        for (index, (key, value)) in entries.into_iter().enumerate() {
                            heap.array_set(keys, index, key);
                            heap.array_set(values, index, value);
                        }
                        (keys, values)
                    });
                    let roots = [keys, values, target];
                    let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                    for index in 0..length {
                        let key = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                            .ok_or(RunError::BadToken)?;
                        let value = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], index))
                            .ok_or(RunError::BadToken)?;
                        zun_gc::with_current_heap_mut(|heap| heap.map_insert(roots[2], key, value));
                    }
                    return Ok(Value::Any(zun_core::Any::object(roots[2].addr() as usize)));
                }
                // A Set source (or an array): each element must be a `[key,
                // value]` pair array. Snapshot the elements into a rooted array
                // and read each pair out of it.
                let elements: Vec<zun_core::Any> =
                    if let Some(values) = zun_gc::with_current_heap(|heap| heap.set_values(source)) {
                        values
                    } else if let Some(length) =
                        zun_gc::with_current_heap(|heap| heap.array_len(source))
                    {
                        (0..length)
                            .map(|index| {
                                zun_gc::with_current_heap(|heap| heap.array_get(source, index))
                                    .ok_or(RunError::BadToken)
                            })
                            .collect::<Result<_, _>>()?
                    } else {
                        // An arbitrary user `[Symbol.iterator]` iterable yielding
                        // `[key, value]` pair arrays: drive the protocol.
                        let target = self.drive_iterable(source, target, |heap, map, value| {
                            if let Some(pair) = value
                                .as_object()
                                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                            {
                                let key = heap.array_get(pair, 0).unwrap_or_else(zun_core::Any::void);
                                let val = heap.array_get(pair, 1).unwrap_or_else(zun_core::Any::void);
                                heap.map_insert(map, key, val);
                            }
                        })?;
                        return Ok(Value::Any(zun_core::Any::object(target.addr() as usize)));
                    };
                let length = elements.len();
                let staged = zun_gc::with_current_heap_mut(|heap| {
                    let staged = heap.alloc_array(length);
                    for (index, value) in elements.into_iter().enumerate() {
                        heap.array_set(staged, index, value);
                    }
                    staged
                });
                let roots = [staged, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let entry = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let pair = entry
                        .as_object()
                        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                        .ok_or(RunError::BadToken)?;
                    let key = zun_gc::with_current_heap(|heap| heap.array_get(pair, 0))
                        .ok_or(RunError::BadToken)?;
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(pair, 1))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.map_insert(roots[1], key, value));
                }
                Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
            }
            "setAdd" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.set_add(object, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "setAddI32" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.set_add(object, zun_core::Any::int(value))).ok_or(RunError::BadToken)?))
            }
            "setAddI64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.set_add(object, zun_core::Any::int(value))).ok_or(RunError::BadToken)?))
            }
            "setAddF64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? { Value::Float(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.set_add(object, zun_core::Any::float(value))).ok_or(RunError::BadToken)?))
            }
            "setHasI32" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| heap.set_has(object, zun_core::Any::int(value))).ok_or(RunError::BadToken)?))
            }
            "setHasI64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| heap.set_has(object, zun_core::Any::int(value))).ok_or(RunError::BadToken)?))
            }
            "setHasF64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? { Value::Float(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| heap.set_has(object, zun_core::Any::float(value))).ok_or(RunError::BadToken)?))
            }
            "setDeleteI32" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.set_remove(object, zun_core::Any::int(value))).ok_or(RunError::BadToken)?))
            }
            "setDeleteI64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? { Value::Int(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.set_remove(object, zun_core::Any::int(value))).ok_or(RunError::BadToken)?))
            }
            "setDeleteF64" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? { Value::Float(value, _) => *value, _ => return Err(RunError::BadToken) };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| heap.set_remove(object, zun_core::Any::float(value))).ok_or(RunError::BadToken)?))
            }
            "setHas" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.set_has(object, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "setSize" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.set_len(object))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "setDelete" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.set_remove(object, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "setClear" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.set_clear(object))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Void)
            }
            "setValues" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let values = zun_gc::with_current_heap(|heap| heap.set_values(object))
                    .ok_or(RunError::BadToken)?;
                let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(values.len()));
                for (index, value) in values.into_iter().enumerate() {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.array_set(array, index, value);
                    });
                }
                let iterator = zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(array))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "setEntries" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let values = zun_gc::with_current_heap(|heap| heap.set_values(object))
                    .ok_or(RunError::BadToken)?;
                let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(values.len()));
                for (index, value) in values.into_iter().enumerate() {
                    let pair = zun_gc::with_current_heap_mut(|heap| {
                        let pair = heap.alloc_array(2);
                        let _ = heap.array_set(pair, 0, value);
                        let _ = heap.array_set(pair, 1, value);
                        pair
                    });
                    zun_gc::with_current_heap_mut(|heap| {
                        let _ = heap.array_set(
                            array,
                            index,
                            zun_core::Any::object(pair.addr() as usize),
                        );
                    });
                }
                let iterator = zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(array))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "setForEach" => {
                // `callback(value, value, set)` for each element, in insertion
                // order (Set passes the value twice per the spec).
                let set_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let values = zun_gc::with_current_heap(|heap| heap.set_values(set_object))
                    .ok_or(RunError::BadToken)?;
                let length = values.len();
                let holder = zun_gc::with_current_heap_mut(|heap| {
                    let holder = heap.alloc_array(length);
                    for (index, value) in values.into_iter().enumerate() {
                        heap.array_set(holder, index, value);
                    }
                    holder
                });
                let roots = [holder, set_object];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(value),
                        Value::Any(value),
                        Value::Any(zun_core::Any::object(roots[1].addr() as usize)),
                    ];
                    self.invoke_callback(callback, callback_args)?;
                }
                Ok(Value::Void)
            }
            "weakMapNew" => {
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_weak_map());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "weakMapSet" => {
                let map_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    object(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.weak_map_set_object(map_object, key, value)
                })))
            }
            "weakMapGet" => {
                let map_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(zun_gc::with_current_heap(|heap| {
                    heap.weak_map_get_value_object(map_object, key)
                })
                .map(|value| Value::Any(zun_core::Any::object(value.addr() as usize)))
                .unwrap_or(Value::Void))
            }
            "weakMapHas" => {
                let map_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| {
                    heap.weak_map_has_object(map_object, key)
                })))
            }
            "weakMapDelete" => {
                let map_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.weak_map_delete_object(map_object, key)
                })))
            }
            "weakSetNew" => {
                // A WeakSet reuses the WeakMap ephemeron floor; entries are
                // stored value-less by keying each object to itself.
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_weak_map());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "weakSetAdd" => {
                let set_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.weak_map_set_object(set_object, key, key)
                })))
            }
            "weakSetHas" => {
                let set_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| {
                    heap.weak_map_has_object(set_object, key)
                })))
            }
            "weakSetDelete" => {
                let set_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.weak_map_delete_object(set_object, key)
                })))
            }
            "weakRefNew" => {
                let target =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_weak_ref(target));
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "weakRefDeref" => {
                let ref_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(zun_gc::with_current_heap(|heap| heap.weak_ref_get_object(ref_object))
                    .map(|target| Value::Any(zun_core::Any::object(target.addr() as usize)))
                    .unwrap_or(Value::Void))
            }
            "regexpNew" => {
                let source = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(source) => source.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let flags = match args.get(1) {
                    Some(Value::Str(flags)) => flags.to_string(),
                    None => String::new(),
                    Some(_) => return Err(RunError::BadToken),
                };
                let object = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_regex_with_flags(source, flags)
                })
                .map_err(|error| RunError::Native(error.to_string()))?;
                Ok(Value::Object(object))
            }
            "regexpExec" => {
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let input = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(input) => input.clone(),
                    _ => return Err(RunError::BadToken),
                };
                match zun_gc::with_current_heap_mut(|heap| heap.regex_exec(object, &input))
                    .ok_or(RunError::BadToken)?
                    .map_err(|error| RunError::Native(error.to_string()))?
                {
                    Some(array) => Ok(Value::Object(array)),
                    None => Ok(Value::Null),
                }
            }
            "stringMatchAll" => {
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let input = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(input) => input.clone(),
                    _ => return Err(RunError::BadToken),
                };
                // ECMAScript requires the `g` flag; V8/Node throw a TypeError
                // otherwise. Surface it as a native error (project convention).
                let global = zun_gc::with_current_heap(|heap| {
                    heap.regex_flags(object).map(|flags| flags.contains('g'))
                })
                .ok_or(RunError::BadToken)?;
                if !global {
                    return Err(RunError::Native(
                        "matchAll must be called with a global RegExp (the `g` flag)".into(),
                    ));
                }
                let array =
                    zun_gc::with_current_heap_mut(|heap| heap.regex_exec_all(object, &input))
                        .ok_or(RunError::BadToken)?
                        .map_err(|error| RunError::Native(error.to_string()))?;
                Ok(Value::Object(array))
            }
            "bigIntNew" => {
                let text = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(text) => text,
                    _ => return Err(RunError::BadToken),
                };
                let value = zun_core::BigInt::parse(text)
                    .map_err(|error| RunError::Native(error.into()))?;
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value));
                Ok(Value::Object(object))
            }
            "bigIntFromNumber" => {
                let number = match args.first().ok_or(RunError::BadToken)? {
                    Value::Float(number, _) => *number,
                    Value::Int(number, _) => *number as f64,
                    _ => return Err(RunError::BadToken),
                };
                let value = zun_core::BigInt::from_f64(number)
                    .map_err(|error| RunError::Native(error.into()))?;
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value));
                Ok(Value::Object(object))
            }
            "bigIntPow" | "bigIntAnd" | "bigIntOr" | "bigIntXor" | "bigIntShl" | "bigIntShr" => {
                let left = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let right = object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let (left, right) = zun_gc::with_current_heap(|heap| {
                    Some((heap.bigint_value(left)?.clone(), heap.bigint_value(right)?.clone()))
                }).ok_or(RunError::BadToken)?;
                let value = match name {
                    "bigIntPow" => left
                        .pow(&right)
                        .map_err(|error| RunError::Native(error.into()))?,
                    "bigIntAnd" => left.bitand(&right),
                    "bigIntOr" => left.bitor(&right),
                    "bigIntXor" => left.bitxor(&right),
                    "bigIntShl" | "bigIntShr" => {
                        let amount = right
                            .to_u64()
                            .ok_or_else(|| RunError::Native("BigInt shift amount too large".into()))?;
                        // A negative shift count flips the direction (spec).
                        let shift_left = (name == "bigIntShl") != right.is_negative();
                        if shift_left {
                            left.shl(amount)
                        } else {
                            left.shr(amount)
                        }
                    }
                    _ => unreachable!(),
                };
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value));
                Ok(Value::Object(object))
            }
            "bigIntNot" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(object).cloned())
                    .ok_or(RunError::BadToken)?;
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value.bitnot()));
                Ok(Value::Object(object))
            }
            "bigIntAsIntN" | "bigIntAsUintN" => {
                let bits = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(bits, _) => *bits,
                    Value::Float(bits, _) => *bits as i64,
                    _ => return Err(RunError::BadToken),
                };
                if bits < 0 {
                    return Err(RunError::Native("BigInt bit width must be non-negative".into()));
                }
                let object = object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(object).cloned())
                    .ok_or(RunError::BadToken)?;
                let result = if name == "bigIntAsIntN" {
                    value.as_int_n(bits as u64)
                } else {
                    value.as_uint_n(bits as u64)
                };
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(result));
                Ok(Value::Object(object))
            }
            "bigIntAdd" | "bigIntSub" | "bigIntMul" | "bigIntDiv" | "bigIntRem" | "bigIntCompare"
            | "bigIntEq" | "bigIntNe" | "bigIntLt" | "bigIntLe" | "bigIntGt" | "bigIntGe" => {
                let left = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let right = object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let (left, right) = zun_gc::with_current_heap(|heap| {
                    Some((heap.bigint_value(left)?.clone(), heap.bigint_value(right)?.clone()))
                }).ok_or(RunError::BadToken)?;
                if name == "bigIntCompare" {
                    let result = match left.cmp(&right) {
                        std::cmp::Ordering::Less => -1,
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                    };
                    Ok(Value::Int(result, zun_il::NumKind::I32))
                } else if matches!(name, "bigIntEq" | "bigIntNe" | "bigIntLt" | "bigIntLe" | "bigIntGt" | "bigIntGe") {
                    let ordering = left.cmp(&right);
                    let result = match name {
                        "bigIntEq" => ordering == std::cmp::Ordering::Equal,
                        "bigIntNe" => ordering != std::cmp::Ordering::Equal,
                        "bigIntLt" => ordering == std::cmp::Ordering::Less,
                        "bigIntLe" => ordering != std::cmp::Ordering::Greater,
                        "bigIntGt" => ordering == std::cmp::Ordering::Greater,
                        _ => ordering != std::cmp::Ordering::Less,
                    };
                    Ok(Value::Bool(result))
                } else {
                    let value = match name {
                        "bigIntAdd" => left.add(&right),
                        "bigIntSub" => left.sub(&right),
                        "bigIntMul" => left.mul(&right),
                        "bigIntDiv" | "bigIntRem" => {
                            let Some((quotient, remainder)) = left.div_rem(&right) else {
                                return Err(RunError::Native("BigInt division by zero".into()));
                            };
                            if name == "bigIntDiv" { quotient } else { remainder }
                        }
                        _ => unreachable!(),
                    };
                    let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value));
                    Ok(Value::Object(object))
                }
            }
            "bigIntNeg" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(object).cloned())
                    .ok_or(RunError::BadToken)?;
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value.neg()));
                Ok(Value::Object(object))
            }
            "bigIntToString" => {
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(object).cloned())
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Str(value.to_string().into()))
            }
            "regexpIsMatch" => {
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let input = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(input) => input,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.regex_is_match(object, input))
                        .ok_or(RunError::BadToken)?
                        .map_err(|error| RunError::Native(error.to_string()))?,
                ))
            }
            "regexpSearch" | "regexpSplit" | "regexpReplace" => {
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let input = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(input) => input.clone(),
                    _ => return Err(RunError::BadToken),
                };
                match name {
                    "regexpSearch" => {
                        let index =
                            zun_gc::with_current_heap(|heap| heap.regex_search(object, &input))
                                .ok_or(RunError::BadToken)?
                                .map_err(|error| RunError::Native(error.to_string()))?;
                        Ok(Value::Int(index, zun_il::NumKind::I32))
                    }
                    "regexpSplit" => {
                        let array =
                            zun_gc::with_current_heap_mut(|heap| heap.regex_split(object, &input))
                                .ok_or(RunError::BadToken)?
                                .map_err(|error| RunError::Native(error.to_string()))?;
                        Ok(Value::Object(array))
                    }
                    _ => {
                        let replacement = match args.get(2).ok_or(RunError::BadToken)? {
                            Value::Str(text) => text.clone(),
                            _ => return Err(RunError::BadToken),
                        };
                        let result = zun_gc::with_current_heap(|heap| {
                            heap.regex_replace_first(object, &input, &replacement)
                        })
                        .ok_or(RunError::BadToken)?
                        .map_err(|error| RunError::Native(error.to_string()))?;
                        Ok(Value::Str(result.into()))
                    }
                }
            }
            "regexpSource" => {
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let source = zun_gc::with_current_heap(|heap| {
                    heap.regex_source(object).map(str::to_owned)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Str(source.into()))
            }
            "finalizationRegistryNew" => {
                let cleanup = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(callback) => *callback,
                    _ => return Err(RunError::BadToken),
                };
                let object = zun_gc::with_current_heap_mut(|heap| {
                    let registry = heap.alloc(0, 0);
                    heap.finalization_registry_set_cleanup(registry, cleanup);
                    registry
                });
                Ok(Value::Object(object))
            }
            "finalizationRegister" => {
                let registry = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let target = object(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                // The held value is an arbitrary immediate `Any` carrier; box a
                // non-`any` operand into one so an object held value is traced.
                let held = zun_gc::with_current_heap_mut(|heap| {
                    args.get(2)
                        .ok_or(RunError::BadToken)
                        .and_then(|value| value.clone().box_any(heap).ok_or(RunError::BadToken))
                })?;
                // The unregister token is an object identity. A non-object
                // value (the surface's sentinel for an omitted token) leaves
                // the registration permanently un-unregisterable, per spec.
                let unregister_token = args
                    .get(3)
                    .and_then(|value| object(value))
                    .map(|token| token.addr() as usize)
                    .unwrap_or(0);
                zun_gc::with_current_heap_mut(|heap| {
                    let cleanup = heap
                        .finalization_registry_cleanup(registry)
                        .ok_or(RunError::BadToken)?;
                    heap.finalization_register_with_held(
                        target,
                        held,
                        cleanup,
                        unregister_token,
                    );
                    Ok::<(), RunError>(())
                })?;
                Ok(Value::Void)
            }
            "finalizationUnregister" => {
                let _registry = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let unregister_token = args
                    .get(1)
                    .and_then(|value| object(value))
                    .map(|token| token.addr() as usize)
                    .unwrap_or(0);
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.finalization_unregister_by_token(unregister_token)
                })))
            }
            "reflectHas" => {
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| {
                    heap.dynamic_has(object, name)
                })))
            }
            "reflectDelete" => {
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_delete(object, name)
                })))
            }
            "proxyNew" => {
                let target = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let handler = object(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let proxy = zun_gc::with_current_heap_mut(|heap| heap.alloc_proxy(target, handler));
                Ok(Value::Object(proxy))
            }
            "proxyGet" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy)) else {
                    return Err(RunError::BadToken);
                };
                let trap = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "get"));
                let result = if let Some(callback) = trap {
                    let mut call_args = Vec::with_capacity(3);
                    if let Some(address) = callback.environment() {
                        call_args.push(Value::Object(unsafe {
                            zun_gc::Obj::from_addr(address as *mut u64)
                        }));
                    }
                    call_args.push(Value::Object(target));
                    call_args.push(Value::Str(key.into()));
                    self.call(callback.method_rid(), call_args, None)?
                } else {
                    zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get(target, &key)
                            .map(Value::Any)
                            .unwrap_or(Value::Void)
                    })
                };
                Ok(Value::Any(any(&result).unwrap_or_else(zun_core::Any::null)))
            }
            "proxySet" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let value = any(args.get(2).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy)) else {
                    return Err(RunError::BadToken);
                };
                let trap = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "set"));
                if let Some(callback) = trap {
                    let mut call_args = Vec::with_capacity(4);
                    if let Some(address) = callback.environment() {
                        call_args.push(Value::Object(unsafe {
                            zun_gc::Obj::from_addr(address as *mut u64)
                        }));
                    }
                    call_args.push(Value::Object(target));
                    call_args.push(Value::Str(key.into()));
                    call_args.push(Value::Any(value));
                    let result = self.call(callback.method_rid(), call_args, None)?;
                    return Ok(Value::Bool(matches!(result, Value::Bool(true))));
                }
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set(target, key.to_string(), value);
                });
                Ok(Value::Bool(true))
            }
            "proxyHas" | "proxyDelete" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy)) else {
                    return Err(RunError::BadToken);
                };
                let trap_name = if name == "proxyHas" { "has" } else { "deleteProperty" };
                if let Some(callback) = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, trap_name)) {
                    let mut call_args = Vec::with_capacity(3);
                    if let Some(address) = callback.environment() {
                        call_args.push(Value::Object(unsafe {
                            zun_gc::Obj::from_addr(address as *mut u64)
                        }));
                    }
                    call_args.push(Value::Object(target));
                    call_args.push(Value::Str(key.into()));
                    let result = self.call(callback.method_rid(), call_args, None)?;
                    return Ok(Value::Bool(matches!(result, Value::Bool(true))));
                }
                let result = zun_gc::with_current_heap_mut(|heap| {
                    if name == "proxyHas" {
                        heap.dynamic_has(target, &key)
                    } else {
                        heap.dynamic_delete(target, &key)
                    }
                });
                Ok(Value::Bool(result))
            }
            "proxyNewCallable" => {
                // A proxy over a callable: the target is a `functionBox` record
                // (a GC object carrying the function under `__boxedFn`), so the
                // proxy's stored target stays a GC object like every other
                // proxy. The handler is an ordinary dynamic object.
                let target = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let handler = object(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let proxy = zun_gc::with_current_heap_mut(|heap| heap.alloc_proxy(target, handler));
                Ok(Value::Object(proxy))
            }
            "proxyApply" => {
                // `proxy(...args)` / `Reflect.apply(proxy, thisArg, args)`. With
                // an `apply` trap: `handler.apply(target, thisArg, argsArray)`.
                // Without one: invoke the underlying boxed function with the
                // `this` argument threaded as the leading positional slot (the
                // same convention `functionApply` uses).
                let proxy = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let this_arg = args.get(1).ok_or(RunError::BadToken)?.clone();
                let array = object(args.get(2).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy)) else {
                    return Err(RunError::BadToken);
                };
                if let Some(callback) = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "apply")) {
                    let result = self.invoke_proxy_trap(
                        callback,
                        vec![Value::Object(target), this_arg, Value::Object(array)],
                    )?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    return Ok(Value::Any(boxed));
                }
                // No `apply` trap: invoke the underlying boxed function with the
                // spread arguments. The `thisArg` is not prepended (a callable
                // proxy called directly, `proxy(...args)`, forwards exactly its
                // arguments to the target).
                let handle = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_function(target, "__boxedFn")
                })
                .ok_or(RunError::BadToken)?;
                let _ = &this_arg;
                let spread = self.read_array_values(array)?;
                let result = self.invoke_function_value(handle, spread)?;
                let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                Ok(Value::Any(boxed))
            }
            "dynamicCall0" | "dynamicCall1" | "dynamicCall2" => {
                let namespace = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let member = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.clone(),
                    _ => return Err(RunError::BadToken),
                };
                // A built-in iterator object (from `Array`/`Map`/`Set`
                // `keys`/`values`/`entries`) answers `next()` with a fresh
                // `{ value, done }` result object, per the iterator protocol.
                if member.as_ref() == "next" {
                    if let Some(result) = self.builtin_iterator_result(namespace)? {
                        return Ok(result);
                    }
                }
                let call_args = args[2..]
                    .iter()
                    .map(|arg| match arg {
                        Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .ok_or(RunError::BadToken),
                        other => Ok(other.clone()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                // A dynamic object may carry a callable member in its function
                // slot (e.g. the `resolve`/`reject` of `Promise.withResolvers`,
                // or a `bind`-stored function). Invoke it directly; the result
                // is re-boxed to `Any` for the dynamic call site. Only when no
                // such member exists is this a namespace call routed to the
                // loader.
                if let Some(handle) =
                    zun_gc::with_current_heap(|heap| heap.dynamic_get_function(namespace, &member))
                {
                    let result = self.invoke_function_value(handle, call_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    return Ok(Value::Any(boxed));
                }
                let loader = self.loader.ok_or(RunError::BadToken)?;
                let result = super::namespace::call_namespace_member(
                    loader,
                    &Value::Object(namespace),
                    &member,
                    call_args,
                    &mut *self.output,
                )?;
                if matches!(result, Value::Object(_)) {
                    // The callee ran on its own heap; references cannot cross.
                    return Err(RunError::BadToken);
                }
                Ok(Value::Any(any(&result).ok_or(RunError::BadToken)?))
            }
            "dynamicSetFunction" => {
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let function = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set_function(object, name.to_string(), function);
                });
                Ok(Value::Void)
            }
            "applyDecorator" => {
                // `applyDecorator(decoratorFn, value, context)` — invoke a
                // decorator-factory result (a function value) with
                // `(value, context)` at class-definition time, discarding the
                // return. Backs decorator factories `@dec(args)`.
                let function = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                let value = args.get(1).ok_or(RunError::BadToken)?.clone();
                let context = args.get(2).ok_or(RunError::BadToken)?.clone();
                self.invoke_function_value(function, vec![value, context])?;
                Ok(Value::Void)
            }
            "classSetSymbolFunction" => {
                // `classSetSymbolFunction(classRid, symbol, fn)` — attach a
                // method under a Symbol key to a typed class's per-type side
                // table (the typed-class analogue of
                // `dynamicSetSymbolFunction`).
                let class_rid = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    Value::Float(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                let symbol = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_symbol().ok_or(RunError::BadToken)?,
                    _ => return Err(RunError::BadToken),
                };
                let function = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                self.class_symbol_methods
                    .insert((class_rid, symbol.get()), function);
                Ok(Value::Void)
            }
            "dynamicSetSymbolFunction" => {
                // Attach a method under a Symbol key (e.g.
                // `C[Symbol.hasInstance]`). The symbol arrives as an immediate
                // `Any`; the function value goes into the dedicated
                // symbol-function slot since functions are not boxable `Any`.
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let symbol = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_symbol().ok_or(RunError::BadToken)?,
                    _ => return Err(RunError::BadToken),
                };
                let function = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set_symbol_function(object, symbol, function);
                });
                Ok(Value::Void)
            }
            "instanceofDynamic" => {
                // `value instanceof C` where `C` is a dynamic object. If `C`
                // carries a `[Symbol.hasInstance]` method, call it with `value`
                // and return `ToBoolean` of the result; otherwise a dynamic
                // object has no class identity, so the answer is `false`.
                let value = args.first().ok_or(RunError::BadToken)?.clone();
                let rhs = object(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let handler = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_symbol_function(
                        rhs,
                        zun_core::WellKnownSymbol::HasInstance.id(),
                    )
                });
                let found = match handler {
                    Some(handle) => {
                        let result = self.invoke_function_value(handle, vec![value])?;
                        zun_gc::with_current_heap(|heap| is_truthy(&result, heap))
                    }
                    None => false,
                };
                Ok(Value::Bool(found))
            }
            "stringCoerce" => {
                // `String(x)` — ECMAScript ToString. Primitives stringify
                // directly; an object runs OrdinaryToPrimitive with hint
                // "string": call `[Symbol.toPrimitive]("string")` if present and
                // ToString its result, else expose `[Symbol.toStringTag]` as
                // `"[object <Tag>]"`, else `"[object Object]"`.
                let value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let text = match value {
                    Value::Object(obj) => {
                        let to_prim = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get_symbol_function(
                                obj,
                                zun_core::WellKnownSymbol::ToPrimitive.id(),
                            )
                        });
                        if let Some(handle) = to_prim {
                            let hint = zun_gc::with_current_heap_mut(|heap| {
                                Value::Any(zun_core::Any::string(
                                    heap.intern_string("string".into()),
                                ))
                            });
                            let result = self.invoke_function_value(handle, vec![hint])?;
                            unbox_arg(&result)?.display()
                        } else {
                            let tag = zun_gc::with_current_heap(|heap| {
                                heap.dynamic_get_symbol(
                                    obj,
                                    zun_core::WellKnownSymbol::ToStringTag.id(),
                                )
                                .and_then(|any| match Value::unbox_any(any, heap) {
                                    Some(Value::Str(text)) => Some(text.to_string()),
                                    _ => None,
                                })
                            });
                            match tag {
                                Some(tag) => format!("[object {tag}]"),
                                None => "[object Object]".to_string(),
                            }
                        }
                    }
                    other => other.display(),
                };
                Ok(Value::Str(text.into()))
            }
            "numberCoerce" => {
                // `Number(x)` — ECMAScript ToNumber. A number is itself; a
                // string parses (NaN on failure); a boolean is 0/1; `null`→0,
                // `undefined`→NaN. An object runs OrdinaryToPrimitive with hint
                // "number": call `[Symbol.toPrimitive]("number")` if present and
                // ToNumber its result, else ToNumber of its string coercion.
                let value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let number = match value {
                    Value::Int(number, _) => number as f64,
                    Value::Float(number, _) => number,
                    Value::Bool(flag) => if flag { 1.0 } else { 0.0 },
                    Value::Null => 0.0,
                    Value::Void => f64::NAN,
                    Value::Str(text) => js_string_to_number(&text),
                    Value::Object(obj) => {
                        let to_prim = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get_symbol_function(
                                obj,
                                zun_core::WellKnownSymbol::ToPrimitive.id(),
                            )
                        });
                        if let Some(handle) = to_prim {
                            let hint = zun_gc::with_current_heap_mut(|heap| {
                                Value::Any(zun_core::Any::string(
                                    heap.intern_string("number".into()),
                                ))
                            });
                            let result = self.invoke_function_value(handle, vec![hint])?;
                            match unbox_arg(&result)? {
                                Value::Str(text) => js_string_to_number(&text),
                                other => zun_gc::with_current_heap(|heap| to_number(&other, heap)),
                            }
                        } else {
                            f64::NAN
                        }
                    }
                    _ => f64::NAN,
                };
                Ok(Value::Float(number, zun_il::NumKind::F64))
            }
            "functionApply" => {
                // `f.apply(thisArg, argsArray)`: invoke `f` with `thisArg` as
                // the leading positional argument, spread with the array.
                let target = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let this_arg = args.get(1).ok_or(RunError::BadToken)?.clone();
                let array = object(args.get(2).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let spread = self.read_array_values(array)?;
                let mut call_args = Vec::with_capacity(1 + spread.len());
                call_args.push(this_arg);
                call_args.extend(spread);
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
                let array = object(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
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
            "arrayForEach" | "arrayMap" | "arrayFilter" | "arrayFind" | "arrayFindIndex"
            | "arraySome" | "arrayEvery" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                // A fresh result array is needed only for `map`/`filter`.
                let result = if name == "arrayMap" {
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_array(length))
                } else if name == "arrayFilter" {
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0))
                } else {
                    source
                };
                // Root both the source and the result across callback re-entry;
                // a GC during `invoke_callback` may relocate either, so every
                // access reads the (possibly rewritten) addresses back out.
                let roots = [source, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let produced = self.invoke_callback(callback, callback_args)?;
                    match name {
                        "arrayForEach" => {}
                        "arrayMap" => {
                            let boxed = zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                                .ok_or(RunError::BadToken)?;
                            zun_gc::with_current_heap_mut(|heap| {
                                heap.array_set(roots[1], index, boxed)
                            });
                        }
                        "arrayFilter" => {
                            let keep = zun_gc::with_current_heap(|heap| is_truthy(&produced, heap));
                            if keep {
                                let boxed =
                                    zun_gc::with_current_heap_mut(|heap| Value::Any(element).box_any(heap))
                                        .ok_or(RunError::BadToken)?;
                                zun_gc::with_current_heap_mut(|heap| {
                                    heap.array_push(roots[1], boxed)
                                });
                            }
                        }
                        "arrayFind" => {
                            if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                                // Return the concrete element value (unboxed),
                                // matching ordinary element access.
                                return zun_gc::with_current_heap(|heap| {
                                    Value::unbox_any(element, heap)
                                })
                                .ok_or(RunError::BadToken);
                            }
                        }
                        "arrayFindIndex" => {
                            if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                                return Ok(Value::Int(index as i64, zun_il::NumKind::I32));
                            }
                        }
                        "arraySome" => {
                            if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                                return Ok(Value::Bool(true));
                            }
                        }
                        "arrayEvery" => {
                            if !zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                                return Ok(Value::Bool(false));
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                match name {
                    "arrayForEach" => Ok(Value::Void),
                    "arrayMap" | "arrayFilter" => Ok(Value::Object(roots[1])),
                    "arrayFind" => Ok(Value::Void),
                    "arrayFindIndex" => Ok(Value::Int(-1, zun_il::NumKind::I32)),
                    "arraySome" => Ok(Value::Bool(false)),
                    "arrayEvery" => Ok(Value::Bool(true)),
                    _ => unreachable!(),
                }
            }
            "arrayReduce" | "arrayReduceRight" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let initial = args.get(2).ok_or(RunError::BadToken)?.clone();
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                // Hold the accumulator inside a one-slot GC array so it survives
                // callback re-entry regardless of whether it is an object.
                let accumulator = zun_gc::with_current_heap_mut(|heap| {
                    let holder = heap.alloc_array(1);
                    if let Some(boxed) = initial.box_any(heap) {
                        heap.array_set(holder, 0, boxed);
                    }
                    holder
                });
                let roots = [source, accumulator];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for step in 0..length {
                    let index = if name == "arrayReduce" {
                        step
                    } else {
                        length - 1 - step
                    };
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let acc = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(acc),
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let next = self.invoke_callback(callback, callback_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| next.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(roots[1], 0, boxed));
                }
                let result = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
                    .ok_or(RunError::BadToken)?;
                // Unbox to the concrete accumulator value so the result matches
                // the representation an in-source reducer would have returned.
                zun_gc::with_current_heap(|heap| Value::unbox_any(result, heap))
                    .ok_or(RunError::BadToken)
            }
            "arraySortBy" | "arraySortDefault" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let comparator = if name == "arraySortBy" {
                    match args.get(1).ok_or(RunError::BadToken)? {
                        Value::Function(handle) => Some(*handle),
                        _ => return Err(RunError::BadToken),
                    }
                } else {
                    None
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let roots = [source];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                // Insertion sort so each comparison re-enters the interpreter at
                // most once; elements are read back from the rooted source after
                // every comparator call in case a GC relocated the backing store.
                for i in 1..length {
                    let mut j = i;
                    while j > 0 {
                        let left = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], j - 1))
                            .ok_or(RunError::BadToken)?;
                        let right = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], j))
                            .ok_or(RunError::BadToken)?;
                        let ordered_before = match comparator {
                            Some(handle) => {
                                let outcome = self.invoke_callback(
                                    handle,
                                    vec![Value::Any(left), Value::Any(right)],
                                )?;
                                // `<= 0` keeps order; a non-numeric (`NaN`)
                                // result is treated as `0` (stable, no swap).
                                !(zun_gc::with_current_heap(|heap| to_number(&outcome, heap)) > 0.0)
                            }
                            None => {
                                let (a, b) = zun_gc::with_current_heap(|heap| {
                                    (
                                        stringify_any(left, heap).unwrap_or_default(),
                                        stringify_any(right, heap).unwrap_or_default(),
                                    )
                                });
                                a <= b
                            }
                        };
                        if ordered_before {
                            break;
                        }
                        zun_gc::with_current_heap_mut(|heap| {
                            let a = heap.array_get(roots[0], j - 1);
                            let b = heap.array_get(roots[0], j);
                            if let (Some(a), Some(b)) = (a, b) {
                                heap.array_set(roots[0], j - 1, b);
                                heap.array_set(roots[0], j, a);
                            }
                        });
                        j -= 1;
                    }
                }
                Ok(Value::Object(roots[0]))
            }
            "arrayJoin" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let separator = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(text) => text.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let joined = zun_gc::with_current_heap(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let mut parts = Vec::with_capacity(length);
                    for index in 0..length {
                        let element = heap.array_get(source, index).unwrap_or_else(zun_core::Any::void);
                        // `null`/`undefined` render as the empty string per spec.
                        parts.push(stringify_any(element, heap).unwrap_or_default());
                    }
                    parts.join(&separator)
                });
                Ok(Value::Str(joined.into()))
            }
            "arrayShift" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let removed = zun_gc::with_current_heap_mut(|heap| {
                    if heap.array_len(source).unwrap_or(0) == 0 {
                        None
                    } else {
                        heap.array_remove(source, 0)
                            .and_then(|value| Value::unbox_any(value, heap))
                    }
                });
                Ok(removed.unwrap_or(Value::Void))
            }
            "arrayUnshift" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let length = zun_gc::with_current_heap_mut(|heap| {
                    heap.array_insert(source, 0, value);
                    heap.array_len(source).unwrap_or(0)
                });
                Ok(Value::Int(length as i64, zun_il::NumKind::U64))
            }
            "arraySplice" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let start = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let delete_count = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let removed = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let start = start.min(length);
                    let delete_count = delete_count.min(length - start);
                    let result = heap.alloc_array(delete_count);
                    for offset in 0..delete_count {
                        // Elements shift down on each removal, so the removal
                        // index stays `start`.
                        if let Some(element) = heap.array_remove(source, start) {
                            heap.array_set(result, offset, element);
                        }
                    }
                    result
                });
                Ok(Value::Object(removed))
            }
            "arrayAt" => {
                // ECMAScript relative indexing: a negative index counts from the
                // end; an out-of-range index yields `undefined`.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let raw = zun_gc::with_current_heap(|heap| {
                    to_number(args.get(1).unwrap_or(&Value::Void), heap)
                });
                let mut index = raw.trunc();
                if index < 0.0 {
                    index += length as f64;
                }
                if index < 0.0 || index >= length as f64 {
                    return Ok(Value::Void);
                }
                let element = zun_gc::with_current_heap(|heap| heap.array_get(source, index as usize))
                    .ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap(|heap| Value::unbox_any(element, heap))
                    .ok_or(RunError::BadToken)
            }
            "arrayFindLast" | "arrayFindLastIndex" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let roots = [source];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let mut step = length;
                while step > 0 {
                    step -= 1;
                    let index = step;
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let produced = self.invoke_callback(callback, callback_args)?;
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        if name == "arrayFindLast" {
                            return zun_gc::with_current_heap(|heap| Value::unbox_any(element, heap))
                                .ok_or(RunError::BadToken);
                        }
                        return Ok(Value::Int(index as i64, zun_il::NumKind::I32));
                    }
                }
                if name == "arrayFindLast" {
                    Ok(Value::Void)
                } else {
                    Ok(Value::Int(-1, zun_il::NumKind::I32))
                }
            }
            "arrayFlat" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let depth_raw = zun_gc::with_current_heap(|heap| {
                    to_number(args.get(1).unwrap_or(&Value::Void), heap)
                });
                let depth = if depth_raw.is_nan() || depth_raw < 0.0 {
                    0
                } else {
                    depth_raw.trunc() as i64
                };
                // No callback re-entry: the whole flatten runs inside one heap
                // borrow, so no GC can relocate the arrays mid-build.
                let result = zun_gc::with_current_heap_mut(|heap| {
                    fn flatten(heap: &mut zun_gc::Heap, src: zun_gc::Obj, depth: i64, out: zun_gc::Obj) {
                        let length = heap.array_len(src).unwrap_or(0);
                        for index in 0..length {
                            let Some(element) = heap.array_get(src, index) else { continue };
                            let child = (depth > 0)
                                .then(|| element.as_object())
                                .flatten()
                                .map(|addr| unsafe { zun_gc::Obj::from_addr(addr as *mut u64) })
                                .filter(|obj| heap.array_len(*obj).is_some());
                            match child {
                                Some(child) => flatten(heap, child, depth - 1, out),
                                None => {
                                    heap.array_push(out, element);
                                }
                            }
                        }
                    }
                    let out = heap.alloc_array(0);
                    flatten(heap, source, depth, out);
                    out
                });
                Ok(Value::Object(result))
            }
            "arrayFlatMap" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                let roots = [source, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let produced = self.invoke_callback(callback, callback_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    // Flatten one level: a returned array has its elements spread.
                    zun_gc::with_current_heap_mut(|heap| {
                        let child = boxed
                            .as_object()
                            .map(|addr| unsafe { zun_gc::Obj::from_addr(addr as *mut u64) })
                            .filter(|obj| heap.array_len(*obj).is_some());
                        match child {
                            Some(child) => {
                                let child_len = heap.array_len(child).unwrap_or(0);
                                for inner in 0..child_len {
                                    if let Some(entry) = heap.array_get(child, inner) {
                                        heap.array_push(roots[1], entry);
                                    }
                                }
                            }
                            None => {
                                heap.array_push(roots[1], boxed);
                            }
                        }
                    });
                }
                Ok(Value::Object(roots[1]))
            }
            "arrayGroup" | "arrayGroupToMap" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let to_map = name == "arrayGroupToMap";
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    if to_map { heap.alloc_map() } else { heap.alloc_dynamic() }
                });
                let roots = [source, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let produced = self.invoke_callback(callback, callback_args)?;
                    let key_text = if to_map { None } else { Some(unbox_arg(&produced)?.display()) };
                    let boxed = zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        let existing = match &key_text {
                            Some(key) => heap.dynamic_get(roots[1], key),
                            None => heap.map_get(roots[1], boxed),
                        };
                        let bucket = existing
                            .and_then(|any| any.as_object())
                            .map(|addr| unsafe { zun_gc::Obj::from_addr(addr as *mut u64) });
                        let bucket = match bucket {
                            Some(bucket) => bucket,
                            None => {
                                let bucket = heap.alloc_array(0);
                                let any = zun_core::Any::object(bucket.addr() as usize);
                                match &key_text {
                                    Some(key) => {
                                        heap.dynamic_set(roots[1], key.clone(), any);
                                    }
                                    None => {
                                        heap.map_insert(roots[1], boxed, any);
                                    }
                                }
                                bucket
                            }
                        };
                        heap.array_push(bucket, element);
                    });
                }
                if to_map {
                    Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
                } else {
                    Ok(Value::Object(roots[1]))
                }
            }
            "arrayKeys" | "arrayValues" | "arrayEntries" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let result = heap.alloc_array(length);
                    for index in 0..length {
                        let value = match name {
                            "arrayKeys" => zun_core::Any::float(index as f64),
                            "arrayValues" => {
                                heap.array_get(source, index).unwrap_or_else(zun_core::Any::void)
                            }
                            _ => {
                                let element = heap
                                    .array_get(source, index)
                                    .unwrap_or_else(zun_core::Any::void);
                                let pair = heap.alloc_array(2);
                                heap.array_set(pair, 0, zun_core::Any::float(index as f64));
                                heap.array_set(pair, 1, element);
                                zun_core::Any::object(pair.addr() as usize)
                            }
                        };
                        heap.array_set(result, index, value);
                    }
                    heap.alloc_array_iterator(result)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(result))
            }
            "arraySpliceInsert" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let start = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let delete_count = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let items =
                    object(args.get(3).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let removed = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let start = start.min(length);
                    let delete_count = delete_count.min(length - start);
                    let result = heap.alloc_array(delete_count);
                    for offset in 0..delete_count {
                        if let Some(element) = heap.array_remove(source, start) {
                            heap.array_set(result, offset, element);
                        }
                    }
                    let insert_count = heap.array_len(items).unwrap_or(0);
                    for offset in 0..insert_count {
                        if let Some(element) = heap.array_get(items, offset) {
                            heap.array_insert(source, start + offset, element);
                        }
                    }
                    result
                });
                Ok(Value::Object(removed))
            }
            "arraySlice" => {
                // `source.slice(start..)` into a fresh managed Array. Backs the
                // `...rest` element of array destructuring.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let start = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let start = start.min(length);
                    let result = heap.alloc_array(length - start);
                    for offset in 0..(length - start) {
                        let element = heap
                            .array_get(source, start + offset)
                            .unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, offset, element);
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "coalesceUndefined" => {
                // Destructuring default: `fallback` when `value` is undefined.
                let value = args.first().ok_or(RunError::BadToken)?.clone();
                let is_undefined = match &value {
                    Value::Void => true,
                    Value::Any(any) => any.tag() == zun_core::AnyTag::Void,
                    _ => false,
                };
                if is_undefined {
                    Ok(args.get(1).ok_or(RunError::BadToken)?.clone())
                } else {
                    Ok(value)
                }
            }
            "objectRest" => {
                // Copy every own-enumerable string key of `source` except the
                // names listed in `excludedKeys` into a fresh dynamic object.
                let source = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let excluded =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let excluded_count = heap.array_len(excluded).unwrap_or(0);
                    let mut excluded_names = Vec::with_capacity(excluded_count);
                    for index in 0..excluded_count {
                        if let Some(any) = heap.array_get(excluded, index) {
                            if let Some(handle) = any.as_string() {
                                if let Some(key) = heap.try_string(handle) {
                                    excluded_names.push(key.to_string());
                                }
                            }
                        }
                    }
                    let result = heap.alloc_dynamic();
                    for key in heap.dynamic_keys(source).unwrap_or_default() {
                        if !excluded_names.iter().any(|name| name == &key) {
                            if let Some(value) = heap.dynamic_get(source, &key) {
                                heap.dynamic_set(result, key, value);
                            }
                        }
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arrayReduceNoInit" | "arrayReduceRightNoInit" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let forward = name == "arrayReduceNoInit";
                if length == 0 {
                    return Err(RunError::Thrown(Value::Str(std::rc::Rc::from(
                        "Reduce of empty array with no initial value",
                    ))));
                }
                // Seed from the first (forward) or last (reverse) element.
                let seed_index = if forward { 0 } else { length - 1 };
                let accumulator = zun_gc::with_current_heap_mut(|heap| {
                    let holder = heap.alloc_array(1);
                    let seed = heap.array_get(source, seed_index).unwrap_or_else(zun_core::Any::void);
                    heap.array_set(holder, 0, seed);
                    holder
                });
                let roots = [source, accumulator];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for step in 1..length {
                    let index = if forward { step } else { length - 1 - step };
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let acc = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(acc),
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let next = self.invoke_callback(callback, callback_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| next.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(roots[1], 0, boxed));
                }
                let result = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
                    .ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap(|heap| Value::unbox_any(result, heap))
                    .ok_or(RunError::BadToken)
            }
            "arrayToSorted" | "arrayToSortedBy" => {
                // ES2023 `toSorted(cmp?)`: a sorted COPY; the receiver is
                // untouched. Clone into a fresh array, then run the same
                // insertion sort `sort` uses (comparator re-enters the interp).
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let comparator = if name == "arrayToSortedBy" {
                    match args.get(1).ok_or(RunError::BadToken)? {
                        Value::Function(handle) => Some(*handle),
                        _ => return Err(RunError::BadToken),
                    }
                } else {
                    None
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let copy = zun_gc::with_current_heap_mut(|heap| {
                    let copy = heap.alloc_array(length);
                    for index in 0..length {
                        let element =
                            heap.array_get(source, index).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(copy, index, element);
                    }
                    copy
                });
                let roots = [copy];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for i in 1..length {
                    let mut j = i;
                    while j > 0 {
                        let left = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], j - 1))
                            .ok_or(RunError::BadToken)?;
                        let right = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], j))
                            .ok_or(RunError::BadToken)?;
                        let ordered_before = match comparator {
                            Some(handle) => {
                                let outcome = self.invoke_callback(
                                    handle,
                                    vec![Value::Any(left), Value::Any(right)],
                                )?;
                                !(zun_gc::with_current_heap(|heap| to_number(&outcome, heap)) > 0.0)
                            }
                            None => {
                                let (a, b) = zun_gc::with_current_heap(|heap| {
                                    (
                                        stringify_any(left, heap).unwrap_or_default(),
                                        stringify_any(right, heap).unwrap_or_default(),
                                    )
                                });
                                a <= b
                            }
                        };
                        if ordered_before {
                            break;
                        }
                        zun_gc::with_current_heap_mut(|heap| {
                            let a = heap.array_get(roots[0], j - 1);
                            let b = heap.array_get(roots[0], j);
                            if let (Some(a), Some(b)) = (a, b) {
                                heap.array_set(roots[0], j - 1, b);
                                heap.array_set(roots[0], j, a);
                            }
                        });
                        j -= 1;
                    }
                }
                Ok(Value::Object(roots[0]))
            }
            "arrayToReversed" => {
                // `toReversed()`: a reversed COPY; the receiver is untouched.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let result = heap.alloc_array(length);
                    for index in 0..length {
                        let element = heap
                            .array_get(source, length - 1 - index)
                            .unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, index, element);
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arrayWith" => {
                // `with(index, value)`: a COPY with `index` replaced. A negative
                // index counts from the end; an out-of-range index throws a
                // RangeError-equivalent.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let raw = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    Value::Float(value, _) => *value as i64,
                    _ => return Err(RunError::BadToken),
                };
                let value = any(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let resolved = if raw < 0 { raw + length as i64 } else { raw };
                if resolved < 0 || resolved >= length as i64 {
                    return Err(RunError::Thrown(Value::Str(std::rc::Rc::from(
                        "Invalid index : out of range",
                    ))));
                }
                let target = resolved as usize;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let result = heap.alloc_array(length);
                    for index in 0..length {
                        let element = if index == target {
                            value
                        } else {
                            heap.array_get(source, index).unwrap_or_else(zun_core::Any::void)
                        };
                        heap.array_set(result, index, element);
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arrayToSpliced" => {
                // `toSpliced(start, deleteCount, ...items)`: a COPY with the
                // splice applied; the receiver is untouched.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let start = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let delete_count = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let items =
                    object(args.get(3).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let start = start.min(length);
                    let delete_count = delete_count.min(length - start);
                    let insert_count = heap.array_len(items).unwrap_or(0);
                    let result = heap.alloc_array(length - delete_count + insert_count);
                    let mut out = 0usize;
                    for index in 0..start {
                        let element =
                            heap.array_get(source, index).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, out, element);
                        out += 1;
                    }
                    for offset in 0..insert_count {
                        let element =
                            heap.array_get(items, offset).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, out, element);
                        out += 1;
                    }
                    for index in (start + delete_count)..length {
                        let element =
                            heap.array_get(source, index).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, out, element);
                        out += 1;
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arrayIsArray" => {
                let value = args.first().ok_or(RunError::BadToken)?;
                let is_array = object(value)
                    .map(|obj| zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()))
                    .unwrap_or(false);
                Ok(Value::Bool(is_array))
            }
            "arrayFrom" | "arrayFromMapped" => {
                // Build the base array from the source (a managed Array or a
                // string; other iterables are deferred).
                let concrete = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .unwrap_or(Value::Void),
                    other => other.clone(),
                };
                let base = match &concrete {
                    Value::Str(text) => {
                        let points: Vec<String> =
                            text.chars().map(|point| point.to_string()).collect();
                        zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(points.len());
                            for (index, point) in points.iter().enumerate() {
                                let value = zun_core::Any::string(
                                    heap.intern_string(point.as_str().into()),
                                );
                                heap.array_set(array, index, value);
                            }
                            array
                        })
                    }
                    _ => {
                        let Some(source) = object(&concrete) else {
                            return Err(RunError::Thrown(Value::Str(std::rc::Rc::from(
                                "Array.from: value is not array-like",
                            ))));
                        };
                        let Some(length) = zun_gc::with_current_heap(|heap| heap.array_len(source))
                        else {
                            return Err(RunError::Thrown(Value::Str(std::rc::Rc::from(
                                "Array.from: value is not array-like",
                            ))));
                        };
                        zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(length);
                            for index in 0..length {
                                let value = heap
                                    .array_get(source, index)
                                    .unwrap_or_else(zun_core::Any::void);
                                heap.array_set(array, index, value);
                            }
                            array
                        })
                    }
                };
                if name == "arrayFrom" {
                    return Ok(Value::Object(base));
                }
                // Mapped form: invoke the callback `(value, index)` per element.
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(base))
                    .ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(length));
                let roots = [base, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                    ];
                    let produced = self.invoke_callback(callback, callback_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(roots[1], index, boxed));
                }
                Ok(Value::Object(roots[1]))
            }
            "arraySpreadAppend" => {
                // Append every element of `source` (a managed Array or a
                // string, matching `arrayFrom`) to `target`, in order.
                let target =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let concrete = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .unwrap_or(Value::Void),
                    other => other.clone(),
                };
                match &concrete {
                    Value::Str(text) => {
                        // Spread a string by Unicode code point.
                        let points: Vec<String> =
                            text.chars().map(|point| point.to_string()).collect();
                        let roots = [target];
                        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                        for point in points {
                            zun_gc::with_current_heap_mut(|heap| {
                                let id = heap.intern_string(point.as_str().into());
                                heap.array_push(roots[0], zun_core::Any::string(id));
                            });
                        }
                    }
                    _ => {
                        let Some(source) = object(&concrete) else {
                            return Err(RunError::Thrown(Value::Str(std::rc::Rc::from(
                                "spread: value is not iterable",
                            ))));
                        };
                        // A managed Array keeps its direct fast path; anything
                        // else (a Set/Map or a user `[Symbol.iterator]` object)
                        // is drained through the iteration protocol.
                        if let Some(length) = zun_gc::with_current_heap(|heap| heap.array_len(source))
                        {
                            let roots = [source, target];
                            let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                            for index in 0..length {
                                let value =
                                    zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                                        .unwrap_or_else(zun_core::Any::void);
                                zun_gc::with_current_heap_mut(|heap| heap.array_push(roots[1], value));
                            }
                        } else {
                            self.drive_iterable(source, target, |heap, array, value| {
                                heap.array_push(array, value);
                            })?;
                        }
                    }
                }
                Ok(Value::Object(target))
            }
            "promiseNew" => {
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                Ok(Value::Object(promise))
            }
            "promiseWithResolvers" => {
                // `Promise.withResolvers()` → `{ promise, resolve, reject }`.
                // `promise` is an ordinary dynamic property; `resolve`/`reject`
                // are stored in the dynamic-FUNCTION slot (function values do
                // not box into `Any`), reached from source via a dynamic method
                // call (`r.resolve(v)`). Each is a native "settler" callable: a
                // bound-function handle over a record naming the promise and the
                // settle mode (see `settler_of` / the settler interception in
                // `invoke_function_value` and `CallIndirect`).
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                let result = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                let roots = [promise, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let resolve = self.make_promise_settler(roots[0], SETTLE_RESOLVE);
                let reject = self.make_promise_settler(roots[0], SETTLE_REJECT);
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set(
                        roots[1],
                        "promise",
                        zun_core::Any::object(roots[0].addr() as usize),
                    );
                    heap.dynamic_set_function(roots[1], "resolve", resolve);
                    heap.dynamic_set_function(roots[1], "reject", reject);
                });
                Ok(Value::Object(roots[1]))
            }
            "promiseNewExecutor" => {
                // `new Promise(executor)`: build a pending promise plus its
                // resolve/reject settlers (the exact `withResolvers` mechanism —
                // see `make_promise_settler`/`settler_of`), then synchronously
                // invoke the executor with them. An executor throw rejects the
                // promise with the thrown value and is swallowed (ECMAScript).
                let executor = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                let roots = [promise];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let resolve = self.make_promise_settler(roots[0], SETTLE_RESOLVE);
                let reject = self.make_promise_settler(roots[0], SETTLE_REJECT);
                match self.invoke_function_value(
                    executor,
                    vec![Value::Function(resolve), Value::Function(reject)],
                ) {
                    Ok(_) => {}
                    Err(RunError::Thrown(value)) => {
                        // Carry an object reason first-class; keep a string
                        // reason's message surface — the same split the reject
                        // settler uses (see `invoke_promise_settler`).
                        match &value {
                            Value::Str(message) => {
                                self.settle_promise_reject(roots[0], message.to_string());
                            }
                            other => {
                                let reason =
                                    zun_gc::with_current_heap_mut(|heap| other.clone().box_any(heap))
                                        .unwrap_or_else(zun_core::Any::null);
                                let message = value_to_error_string(other);
                                self.settle_promise_reject_value(roots[0], reason, message);
                            }
                        }
                    }
                    Err(other) => return Err(other),
                }
                Ok(Value::Object(roots[0]))
            }
            "promiseResolve" => {
                let promise = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let value = any(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                // Settle through the scheduler directly: the heap cell holds the
                // value, and bridged reactions/frames wake via the scheduler.
                Ok(Value::Bool(self.settle_promise_resolve(promise, value)))
            }
            "promiseReject" => {
                let promise = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let error = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(error) => error.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(self.settle_promise_reject(promise, error)))
            }
            "promiseState" => {
                let promise = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let state = zun_gc::with_current_heap(|heap| heap.promise_state(promise))
                    .ok_or(RunError::BadToken)?;
                let state = match state {
                    zun_core::ManagedPromiseState::Pending => 0,
                    zun_core::ManagedPromiseState::Resolved => 1,
                    zun_core::ManagedPromiseState::Rejected => 2,
                };
                Ok(Value::Int(state, zun_il::NumKind::U8))
            }
            "promiseResult" => {
                let promise = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.promise_result(promise))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Any(value))
            }
            "promiseIsReady" => {
                let promise = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.promise_is_ready(promise))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "promiseThen" => {
                let source = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let on_fulfilled = callback_arg(args.get(1));
                let on_rejected = callback_arg(args.get(2));
                self.promise_then(source, on_fulfilled, on_rejected)
            }
            "promiseCatch" => {
                let source = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let on_rejected = callback_arg(args.get(1));
                self.promise_then(source, None, on_rejected)
            }
            "promiseFinally" => {
                let source = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let on_finally = callback_arg(args.get(1));
                self.promise_finally(source, on_finally)
            }
            "promiseResolveStatic" => {
                let value = args.first().ok_or(RunError::BadToken)?;
                // A value that is already a Promise is returned unchanged.
                if let Some(existing) = object(value) {
                    if zun_gc::with_current_heap(|heap| heap.promise_state(existing)).is_some() {
                        return Ok(Value::Object(existing));
                    }
                }
                let boxed = any(value).ok_or(RunError::BadToken)?;
                let promise = zun_gc::with_current_heap_mut(|heap| {
                    let promise = heap.alloc_promise();
                    heap.promise_resolve(promise, boxed);
                    promise
                });
                Ok(Value::Object(promise))
            }
            "promiseRejectStatic" => {
                let value = args.first().ok_or(RunError::BadToken)?;
                let message = value_to_error_string(value);
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                // Settle through the scheduler so a detached rejection (nothing
                // awaiting or `.catch`-ing it) is tracked for turn-end
                // unhandled-rejection reporting.
                self.settle_promise_reject(promise, message);
                Ok(Value::Object(promise))
            }
            "promiseAll" | "promiseAllSettled" | "promiseRace" | "promiseAny" => {
                let array = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                self.promise_combinator(name, array)
            }
            "setTimeout" | "setInterval" | "setImmediate" => {
                // The callback is required and must be a function value; a
                // missing/non-function callback is a no-op that still returns a
                // (dead) handle, matching the lenient host surface.
                let callback = match args.first() {
                    Some(value @ Value::Function(_)) => value.clone(),
                    _ => return Ok(Value::Float(f64::NAN, zun_il::NumKind::F64)),
                };
                // `setImmediate` is a 0-delay macrotask; the others read the
                // (checker-defaulted) delay argument.
                let delay = if name == "setImmediate" {
                    0
                } else {
                    match args.get(1) {
                        Some(Value::Float(ms, _)) => *ms as i64,
                        Some(Value::Int(ms, _)) => *ms,
                        _ => 0,
                    }
                };
                let delay = delay.max(0);
                let period = if name == "setInterval" { Some(delay.max(0)) } else { None };
                let handle = self.timer_arm(callback, delay, period);
                Ok(Value::Float(handle as f64, zun_il::NumKind::F64))
            }
            "clearTimeout" | "clearInterval" => {
                let handle = match args.first() {
                    Some(Value::Float(handle, _)) => *handle as i64,
                    Some(Value::Int(handle, _)) => *handle,
                    // A nullish/absent handle clears nothing (JS is lenient).
                    _ => return Ok(Value::Void),
                };
                if handle >= 0 {
                    self.timer_clear(handle as u32);
                }
                Ok(Value::Void)
            }
            "queueMicrotask" => {
                let callback = match args.first() {
                    Some(Value::Function(handle)) => *handle,
                    _ => return Ok(Value::Void),
                };
                self.queue_microtask(callback)?;
                Ok(Value::Void)
            }
            "objectKeys" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = self.object_own_entries(obj)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(entries.len());
                    for (index, (key, _)) in entries.iter().enumerate() {
                        let item = Value::Str(key.clone().into())
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        let _ = heap.array_set(array, index, item);
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectValues" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = self.object_own_entries(obj)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(entries.len());
                    for (index, (_, value)) in entries.iter().enumerate() {
                        let item = value.box_any(heap).unwrap_or_else(zun_core::Any::null);
                        let _ = heap.array_set(array, index, item);
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectEntries" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = self.object_own_entries(obj)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(entries.len());
                    for (index, (key, value)) in entries.iter().enumerate() {
                        let key_any = Value::Str(key.clone().into())
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        let value_any = value.box_any(heap).unwrap_or_else(zun_core::Any::null);
                        let pair = heap.alloc_array(2);
                        let _ = heap.array_set(pair, 0, key_any);
                        let _ = heap.array_set(pair, 1, value_any);
                        let _ = heap.array_set(array, index, zun_core::Any::object(pair.addr() as usize));
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectAssign" => {
                let target = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let source = object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = self.object_own_entries(source)?;
                for (key, value) in entries {
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(target, key, boxed);
                    });
                }
                Ok(Value::Object(target))
            }
            "globalSet" => {
                // `global.set(name, value, readonly)` — register `name -> value`
                // on the realm global singleton, non-writable when `readonly`.
                let name = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let value = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let readonly = matches!(args.get(2), Some(Value::Bool(true)));
                zun_gc::with_current_heap_mut(|heap| {
                    let global = heap.global_object();
                    heap.dynamic_define(global, name, value, !readonly);
                });
                Ok(Value::Void)
            }
            "realmDescriptor" => {
                // Box a base-type metadata token into an opaque class-descriptor
                // `Any` (PLAN §3.1). The token arrives as a `number`.
                let token = match args.first().ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value as u32,
                    Value::Int(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Any(zun_core::Any::class_descriptor(token)))
            }
            "classDescriptorToken" => {
                // Read a class descriptor's token back, or `-1` for anything that
                // is not a descriptor.
                let token = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_class_descriptor().map(|t| t as f64).unwrap_or(-1.0),
                    _ => -1.0,
                };
                Ok(Value::Float(token, zun_il::NumKind::F64))
            }
            "symbolNew" => {
                // `Symbol(description?)` mints a fresh, unique symbol. `Void`
                // (undefined) or `Null` means no description.
                let description = match args.first() {
                    Some(Value::Str(text)) => Some(text.to_string()),
                    Some(Value::Any(any)) => zun_gc::with_current_heap(|heap| {
                        any.as_string().and_then(|handle| heap.try_string(handle).map(|s| s.to_string()))
                    }),
                    _ => None,
                };
                let id = zun_gc::with_current_heap_mut(|heap| heap.symbol_create(description));
                Ok(Value::Any(zun_core::Any::symbol(id)))
            }
            "symbolFor" => {
                // `Symbol.for(key)` — the same symbol for the same key.
                let key = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(text) => text.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let id = zun_gc::with_current_heap_mut(|heap| heap.symbol_for(key));
                Ok(Value::Any(zun_core::Any::symbol(id)))
            }
            "symbolKeyFor" => {
                // `Symbol.keyFor(symbol)` — the registry key, or `undefined`.
                let id = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_symbol().ok_or(RunError::BadToken)?,
                    _ => return Err(RunError::BadToken),
                };
                Ok(zun_gc::with_current_heap(|heap| {
                    heap.symbol_key_for(id).map(|key| Value::Str(key.into()))
                })
                .unwrap_or(Value::Void))
            }
            "symbolWellKnown" => {
                // Materialize a well-known symbol value by its stable index.
                let index = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u64,
                    Value::Float(value, _) => *value as u64,
                    _ => return Err(RunError::BadToken),
                };
                let id = zun_core::SymbolId::from_raw(index);
                Ok(Value::Any(zun_core::Any::symbol(id)))
            }
            "symbolDescription" => {
                // `symbol.description` — the creation description, or `undefined`.
                let id = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_symbol().ok_or(RunError::BadToken)?,
                    _ => return Err(RunError::BadToken),
                };
                Ok(zun_gc::with_current_heap(|heap| {
                    heap.symbol_description(id).map(|text| Value::Str(text.into()))
                })
                .unwrap_or(Value::Void))
            }
            "objectFreeze" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_freeze(obj));
                Ok(Value::Object(obj))
            }
            "objectIsFrozen" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // A dynamic object is frozen only when `Object.freeze` ran on it;
                // a fixed-layout typed object has an immutable shape, so it is
                // reported frozen.
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let frozen = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_frozen(obj))
                } else {
                    true
                };
                Ok(Value::Bool(frozen))
            }
            "objectCreate" => {
                let prototype = prototype_from_arg(args.first().ok_or(RunError::BadToken)?);
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_set_prototype(obj, prototype));
                Ok(Value::Object(obj))
            }
            "objectGetPrototypeOf" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let obj = match self.proxy_route(proxy, "getPrototypeOf") {
                    ProxyRoute::Trap(callback, target) => {
                        let result =
                            self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::null);
                        return Ok(Value::Any(boxed));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                let prototype = zun_gc::with_current_heap(|heap| heap.dynamic_prototype(obj));
                Ok(Value::Any(prototype.unwrap_or_else(zun_core::Any::null)))
            }
            "objectSetPrototypeOf" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let obj = match self.proxy_route(proxy, "setPrototypeOf") {
                    ProxyRoute::Trap(callback, target) => {
                        let prototype = args.get(1).ok_or(RunError::BadToken)?.clone();
                        self.invoke_proxy_trap(
                            callback,
                            vec![Value::Object(target), prototype],
                        )?;
                        return Ok(Value::Object(proxy));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                let prototype = prototype_from_arg(args.get(1).ok_or(RunError::BadToken)?);
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_set_prototype(obj, prototype));
                Ok(Value::Object(obj))
            }
            "objectFromEntries" => {
                let array = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let length = zun_gc::with_current_heap(|heap| heap.array_len(array))
                    .ok_or(RunError::BadToken)?;
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                for index in 0..length {
                    let pair_any = zun_gc::with_current_heap(|heap| heap.array_get(array, index))
                        .ok_or(RunError::BadToken)?;
                    let pair = pair_any.as_object().ok_or(RunError::BadToken)?;
                    let pair = unsafe { zun_gc::Obj::from_addr(pair as *mut u64) };
                    let (key_any, value_any) = zun_gc::with_current_heap(|heap| {
                        (heap.array_get(pair, 0), heap.array_get(pair, 1))
                    });
                    let key_any = key_any.ok_or(RunError::BadToken)?;
                    let value_any = value_any.ok_or(RunError::BadToken)?;
                    let key = zun_gc::with_current_heap(|heap| stringify_any(key_any, heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(obj, key, value_any);
                    });
                }
                Ok(Value::Object(obj))
            }
            "objectIs" => {
                let left = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let right = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)?;
                Ok(Value::Bool(same_value(&left, &right)))
            }
            "objectHasOwn" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let has = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_has(obj, &key))
                } else {
                    self.object_own_entries(obj)?.iter().any(|(name, _)| name == &key)
                };
                Ok(Value::Bool(has))
            }
            "objectIsPrototypeOf" => {
                let prototype = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target = object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let prototype_addr = prototype.addr() as usize;
                let mut current = zun_gc::with_current_heap(|heap| heap.dynamic_prototype(target));
                let mut found = false;
                while let Some(link) = current {
                    match link.as_object() {
                        Some(addr) if addr == prototype_addr => {
                            found = true;
                            break;
                        }
                        Some(addr) => {
                            let next = unsafe { zun_gc::Obj::from_addr(addr as *mut u64) };
                            current = zun_gc::with_current_heap(|heap| heap.dynamic_prototype(next));
                        }
                        None => break,
                    }
                }
                Ok(Value::Bool(found))
            }
            "objectDefineProperty" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let desc = object(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let obj = match self.proxy_route(proxy, "defineProperty") {
                    ProxyRoute::Trap(callback, target) => {
                        self.invoke_proxy_trap(
                            callback,
                            vec![
                                Value::Object(target),
                                Value::Str(key.clone().into()),
                                Value::Object(desc),
                            ],
                        )?;
                        return Ok(Value::Object(proxy));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                self.object_define_one(obj, &key, desc);
                Ok(Value::Object(obj))
            }
            "objectDefineProperties" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let descs = object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // Each own enumerable key of `descs` maps a property name to a
                // descriptor object.
                let names = self.object_own_names(descs)?;
                for name in names {
                    if !zun_gc::with_current_heap(|heap| heap.dynamic_is_enumerable(descs, &name)) {
                        continue;
                    }
                    let desc_any = zun_gc::with_current_heap(|heap| heap.dynamic_get(descs, &name))
                        .ok_or(RunError::BadToken)?;
                    let desc = desc_any.as_object().ok_or(RunError::BadToken)?;
                    let desc = unsafe { zun_gc::Obj::from_addr(desc as *mut u64) };
                    self.object_define_one(obj, &name, desc);
                }
                Ok(Value::Object(obj))
            }
            "objectGetOwnPropertyDescriptor" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let obj = match self.proxy_route(proxy, "getOwnPropertyDescriptor") {
                    ProxyRoute::Trap(callback, target) => {
                        let result = self.invoke_proxy_trap(
                            callback,
                            vec![Value::Object(target), Value::Str(key.clone().into())],
                        )?;
                        return Ok(result);
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                match self.build_descriptor(obj, &key)? {
                    Some(descriptor) => Ok(Value::Object(descriptor)),
                    None => Ok(Value::Void),
                }
            }
            "objectGetOwnPropertyDescriptors" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let names = self.object_own_names(obj)?;
                let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                for name in names {
                    if let Some(descriptor) = self.build_descriptor(obj, &name)? {
                        let boxed = zun_core::Any::object(descriptor.addr() as usize);
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set(out, name, boxed);
                        });
                    }
                }
                Ok(Value::Object(out))
            }
            "objectGetOwnPropertyNames" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // `Reflect.ownKeys(proxy)` consults the `ownKeys` trap; the trap
                // returns the key array directly.
                let obj = match self.proxy_route(proxy, "ownKeys") {
                    ProxyRoute::Trap(callback, target) => {
                        let result =
                            self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
                        return Ok(result);
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                let names = self.object_own_names(obj)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(names.len());
                    for (index, name) in names.iter().enumerate() {
                        let item = Value::Str(name.clone().into())
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        let _ = heap.array_set(array, index, item);
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectGetOwnPropertySymbols" => {
                // `Object.getOwnPropertySymbols(obj)` — an array of the object's
                // own symbol keys (as `Any` symbol values), in insertion-stable
                // id order. A non-dynamic/typed object has no symbol keys.
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let symbols = zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(obj));
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(symbols.len());
                    for (index, symbol) in symbols.iter().enumerate() {
                        let _ = heap.array_set(array, index, zun_core::Any::symbol(*symbol));
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectSeal" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_seal(obj));
                Ok(Value::Object(obj))
            }
            "objectIsSealed" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let sealed = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_sealed(obj))
                } else {
                    // A fixed-layout typed object has an immutable shape.
                    true
                };
                Ok(Value::Bool(sealed))
            }
            "objectPreventExtensions" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let obj = match self.proxy_route(proxy, "preventExtensions") {
                    ProxyRoute::Trap(callback, target) => {
                        self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
                        return Ok(Value::Object(proxy));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_prevent_extensions(obj));
                Ok(Value::Object(obj))
            }
            "objectIsExtensible" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let obj = match self.proxy_route(proxy, "isExtensible") {
                    ProxyRoute::Trap(callback, target) => {
                        let result =
                            self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
                        return Ok(Value::Bool(matches!(result, Value::Bool(true))));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let extensible = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_extensible(obj))
                } else {
                    // Typed objects cannot gain fields.
                    false
                };
                Ok(Value::Bool(extensible))
            }
            "objectPropertyIsEnumerable" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let enumerable = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_enumerable(obj, &key))
                } else {
                    // Typed declared fields are enumerable own properties.
                    self.object_own_entries(obj)?.iter().any(|(name, _)| name == &key)
                };
                Ok(Value::Bool(enumerable))
            }
            "captureStackTrace" => {
                // Runtime stack capture for `Error` construction (PLAN §3.1):
                // walk the live activation chain, innermost first, rendering
                // one frame line each. The managed `Error.stack` getter
                // prepends the `name: message` header, so this returns only the
                // frame lines.
                //
                // Fidelity: method names come from the module's `MethodDef`
                // table and the source location from the sequence-point debug
                // info, which records *byte spans* into the original source —
                // the interpreter has no source text, so a frame renders
                // `(<start>..<end>)` byte offsets rather than `file:line:col`.
                // The innermost frame is the one constructing the `Error`;
                // `captureStackTrace` itself is a `call.builtin` (no activation
                // of its own), so it does not appear. Only frames of the
                // current module are walked; a frame in another loaded module
                // runs on its own interpreter and is not visible here.
                Ok(Value::Str(self.render_stack_trace().into()))
            }
            "typeofAny" => {
                // Runtime `typeof` for an `any` operand. A managed reference is
                // refined with the heap: a BigInt payload is `"bigint"`, every
                // other object is `"object"`. `null` is `"object"` and
                // `undefined`/absent is `"undefined"`, as in JavaScript.
                let classify_object = |obj: zun_gc::Obj| {
                    if zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some()) {
                        "bigint"
                    } else {
                        "object"
                    }
                };
                let name = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(..) | Value::Float(..) => "number",
                    Value::Bool(_) => "boolean",
                    Value::Str(_) => "string",
                    Value::Function(_) => "function",
                    Value::Void => "undefined",
                    Value::Null => "object",
                    Value::Object(obj) => classify_object(*obj),
                    Value::Any(any) => match any.tag() {
                        zun_core::AnyTag::Int | zun_core::AnyTag::Float => "number",
                        zun_core::AnyTag::Bool => "boolean",
                        zun_core::AnyTag::String => "string",
                        zun_core::AnyTag::Symbol => "symbol",
                        // A realm class descriptor is a constructor value.
                        zun_core::AnyTag::ClassDescriptor => "function",
                        zun_core::AnyTag::Void => "undefined",
                        zun_core::AnyTag::Null => "object",
                        zun_core::AnyTag::Object => match any.as_object() {
                            Some(address) => classify_object(unsafe {
                                zun_gc::Obj::from_addr(address as *mut u64)
                            }),
                            None => "object",
                        },
                    },
                };
                Ok(Value::Str(name.into()))
            }
            "instanceofClass" => {
                // Runtime `instanceof` for an operand whose static type does not
                // fold the answer. The second argument is the right-hand class's
                // metadata TypeDef rid; walk the operand's runtime class
                // hierarchy against it. A non-object operand (number, string,
                // `null`, …) and a plain dynamic object are never an instance.
                let class_rid = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    Value::Float(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                // A typed class carrying a `[Symbol.hasInstance]` method decides
                // membership itself: call it with the operand (boxed to `any`)
                // and return `ToBoolean` of the result.
                if let Some(&handler) = self.class_symbol_methods.get(&(
                    class_rid,
                    zun_core::WellKnownSymbol::HasInstance.id().get(),
                )) {
                    let operand = args.first().ok_or(RunError::BadToken)?.clone();
                    let boxed = match operand {
                        Value::Any(_) => operand,
                        other => zun_gc::with_current_heap_mut(|heap| {
                            other.box_any(heap).map(Value::Any)
                        })
                        .ok_or(RunError::BadToken)?,
                    };
                    let result = self.invoke_function_value(handler, vec![boxed])?;
                    let truthy = zun_gc::with_current_heap(|heap| is_truthy(&result, heap));
                    return Ok(Value::Bool(truthy));
                }
                let found = match object(args.first().ok_or(RunError::BadToken)?) {
                    Some(obj) => self.types.is_assignable(obj.type_rid(), class_rid),
                    None => false,
                };
                Ok(Value::Bool(found))
            }
            // Binary ser/de PRIMITIVE (NUCLEUS #27). `serialize` inspects the
            // raw operand (so a function value is rejected cleanly, not boxed
            // away); `deserialize` reads a byte view and rebuilds the graph.
            "serialize" => self.serialize_value(args.first().ok_or(RunError::BadToken)?),
            "deserialize" => {
                let view = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                self.deserialize_value(view)
            }
            _ => Err(RunError::BadToken),
        }
    }

    /// Render the live activation chain as the frame lines of an `Error.stack`
    /// string: innermost frame first, one `    at <fn> (<span>)` per frame. See
    /// the `captureStackTrace` arm for the fidelity notes.
    pub(crate) fn render_stack_trace(&self) -> String {
        let mut lines = String::new();
        for (index, frame) in self.call_stack.iter().rev().enumerate() {
            if index > 0 {
                lines.push('\n');
            }
            let (name, span) = match self.module.debug_frame_at(frame.method_rid, frame.pc) {
                Some(debug) => (debug.method_name, debug.source_span),
                None => ("<anonymous>", None),
            };
            let name = if name.is_empty() { "<anonymous>" } else { name };
            match span {
                Some((start, end)) => {
                    lines.push_str(&format!("    at {name} ({start}..{end})"));
                }
                None => lines.push_str(&format!("    at {name}")),
            }
        }
        lines
    }

    /// Enumerate the own enumerable string keys and their values for either a
    /// dynamic object (insertion-ordered dynamic keys) or a fixed-layout typed
    /// object (declared instance fields, base classes first, static and private
    /// fields excluded).
    pub(crate) fn object_own_entries(
        &self,
        obj: zun_gc::Obj,
    ) -> Result<Vec<(String, Value)>, RunError> {
        if zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj)) {
            let entries = zun_gc::with_current_heap(|heap| heap.dynamic_entries(obj))
                .ok_or(RunError::BadToken)?;
            let mut out = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                let value = zun_gc::with_current_heap(|heap| Value::unbox_any(value, heap))
                    .ok_or(RunError::BadToken)?;
                out.push((key, value));
            }
            return Ok(out);
        }
        // Typed object: walk the declared-field layout from the base-most type
        // down to the concrete type so keys read in definition order.
        let type_rid = obj.type_rid();
        let mut chain = Vec::new();
        let mut current = type_rid;
        loop {
            chain.push(current);
            let ty = self
                .module
                .tables
                .type_defs
                .get(current as usize - 1)
                .ok_or(RunError::BadToken)?;
            if ty.base.is_null() || ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                break;
            }
            current = ty.base.rid();
        }
        chain.reverse();
        let mut out = Vec::new();
        for type_rid in chain {
            let (field_start, field_end) = {
                let ty = self
                    .module
                    .tables
                    .type_defs
                    .get(type_rid as usize - 1)
                    .ok_or(RunError::BadToken)?;
                (ty.field_start, ty.field_end)
            };
            for field_rid in field_start..field_end {
                let field = self
                    .module
                    .tables
                    .field_defs
                    .get(field_rid as usize - 1)
                    .ok_or(RunError::BadToken)?;
                if field.is_static || field.is_private {
                    continue;
                }
                let name = self.module.strings.get(field.name).to_string();
                let layout = self.types.get_field(field_rid).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| {
                    super::value::load_field(layout.kind, obj, layout.slot, heap)
                });
                out.push((name, value));
            }
        }
        Ok(out)
    }

    /// Read a boolean descriptor attribute (`writable`/`enumerable`/
    /// `configurable`) from a descriptor object, defaulting to `false` when the
    /// field is absent (the `Object.defineProperty` default).
    fn descriptor_flag(desc: zun_gc::Obj, name: &str) -> bool {
        zun_gc::with_current_heap(|heap| match heap.dynamic_get(desc, name) {
            Some(any) => is_truthy(&Value::Any(any), heap),
            None => false,
        })
    }

    /// Apply one `{ value, writable, enumerable, configurable }` or
    /// `{ get, set, enumerable, configurable }` descriptor object to `obj[key]`.
    /// A data descriptor reads `value` from the value slot and the boolean
    /// flags; an accessor descriptor reads `get`/`set` from the function slot
    /// (function values do not box into `Any`). Rejection (non-configurable
    /// redefine, or a new key on a non-extensible object) is silent and
    /// freeze-consistent. Returns whether the definition took effect.
    fn object_define_one(&self, obj: zun_gc::Obj, key: &str, desc: zun_gc::Obj) -> bool {
        let get_fn = zun_gc::with_current_heap(|heap| heap.dynamic_get_function(desc, "get"));
        let set_fn = zun_gc::with_current_heap(|heap| heap.dynamic_get_function(desc, "set"));
        let enumerable = Self::descriptor_flag(desc, "enumerable");
        let configurable = Self::descriptor_flag(desc, "configurable");
        if get_fn.is_some() || set_fn.is_some() {
            zun_gc::with_current_heap_mut(|heap| {
                heap.dynamic_define_accessor(obj, key.to_owned(), get_fn, set_fn, enumerable, configurable)
            })
        } else {
            let writable = Self::descriptor_flag(desc, "writable");
            let value = zun_gc::with_current_heap(|heap| heap.dynamic_get(desc, "value"))
                .unwrap_or_else(zun_core::Any::null);
            zun_gc::with_current_heap_mut(|heap| {
                heap.dynamic_define_data(obj, key.to_owned(), value, writable, enumerable, configurable)
            })
        }
    }

    /// Own string property names of `obj` (dynamic or typed), including
    /// non-enumerable keys (`getOwnPropertyNames`).
    fn object_own_names(&self, obj: zun_gc::Obj) -> Result<Vec<String>, RunError> {
        if zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj)) {
            zun_gc::with_current_heap(|heap| heap.dynamic_keys(obj)).ok_or(RunError::BadToken)
        } else {
            Ok(self.object_own_entries(obj)?.into_iter().map(|(name, _)| name).collect())
        }
    }

    /// Build a descriptor object `{ value, writable, enumerable, configurable }`
    /// or `{ get, set, enumerable, configurable }` for `obj[key]`, or `None` if
    /// there is no own property.
    fn build_descriptor(&self, obj: zun_gc::Obj, key: &str) -> Result<Option<zun_gc::Obj>, RunError> {
        let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
        if is_dynamic {
            let Some(descriptor) = zun_gc::with_current_heap(|heap| heap.dynamic_descriptor(obj, key)) else {
                return Ok(None);
            };
            let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
            let set_bool = |name: &str, flag: bool| {
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set(out, name.to_owned(), zun_core::Any::bool(flag));
                });
            };
            match descriptor {
                zun_core::PropertyDescriptor::Data { value, writable, enumerable, configurable } => {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(out, "value".to_owned(), value);
                    });
                    set_bool("writable", writable);
                    set_bool("enumerable", enumerable);
                    set_bool("configurable", configurable);
                }
                zun_core::PropertyDescriptor::Accessor { get, set, enumerable, configurable } => {
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(handle) = get {
                            heap.dynamic_set_function(out, "get", handle);
                        } else {
                            heap.dynamic_set(out, "get".to_owned(), zun_core::Any::null());
                        }
                        if let Some(handle) = set {
                            heap.dynamic_set_function(out, "set", handle);
                        } else {
                            heap.dynamic_set(out, "set".to_owned(), zun_core::Any::null());
                        }
                    });
                    set_bool("enumerable", enumerable);
                    set_bool("configurable", configurable);
                }
            }
            Ok(Some(out))
        } else {
            // Typed object: a declared field is a writable, enumerable data
            // property (the fixed layout is non-configurable).
            let entries = self.object_own_entries(obj)?;
            let Some((_, value)) = entries.into_iter().find(|(name, _)| name == key) else {
                return Ok(None);
            };
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
            zun_gc::with_current_heap_mut(|heap| {
                heap.dynamic_set(out, "value".to_owned(), boxed);
                heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(true));
                heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(true));
                heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(false));
            });
            Ok(Some(out))
        }
    }

    /// Classify how a reflective operation on `obj` resolves for a given trap
    /// name (see [`ProxyRoute`]). A proxy with the trap routes through it; a
    /// proxy without it falls back to its target; a plain object operates on
    /// itself.
    fn proxy_route(&self, obj: zun_gc::Obj, trap: &str) -> ProxyRoute {
        match zun_gc::with_current_heap(|heap| heap.proxy_target(obj)) {
            Some(target) => match zun_gc::with_current_heap(|heap| heap.proxy_trap(obj, trap)) {
                Some(callback) => ProxyRoute::Trap(callback, target),
                None => ProxyRoute::Target(target),
            },
            None => ProxyRoute::NotProxy,
        }
    }

    /// Invoke a proxy handler trap, threading its closure environment (if any)
    /// as the leading positional slot and appending `leading` (the target and
    /// the trap-specific operands). Mirrors the dispatch convention of the
    /// `get`/`set`/`has`/`deleteProperty` traps.
    fn invoke_proxy_trap(
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
    fn read_array_values(&self, array: zun_gc::Obj) -> Result<Vec<Value>, RunError> {
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
    pub(super) fn make_promise_settler(
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
    pub(super) fn settler_of(
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
    pub(super) fn invoke_promise_settler(
        &mut self,
        promise: zun_gc::Obj,
        mode: f64,
        mut args: Vec<Value>,
    ) -> Result<Value, RunError> {
        let value = if args.is_empty() { Value::Void } else { args.remove(0) };
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
                let args_any = heap.dynamic_get(record, "__args").ok_or(RunError::BadToken)?;
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

    /// The managed object an interpreter value refers to, unboxing an `Any`.
    fn value_object(value: &Value) -> Option<zun_gc::Obj> {
        match value {
            Value::Object(object) => Some(*object),
            Value::Any(any) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
            _ => None,
        }
    }

    /// ECMAScript `GetIterator(source)`: resolve the iterator object that drives
    /// a for-of / spread / collection-constructor over `source`.
    ///
    /// A built-in iterator object drives itself; a raw array/set/map is wrapped
    /// in the matching built-in iterator; a user value implementing
    /// `[Symbol.iterator]` (an object literal's symbol-function slot or a typed
    /// class's computed-symbol method) has it invoked to produce the iterator.
    fn resolve_iterator(&mut self, source: zun_gc::Obj) -> Result<zun_gc::Obj, RunError> {
        let iterator_id = zun_core::WellKnownSymbol::Iterator.id();
        let is_builtin_iter = zun_gc::with_current_heap(|heap| {
            heap.array_iterator_done(source).is_some()
                || heap.set_iterator_done(source).is_some()
                || heap.map_iterator_done(source).is_some()
        });
        if is_builtin_iter {
            return Ok(source);
        }
        if zun_gc::with_current_heap(|heap| heap.array_len(source)).is_some() {
            return zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(source))
                .ok_or(RunError::BadToken);
        }
        if zun_gc::with_current_heap(|heap| heap.set_values(source)).is_some() {
            return zun_gc::with_current_heap_mut(|heap| heap.alloc_set_iterator(source))
                .ok_or(RunError::BadToken);
        }
        if zun_gc::with_current_heap(|heap| heap.map_entries(source)).is_some() {
            return zun_gc::with_current_heap_mut(|heap| heap.alloc_map_iterator(source))
                .ok_or(RunError::BadToken);
        }
        let handle = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get_symbol_function(source, iterator_id)
        })
        .or_else(|| {
            self.class_symbol_methods
                .get(&(source.type_rid(), iterator_id.get()))
                .copied()
        });
        let Some(handle) = handle else {
            return Err(RunError::Thrown(Value::Str(std::rc::Rc::from(
                "value is not iterable",
            ))));
        };
        let result = self.invoke_function_value(handle, Vec::new())?;
        Self::value_object(&result).ok_or_else(|| {
            RunError::Thrown(Value::Str(std::rc::Rc::from(
                "[Symbol.iterator]() did not return an object",
            )))
        })
    }

    /// Advance a resolved iterator object once, returning `(value, done)`. A
    /// built-in iterator advances through its native cursor; a user iterator has
    /// its `next()` method called and the `{ value, done }` result read back.
    fn protocol_step(&mut self, it: zun_gc::Obj) -> Result<(Value, bool), RunError> {
        if let Some(done) = zun_gc::with_current_heap(|heap| heap.array_iterator_done(it)) {
            if done {
                return Ok((Value::Void, true));
            }
            let value = zun_gc::with_current_heap_mut(|heap| heap.array_iterator_next(it))
                .and_then(|any| zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)))
                .unwrap_or(Value::Void);
            return Ok((value, false));
        }
        if let Some(done) = zun_gc::with_current_heap(|heap| heap.set_iterator_done(it)) {
            if done {
                return Ok((Value::Void, true));
            }
            let value = zun_gc::with_current_heap_mut(|heap| heap.set_iterator_next(it))
                .and_then(|any| zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)))
                .unwrap_or(Value::Void);
            return Ok((value, false));
        }
        if let Some(done) = zun_gc::with_current_heap(|heap| heap.map_iterator_done(it)) {
            if done {
                return Ok((Value::Void, true));
            }
            let value = zun_gc::with_current_heap_mut(|heap| heap.map_iterator_next(it))
                .map(Value::Object)
                .unwrap_or(Value::Void);
            return Ok((value, false));
        }
        let next = zun_gc::with_current_heap(|heap| heap.dynamic_get_function(it, "next"));
        let Some(next) = next else {
            return Err(RunError::Thrown(Value::Str(std::rc::Rc::from(
                "iterator has no next() method",
            ))));
        };
        let roots = [it];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let result = self.invoke_function_value(next, Vec::new())?;
        let Some(result_obj) = Self::value_object(&result) else {
            return Err(RunError::Thrown(Value::Str(std::rc::Rc::from(
                "iterator.next() did not return an object",
            ))));
        };
        let done = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(result_obj, "done")
                .and_then(|any| Value::unbox_any(any, heap))
                .map(|value| is_truthy(&value, heap))
                .unwrap_or(false)
        });
        if done {
            return Ok((Value::Void, true));
        }
        let value = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(result_obj, "value")
                .and_then(|any| Value::unbox_any(any, heap))
        })
        .unwrap_or(Value::Void);
        Ok((value, false))
    }

    /// Build the look-ahead wrapper that backs the generic for-of protocol path
    /// (`generatorIterator*` over a non-generator). It carries the resolved
    /// iterator (`__zun_it`) plus a one-slot cache (`__zun_cur` / `__zun_done` /
    /// `__zun_has`) so the desugared `done`/`next` pair reads one lookahead.
    pub(crate) fn protocol_iterator_new(
        &mut self,
        source: zun_gc::Obj,
    ) -> Result<Value, RunError> {
        let it = self.resolve_iterator(source)?;
        let roots = [it];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let wrapper = zun_gc::with_current_heap_mut(|heap| {
            let wrapper = heap.alloc_dynamic();
            heap.dynamic_set(
                wrapper,
                "__zun_it",
                zun_core::Any::object(roots[0].addr() as usize),
            );
            wrapper
        });
        Ok(Value::Object(wrapper))
    }

    /// The `done` side of the protocol for-of path: prefetch the next value into
    /// the wrapper's cache (unless one is already pending or the iterator is
    /// exhausted) and report whether iteration is complete.
    fn protocol_done(&mut self, wrapper: zun_gc::Obj) -> Result<Value, RunError> {
        let roots = [wrapper];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let has = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(roots[0], "__zun_has").and_then(|any| any.as_bool())
        })
        .unwrap_or(false);
        let done_cached = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(roots[0], "__zun_done").and_then(|any| any.as_bool())
        })
        .unwrap_or(false);
        if has || done_cached {
            return Ok(Value::Bool(done_cached));
        }
        let it = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(roots[0], "__zun_it").and_then(|any| any.as_object())
        })
        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
        .ok_or(RunError::BadToken)?;
        let (value, done) = self.protocol_step(it)?;
        let boxed = if done {
            zun_core::Any::void()
        } else {
            zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::null)
        };
        zun_gc::with_current_heap_mut(|heap| {
            heap.dynamic_set(roots[0], "__zun_cur", boxed);
            heap.dynamic_set(roots[0], "__zun_done", zun_core::Any::bool(done));
            heap.dynamic_set(roots[0], "__zun_has", zun_core::Any::bool(true));
        });
        Ok(Value::Bool(done))
    }

    /// The `next` side of the protocol for-of path: hand back the value the
    /// matching `protocol_done` prefetched and clear the cache slot.
    fn protocol_next(&mut self, wrapper: zun_gc::Obj) -> Result<Value, RunError> {
        let current = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(wrapper, "__zun_cur")
                .and_then(|any| Value::unbox_any(any, heap))
        })
        .unwrap_or(Value::Void);
        zun_gc::with_current_heap_mut(|heap| {
            heap.dynamic_set(wrapper, "__zun_has", zun_core::Any::bool(false));
        });
        Ok(current)
    }

    /// `iterator.next()` for a built-in iterator object reached through a dynamic
    /// member call (`[10, 20].values().next()`): advance it and return a fresh
    /// `{ value, done }` result object. Returns `None` for a non-iterator so the
    /// dynamic-call dispatch carries on.
    fn builtin_iterator_result(
        &mut self,
        it: zun_gc::Obj,
    ) -> Result<Option<Value>, RunError> {
        let is_builtin = zun_gc::with_current_heap(|heap| {
            heap.array_iterator_done(it).is_some()
                || heap.set_iterator_done(it).is_some()
                || heap.map_iterator_done(it).is_some()
        });
        if !is_builtin {
            return Ok(None);
        }
        let (value, done) = self.protocol_step(it)?;
        let boxed = if done {
            zun_core::Any::void()
        } else {
            zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::null)
        };
        let value_root: Vec<zun_gc::Obj> = boxed
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
            .into_iter()
            .collect();
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&value_root) };
        let result = zun_gc::with_current_heap_mut(|heap| {
            let result = heap.alloc_dynamic();
            let value_any = match value_root.first() {
                Some(object) => zun_core::Any::object(object.addr() as usize),
                None => boxed,
            };
            heap.dynamic_set(result, "value", value_any);
            heap.dynamic_set(result, "done", zun_core::Any::bool(done));
            result
        });
        Ok(Some(Value::Any(zun_core::Any::object(result.addr() as usize))))
    }

    /// Drive `source` fully through the iteration protocol, handing each boxed
    /// value to `sink` to deposit into `target`. `target` (and the look-ahead
    /// wrapper) stay rooted across the drive, and the possibly-relocated target
    /// is returned. Backs the `Set`/`Map` iterable constructors and array spread
    /// for arbitrary user iterables.
    pub(crate) fn drive_iterable<F>(
        &mut self,
        source: zun_gc::Obj,
        target: zun_gc::Obj,
        mut sink: F,
    ) -> Result<zun_gc::Obj, RunError>
    where
        F: FnMut(&mut zun_gc::Heap, zun_gc::Obj, zun_core::Any),
    {
        let setup = [target];
        let wrapper = {
            let _scope = unsafe { zun_gc::NativeRootScope::enter(&setup) };
            Self::value_object(&self.protocol_iterator_new(source)?)
                .ok_or(RunError::BadToken)?
        };
        let roots = [wrapper, setup[0]];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        loop {
            if matches!(self.protocol_done(roots[0])?, Value::Bool(true)) {
                break;
            }
            let value = self.protocol_next(roots[0])?;
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            zun_gc::with_current_heap_mut(|heap| sink(heap, roots[1], boxed));
        }
        Ok(roots[1])
    }

    /// Invoke a function value with the given positional arguments. The engine
    /// behind `apply`; exceptions propagate as `RunError::Thrown` (matching the
    /// proxy-trap invocation path).
    pub(crate) fn invoke_function_value(
        &mut self,
        handle: zun_core::FunctionHandle,
        call_args: Vec<Value>,
    ) -> Result<Value, RunError> {
        if let Some((promise, mode)) = self.settler_of(handle) {
            return self.invoke_promise_settler(promise, mode, call_args);
        }
        let (method_rid, final_args) = self.resolve_call_target(handle, call_args)?;
        self.call(method_rid, final_args, None)
    }

    /// Invoke a dynamic callback with ECMAScript "extra arguments ignored"
    /// semantics: after the `bind`/closure environment is threaded, the
    /// positional argument vector is truncated to the callback's declared
    /// parameter count before dispatch. A callback declaring fewer parameters
    /// than the host supplies (`(value, index, array)`) therefore never has its
    /// own local slots clobbered, and one declaring more sees `undefined` in the
    /// surplus slots (padded by [`Interp::call`]).
    /// The kind-agnostic second-generation typed-array surface: bulk `set`,
    /// `slice`/`copyWithin`, the functional methods, and the `from`/`of`
    /// statics. Returns `None` when `name` is not one of these, so the caller
    /// carries on with the rest of its dispatch.
    ///
    /// The element kind is read from the view at run time (for instance
    /// methods) or decoded from a table-order discriminant argument (for the
    /// statics), so there is one handler per method rather than one per kind.
    /// Callback methods re-enter the interpreter and root their view/result
    /// across the call, mirroring the `Array` functional methods.
    fn call_typed_array_ext(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        let obj = |value: &Value| match value {
            Value::Object(object) => Some(*object),
            Value::Any(any) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
            _ => None,
        };
        // Every value a source (array or typed array) holds, as interpreter
        // values — the input shape for `set`/`from`/`of`.
        let source_elements = |src: zun_gc::Obj| -> Option<Vec<Value>> {
            if let Some(len) = crate::typed_arrays::element_len(src) {
                let mut out = Vec::with_capacity(len);
                for index in 0..len {
                    out.push(crate::typed_arrays::read_element(src, index)?);
                }
                return Some(out);
            }
            let len = zun_gc::with_current_heap(|heap| heap.array_len(src))?;
            let mut out = Vec::with_capacity(len);
            for index in 0..len {
                let any = zun_gc::with_current_heap(|heap| heap.array_get(src, index))?;
                let value = zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))?;
                out.push(value);
            }
            Some(out)
        };
        // A relative index (JS `slice`/`copyWithin` style): negative counts
        // from the end; the result is clamped into `0..=length`.
        let relative = |value: i64, length: usize| -> usize {
            if value < 0 {
                (length as i64 + value).max(0) as usize
            } else {
                (value as usize).min(length)
            }
        };
        match name {
            "typedArraySetFrom" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let src = obj(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let elements = source_elements(src).ok_or(RunError::BadToken)?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                // Out of range reports `false` rather than throwing a
                // RangeError (the whole set surface here yields a bool).
                if offset.checked_add(elements.len()).is_none_or(|end| end > length) {
                    return Ok(Value::Bool(false));
                }
                for (index, value) in elements.iter().enumerate() {
                    if crate::typed_arrays::write_element(view, offset + index, value)
                        != Some(true)
                    {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            })()),
            "typedArraySlice" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let (start, end) = (int_arg(args.get(1))?, int_arg(args.get(2))?);
                let element = crate::typed_arrays::element_of(view).ok_or(RunError::BadToken)?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let begin = relative(start, length);
                let finish = relative(end, length).max(begin);
                // Read the source range before allocating the result so the
                // view is never read across the allocation that could move it.
                let mut values = Vec::with_capacity(finish - begin);
                for index in begin..finish {
                    values.push(crate::typed_arrays::read_element(view, index).ok_or(RunError::BadToken)?);
                }
                let result = crate::typed_arrays::alloc_of(element, values.len())
                    .ok_or(RunError::BadToken)?;
                for (index, value) in values.iter().enumerate() {
                    crate::typed_arrays::write_element(result, index, value);
                }
                Ok(Value::Object(result))
            })()),
            "typedArrayCopyWithin" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target = int_arg(args.get(1))?;
                let start = int_arg(args.get(2))?;
                let end = int_arg(args.get(3))?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let dest = relative(target, length);
                let from = relative(start, length);
                let to = relative(end, length).max(from);
                // The copied count is bounded by what fits at the destination.
                let count = (to - from).min(length - dest);
                // Snapshot the source range first: source and destination may
                // overlap, so a straight element-by-element copy would clobber.
                let mut values = Vec::with_capacity(count);
                for index in 0..count {
                    values.push(crate::typed_arrays::read_element(view, from + index).ok_or(RunError::BadToken)?);
                }
                for (index, value) in values.iter().enumerate() {
                    crate::typed_arrays::write_element(view, dest + index, value);
                }
                Ok(Value::Object(view))
            })()),
            "typedArrayIndexOf" | "typedArrayIncludes" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // The search value arrives boxed to `Any`; unbox it to the
                // concrete numeric value the elements compare against.
                let needle = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .ok_or(RunError::BadToken)?,
                    other => other.clone(),
                };
                let from = int_arg(args.get(2))?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let start = relative(from, length);
                let includes = name == "typedArrayIncludes";
                for index in start..length {
                    let element = crate::typed_arrays::read_element(view, index).ok_or(RunError::BadToken)?;
                    if numeric_same(&element, &needle, includes) {
                        return Ok(if includes {
                            Value::Bool(true)
                        } else {
                            Value::Int(index as i64, zun_il::NumKind::I64)
                        });
                    }
                }
                Ok(if includes {
                    Value::Bool(false)
                } else {
                    Value::Int(-1, zun_il::NumKind::I64)
                })
            })()),
            "typedArrayJoin" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let separator = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(text) => text.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let mut parts = Vec::with_capacity(length);
                for index in 0..length {
                    let element = crate::typed_arrays::read_element(view, index).ok_or(RunError::BadToken)?;
                    let text = zun_gc::with_current_heap_mut(|heap| {
                        element.box_any(heap).and_then(|any| stringify_any(any, heap))
                    })
                    .unwrap_or_default();
                    parts.push(text);
                }
                Ok(Value::Str(parts.join(&separator).into()))
            })()),
            "typedArrayForEach" | "typedArrayMap" | "typedArrayFilter" | "typedArrayFind"
            | "typedArrayFindIndex" | "typedArraySome" | "typedArrayEvery" => {
                Some(self.typed_array_iterating(name, args))
            }
            "typedArrayReduce" => Some(self.typed_array_reduce(args, true, true)),
            "typedArrayReduceNoInit" => Some(self.typed_array_reduce(args, true, false)),
            "typedArrayReduceRight" => Some(self.typed_array_reduce(args, false, true)),
            "typedArrayReduceRightNoInit" => Some(self.typed_array_reduce(args, false, false)),
            "typedArrayFrom" | "typedArrayOf" => Some((|| {
                let src = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let kind = kind_arg(args.get(1))?;
                let elements = source_elements(src).ok_or(RunError::BadToken)?;
                let result = crate::typed_arrays::alloc_of(kind, elements.len())
                    .ok_or(RunError::BadToken)?;
                for (index, value) in elements.iter().enumerate() {
                    crate::typed_arrays::write_element(result, index, value);
                }
                Ok(Value::Object(result))
            })()),
            "typedArrayFromMapped" => Some(self.typed_array_from_mapped(args)),
            "typedArrayTrackingNew" => Some((|| {
                let buffer = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let kind = kind_arg(args.get(2))?;
                let view = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_typed_array_tracking(buffer, offset, kind)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(view))
            })()),
            _ => None,
        }
    }

    /// The typed-array functional methods whose shape matches the `Array`
    /// ones: `forEach`/`map`/`filter`/`find`/`findIndex`/`some`/`every`. The
    /// element kind is the view's; `map`/`filter` build a new view of that
    /// kind.
    fn typed_array_iterating(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        let view = match args.first() {
            Some(Value::Object(object)) => *object,
            Some(Value::Any(any)) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                .ok_or(RunError::BadToken)?,
            _ => return Err(RunError::BadToken),
        };
        let callback = match args.get(1).ok_or(RunError::BadToken)? {
            Value::Function(handle) => *handle,
            _ => return Err(RunError::BadToken),
        };
        let element = crate::typed_arrays::element_of(view).ok_or(RunError::BadToken)?;
        let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
        // `map` writes into a fresh same-kind view of equal length; `filter`
        // collects kept elements (plain numeric values, GC-safe) and sizes its
        // result after the loop. Both root the source view across callbacks.
        let result = if name == "typedArrayMap" {
            crate::typed_arrays::alloc_of(element, length).ok_or(RunError::BadToken)?
        } else {
            view
        };
        let roots = [view, result];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let mut kept: Vec<Value> = Vec::new();
        for index in 0..length {
            let value = crate::typed_arrays::read_element(roots[0], index).ok_or(RunError::BadToken)?;
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let callback_args = vec![
                Value::Any(boxed),
                Value::Any(zun_core::Any::float(index as f64)),
                Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
            ];
            let produced = self.invoke_callback(callback, callback_args)?;
            match name {
                "typedArrayForEach" => {}
                "typedArrayMap" => {
                    crate::typed_arrays::write_element(roots[1], index, &produced);
                }
                "typedArrayFilter" => {
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        kept.push(value);
                    }
                }
                "typedArrayFind" => {
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        return Ok(value);
                    }
                }
                "typedArrayFindIndex" => {
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        return Ok(Value::Int(index as i64, zun_il::NumKind::I64));
                    }
                }
                "typedArraySome" => {
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        return Ok(Value::Bool(true));
                    }
                }
                "typedArrayEvery" => {
                    if !zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        return Ok(Value::Bool(false));
                    }
                }
                _ => unreachable!(),
            }
        }
        match name {
            "typedArrayForEach" => Ok(Value::Void),
            "typedArrayMap" => Ok(Value::Object(roots[1])),
            "typedArrayFilter" => {
                let result = crate::typed_arrays::alloc_of(element, kept.len())
                    .ok_or(RunError::BadToken)?;
                for (index, value) in kept.iter().enumerate() {
                    crate::typed_arrays::write_element(result, index, value);
                }
                Ok(Value::Object(result))
            }
            "typedArrayFind" => Ok(Value::Void),
            "typedArrayFindIndex" => Ok(Value::Int(-1, zun_il::NumKind::I64)),
            "typedArraySome" => Ok(Value::Bool(false)),
            "typedArrayEvery" => Ok(Value::Bool(true)),
            _ => unreachable!(),
        }
    }

    /// `typedArray.reduce(callback, initialValue)`. The initial value is
    /// required here; the no-initial form (which seeds from the first element)
    /// is deferred. The accumulator is held in a one-slot GC array so it
    /// survives callback re-entry whether or not it is an object.
    /// `reduce`/`reduceRight` over a typed array. `forward` folds left-to-right
    /// (`reduce`) or right-to-left (`reduceRight`); when `has_init` is false the
    /// accumulator seeds from the first/last element and an empty typed array
    /// throws a TypeError-equivalent (NUCLEUS #19).
    fn typed_array_reduce(
        &mut self,
        args: &[Value],
        forward: bool,
        has_init: bool,
    ) -> Result<Value, RunError> {
        let view = match args.first() {
            Some(Value::Object(object)) => *object,
            Some(Value::Any(any)) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                .ok_or(RunError::BadToken)?,
            _ => return Err(RunError::BadToken),
        };
        let callback = match args.get(1).ok_or(RunError::BadToken)? {
            Value::Function(handle) => *handle,
            _ => return Err(RunError::BadToken),
        };
        let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
        // Index of the step `step` visits (0..length iterations), walking from
        // the appropriate end.
        let at = |step: usize| if forward { step } else { length - 1 - step };
        // Seed the accumulator: the initial value, or the first/last element.
        let (seed, start_step) = if has_init {
            (args.get(2).ok_or(RunError::BadToken)?.clone(), 0usize)
        } else {
            if length == 0 {
                return Err(RunError::Thrown(Value::Str(std::rc::Rc::from(
                    "Reduce of empty array with no initial value",
                ))));
            }
            let seed = crate::typed_arrays::read_element(view, at(0)).ok_or(RunError::BadToken)?;
            (seed, 1usize)
        };
        let accumulator = zun_gc::with_current_heap_mut(|heap| {
            let holder = heap.alloc_array(1);
            if let Some(boxed) = seed.box_any(heap) {
                heap.array_set(holder, 0, boxed);
            }
            holder
        });
        let roots = [view, accumulator];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        for step in start_step..length {
            let index = at(step);
            let element = crate::typed_arrays::read_element(roots[0], index).ok_or(RunError::BadToken)?;
            let boxed_element = zun_gc::with_current_heap_mut(|heap| element.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let acc = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
                .ok_or(RunError::BadToken)?;
            let callback_args = vec![
                Value::Any(acc),
                Value::Any(boxed_element),
                Value::Any(zun_core::Any::float(index as f64)),
                Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
            ];
            let next = self.invoke_callback(callback, callback_args)?;
            let boxed = zun_gc::with_current_heap_mut(|heap| next.box_any(heap))
                .ok_or(RunError::BadToken)?;
            zun_gc::with_current_heap_mut(|heap| heap.array_set(roots[1], 0, boxed));
        }
        let result = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
            .ok_or(RunError::BadToken)?;
        zun_gc::with_current_heap(|heap| Value::unbox_any(result, heap)).ok_or(RunError::BadToken)
    }

    /// `TypedArray.from(source, mapFn)`: the mapped static. Reads the source
    /// into values, runs each through the callback, and stores the result into
    /// a fresh view of the discriminant-named kind.
    fn typed_array_from_mapped(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let src = match args.first() {
            Some(Value::Object(object)) => *object,
            Some(Value::Any(any)) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                .ok_or(RunError::BadToken)?,
            _ => return Err(RunError::BadToken),
        };
        let callback = match args.get(1).ok_or(RunError::BadToken)? {
            Value::Function(handle) => *handle,
            _ => return Err(RunError::BadToken),
        };
        let kind = kind_arg(args.get(2))?;
        // Read the source eagerly into plain values (GC-safe across callbacks),
        // then build and fill the result.
        let elements = {
            let len = if let Some(len) = crate::typed_arrays::element_len(src) {
                len
            } else {
                zun_gc::with_current_heap(|heap| heap.array_len(src)).ok_or(RunError::BadToken)?
            };
            let mut out = Vec::with_capacity(len);
            for index in 0..len {
                let value = if crate::typed_arrays::element_len(src).is_some() {
                    crate::typed_arrays::read_element(src, index).ok_or(RunError::BadToken)?
                } else {
                    let any = zun_gc::with_current_heap(|heap| heap.array_get(src, index))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                        .ok_or(RunError::BadToken)?
                };
                out.push(value);
            }
            out
        };
        let result = crate::typed_arrays::alloc_of(kind, elements.len()).ok_or(RunError::BadToken)?;
        let roots = [result];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        for (index, value) in elements.iter().enumerate() {
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let callback_args = vec![
                Value::Any(boxed),
                Value::Any(zun_core::Any::float(index as f64)),
            ];
            let produced = self.invoke_callback(callback, callback_args)?;
            crate::typed_arrays::write_element(roots[0], index, &produced);
        }
        Ok(Value::Object(roots[0]))
    }

    pub(crate) fn invoke_callback(
        &mut self,
        handle: zun_core::FunctionHandle,
        call_args: Vec<Value>,
    ) -> Result<Value, RunError> {
        let (method_rid, mut final_args) = self.resolve_call_target(handle, call_args)?;
        let target = self
            .module
            .tables
            .method_defs
            .get(method_rid as usize - 1)
            .ok_or(RunError::BadToken)?;
        let body = read_body(&self.module.code, target.body_offset)
            .map_err(|error| RunError::Malformed(error.to_string()))?;
        final_args.truncate(body.param_count as usize);
        self.call(method_rid, final_args, None)
    }

    // --- Timers: the macrotask source of the unified event loop (slice 4) ---

    /// The current virtual time (ms), seeded lazily from the real `nowMillis`
    /// clock on first use. The real clock is only the *base*; the loop advances
    /// this virtual clock by fast-forward between macrotasks rather than
    /// sleeping (see [`Self::fire_one_timer`]).
    fn timer_now(&mut self) -> i64 {
        if !self.async_state.timers.seeded {
            let real = real_now_millis(self.output);
            self.async_state.timers.clock = real;
            self.async_state.timers.seeded = true;
        }
        self.async_state.timers.clock
    }

    /// Arm a timer: allocate a scheduler [`TaskId`] (which doubles as the
    /// numeric handle), compute its due time from the current virtual clock,
    /// and push it onto the timer heap. `period` is `Some` for an interval.
    pub(super) fn timer_arm(&mut self, callback: Value, delay: i64, period: Option<i64>) -> u32 {
        let now = self.timer_now();
        let task = TaskId(self.async_state.next_task);
        self.async_state.next_task += 1;
        let seq = self.async_state.timers.next_seq;
        self.async_state.timers.next_seq += 1;
        self.async_state.timers.entries.push(TimerEntry {
            task,
            due: now.saturating_add(delay.max(0)),
            seq,
            period,
            callback,
            args: Vec::new(),
        });
        task.0
    }

    /// Clear a timer by handle: cancel it on the scheduler's task queue (which
    /// also stops an interval re-arming) and drop any still-pending entry.
    pub(super) fn timer_clear(&mut self, handle: u32) {
        let task = TaskId(handle);
        self.async_state.scheduler.cancel_task(task);
        self.async_state.timers.entries.retain(|timer| timer.task != task);
    }

    /// `queueMicrotask(cb)`: enqueue a callback directly on the reaction
    /// (microtask) queue, so it runs before any macrotask on the next drain.
    /// Modelled as a `then` on an already-resolved promise — the same
    /// enqueue-not-run-inline discipline every reaction follows.
    pub(super) fn queue_microtask(
        &mut self,
        callback: zun_core::FunctionHandle,
    ) -> Result<(), RunError> {
        let promise = zun_gc::with_current_heap_mut(|heap| {
            let promise = heap.alloc_promise();
            heap.promise_resolve(promise, zun_core::Any::null());
            promise
        });
        self.promise_then(promise, Some(callback), None)?;
        Ok(())
    }

    /// Index of the earliest-due pending timer (min by `(due, seq)`), or `None`
    /// when the heap is empty. Cleared timers are already purged from the heap.
    fn earliest_timer_index(&self) -> Option<usize> {
        let entries = &self.async_state.timers.entries;
        if entries.is_empty() {
            return None;
        }
        let mut best = 0;
        for index in 1..entries.len() {
            let candidate = &entries[index];
            let current = &entries[best];
            if (candidate.due, candidate.seq) < (current.due, current.seq) {
                best = index;
            }
        }
        Some(best)
    }

    /// Fire the single earliest-due ready timer as one macrotask, returning
    /// whether a timer was fired (`false` when none remain). The earliest timer
    /// is always fired regardless of real wall-clock time: if it is not yet due
    /// the virtual clock fast-forwards to its due time (no real sleeping), which
    /// is what makes a timed program terminate deterministically. A due timer is
    /// scheduled on the scheduler's task tier and consumed through it, so the
    /// scheduler stays the authority on micro-before-macro ordering. An interval
    /// re-arms itself (at `now + period`) unless it was cleared meanwhile.
    pub(super) fn fire_one_timer(&mut self) -> Result<bool, RunError> {
        let Some(index) = self.earliest_timer_index() else {
            return Ok(false);
        };
        let entry = self.async_state.timers.entries.remove(index);
        if entry.due > self.async_state.timers.clock {
            self.async_state.timers.clock = entry.due;
        }
        self.async_state.scheduler.schedule_task(entry.task);
        let ran = matches!(
            self.async_state.scheduler.next(),
            Some(zun_sched::Ran::Task(task)) if task == entry.task
        );
        if ran {
            if let Value::Function(handle) = &entry.callback {
                // A throw from a timer callback is swallowed (detached-task
                // semantics, as in the microtask drain); a non-throw fault
                // still aborts the turn.
                match self.invoke_callback(*handle, entry.args.clone()) {
                    Ok(_) | Err(RunError::Thrown(_)) => {}
                    Err(other) => return Err(other),
                }
            }
            if let Some(period) = entry.period {
                if !self.async_state.scheduler.is_task_cancelled(entry.task) {
                    let now = self.async_state.timers.clock;
                    let seq = self.async_state.timers.next_seq;
                    self.async_state.timers.next_seq += 1;
                    self.async_state.timers.entries.push(TimerEntry {
                        task: entry.task,
                        due: now.saturating_add(period.max(0)),
                        seq,
                        period: Some(period),
                        callback: entry.callback.clone(),
                        args: entry.args,
                    });
                }
            }
        }
        Ok(true)
    }

    /// Read the settlement of a managed Promise. Errors if the object is not a
    /// Promise cell.
    fn promise_settlement_of(&self, promise: zun_gc::Obj) -> Result<PromiseSettlement, RunError> {
        zun_gc::with_current_heap(|heap| match heap.promise_state(promise) {
            Some(zun_core::ManagedPromiseState::Resolved) => Ok(PromiseSettlement::Resolved(
                heap.promise_result(promise).unwrap_or_else(zun_core::Any::null),
            )),
            Some(zun_core::ManagedPromiseState::Rejected) => Ok(PromiseSettlement::Rejected(
                heap.promise_error(promise).unwrap_or_else(|| "Promise rejected".into()),
                heap.promise_reason(promise),
            )),
            Some(zun_core::ManagedPromiseState::Pending) => Ok(PromiseSettlement::Pending),
            None => Err(RunError::BadToken),
        })
    }

    /// Resolve a managed Promise through the scheduler: the settled value lives
    /// in the heap cell, and the scheduler is told so every reaction and parked
    /// `await` frame bridged to this promise wakes. Returns whether this call
    /// was the settling one (a second settlement is a no-op).
    pub(super) fn settle_promise_resolve(&mut self, promise: zun_gc::Obj, value: zun_core::Any) -> bool {
        let settled = zun_gc::with_current_heap_mut(|heap| heap.promise_resolve(promise, value))
            .unwrap_or(false);
        if settled {
            self.wake_scheduler(promise, Settlement::Fulfilled);
        }
        settled
    }

    /// Reject a managed Promise through the scheduler (see
    /// [`Self::settle_promise_resolve`]). The promise is bridged first so the
    /// scheduler always learns of the rejection — this is what lets a detached
    /// rejection (nothing awaiting/`.catch`-ing it) be tracked as unhandled.
    pub(super) fn settle_promise_reject(&mut self, promise: zun_gc::Obj, error: String) -> bool {
        self.bridge_pid(promise);
        let settled = zun_gc::with_current_heap_mut(|heap| heap.promise_reject(promise, error))
            .unwrap_or(false);
        if settled {
            self.wake_scheduler(promise, Settlement::Rejected);
        }
        settled
    }

    /// Reject a managed Promise with a first-class reason value (an object such
    /// as an `AggregateError`, or any boxed value) plus its display message.
    /// The value is delivered to `catch`/`await`; the message keeps the string
    /// surface working. Bridged first, exactly like [`Self::settle_promise_reject`].
    pub(super) fn settle_promise_reject_value(
        &mut self,
        promise: zun_gc::Obj,
        reason: zun_core::Any,
        message: String,
    ) -> bool {
        self.bridge_pid(promise);
        let settled =
            zun_gc::with_current_heap_mut(|heap| heap.promise_reject_value(promise, reason, message))
                .unwrap_or(false);
        if settled {
            self.wake_scheduler(promise, Settlement::Rejected);
        }
        settled
    }

    /// Settle `target` with a reaction callback's return value, adopting a
    /// returned Promise (chaining, not wrapping). A still-pending returned
    /// Promise is adopted by attaching a pass-through reaction, so `target`
    /// settles exactly when the returned Promise does — the proper async
    /// chaining the old eager path could not express.
    fn settle_target_with(&mut self, target: zun_gc::Obj, value: Value) {
        if let Some(obj) = as_object_ref(&value) {
            if let Some(state) = zun_gc::with_current_heap(|heap| heap.promise_state(obj)) {
                match state {
                    zun_core::ManagedPromiseState::Resolved => {
                        let inner = zun_gc::with_current_heap(|heap| heap.promise_result(obj))
                            .unwrap_or_else(zun_core::Any::null);
                        self.settle_promise_resolve(target, inner);
                    }
                    zun_core::ManagedPromiseState::Rejected => {
                        let error = zun_gc::with_current_heap(|heap| heap.promise_error(obj))
                            .unwrap_or_else(|| "Promise rejected".into());
                        self.settle_promise_reject(target, error);
                    }
                    // Adopt a pending Promise: a pass-through reaction copies its
                    // settlement to `target` when it settles.
                    zun_core::ManagedPromiseState::Pending => {
                        self.attach_reaction(
                            obj,
                            target,
                            ReactionKind::Then {
                                on_fulfilled: None,
                                on_rejected: None,
                            },
                        );
                    }
                }
                return;
            }
            // Generic thenable: an object with a callable `then` is adopted by
            // calling `then(resolve, reject)` with fresh settlers for `target`
            // (the `new Promise` executor mechanism). Function values do not
            // box into `Any`, so a callable `then` lives in the function slot.
            // Note: the call runs synchronously here rather than as a separate
            // microtask job.
            if let Some(then) =
                zun_gc::with_current_heap(|heap| heap.dynamic_get_function(obj, "then"))
            {
                let roots = [target, obj];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let resolve = self.make_promise_settler(roots[0], SETTLE_RESOLVE);
                let reject = self.make_promise_settler(roots[0], SETTLE_REJECT);
                match self.invoke_function_value(
                    then,
                    vec![Value::Function(resolve), Value::Function(reject)],
                ) {
                    Ok(_) => {}
                    Err(RunError::Thrown(thrown)) => {
                        let _ = self.invoke_promise_settler(roots[0], SETTLE_REJECT, vec![thrown]);
                    }
                    Err(_) => {
                        self.settle_promise_reject(roots[0], "thenable.then failed".into());
                    }
                }
                return;
            }
        }
        let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        self.settle_promise_resolve(target, boxed);
    }

    /// Attach a reaction to `source` and bridge its control onto the scheduler.
    /// The reaction waits on `source`'s scheduler promise; when that settles
    /// (now or later), the scheduler enqueues this continuation and the
    /// turn-drain runs [`Self::run_reaction`]. An already-settled source yields
    /// `ReadyNow`, which *enqueues* the reaction rather than running it inline —
    /// the async-not-sync guarantee.
    fn attach_reaction(
        &mut self,
        source: zun_gc::Obj,
        target: zun_gc::Obj,
        kind: ReactionKind,
    ) -> ContinuationId {
        let source_pid = self.bridge_pid(source);
        // Bridge the target too (for its side effect: it may reconcile an
        // already-settled target and is the identity a later settle wakes).
        self.bridge_pid(target);
        let cid = ContinuationId(self.async_state.next_cont);
        self.async_state.next_cont += 1;
        self.async_state.reactions.insert(
            cid,
            Reaction {
                source: Value::Object(source),
                target: Value::Object(target),
                kind,
            },
        );
        let state = &mut self.async_state;
        state
            .scheduler
            .park(&mut state.promises, Continuation::new(cid, source_pid));
        cid
    }

    /// Run one scheduled reaction (a microtask): read the source's settlement
    /// from the heap, run the matching callback (or pass the settlement
    /// through), and settle the reaction's target through the scheduler so its
    /// own reactions/frames wake in turn.
    pub(super) fn run_reaction(&mut self, cid: ContinuationId) -> Result<(), RunError> {
        let Some(reaction) = self.async_state.reactions.remove(&cid) else {
            return Ok(());
        };
        let source = as_object_ref(&reaction.source).ok_or(RunError::BadToken)?;
        let target = as_object_ref(&reaction.target).ok_or(RunError::BadToken)?;
        // Root source and target across any callback re-entry; the scope keeps
        // the array current if a collection relocates them.
        let roots = [source, target];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        match reaction.kind {
            ReactionKind::Then {
                on_fulfilled,
                on_rejected,
            } => match self.promise_settlement_of(roots[0])? {
                PromiseSettlement::Resolved(value) => match as_function(&on_fulfilled) {
                    Some(callback) => self.run_then_callback(callback, Value::Any(value), &roots)?,
                    None => {
                        self.settle_promise_resolve(roots[1], value);
                    }
                },
                PromiseSettlement::Rejected(error, reason) => match as_function(&on_rejected) {
                    Some(callback) => {
                        // Deliver a first-class reason value (an object such as
                        // an `AggregateError`) as-is; otherwise hand the
                        // callback the interned string message.
                        let arg = match reason {
                            Some(reason) => reason,
                            None => zun_gc::with_current_heap_mut(|heap| {
                                zun_core::Any::string(
                                    heap.intern_string(std::rc::Rc::from(error.as_str())),
                                )
                            }),
                        };
                        self.run_then_callback(callback, Value::Any(arg), &roots)?;
                    }
                    None => match reason {
                        Some(reason) => {
                            self.settle_promise_reject_value(roots[1], reason, error);
                        }
                        None => {
                            self.settle_promise_reject(roots[1], error);
                        }
                    },
                },
                // A reaction only runs once its source has settled, so a pending
                // read here is a spurious wake; leave the target pending.
                PromiseSettlement::Pending => {}
            },
            ReactionKind::Finally { on_finally } => {
                let settlement = self.promise_settlement_of(roots[0])?;
                let cleanup_error = match as_function(&on_finally) {
                    Some(callback) => match self.invoke_callback(callback, vec![]) {
                        Ok(_) => None,
                        Err(RunError::Thrown(value)) => Some(value_to_error_string(&value)),
                        Err(other) => return Err(other),
                    },
                    None => None,
                };
                if let Some(error) = cleanup_error {
                    self.settle_promise_reject(roots[1], error);
                } else {
                    match settlement {
                        PromiseSettlement::Resolved(value) => {
                            self.settle_promise_resolve(roots[1], value);
                        }
                        PromiseSettlement::Rejected(error, Some(reason)) => {
                            self.settle_promise_reject_value(roots[1], reason, error);
                        }
                        PromiseSettlement::Rejected(error, None) => {
                            self.settle_promise_reject(roots[1], error);
                        }
                        PromiseSettlement::Pending => {}
                    }
                }
            }
            ReactionKind::Combinator { aggregate, index } => {
                let settlement = self.promise_settlement_of(roots[0])?;
                self.combinator_input_settled(aggregate, index, settlement);
            }
        }
        Ok(())
    }

    /// Invoke a `then`/`catch` callback and settle the reaction's target from
    /// its result (adopting a returned Promise) or reject it on a throw.
    /// `roots[1]` is re-read after the re-entry, which may have relocated it.
    fn run_then_callback(
        &mut self,
        callback: zun_core::FunctionHandle,
        arg: Value,
        roots: &[zun_gc::Obj],
    ) -> Result<(), RunError> {
        match self.invoke_callback(callback, vec![arg]) {
            Ok(result) => {
                self.settle_target_with(roots[1], result);
                Ok(())
            }
            Err(RunError::Thrown(value)) => {
                let message = value_to_error_string(&value);
                self.settle_promise_reject(roots[1], message);
                Ok(())
            }
            Err(other) => Err(other),
        }
    }

    /// `promise.then(onFulfilled?, onRejected?)` / `catch`: create a NEW promise
    /// and attach a reaction to the source. When the source settles (now or
    /// later), the reaction runs the matching callback as a microtask and
    /// settles the new promise. A `then` on a pending source now works — the
    /// behaviour the old eager-at-settle path could not provide.
    fn promise_then(
        &mut self,
        source: zun_gc::Obj,
        on_fulfilled: Option<zun_core::FunctionHandle>,
        on_rejected: Option<zun_core::FunctionHandle>,
    ) -> Result<Value, RunError> {
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        let roots = [source, target];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        self.attach_reaction(
            roots[0],
            roots[1],
            ReactionKind::Then {
                on_fulfilled: on_fulfilled.map(Value::Function),
                on_rejected: on_rejected.map(Value::Function),
            },
        );
        Ok(Value::Object(roots[1]))
    }

    /// `promise.finally(onFinally)`: create a NEW promise and attach a reaction
    /// that runs the cleanup on settlement and passes the source settlement
    /// through, unless the cleanup itself throws.
    fn promise_finally(
        &mut self,
        source: zun_gc::Obj,
        on_finally: Option<zun_core::FunctionHandle>,
    ) -> Result<Value, RunError> {
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        let roots = [source, target];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        self.attach_reaction(
            roots[0],
            roots[1],
            ReactionKind::Finally {
                on_finally: on_finally.map(Value::Function),
            },
        );
        Ok(Value::Object(roots[1]))
    }

    /// Normalize a combinator input to a Promise object: an existing Promise is
    /// used as-is, any other value is wrapped in an already-resolved Promise
    /// (`Promise.resolve` semantics) so every input drives a reaction.
    fn as_input_promise(&mut self, value: &Value) -> zun_gc::Obj {
        if let Some(obj) = as_object_ref(value) {
            if zun_gc::with_current_heap(|heap| heap.promise_state(obj)).is_some() {
                return obj;
            }
        }
        let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        zun_gc::with_current_heap_mut(|heap| {
            let promise = heap.alloc_promise();
            heap.promise_resolve(promise, boxed);
            promise
        })
    }

    /// `Promise.all/allSettled/race/any`: create a NEW promise and an aggregate,
    /// then attach a reaction to each input promise. The aggregate is settled
    /// per spec as the inputs settle through the scheduler, so pending inputs
    /// are handled (the eager path could only read already-settled inputs).
    fn promise_combinator(&mut self, name: &str, array: zun_gc::Obj) -> Result<Value, RunError> {
        let elements = self.read_array_values(array)?;
        let kind = match name {
            "promiseAll" => CombinatorKind::All,
            "promiseAllSettled" => CombinatorKind::AllSettled,
            "promiseRace" => CombinatorKind::Race,
            "promiseAny" => CombinatorKind::Any,
            _ => return Err(RunError::BadToken),
        };
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        // `all`/`allSettled` collect an ordered result array; `any` collects the
        // per-input rejection reasons for its `AggregateError`; `race` keeps no
        // array (first settlement wins).
        let holder = match kind {
            CombinatorKind::All | CombinatorKind::AllSettled | CombinatorKind::Any => {
                Value::Object(zun_gc::with_current_heap_mut(|heap| heap.alloc_array(elements.len())))
            }
            CombinatorKind::Race => Value::Void,
        };
        let aggregate = self.async_state.aggregates.len();
        self.async_state.aggregates.push(Aggregate {
            kind,
            target: Value::Object(target),
            holder,
            remaining: elements.len(),
            settled: false,
        });
        // An empty input settles the aggregate immediately per spec.
        if elements.is_empty() {
            self.conclude_empty_combinator(aggregate);
            return Ok(Value::Object(target));
        }
        for (index, element) in elements.iter().enumerate() {
            let source = self.as_input_promise(element);
            self.attach_reaction(source, target, ReactionKind::Combinator { aggregate, index });
        }
        Ok(Value::Object(target))
    }

    /// Settle an empty-input combinator: `all`/`allSettled` resolve to an empty
    /// array, `race` stays pending forever (spec), `any` rejects (no fulfilment).
    fn conclude_empty_combinator(&mut self, aggregate: usize) {
        let (kind, target, holder) = {
            let agg = &self.async_state.aggregates[aggregate];
            (agg.kind, agg.target.clone(), agg.holder.clone())
        };
        let target = as_object_ref(&target).expect("aggregate target is an object");
        match kind {
            CombinatorKind::All | CombinatorKind::AllSettled => {
                let array = as_object_ref(&holder).expect("all/allSettled holder is an array");
                let boxed = zun_core::Any::object(array.addr() as usize);
                self.settle_promise_resolve(target, boxed);
            }
            CombinatorKind::Race => {}
            CombinatorKind::Any => {
                let errors = as_object_ref(&holder).expect("any holder is an array");
                let reason = self.build_aggregate_error(errors, "All promises were rejected");
                self.settle_promise_reject_value(
                    target,
                    reason,
                    "All promises were rejected".into(),
                );
            }
        }
        self.async_state.aggregates[aggregate].settled = true;
    }

    /// Build an `AggregateError`-shaped dynamic object carrying `errors` (the
    /// collected rejection reasons), `message`, and `name`. A dynamic object
    /// rather than the typed `AggregateError` class: the class is not
    /// guaranteed to be present in the running module (it ships in
    /// `builtin/error.zun`, compiled in only when a program imports it), so the
    /// combinator cannot depend on it. The shape matches the class surface, so
    /// `reason.errors` / `reason.message` / `reason.name` read the same.
    fn build_aggregate_error(&mut self, errors: zun_gc::Obj, message: &str) -> zun_core::Any {
        let roots = [errors];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        zun_gc::with_current_heap_mut(|heap| {
            let object = heap.alloc_dynamic();
            let name = Value::Str(std::rc::Rc::from("AggregateError"))
                .box_any(heap)
                .unwrap_or_else(zun_core::Any::null);
            heap.dynamic_set(object, "name", name);
            let message = Value::Str(std::rc::Rc::from(message))
                .box_any(heap)
                .unwrap_or_else(zun_core::Any::null);
            heap.dynamic_set(object, "message", message);
            heap.dynamic_set(object, "errors", zun_core::Any::object(roots[0].addr() as usize));
            zun_core::Any::object(object.addr() as usize)
        })
    }

    /// Fold one settled input into its combinator aggregate and settle the
    /// aggregate target when the per-spec condition is met.
    fn combinator_input_settled(
        &mut self,
        aggregate: usize,
        index: usize,
        settlement: PromiseSettlement,
    ) {
        if self.async_state.aggregates[aggregate].settled {
            return;
        }
        let kind = self.async_state.aggregates[aggregate].kind;
        match kind {
            CombinatorKind::All => match settlement {
                PromiseSettlement::Resolved(value) => {
                    let holder =
                        as_object_ref(&self.async_state.aggregates[aggregate].holder).unwrap();
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(holder, index, value));
                    let remaining = {
                        let agg = &mut self.async_state.aggregates[aggregate];
                        agg.remaining -= 1;
                        agg.remaining
                    };
                    if remaining == 0 {
                        let (target, holder) = {
                            let agg = &self.async_state.aggregates[aggregate];
                            (
                                as_object_ref(&agg.target).unwrap(),
                                as_object_ref(&agg.holder).unwrap(),
                            )
                        };
                        let boxed = zun_core::Any::object(holder.addr() as usize);
                        self.settle_promise_resolve(target, boxed);
                        self.async_state.aggregates[aggregate].settled = true;
                    }
                }
                PromiseSettlement::Rejected(error, reason) => {
                    let target =
                        as_object_ref(&self.async_state.aggregates[aggregate].target).unwrap();
                    match reason {
                        Some(reason) => {
                            self.settle_promise_reject_value(target, reason, error);
                        }
                        None => {
                            self.settle_promise_reject(target, error);
                        }
                    }
                    self.async_state.aggregates[aggregate].settled = true;
                }
                PromiseSettlement::Pending => {}
            },
            CombinatorKind::AllSettled => {
                let descriptor = match settlement {
                    PromiseSettlement::Resolved(value) => zun_gc::with_current_heap_mut(|heap| {
                        let object = heap.alloc_dynamic();
                        let status = Value::Str(std::rc::Rc::from("fulfilled"))
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        heap.dynamic_set(object, "status", status);
                        heap.dynamic_set(object, "value", value);
                        zun_core::Any::object(object.addr() as usize)
                    }),
                    PromiseSettlement::Rejected(error, reason) => {
                        zun_gc::with_current_heap_mut(|heap| {
                            let object = heap.alloc_dynamic();
                            let status = Value::Str(std::rc::Rc::from("rejected"))
                                .box_any(heap)
                                .unwrap_or_else(zun_core::Any::null);
                            heap.dynamic_set(object, "status", status);
                            // Carry the first-class reason when present, else the
                            // string message.
                            let reason = reason.unwrap_or_else(|| {
                                Value::Str(std::rc::Rc::from(error.as_str()))
                                    .box_any(heap)
                                    .unwrap_or_else(zun_core::Any::null)
                            });
                            heap.dynamic_set(object, "reason", reason);
                            zun_core::Any::object(object.addr() as usize)
                        })
                    }
                    PromiseSettlement::Pending => return,
                };
                let holder = as_object_ref(&self.async_state.aggregates[aggregate].holder).unwrap();
                zun_gc::with_current_heap_mut(|heap| heap.array_set(holder, index, descriptor));
                let remaining = {
                    let agg = &mut self.async_state.aggregates[aggregate];
                    agg.remaining -= 1;
                    agg.remaining
                };
                if remaining == 0 {
                    let (target, holder) = {
                        let agg = &self.async_state.aggregates[aggregate];
                        (
                            as_object_ref(&agg.target).unwrap(),
                            as_object_ref(&agg.holder).unwrap(),
                        )
                    };
                    let boxed = zun_core::Any::object(holder.addr() as usize);
                    self.settle_promise_resolve(target, boxed);
                    self.async_state.aggregates[aggregate].settled = true;
                }
            }
            CombinatorKind::Race => {
                let target = as_object_ref(&self.async_state.aggregates[aggregate].target).unwrap();
                match settlement {
                    PromiseSettlement::Resolved(value) => {
                        self.settle_promise_resolve(target, value);
                    }
                    PromiseSettlement::Rejected(error, Some(reason)) => {
                        self.settle_promise_reject_value(target, reason, error);
                    }
                    PromiseSettlement::Rejected(error, None) => {
                        self.settle_promise_reject(target, error);
                    }
                    PromiseSettlement::Pending => return,
                }
                self.async_state.aggregates[aggregate].settled = true;
            }
            CombinatorKind::Any => match settlement {
                PromiseSettlement::Resolved(value) => {
                    let target =
                        as_object_ref(&self.async_state.aggregates[aggregate].target).unwrap();
                    self.settle_promise_resolve(target, value);
                    self.async_state.aggregates[aggregate].settled = true;
                }
                PromiseSettlement::Rejected(error, reason) => {
                    // Collect this input's rejection reason into the ordered
                    // errors array (a first-class reason as-is, else the string
                    // message boxed).
                    let holder =
                        as_object_ref(&self.async_state.aggregates[aggregate].holder).unwrap();
                    let reason = reason.unwrap_or_else(|| {
                        zun_gc::with_current_heap_mut(|heap| {
                            Value::Str(std::rc::Rc::from(error.as_str()))
                                .box_any(heap)
                                .unwrap_or_else(zun_core::Any::null)
                        })
                    });
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(holder, index, reason));
                    let remaining = {
                        let agg = &mut self.async_state.aggregates[aggregate];
                        agg.remaining -= 1;
                        agg.remaining
                    };
                    if remaining == 0 {
                        let (target, holder) = {
                            let agg = &self.async_state.aggregates[aggregate];
                            (
                                as_object_ref(&agg.target).unwrap(),
                                as_object_ref(&agg.holder).unwrap(),
                            )
                        };
                        // All inputs rejected: reject with an `AggregateError`
                        // carrying the collected reasons (per spec).
                        let aggregate_error =
                            self.build_aggregate_error(holder, "All promises were rejected");
                        self.settle_promise_reject_value(
                            target,
                            aggregate_error,
                            "All promises were rejected".into(),
                        );
                        self.async_state.aggregates[aggregate].settled = true;
                    }
                }
                PromiseSettlement::Pending => {}
            },
        }
    }
}

/// Settlement of a managed Promise, read into owned data so it survives a
/// re-entry that may relocate the heap.
enum PromiseSettlement {
    Pending,
    Resolved(zun_core::Any),
    /// The display message plus an optional first-class reason value (an object
    /// such as an `AggregateError`, or any boxed value) when the rejection
    /// carried one.
    Rejected(String, Option<zun_core::Any>),
}

/// A stored reaction callback's function handle, if present.
fn as_function(value: &Option<Value>) -> Option<zun_core::FunctionHandle> {
    match value {
        Some(Value::Function(handle)) => Some(*handle),
        _ => None,
    }
}

/// The real wall-clock time in milliseconds, read through the `zun:time`
/// `nowMillis` native. Used only as the *base* the virtual timer clock is
/// seeded from; the event loop advances from there by fast-forward.
fn real_now_millis(output: &mut dyn super::Output) -> i64 {
    match zun_natives::resolve("time", "nowMillis") {
        Some((id, _)) => match zun_natives::dispatch(id, &[], output) {
            Ok(zun_natives::NativeValue::Int(ms)) => ms,
            _ => 0,
        },
        None => 0,
    }
}

/// A value's object reference, unwrapping an object-tagged `Any`.
fn as_object_ref(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Object(object) => Some(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
        _ => None,
    }
}

/// Unwrap a `functionBox` wrapper back to its callable. Function values cannot
/// be boxed into `Any`, so a function carried through an `Any`-boxing argument
/// lane (the bound-function record's `__args` array) is wrapped in a dynamic
/// object holding the callable in a `__boxedFn` function slot. A plain object
/// without that slot, and every non-object value, passes through unchanged.
fn unbox_function_box(value: Value, heap: &zun_gc::Heap) -> Value {
    if let Value::Object(object) = value {
        if let Some(handle) = heap.dynamic_get_function(object, "__boxedFn") {
            return Value::Function(handle);
        }
    }
    value
}

/// Render a thrown value / rejection reason as a Promise rejection message
/// (Zun's managed Promise stores a string reason). A boxed `Any` is unwrapped
/// first, so a `string` reason survives being carried as `any`.
fn value_to_error_string(value: &Value) -> String {
    match value {
        Value::Str(message) => message.to_string(),
        Value::Any(any) => match zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)) {
            Some(Value::Str(message)) => message.to_string(),
            Some(inner) => inner.display(),
            None => value.display(),
        },
        other => other.display(),
    }
}

/// Interpret an argument as a `[[Prototype]]` link: `null`/`undefined` clears
/// it, an object reference becomes the prototype, anything else is ignored.
fn prototype_from_arg(value: &Value) -> Option<zun_core::Any> {
    match value {
        Value::Null | Value::Void => None,
        Value::Object(object) => Some(zun_core::Any::object(object.addr() as usize)),
        Value::Any(any) => match any.tag() {
            zun_core::AnyTag::Null | zun_core::AnyTag::Void => None,
            zun_core::AnyTag::Object => Some(*any),
            _ => None,
        },
        _ => None,
    }
}

/// An integer index argument, accepting either numeric representation and
/// truncating a float toward zero (the IL carries relative indices as `i64`).
fn int_arg(value: Option<&Value>) -> Result<i64, RunError> {
    match value.ok_or(RunError::BadToken)? {
        Value::Int(value, _) => Ok(*value),
        Value::Float(value, _) => Ok(*value as i64),
        _ => Err(RunError::BadToken),
    }
}

/// The target element kind a `from`/`of` static carries, decoded from its
/// table-order discriminant argument.
fn kind_arg(value: Option<&Value>) -> Result<zun_core::TypedElement, RunError> {
    match value.ok_or(RunError::BadToken)? {
        Value::Int(value, _) => {
            zun_core::TypedElement::from_index(*value as u32).ok_or(RunError::BadToken)
        }
        _ => Err(RunError::BadToken),
    }
}

/// Numeric equality between a typed-array element and a search value.
/// `indexOf` uses strict equality (NaN never matches); `includes` uses
/// SameValueZero (NaN matches NaN). Comparison is on `f64` for mixed kinds;
/// two integers compare exactly.
fn numeric_same(element: &Value, needle: &Value, same_value_zero: bool) -> bool {
    if let (Value::Int(a, _), Value::Int(b, _)) = (element, needle) {
        return a == b;
    }
    let as_f64 = |value: &Value| match value {
        Value::Int(value, _) => Some(*value as f64),
        Value::Float(value, _) => Some(*value),
        _ => None,
    };
    match (as_f64(element), as_f64(needle)) {
        (Some(a), Some(b)) => {
            if a.is_nan() && b.is_nan() {
                same_value_zero
            } else {
                a == b
            }
        }
        _ => false,
    }
}

/// Coerce a boxed value to a property-key string (ECMAScript ToPropertyKey for
/// the string cases this floor supports).
fn stringify_any(any: zun_core::Any, heap: &zun_gc::Heap) -> Option<String> {
    match Value::unbox_any(any, heap)? {
        Value::Str(text) => Some(text.to_string()),
        Value::Int(value, _) => Some(value.to_string()),
        Value::Float(value, _) => Some(
            if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
                format!("{}", value as i64)
            } else {
                format!("{value}")
            },
        ),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

/// Unbox an argument to its concrete value (an `Any` carrier is unwrapped).
fn unbox_arg(value: &Value) -> Result<Value, RunError> {
    match value {
        Value::Any(any) => {
            zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)).ok_or(RunError::BadToken)
        }
        other => Ok(other.clone()),
    }
}

/// ECMAScript truthiness for a callback result (unboxing an `Any` carrier
/// first): `false`, `0`/`NaN`, `""`, `null` and `undefined` are falsy; every
/// object, function and non-empty string is truthy.
fn is_truthy(value: &Value, heap: &zun_gc::Heap) -> bool {
    let concrete = match value {
        Value::Any(any) => Value::unbox_any(*any, heap).unwrap_or(Value::Void),
        other => other.clone(),
    };
    match concrete {
        Value::Bool(flag) => flag,
        Value::Int(number, _) => number != 0,
        Value::Float(number, _) => number != 0.0 && !number.is_nan(),
        Value::Str(text) => !text.is_empty(),
        Value::Null | Value::Void => false,
        Value::Object(_) | Value::Function(_) | Value::Any(_) => true,
    }
}

/// Coerce a comparator result (unboxing an `Any` carrier) to the number whose
/// sign decides the ordering. Non-numeric results yield `NaN`, treated by the
/// caller as "already ordered" (a stable no-swap).
/// ECMAScript ToNumber applied to a string: trim leading/trailing whitespace,
/// an empty result is `0`, `0x`/`0o`/`0b` prefixes parse in their radix, and
/// everything else (decimal, scientific, `Infinity`) goes through Rust's `f64`
/// parser; a failed parse is `NaN`.
fn js_string_to_number(text: &str) -> f64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    let radix = |prefix: &str, base: u32| -> Option<f64> {
        trimmed
            .strip_prefix(prefix)
            .filter(|rest| !rest.is_empty())
            .and_then(|rest| i64::from_str_radix(rest, base).ok())
            .map(|value| value as f64)
    };
    if let Some(value) = radix("0x", 16).or_else(|| radix("0X", 16)) {
        return value;
    }
    if let Some(value) = radix("0o", 8).or_else(|| radix("0O", 8)) {
        return value;
    }
    if let Some(value) = radix("0b", 2).or_else(|| radix("0B", 2)) {
        return value;
    }
    match trimmed {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => trimmed.parse::<f64>().unwrap_or(f64::NAN),
    }
}

fn to_number(value: &Value, heap: &zun_gc::Heap) -> f64 {
    let concrete = match value {
        Value::Any(any) => Value::unbox_any(*any, heap).unwrap_or(Value::Void),
        other => other.clone(),
    };
    match concrete {
        Value::Int(number, _) => number as f64,
        Value::Float(number, _) => number,
        Value::Bool(flag) => {
            if flag {
                1.0
            } else {
                0.0
            }
        }
        _ => f64::NAN,
    }
}

/// ECMAScript `SameValue`: like `===` but `NaN` equals `NaN` and `+0` differs
/// from `-0`. Numbers compare by IEEE-754 bit pattern after the `NaN` check.
fn same_value(left: &Value, right: &Value) -> bool {
    let number = |value: &Value| match value {
        Value::Int(value, _) => Some(*value as f64),
        Value::Float(value, _) => Some(*value),
        _ => None,
    };
    if let (Some(left), Some(right)) = (number(left), number(right)) {
        if left.is_nan() && right.is_nan() {
            return true;
        }
        return left.to_bits() == right.to_bits();
    }
    match (left, right) {
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Str(left), Value::Str(right)) => left == right,
        (Value::Object(left), Value::Object(right)) => left.addr() == right.addr(),
        (Value::Null, Value::Null) => true,
        (Value::Void, Value::Void) => true,
        _ => false,
    }
}
