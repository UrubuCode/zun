//! JIT runtime thunks for dynamic arrays, dates, Math, Number and class statics.
//!
//! These `extern "C"` symbols are linked into generated native code by address
//! (see `Jit::build_*_builder` in `lib.rs`). Signatures, visibility and
//! attributes are the FFI contract and must not change.

use crate::JitAnyAbi;

#[inline]
pub extern "C" fn zun_jit_date_year(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_date()
        .unwrap()
        .0
}

/// Read a GC-owned array length without allocating or collecting.
/// Mutating and allocating array operations remain Tier 0 until JIT stack-map
/// and write-barrier support is available.
#[inline]
pub extern "C" fn zun_jit_array_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.array_len(object).map(|len| len as i64).unwrap_or(-1))
}

/// Allocate an empty managed array without triggering collection.
#[inline]
pub extern "C" fn zun_jit_array_new(length: i64) -> i64 {
    if length < 0 {
        return 0;
    }
    zun_gc::with_current_heap_mut(|heap| heap.alloc_array(length as usize).addr() as i64)
}

/// Observe a Tier 1 loop backedge and replenish its local OSR counter.
///
/// Frame replacement is intentionally not performed here yet; keeping the
/// counter in native code establishes the patchpoint contract without
/// changing execution semantics.
#[inline]
pub extern "C" fn zun_jit_osr_poll(counter: i64) -> i64 {
    if counter <= 0 { 1000 } else { counter }
}

/// Read a numeric dynamic array element without allocating or collecting.
///
/// This helper is intentionally limited to numeric `Any` values. Array
/// mutation, allocation, and reference-valued elements remain Tier 0 until
/// their managed ABI and write-barrier contracts are lowered.
#[inline]
pub extern "C" fn zun_jit_array_load_f64(address: i64, index: i64) -> f64 {
    if index < 0 {
        return f64::NAN;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.array_get(object, index as usize)
            .and_then(|value| value.as_float().or_else(|| value.as_int().map(|value| value as f64)))
            .unwrap_or(f64::NAN)
    })
}

/// Read an unsigned 64-bit array element for the narrow typed-array ABI.
#[inline]
pub extern "C" fn zun_jit_array_load_u64(address: i64, index: i64) -> i64 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.array_get(object, index as usize)
            .and_then(|value| value.as_int())
            .unwrap_or(0) as i64
    })
}

pub extern "C" fn zun_jit_array_load_i32(address: i64, index: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.array_get(object, index as usize)
            .and_then(|value| value.as_int())
            .map(|value| value as i32)
            .unwrap_or_default()
    })
}

/// Read a boolean dynamic-array element as the narrow i32 JIT boolean ABI.
#[inline]
pub extern "C" fn zun_jit_array_load_bool(address: i64, index: i64) -> i32 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.array_get(object, index as usize)
            .and_then(|value| value.as_bool())
            .unwrap_or(false) as i32
    })
}

/// Read an object-valued dynamic array element through the managed pointer ABI.
#[inline]
pub extern "C" fn zun_jit_array_load_object(address: i64, index: i64) -> i64 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.array_get(object, index as usize)
            .and_then(|value| value.as_object())
            .unwrap_or(0) as i64
    })
}

/// Store a numeric array element without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_array_store_f64(address: i64, index: i64, value: f64) -> i32 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_set(object, index as usize, zun_core::Any::float(value)).is_some() as i32
    })
}

/// Store an unsigned 64-bit array element for the narrow typed-array ABI.
#[inline]
pub extern "C" fn zun_jit_array_store_u64(address: i64, index: i64, value: i64) -> i32 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_set(object, index as usize, zun_core::Any::int(value)).is_some() as i32
    })
}

pub extern "C" fn zun_jit_array_store_i32(address: i64, index: i64, value: i32) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_set(object, index as usize, zun_core::Any::int(value as i64))
            .is_some() as i32
    })
}

/// Store a boolean dynamic-array element through the narrow i32 JIT ABI.
#[inline]
pub extern "C" fn zun_jit_array_store_bool(address: i64, index: i64, value: i32) -> i32 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_set(object, index as usize, zun_core::Any::bool(value != 0)).is_some() as i32
    })
}

/// Store an object-valued dynamic array element and apply the write barrier.
#[inline]
pub extern "C" fn zun_jit_array_store_object(address: i64, index: i64, value: i64) -> i32 {
    if index < 0 || value == 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    let child = unsafe { zun_gc::Obj::from_addr(value as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_set(object, index as usize, zun_core::Any::object(child.addr() as usize))
            .is_some() as i32
    })
}

/// Store a managed object field and run the generational write barrier.
#[inline]
pub extern "C" fn zun_jit_store_ref(address: i64, slot: i64, value: i64) -> i32 {
    if slot < 0 {
        return 0;
    }
    let owner = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.store_field(owner, slot as usize, value as u64, true);
        1
    })
}

/// Pop a numeric array element without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_array_pop_f64(address: i64) -> f64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_pop(object)
            .and_then(|value| value.as_float().or_else(|| value.as_int().map(|value| value as f64)))
            .unwrap_or(f64::NAN)
    })
}

/// Append a numeric array element and return the new length.
#[inline]
pub extern "C" fn zun_jit_array_push_f64(address: i64, value: f64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_push(object, zun_core::Any::float(value))
            .map(|length| length as i64)
            .unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_array_pop_u64(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_pop(object)
            .and_then(|value| value.as_int())
            .unwrap_or(0)
    })
}

pub extern "C" fn zun_jit_array_pop_i32(address: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_pop(object).and_then(|value| value.as_int()).unwrap_or_default() as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_array_push_u64(address: i64, value: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_push(object, zun_core::Any::int(value))
            .map(|length| length as i64)
            .unwrap_or(-1)
    })
}

pub extern "C" fn zun_jit_array_push_i32(address: i64, value: i32) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_push(object, zun_core::Any::int(value as i64)).map(|length| length as i64).unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_array_pop_bool(address: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_pop(object).and_then(|value| value.as_bool()).unwrap_or(false) as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_array_push_bool(address: i64, value: i32) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_push(object, zun_core::Any::bool(value != 0)).map(|length| length as i64).unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_array_pop_object(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_pop(object).and_then(|value| value.as_object()).unwrap_or(0) as i64
    })
}

#[inline]
pub extern "C" fn zun_jit_array_push_object(address: i64, value: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    let child = unsafe { zun_gc::Obj::from_addr(value as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_push(object, zun_core::Any::object(child.addr() as usize))
            .map(|length| length as i64).unwrap_or(-1)
    })
}

/// Copy one dynamic array element into the stable two-word JIT carrier.
///
/// The out-parameter ABI is intentional: it keeps `tag` and `payload`
/// independently addressable until Cranelift has a managed aggregate type and
/// precise reference metadata for dynamic values. The helper does not allocate
/// or collect, so the caller may safely keep the array reference live across
/// the call.
#[inline]
pub unsafe extern "C" fn zun_jit_array_load_any(
    address: i64,
    index: i64,
    out: *mut JitAnyAbi,
) -> i32 {
    if out.is_null() || index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    let Some(value) = zun_gc::with_current_heap(|heap| heap.array_get(object, index as usize)) else {
        return 0;
    };
    unsafe { out.write(JitAnyAbi::from_any(value)) };
    1
}

/// Store a dynamic two-word carrier into an array element.
///
/// The raw parts are validated before entering managed storage. This keeps
/// the dynamic ABI explicit while leaving aggregate stack lowering gated.
#[inline]
pub extern "C" fn zun_jit_array_store_any(
    address: i64,
    index: i64,
    tag: u64,
    payload: u64,
) -> i32 {
    if index < 0 {
        return 0;
    }
    let Some(value) = zun_core::Any::from_raw_parts(tag as u8, payload) else {
        return 0;
    };
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_set(object, index as usize, value).is_some() as i32
    })
}
#[inline]
pub extern "C" fn zun_jit_date_is_valid(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis).millis().is_some() as i32
}
#[inline]
pub extern "C" fn zun_jit_date_month(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_date()
        .unwrap()
        .1 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_day(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_date()
        .unwrap()
        .2 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_hour(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_time()
        .unwrap()
        .0 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_minute(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_time()
        .unwrap()
        .1 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_second(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_time()
        .unwrap()
        .2 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_millisecond(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_time()
        .unwrap()
        .3 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_weekday(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis).weekday().unwrap() as i32
}
#[inline]
pub extern "C" fn zun_jit_date_day_of_year(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis).ordinal().unwrap() as i32
}

#[inline]
pub extern "C" fn zun_jit_math_abs(value: f64) -> f64 {
    value.abs()
}
#[inline]
pub extern "C" fn zun_jit_math_sqrt(value: f64) -> f64 {
    value.sqrt()
}
#[inline]
pub extern "C" fn zun_jit_math_cbrt(value: f64) -> f64 {
    value.cbrt()
}
#[inline]
pub extern "C" fn zun_jit_math_sign(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        value
    } else if value.is_sign_positive() {
        1.0
    } else {
        -1.0
    }
}
#[inline]
pub extern "C" fn zun_jit_math_floor(value: f64) -> f64 {
    value.floor()
}
#[inline]
pub extern "C" fn zun_jit_math_ceil(value: f64) -> f64 {
    value.ceil()
}
#[inline]
pub extern "C" fn zun_jit_math_round(value: f64) -> f64 {
    value.round()
}
#[inline]
pub extern "C" fn zun_jit_math_trunc(value: f64) -> f64 {
    value.trunc()
}
#[inline]
pub extern "C" fn zun_jit_math_sin(value: f64) -> f64 {
    value.sin()
}
#[inline]
pub extern "C" fn zun_jit_math_cos(value: f64) -> f64 {
    value.cos()
}
#[inline]
pub extern "C" fn zun_jit_math_exp(value: f64) -> f64 {
    value.exp()
}
#[inline]
pub extern "C" fn zun_jit_math_log(value: f64) -> f64 {
    value.ln()
}
#[inline]
pub extern "C" fn zun_jit_math_log10(value: f64) -> f64 {
    value.log10()
}
#[inline]
pub extern "C" fn zun_jit_math_log2(value: f64) -> f64 {
    value.log2()
}
#[inline]
pub extern "C" fn zun_jit_math_expm1(value: f64) -> f64 {
    value.exp_m1()
}
#[inline]
pub extern "C" fn zun_jit_math_log1p(value: f64) -> f64 {
    value.ln_1p()
}
#[inline]
pub extern "C" fn zun_jit_math_sinh(value: f64) -> f64 {
    value.sinh()
}
#[inline]
pub extern "C" fn zun_jit_math_cosh(value: f64) -> f64 {
    value.cosh()
}
#[inline]
pub extern "C" fn zun_jit_math_tanh(value: f64) -> f64 {
    value.tanh()
}
#[inline]
pub extern "C" fn zun_jit_math_asinh(value: f64) -> f64 {
    value.asinh()
}
#[inline]
pub extern "C" fn zun_jit_math_acosh(value: f64) -> f64 {
    value.acosh()
}
#[inline]
pub extern "C" fn zun_jit_math_atanh(value: f64) -> f64 {
    value.atanh()
}
#[inline]
pub extern "C" fn zun_jit_math_tan(value: f64) -> f64 {
    value.tan()
}
#[inline]
pub extern "C" fn zun_jit_math_min(left: f64, right: f64) -> f64 {
    left.min(right)
}
#[inline]
pub extern "C" fn zun_jit_math_max(left: f64, right: f64) -> f64 {
    left.max(right)
}
#[inline]
pub extern "C" fn zun_jit_math_pow(left: f64, right: f64) -> f64 {
    left.powf(right)
}
#[inline]
pub extern "C" fn zun_jit_math_atan2(left: f64, right: f64) -> f64 {
    left.atan2(right)
}
#[inline]
pub extern "C" fn zun_jit_math_hypot(left: f64, right: f64) -> f64 {
    left.hypot(right)
}
/// Read a class-static field as a raw i64, matching the field-slot encoding that
/// [`crate::shim::unpack_arg`] decodes (f64/f32 as bits, ints sign/zero handled by
/// the caller, object as address, string as handle). The `kind` is the field's
/// `JitKind` byte. Reads the shared heap statics store, so it sees interpreter
/// writes and vice versa.
#[inline]
pub extern "C" fn zun_jit_static_get(owner: i64, field: i64, kind: i64) -> i64 {
    use zun_il::{JitKind, NumKind};
    zun_gc::with_current_heap(|heap| {
        let any = heap.static_get(owner as u32, field as u32);
        match JitKind::from_byte(kind as u8) {
            JitKind::Num(NumKind::F64) => {
                any.and_then(|a| a.as_float()).unwrap_or(0.0).to_bits() as i64
            }
            JitKind::Num(NumKind::F32) => {
                (any.and_then(|a| a.as_float()).unwrap_or(0.0) as f32).to_bits() as i64
            }
            JitKind::Num(_) => any.and_then(|a| a.as_int()).unwrap_or(0),
            JitKind::Bool => any.and_then(|a| a.as_bool()).unwrap_or(false) as i64,
            JitKind::Object | JitKind::Array => any.and_then(|a| a.as_object()).unwrap_or(0) as i64,
            JitKind::String => any.and_then(|a| a.as_string()).unwrap_or(0) as i64,
            JitKind::Function | JitKind::Other => 0,
        }
    })
}

/// Write a class-static field from a raw i64 (the [`crate::shim::pack_result`]
/// encoding), tagging the stored `Any` by the field's `JitKind` so the
/// interpreter decodes it to the right value and the GC traces object/string
/// references. Writes the shared heap statics store.
#[inline]
pub extern "C" fn zun_jit_static_set(owner: i64, field: i64, raw: i64, kind: i64) {
    use zun_il::{JitKind, NumKind};
    zun_gc::with_current_heap_mut(|heap| {
        let any = match JitKind::from_byte(kind as u8) {
            JitKind::Num(NumKind::F64) => zun_core::Any::float(f64::from_bits(raw as u64)),
            JitKind::Num(NumKind::F32) => zun_core::Any::float(f32::from_bits(raw as u32) as f64),
            JitKind::Num(_) => zun_core::Any::int(raw),
            JitKind::Bool => zun_core::Any::bool(raw != 0),
            JitKind::Object | JitKind::Array => zun_core::Any::object(raw as usize),
            JitKind::String => zun_core::Any::string(raw as u32),
            JitKind::Function | JitKind::Other => zun_core::Any::null(),
        };
        heap.static_set(owner as u32, field as u32, any);
    });
}

#[inline]
pub extern "C" fn zun_jit_math_fmod(left: f64, right: f64) -> f64 {
    // JS `%` remainder (truncated), matching the Tier 0 `a % b`: C `fmod`
    // semantics, including `fmod(finite, ±inf) == finite` and NaN for a zero or
    // infinite divisor. A Cranelift inline (`a - b*trunc(a/b)`) would diverge on
    // an infinite divisor, so the remainder goes through this libcall.
    left % right
}
#[inline]
pub extern "C" fn zun_jit_number_is_finite(value: f64) -> i32 {
    value.is_finite() as i32
}
#[inline]
pub extern "C" fn zun_jit_number_is_nan(value: f64) -> i32 {
    value.is_nan() as i32
}
#[inline]
pub extern "C" fn zun_jit_number_is_negative_zero(value: f64) -> i32 {
    (value == 0.0 && value.is_sign_negative()) as i32
}
#[inline]
pub extern "C" fn zun_jit_number_is_safe_integer(value: f64) -> i32 {
    (value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0) as i32
}
