//! ArrayBuffer / Buffer / DataView native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_buffer(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
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
                let copy =
                    zun_gc::with_current_heap_mut(|heap| heap.buffer_slice(object, begin, end))
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
                let object =
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_resizable_buffer(length, max))
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
                let ok =
                    zun_gc::with_current_heap_mut(|heap| heap.buffer_resize(object, new_length))
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
                let transferred = zun_gc::with_current_heap_mut(|heap| {
                    heap.buffer_transfer(object, new_length, fixed)
                })
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
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.data_view_len(view))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "dataViewByteLength" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.data_view_byte_len(view))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "dataViewByteOffset" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.data_view_byte_offset(view))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "dataViewBuffer" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let buffer = zun_gc::with_current_heap(|heap| heap.data_view_buffer(view))
                    .ok_or(RunError::BadToken)?;
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
                    zun_gc::with_current_heap(|heap| heap.data_view_get_i8(view, relative))
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
                    zun_gc::with_current_heap(|heap| heap.data_view_get_u8(view, relative))
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
            _ => Err(RunError::BadToken),
        }
    }
}
