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
            "bigIntUshr" => Err(RunError::Native(
                "TypeError: BigInts have no unsigned right shift, use >> instead".into(),
            )),
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
            _ => Err(RunError::BadToken),
        }
    }
}
