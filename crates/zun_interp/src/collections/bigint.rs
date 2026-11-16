//! BigInt native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_bigint(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "bigIntNew" => {
                let text = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(text) => text,
                    _ => return Err(RunError::BadToken),
                };
                let value = zun_core::BigInt::parse(text)
                    .map_err(|error| RunError::Native(format!("SyntaxError: {}", error)))?;
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
                    .map_err(|error| RunError::Native(format!("RangeError: {}", error)))?;
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value));
                Ok(Value::Object(object))
            }
            "bigIntFromAny" => {
                // `BigInt(value)` for a dynamic operand: ToPrimitive(number), then
                // a number must be integral, a string parses, a boolean is 0n/1n,
                // a BigInt passes through, anything else is a TypeError.
                let mut value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                if let Value::Object(obj) = &value {
                    let is_bigint =
                        zun_gc::with_current_heap(|heap| heap.bigint_value(*obj).is_some());
                    if !is_bigint {
                        value = match array_join_text(*obj) {
                            Some(text) => Value::Str(text.into()),
                            None => match self.ordinary_to_primitive(*obj, "number")? {
                                Some(primitive) => unbox_arg(&primitive)?,
                                None => Value::Str("[object Object]".into()),
                            },
                        };
                    }
                }
                let value = match value {
                    Value::Object(obj) => return Ok(Value::Object(obj)),
                    Value::Str(text) => zun_core::BigInt::parse_js_string(&text)
                        .map_err(|error| RunError::Native(format!("SyntaxError: {}", error)))?,
                    Value::Float(number, _) => zun_core::BigInt::from_f64(number)
                        .map_err(|error| RunError::Native(format!("RangeError: {}", error)))?,
                    Value::Int(number, _) => zun_core::BigInt::from_f64(number as f64)
                        .map_err(|error| RunError::Native(format!("RangeError: {}", error)))?,
                    Value::Bool(flag) => zun_core::BigInt::from_u64(u64::from(flag)),
                    Value::Null => {
                        return Err(RunError::Native(
                            "TypeError: Cannot convert null to a BigInt".into(),
                        ))
                    }
                    _ => {
                        return Err(RunError::Native(
                            "TypeError: Cannot convert undefined to a BigInt".into(),
                        ))
                    }
                };
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value));
                Ok(Value::Object(object))
            }
            "bigIntUshr" => Err(RunError::Native(
                "TypeError: BigInts have no unsigned right shift, use >> instead".into(),
            )),
            "bigIntPow" | "bigIntAnd" | "bigIntOr" | "bigIntXor" | "bigIntShl" | "bigIntShr" => {
                let left =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let right =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let (left, right) = zun_gc::with_current_heap(|heap| {
                    Some((
                        heap.bigint_value(left)?.clone(),
                        heap.bigint_value(right)?.clone(),
                    ))
                })
                .ok_or(RunError::BadToken)?;
                let value = match name {
                    "bigIntPow" => left
                        .pow(&right)
                        .map_err(|error| RunError::Native(format!("RangeError: {}", error)))?,
                    "bigIntAnd" => left.bitand(&right),
                    "bigIntOr" => left.bitor(&right),
                    "bigIntXor" => left.bitxor(&right),
                    "bigIntShl" | "bigIntShr" => {
                        let amount = right.to_u64().ok_or_else(|| {
                            RunError::Native("RangeError: BigInt shift amount too large".into())
                        })?;
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
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(object).cloned())
                    .ok_or(RunError::BadToken)?;
                let object =
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value.bitnot()));
                Ok(Value::Object(object))
            }
            "bigIntAsIntN" | "bigIntAsUintN" => {
                // ToIndex(bits): an integer or float directly, or a boxed
                // number / numeric string unwrapped through `Any`.
                let bits_arg = args.first().ok_or(RunError::BadToken)?;
                let bits = match bits_arg {
                    Value::Int(bits, _) => *bits,
                    Value::Float(bits, _) => *bits as i64,
                    other => match unbox_arg(other) {
                        Ok(Value::Int(bits, _)) => bits,
                        Ok(Value::Float(bits, _)) => bits as i64,
                        Ok(Value::Str(text)) => text.trim().parse::<f64>().map(|f| f as i64).map_err(
                            |_| {
                                RunError::Native(
                                    "RangeError: BigInt bit width must be a non-negative integer"
                                        .into(),
                                )
                            },
                        )?,
                        _ => {
                            return Err(RunError::Native(
                                "TypeError: Cannot convert the bit width to an integer".into(),
                            ))
                        }
                    },
                };
                if bits < 0 {
                    return Err(RunError::Native(
                        "RangeError: BigInt bit width must be non-negative".into(),
                    ));
                }
                // ToBigInt(value): a BigInt passes through, a string parses,
                // and a number or anything else throws, matching ToBigInt.
                let value_arg = args.get(1).ok_or(RunError::BadToken)?;
                let value = if let Some(object) = object(value_arg) {
                    zun_gc::with_current_heap(|heap| heap.bigint_value(object).cloned()).ok_or_else(
                        || {
                            RunError::Native(
                                "TypeError: Cannot convert the given value to a BigInt".into(),
                            )
                        },
                    )?
                } else {
                    match unbox_arg(value_arg) {
                        Ok(Value::Str(text)) => zun_core::BigInt::parse_js_string(&text)
                            .map_err(|error| RunError::Native(format!("SyntaxError: {}", error)))?,
                        _ => {
                            return Err(RunError::Native(
                                "TypeError: Cannot convert the given value to a BigInt".into(),
                            ))
                        }
                    }
                };
                let result = if name == "bigIntAsIntN" {
                    value.as_int_n(bits as u64)
                } else {
                    value.as_uint_n(bits as u64)
                };
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(result));
                Ok(Value::Object(object))
            }
            "bigIntAdd" | "bigIntSub" | "bigIntMul" | "bigIntDiv" | "bigIntRem"
            | "bigIntCompare" | "bigIntEq" | "bigIntNe" | "bigIntLt" | "bigIntLe" | "bigIntGt"
            | "bigIntGe" => {
                let left =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let right =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let (left, right) = zun_gc::with_current_heap(|heap| {
                    Some((
                        heap.bigint_value(left)?.clone(),
                        heap.bigint_value(right)?.clone(),
                    ))
                })
                .ok_or(RunError::BadToken)?;
                if name == "bigIntCompare" {
                    let result = match left.cmp(&right) {
                        std::cmp::Ordering::Less => -1,
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                    };
                    Ok(Value::Int(result, zun_il::NumKind::I32))
                } else if matches!(
                    name,
                    "bigIntEq" | "bigIntNe" | "bigIntLt" | "bigIntLe" | "bigIntGt" | "bigIntGe"
                ) {
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
                                return Err(RunError::Native("RangeError: BigInt division by zero".into()));
                            };
                            if name == "bigIntDiv" {
                                quotient
                            } else {
                                remainder
                            }
                        }
                        _ => unreachable!(),
                    };
                    let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value));
                    Ok(Value::Object(object))
                }
            }
            "bigIntNeg" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(object).cloned())
                    .ok_or(RunError::BadToken)?;
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value.neg()));
                Ok(Value::Object(object))
            }
            "bigIntToStringRadix" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(object).cloned())
                    .ok_or(RunError::BadToken)?;
                // ToIntegerOrInfinity(radix); `undefined` means 10.
                let radix = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                    Value::Void => 10.0,
                    other => {
                        let number = self.dyn_to_number(&other)?;
                        if number.is_nan() {
                            0.0
                        } else {
                            number.trunc()
                        }
                    }
                };
                if !(2.0..=36.0).contains(&radix) {
                    return Err(RunError::Native(
                        "RangeError: toString() radix argument must be between 2 and 36".into(),
                    ));
                }
                Ok(Value::Str(
                    crate::proto_methods::bigint_to_radix(&value, radix as u64).into(),
                ))
            }
            "bigIntToString" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(object).cloned())
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Str(value.to_string().into()))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
