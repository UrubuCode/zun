//! Managed collection dispatch extracted from the interpreter core.

use super::{Interp, RunError};
use super::value::Value;
use super::state::{ExecutionFrame, ExecutionOutcome};
use zun_module::read_body;

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
            "dataViewLength" => {
                let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(zun_gc::with_current_heap(|heap| heap.data_view_len(view)).ok_or(RunError::BadToken)? as i64, zun_il::NumKind::U64))
            }
            "dataViewByteLength" => {
                let view = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(zun_gc::with_current_heap(|heap| heap.data_view_byte_len(view)).ok_or(RunError::BadToken)? as i64, zun_il::NumKind::U64))
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
            "u32ArrayLength" | "u8ArrayLength" | "f64ArrayLength" | "u16ArrayLength" => {
                let array = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(zun_gc::with_current_heap(|heap| heap.typed_array_len(array)).ok_or(RunError::BadToken)? as i64, zun_il::NumKind::U64))
            }
            "u32ArrayByteLength" | "u8ArrayByteLength" | "f64ArrayByteLength" | "u16ArrayByteLength" => {
                let array = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(zun_gc::with_current_heap(|heap| heap.typed_array_byte_len(array)).ok_or(RunError::BadToken)? as i64, zun_il::NumKind::U64))
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
            "generatorIteratorNew" => {
                if let Some(method_rid) = args.first().and_then(|value| match value {
                    Value::Int(value, _) => Some(*value as u32),
                    Value::Float(value, _) => Some(*value as u32),
                    _ => None,
                }) {
                    return self.create_generator(method_rid, &[]);
                }
                let generator = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Object(generator))
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
            "bigIntNew" => {
                let text = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(text) => text,
                    _ => return Err(RunError::BadToken),
                };
                let value = zun_core::BigInt::parse_decimal(text)
                    .map_err(|error| RunError::Native(error.into()))?;
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value));
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
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc(0, 0));
                Ok(Value::Object(object))
            }
            "finalizationRegister" => {
                let _registry = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let target = object(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let callback = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Function(callback) => *callback,
                    _ => return Err(RunError::BadToken),
                };
                let id = zun_gc::with_current_heap_mut(|heap| {
                    heap.finalization_register_with_function(target, 0, callback)
                });
                Ok(Value::Int(id.get() as i64, zun_il::NumKind::U64))
            }
            "finalizationUnregister" => {
                let _registry = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let id = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(id, _) if *id >= 0 => *id as u32,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.finalization_unregister(zun_gc::FinalizationId::from_raw(id))
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
            "dynamicCall0" | "dynamicCall1" | "dynamicCall2" => {
                let namespace = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let member = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let call_args = args[2..]
                    .iter()
                    .map(|arg| match arg {
                        Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .ok_or(RunError::BadToken),
                        other => Ok(other.clone()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
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
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let prototype = zun_gc::with_current_heap(|heap| heap.dynamic_prototype(obj));
                Ok(Value::Any(prototype.unwrap_or_else(zun_core::Any::null)))
            }
            "objectSetPrototypeOf" => {
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
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
            _ => Err(RunError::BadToken),
        }
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
                    super::value::decode_field(layout.kind, obj.field(layout.slot), heap)
                });
                out.push((name, value));
            }
        }
        Ok(out)
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
                    prepended.push(Value::unbox_any(any, heap).ok_or(RunError::BadToken)?);
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

    /// Invoke a function value with the given positional arguments. The engine
    /// behind `apply`; exceptions propagate as `RunError::Thrown` (matching the
    /// proxy-trap invocation path).
    pub(crate) fn invoke_function_value(
        &mut self,
        handle: zun_core::FunctionHandle,
        call_args: Vec<Value>,
    ) -> Result<Value, RunError> {
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
