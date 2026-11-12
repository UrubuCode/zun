//! Array / Map / Set / String iterator new+next+done native methods (hot).
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    #[inline]
    pub(super) fn col_iterator(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
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
                    Some(Value::Int(limit, _)) if *limit >= 0 => *limit & 0xFFFF_FFFF,
                    Some(Value::Int(limit, _)) => *limit,
                    _ => -1,
                };
                let mut pieces: Vec<String> = if separator.is_empty() {
                    zun_core::wtf16::encode(&value)
                        .map(|unit| zun_core::wtf16::from_units(&[unit]))
                        .collect()
                } else {
                    value
                        .split(separator.as_ref())
                        .map(str::to_string)
                        .collect()
                };
                if limit >= 0 {
                    pieces.truncate(limit as usize);
                }
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(pieces.len());
                    for (index, piece) in pieces.iter().enumerate() {
                        let value = zun_core::Any::string(heap.intern_string(piece.as_str()));
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
                        let value = zun_core::Any::string(heap.intern_string(point.as_str()));
                        heap.array_set(array, index, value);
                    }
                    heap.alloc_array_iterator_mode(array, 3)
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
            _ => Err(RunError::BadToken),
        }
    }
}
