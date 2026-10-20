//! Managed collection dispatch extracted from the interpreter core.

use super::{Interp, RunError};
use super::value::Value;

impl<'a> Interp<'a> {
    pub(crate) fn call_managed_collection(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        let object = |value: &Value| match value {
            Value::Object(object) => Some(*object),
            Value::Any(any) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
            _ => None,
        };
        let any = |value: &Value| zun_gc::with_current_heap_mut(|heap| value.box_any(heap));
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
            "u32ArrayNew" => {
                let buffer =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let number = |value: &Value| match value {
                    Value::Int(value, _) => Ok(*value as usize),
                    _ => Err(RunError::BadToken),
                };
                let offset = number(args.get(1).ok_or(RunError::BadToken)?)?;
                let length = number(args.get(2).ok_or(RunError::BadToken)?)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_u32_array(buffer, offset, length)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(array))
            }
            "u32ArrayGet" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.u32_array_get(array, index))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U32,
                ))
            }
            "u32ArraySet" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.u32_array_set(array, index, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "u8ArrayNew" => {
                let buffer =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let number = |value: &Value| match value {
                    Value::Int(value, _) => Ok(*value as usize),
                    _ => Err(RunError::BadToken),
                };
                let offset = number(args.get(1).ok_or(RunError::BadToken)?)?;
                let length = number(args.get(2).ok_or(RunError::BadToken)?)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_u8_array(buffer, offset, length)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(array))
            }
            "u8ArrayGet" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.u8_array_get(array, index))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U8,
                ))
            }
            "u8ArraySet" => {
                let array =
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
                    zun_gc::with_current_heap_mut(|heap| heap.u8_array_set(array, index, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "f64ArrayNew" => {
                let buffer =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let number = |value: &Value| match value {
                    Value::Int(value, _) => Ok(*value as usize),
                    _ => Err(RunError::BadToken),
                };
                let offset = number(args.get(1).ok_or(RunError::BadToken)?)?;
                let length = number(args.get(2).ok_or(RunError::BadToken)?)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_f64_array(buffer, offset, length)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(array))
            }
            "f64ArrayGet" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Float(
                    zun_gc::with_current_heap(|heap| heap.f64_array_get(array, index))
                        .ok_or(RunError::BadToken)?,
                    zun_il::NumKind::F64,
                ))
            }
            "f64ArraySet" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.f64_array_set(array, index, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "u16ArrayNew" => {
                let buffer =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let number = |value: &Value| match value {
                    Value::Int(value, _) => Ok(*value as usize),
                    _ => Err(RunError::BadToken),
                };
                let offset = number(args.get(1).ok_or(RunError::BadToken)?)?;
                let length = number(args.get(2).ok_or(RunError::BadToken)?)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_u16_array(buffer, offset, length)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(array))
            }
            "u16ArrayGet" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.u16_array_get(array, index))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U16,
                ))
            }
            "u16ArraySet" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u16,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.u16_array_set(array, index, value))
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
            "mapNew" => {
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_map());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
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
                Ok(Value::Object(array))
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
                Ok(Value::Object(array))
            }
            "setNew" => {
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_set());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
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
                Ok(Value::Object(array))
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
                Ok(Value::Object(array))
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
            "promiseNew" => {
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                Ok(Value::Object(promise))
            }
            "promiseResolve" => {
                let promise = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let value = any(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.promise_resolve(promise, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "promiseReject" => {
                let promise = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let error = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(error) => error.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.promise_reject(promise, error))
                        .ok_or(RunError::BadToken)?,
                ))
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
            _ => Err(RunError::BadToken),
        }
    }

}
