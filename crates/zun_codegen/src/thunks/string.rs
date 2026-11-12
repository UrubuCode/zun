//! JIT runtime thunks for string operations.
//!
//! These `extern "C"` symbols are linked into generated native code by address
//! (see `Jit::build_*_builder` in `lib.rs`). Signatures, visibility and
//! attributes are the FFI contract and must not change.

/// Read the UTF-16 length of an interned string handle without allocation.
#[inline]
pub extern "C" fn zun_jit_string_length(handle: i64) -> i64 {
    zun_gc::with_current_heap(|heap| {
        heap.try_string(handle as u32)
            .map(|value| zun_core::wtf16::len16(value) as i64)
            .unwrap_or(-1)
    })
}

/// Intern JIT-owned UTF-8 literal bytes and return the stable managed handle.
#[inline]
pub extern "C" fn zun_jit_string_literal(bytes: i64, len: i64) -> i64 {
    if bytes == 0 || len < 0 {
        return 0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(bytes as *const u8, len as usize) };
    let value = String::from_utf8_lossy(bytes);
    zun_gc::with_current_heap_mut(|heap| {
        heap.intern_string(std::rc::Rc::from(value.as_ref())).into()
    })
}

/// Compare two interned UTF-16 strings without allocating.
#[inline]
pub extern "C" fn zun_jit_string_contains(value: i64, needle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else {
            return 0;
        };
        let Some(needle) = heap.try_string(needle as u32) else {
            return 0;
        };
        i32::from(value.contains(needle.as_ref()))
    })
}

/// Read one UTF-16 code unit from an interned string without allocation.
#[inline]
pub extern "C" fn zun_jit_string_char_code_at(handle: i64, index: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        heap.try_string(handle as u32)
            .and_then(|value| zun_core::wtf16::encode(value).nth(index as usize))
            .map(|code| code as i32)
            .unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_string_starts_with(value: i64, prefix: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else {
            return 0;
        };
        let Some(prefix) = heap.try_string(prefix as u32) else {
            return 0;
        };
        i32::from(value.starts_with(prefix.as_ref()))
    })
}

#[inline]
pub extern "C" fn zun_jit_string_ends_with(value: i64, suffix: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else {
            return 0;
        };
        let Some(suffix) = heap.try_string(suffix as u32) else {
            return 0;
        };
        i32::from(value.ends_with(suffix.as_ref()))
    })
}

#[inline]
pub extern "C" fn zun_jit_string_index_of(value: i64, needle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else {
            return -1;
        };
        let Some(needle) = heap.try_string(needle as u32) else {
            return -1;
        };
        let value_units: Vec<u16> = zun_core::wtf16::encode(value).collect();
        let needle_units: Vec<u16> = zun_core::wtf16::encode(needle).collect();
        zun_core::string_ops::find_from(&value_units, &needle_units, 0)
            .map(|index| index as i32)
            .unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_string_last_index_of(value: i64, needle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else {
            return -1;
        };
        let Some(needle) = heap.try_string(needle as u32) else {
            return -1;
        };
        let value_units: Vec<u16> = zun_core::wtf16::encode(value).collect();
        let needle_units: Vec<u16> = zun_core::wtf16::encode(needle).collect();
        zun_core::string_ops::last_index_from(&value_units, &needle_units, value_units.len())
            .map(|index| index as i32)
            .unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_string_count(value: i64, needle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else {
            return 0;
        };
        let Some(needle) = heap.try_string(needle as u32) else {
            return 0;
        };
        let value_units: Vec<u16> = zun_core::wtf16::encode(value).collect();
        let needle_units: Vec<u16> = zun_core::wtf16::encode(needle).collect();
        zun_core::string_ops::count_units(&value_units, &needle_units) as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_string_compare(left: i64, right: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(left) = heap.try_string(left as u32) else {
            return 0;
        };
        let Some(right) = heap.try_string(right as u32) else {
            return 0;
        };
        if left.as_str() < right.as_str() {
            -1
        } else if left.as_str() > right.as_str() {
            1
        } else {
            0
        }
    })
}

#[inline]
pub extern "C" fn zun_jit_string_concat(left: i64, right: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(left) = heap.try_string(left as u32).cloned() else {
            return 0;
        };
        let Some(right) = heap.try_string(right as u32).cloned() else {
            return 0;
        };
        let mut value = String::with_capacity(left.len() + right.len());
        value.push_str(left.as_ref());
        value.push_str(right.as_ref());
        heap.intern_string(std::rc::Rc::from(value)).into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_substring(handle: i64, start: i64, end: i64) -> i64 {
    if start < 0 || end < 0 {
        return 0;
    }
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else {
            return 0;
        };
        let units: Vec<u16> = zun_core::wtf16::encode(&value).collect();
        let result = zun_core::wtf16::from_units(zun_core::string_ops::substring_units(
            &units,
            start as usize,
            end as usize,
        ));
        heap.intern_string(std::rc::Rc::from(result)).into()
    })
}

/// Trim Unicode whitespace from an interned string handle and return a new
/// managed string handle.
#[inline]
pub extern "C" fn zun_jit_string_trim(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else {
            return 0;
        };
        heap.intern_string(std::rc::Rc::from(zun_core::wtf16::js_trim(&value)))
            .into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_trim_start(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else {
            return 0;
        };
        heap.intern_string(std::rc::Rc::from(zun_core::wtf16::js_trim_start(&value)))
            .into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_trim_end(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else {
            return 0;
        };
        heap.intern_string(std::rc::Rc::from(zun_core::wtf16::js_trim_end(&value)))
            .into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_to_lower_case(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else {
            return 0;
        };
        heap.intern_string(std::rc::Rc::from(value.to_lowercase()))
            .into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_to_upper_case(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else {
            return 0;
        };
        heap.intern_string(std::rc::Rc::from(value.to_uppercase()))
            .into()
    })
}

// `repeat` has no thunk: Tier 0 throws a RangeError when the result would
// exceed `zun_core::string_ops::REPEAT_MAX`, and a thunk cannot raise it, so
// eligibility rejects `repeat` and such methods stay on the interpreter.

#[inline]
pub extern "C" fn zun_jit_string_reverse(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else {
            return 0;
        };
        let mut units: Vec<u16> = zun_core::wtf16::encode(&value).collect();
        units.reverse();
        heap.intern_string(std::rc::Rc::from(zun_core::wtf16::from_units(&units)))
            .into()
    })
}

fn jit_pad_string(handle: i64, target: i64, fill: i64, at_start: bool) -> i64 {
    if target < 0 {
        return 0;
    }
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else {
            return 0;
        };
        let Some(fill) = heap.try_string(fill as u32).cloned() else {
            return 0;
        };
        // An empty fill leaves the receiver unchanged (same as Tier 0).
        if fill.is_empty() {
            return handle;
        }
        let units: Vec<u16> = zun_core::wtf16::encode(&value).collect();
        let target = target as usize;
        if target <= units.len() {
            return handle;
        }
        let fill_units: Vec<u16> = zun_core::wtf16::encode(&fill).collect();
        let result = zun_core::wtf16::from_units(&zun_core::string_ops::pad_units(
            &units,
            target,
            &fill_units,
            at_start,
        ));
        heap.intern_string(std::rc::Rc::from(result)).into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_pad_start(handle: i64, target: i64, fill: i64) -> i64 {
    jit_pad_string(handle, target, fill, true)
}

#[inline]
pub extern "C" fn zun_jit_string_pad_end(handle: i64, target: i64, fill: i64) -> i64 {
    jit_pad_string(handle, target, fill, false)
}

#[inline]
pub extern "C" fn zun_jit_string_replace(handle: i64, needle: i64, replacement: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else {
            return 0;
        };
        let Some(needle) = heap.try_string(needle as u32).cloned() else {
            return 0;
        };
        let Some(replacement) = heap.try_string(replacement as u32).cloned() else {
            return 0;
        };
        heap.intern_string(std::rc::Rc::from(zun_core::string_ops::replace_first(
            &value,
            &needle,
            &replacement,
        )))
        .into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_to_int(handle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        heap.try_string(handle as u32)
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(0) as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_string_to_float(handle: i64) -> f64 {
    zun_gc::with_current_heap(|heap| {
        heap.try_string(handle as u32)
            .and_then(|value| value.trim().parse::<f64>().ok())
            .unwrap_or(f64::NAN)
    })
}

#[inline]
pub extern "C" fn zun_jit_string_to_boolean(handle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        matches!(
            heap.try_string(handle as u32).map(|value| value.trim()),
            Some("true")
        ) as i32
    })
}
