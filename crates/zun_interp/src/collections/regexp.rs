//! RegExp native methods (exec/test/match/replace/flags).
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_regexp(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "regexpNew" => {
                let source = match unbox_arg(args.first().ok_or(RunError::BadToken)?)? {
                    Value::Str(source) => source.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let flags = match args.get(1) {
                    Some(flags) => match unbox_arg(flags)? {
                        Value::Str(flags) => flags.to_string(),
                        _ => return Err(RunError::BadToken),
                    },
                    None => String::new(),
                };
                let object = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_regex_with_flags(source, flags)
                })
                .map_err(|error| {
                    RunError::Native(format!("SyntaxError: Invalid regular expression: {error}"))
                })?;
                Ok(Value::Object(object))
            }
            "regexpExec" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let input = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(input) => input.clone(),
                    _ => return Err(RunError::BadToken),
                };
                match zun_gc::with_current_heap_mut(|heap| heap.regex_exec_stateful(object, &input))
                    .ok_or(RunError::BadToken)?
                    .map_err(|error| RunError::Native(error.to_string()))?
                {
                    Some(array) => Ok(Value::Object(array)),
                    None => Ok(Value::Null),
                }
            }
            "regexpTest" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let input = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(input) => input.clone(),
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.regex_test_stateful(object, &input))
                        .ok_or(RunError::BadToken)?
                        .map_err(|error| RunError::Native(error.to_string()))?,
                ))
            }
            "regexpLastIndex" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.regex_last_index_any(object))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Any(value))
            }
            "regexpSetLastIndex" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = args.get(1).ok_or(RunError::BadToken)?.clone();
                let any = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                    .ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.regex_assign_last_index(object, any))
                    .map_err(|error| RunError::Native(error.to_string()))?;
                Ok(Value::Any(any))
            }
            "regexpReplaceAllGuard" => {
                let regex =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let global = zun_gc::with_current_heap(|heap| {
                    heap.regex_flags(regex).map(|flags| flags.contains('g'))
                })
                .ok_or(RunError::BadToken)?;
                if !global {
                    return Err(RunError::Native(
                        "TypeError: replaceAll must be called with a global RegExp".into(),
                    ));
                }
                Ok(Value::Object(regex))
            }
            "regexpMatch" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let input = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(input) => input.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let global = zun_gc::with_current_heap(|heap| {
                    heap.regex_flags(object).map(|flags| flags.contains('g'))
                })
                .ok_or(RunError::BadToken)?;
                let found = if global {
                    zun_gc::with_current_heap_mut(|heap| {
                        match heap.regex_store_last_index(object, 0.0) {
                            Ok(()) => heap.regex_match_global(object, &input),
                            Err(error) => Some(Err(error)),
                        }
                    })
                } else {
                    zun_gc::with_current_heap_mut(|heap| heap.regex_exec(object, &input))
                }
                .ok_or(RunError::BadToken)?
                .map_err(|error| RunError::Native(error.to_string()))?;
                match found {
                    Some(array) => Ok(Value::Object(array)),
                    None => Ok(Value::Null),
                }
            }
            "stringMatchAll" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
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
            "regexpIsMatch" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
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
            "regexpSearch" | "regexpSplit" | "regexpSplitLimit" | "regexpReplace" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
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
                    "regexpSplit" | "regexpSplitLimit" => {
                        let limit = match (name, args.get(2)) {
                            ("regexpSplitLimit", Some(Value::Int(n, _))) => {
                                Some((*n as u64 & 0xFFFF_FFFF) as usize)
                            }
                            _ => None,
                        };
                        let array = zun_gc::with_current_heap_mut(|heap| {
                            heap.regex_split(object, &input, limit)
                        })
                        .ok_or(RunError::BadToken)?
                        .map_err(|error| RunError::Native(error.to_string()))?;
                        Ok(Value::Object(array))
                    }
                    _ => {
                        let replacement = match args.get(2).ok_or(RunError::BadToken)? {
                            Value::Str(text) => text.clone(),
                            _ => return Err(RunError::BadToken),
                        };
                        let result = zun_gc::with_current_heap_mut(|heap| {
                            heap.regex_replace_first(object, &input, &replacement)
                        })
                        .ok_or(RunError::BadToken)?
                        .map_err(|error| RunError::Native(error.to_string()))?;
                        zun_gc::with_current_heap_mut(|heap| {
                            if heap
                                .regex_flags(object)
                                .is_some_and(|flags| flags.contains('g'))
                            {
                                heap.regex_store_last_index(object, 0.0)
                            } else {
                                Ok(())
                            }
                        })
                        .map_err(|error| RunError::Native(error.to_string()))?;
                        Ok(Value::Str(result.into()))
                    }
                }
            }
            "regexpSource" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let source =
                    zun_gc::with_current_heap(|heap| heap.regex_source(object).map(str::to_owned))
                        .ok_or(RunError::BadToken)?;
                Ok(Value::Str(source.into()))
            }
            "regexpFlags" | "regexpFlag" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let flags =
                    zun_gc::with_current_heap(|heap| heap.regex_flags(object).map(str::to_owned))
                        .ok_or(RunError::BadToken)?;
                if name == "regexpFlags" {
                    // The `flags` accessor reports canonical (alphabetical
                    // by spec order) `dgimsuvy`.
                    return Ok(Value::Str(zun_core::ordered_flags(&flags).into()));
                }
                let flag = match args.get(1) {
                    Some(Value::Str(text)) => match &**text {
                        "hasIndices" => 'd',
                        "global" => 'g',
                        "ignoreCase" => 'i',
                        "multiline" => 'm',
                        "dotAll" => 's',
                        "unicode" => 'u',
                        "unicodeSets" => 'v',
                        "sticky" => 'y',
                        _ => return Err(RunError::BadToken),
                    },
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(flags.contains(flag)))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
