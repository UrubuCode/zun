//! Tier 1: a Cranelift JIT for the numeric ZIL subset.
//!
//! [`Jit`] compiles every eligible method (see [`eligible`]) to native code and
//! calls it directly. Each method becomes a Cranelift function whose parameters
//! and result are all `f64`; booleans travel as `0.0`/`1.0`. Control flow is
//! rebuilt into a Cranelift CFG: because the front-end leaves the evaluation
//! stack empty at every branch and branch target, basic blocks need no stack
//! parameters, and locals become Cranelift [`Variable`]s.
//!
//! This is the second implementation of ZIL execution; the interpreter is the
//! first. The equivalence tests run numeric programs through both and require
//! identical results — the contract the plan sets for Tier 0 vs Tier 1.

mod eligible;
mod any_abi;
mod shim;
mod translate;

pub use eligible::eligible_methods;
pub use any_abi::JitAnyAbi;
pub use zun_il::{JitKind, NumKind};

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

/// Read the UTF-16 length of an interned string handle without allocation.
#[inline]
pub extern "C" fn zun_jit_string_length(handle: i64) -> i64 {
    zun_gc::with_current_heap(|heap| {
        heap.try_string(handle as u32)
            .map(|value| value.encode_utf16().count() as i64)
            .unwrap_or(-1)
    })
}

/// Compare two interned UTF-16 strings without allocating.
#[inline]
pub extern "C" fn zun_jit_string_contains(value: i64, needle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else { return 0 };
        let Some(needle) = heap.try_string(needle as u32) else { return 0 };
        i32::from(value.contains(needle.as_ref()))
    })
}

/// Read one UTF-16 code unit from an interned string without allocation.
#[inline]
pub extern "C" fn zun_jit_string_char_code_at(handle: i64, index: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        heap.try_string(handle as u32)
            .and_then(|value| value.encode_utf16().nth(index as usize))
            .map(|code| code as i32)
            .unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_string_starts_with(value: i64, prefix: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else { return 0 };
        let Some(prefix) = heap.try_string(prefix as u32) else { return 0 };
        i32::from(value.starts_with(prefix.as_ref()))
    })
}

#[inline]
pub extern "C" fn zun_jit_string_ends_with(value: i64, suffix: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else { return 0 };
        let Some(suffix) = heap.try_string(suffix as u32) else { return 0 };
        i32::from(value.ends_with(suffix.as_ref()))
    })
}

#[inline]
pub extern "C" fn zun_jit_string_index_of(value: i64, needle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else { return -1 };
        let Some(needle) = heap.try_string(needle as u32) else { return -1 };
        let value_units: Vec<u16> = value.encode_utf16().collect();
        let needle_units: Vec<u16> = needle.encode_utf16().collect();
        if needle_units.is_empty() {
            0
        } else {
            value_units
                .windows(needle_units.len())
                .position(|window| window == needle_units.as_slice())
                .map(|index| index as i32)
                .unwrap_or(-1)
        }
    })
}

#[inline]
pub extern "C" fn zun_jit_string_last_index_of(value: i64, needle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else { return -1 };
        let Some(needle) = heap.try_string(needle as u32) else { return -1 };
        let value_units: Vec<u16> = value.encode_utf16().collect();
        let needle_units: Vec<u16> = needle.encode_utf16().collect();
        if needle_units.is_empty() {
            value_units.len() as i32
        } else {
            value_units
                .windows(needle_units.len())
                .rposition(|window| window == needle_units.as_slice())
                .map(|index| index as i32)
                .unwrap_or(-1)
        }
    })
}

#[inline]
pub extern "C" fn zun_jit_string_count(value: i64, needle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(value) = heap.try_string(value as u32) else { return 0 };
        let Some(needle) = heap.try_string(needle as u32) else { return 0 };
        let value_units: Vec<u16> = value.encode_utf16().collect();
        let needle_units: Vec<u16> = needle.encode_utf16().collect();
        if needle_units.is_empty() {
            return 0;
        }
        let mut total = 0i32;
        let mut start = 0usize;
        while start + needle_units.len() <= value_units.len() {
            if value_units[start..start + needle_units.len()] == needle_units {
                total += 1;
                start += needle_units.len();
            } else {
                start += 1;
            }
        }
        total
    })
}

#[inline]
pub extern "C" fn zun_jit_string_compare(left: i64, right: i64) -> i32 {
    zun_gc::with_current_heap(|heap| {
        let Some(left) = heap.try_string(left as u32) else { return 0 };
        let Some(right) = heap.try_string(right as u32) else { return 0 };
        if left < right { -1 } else if left > right { 1 } else { 0 }
    })
}

#[inline]
pub extern "C" fn zun_jit_string_concat(left: i64, right: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(left) = heap.try_string(left as u32).cloned() else { return 0 };
        let Some(right) = heap.try_string(right as u32).cloned() else { return 0 };
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
        let Some(value) = heap.try_string(handle as u32).cloned() else { return 0 };
        let units: Vec<u16> = value.encode_utf16().collect();
        let start = (start as usize).min(units.len());
        let end = (end as usize).min(units.len());
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let result = String::from_utf16_lossy(&units[start..end]);
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
        heap.intern_string(std::rc::Rc::from(value.trim())).into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_trim_start(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else { return 0 };
        heap.intern_string(std::rc::Rc::from(value.trim_start())).into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_trim_end(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else { return 0 };
        heap.intern_string(std::rc::Rc::from(value.trim_end())).into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_to_lower_case(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else { return 0 };
        heap.intern_string(std::rc::Rc::from(value.to_lowercase())).into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_to_upper_case(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else { return 0 };
        heap.intern_string(std::rc::Rc::from(value.to_uppercase())).into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_repeat(handle: i64, count: i64) -> i64 {
    if count < 0 { return 0; }
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else { return 0 };
        heap.intern_string(std::rc::Rc::from(value.repeat(count as usize))).into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_reverse(handle: i64) -> i64 {
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else { return 0 };
        let mut units: Vec<u16> = value.encode_utf16().collect();
        units.reverse();
        heap.intern_string(std::rc::Rc::from(String::from_utf16_lossy(&units))).into()
    })
}

fn jit_pad_string(handle: i64, target: i64, fill: i64, at_start: bool) -> i64 {
    if target < 0 { return 0; }
    zun_gc::with_current_heap_mut(|heap| {
        let Some(value) = heap.try_string(handle as u32).cloned() else { return 0 };
        let Some(fill) = heap.try_string(fill as u32).cloned() else { return 0 };
        if fill.is_empty() { return 0; }
        let value_len = value.encode_utf16().count();
        let target = target as usize;
        if target <= value_len { return handle; }
        let padding_units = target - value_len;
        let mut padding = String::new();
        while padding.encode_utf16().count() < padding_units { padding.push_str(&fill); }
        let padding = String::from_utf16_lossy(
            &padding.encode_utf16().take(padding_units).collect::<Vec<_>>(),
        );
        let result = if at_start { format!("{padding}{value}") } else { format!("{value}{padding}") };
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
        let Some(value) = heap.try_string(handle as u32).cloned() else { return 0 };
        let Some(needle) = heap.try_string(needle as u32).cloned() else { return 0 };
        let Some(replacement) = heap.try_string(replacement as u32).cloned() else { return 0 };
        heap.intern_string(std::rc::Rc::from(value.replacen(needle.as_ref(), replacement.as_ref(), 1))).into()
    })
}

#[inline]
pub extern "C" fn zun_jit_string_to_int(handle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| heap.try_string(handle as u32).and_then(|value| value.trim().parse::<i64>().ok()).unwrap_or(0) as i32)
}

#[inline]
pub extern "C" fn zun_jit_string_to_float(handle: i64) -> f64 {
    zun_gc::with_current_heap(|heap| heap.try_string(handle as u32).and_then(|value| value.trim().parse::<f64>().ok()).unwrap_or(f64::NAN))
}

#[inline]
pub extern "C" fn zun_jit_string_to_boolean(handle: i64) -> i32 {
    zun_gc::with_current_heap(|heap| matches!(heap.try_string(handle as u32).map(|value| value.trim()), Some("true")) as i32)
}

/// Read a managed ArrayBuffer length without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_buffer_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.buffer_len(object).map(|len| len as i64).unwrap_or(-1))
}

#[inline]
pub extern "C" fn zun_jit_data_view_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.data_view_len(object).map(|len| len as i64).unwrap_or(-1))
}

#[inline]
pub extern "C" fn zun_jit_typed_array_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.typed_array_len(object).map(|len| len as i64).unwrap_or(-1))
}

#[inline]
pub extern "C" fn zun_jit_data_view_byte_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.data_view_byte_len(object).map(|len| len as i64).unwrap_or(-1))
}

#[inline]
pub extern "C" fn zun_jit_typed_array_byte_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.typed_array_byte_len(object).map(|len| len as i64).unwrap_or(-1))
}

/// Read one byte from a managed ArrayBuffer without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_buffer_get_u8(address: i64, index: i64) -> i8 {
    if index < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.buffer_get(object, index as usize)
            .map(|value| value as i8)
            .unwrap_or(-1)
    })
}

/// Read a little-endian u32 from a managed ArrayBuffer without allocation.
#[inline]
pub extern "C" fn zun_jit_buffer_read_u32(address: i64, offset: i64) -> i32 {
    if offset < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.buffer_read_u32(object, offset as usize)
            .map(|value| value as i32)
            .unwrap_or(-1)
    })
}

/// Read an f64 from a managed ArrayBuffer without allocation.
#[inline]
pub extern "C" fn zun_jit_buffer_read_f64(address: i64, offset: i64) -> f64 {
    if offset < 0 {
        return f64::NAN;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.buffer_read_f64(object, offset as usize).unwrap_or(f64::NAN)
    })
}

/// Store one byte in a managed ArrayBuffer without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_buffer_set_u8(address: i64, index: i64, value: i8) -> i32 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.buffer_set(object, index as usize, value as u8).unwrap_or(false) as i32
    })
}

/// Store a little-endian u32 in a managed ArrayBuffer without allocation.
#[inline]
pub extern "C" fn zun_jit_buffer_write_u32(address: i64, offset: i64, value: i32) -> i32 {
    if offset < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.buffer_write_u32(object, offset as usize, value as u32).unwrap_or(false) as i32
    })
}

/// Store an f64 in a managed ArrayBuffer without allocation.
#[inline]
pub extern "C" fn zun_jit_buffer_write_f64(address: i64, offset: i64, value: f64) -> i32 {
    if offset < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.buffer_write_f64(object, offset as usize, value).unwrap_or(false) as i32
    })
}

/// Read a u32 from a managed DataView without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_data_view_get_u32(
    address: i64,
    offset: i64,
    little_endian: i32,
) -> i32 {
    if offset < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.data_view_get_u32(object, offset as usize, little_endian != 0)
            .map(|value| value as i32)
            .unwrap_or(-1)
    })
}

/// Read a u16 from a managed DataView without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_data_view_get_u16(
    address: i64,
    offset: i64,
    little_endian: i32,
) -> i16 {
    if offset < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.data_view_get_u16(object, offset as usize, little_endian != 0)
            .map(|value| value as i16)
            .unwrap_or(-1)
    })
}

/// Read an f64 from a managed DataView without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_data_view_get_f64(
    address: i64,
    offset: i64,
    little_endian: i32,
) -> f64 {
    if offset < 0 {
        return f64::NAN;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.data_view_get_f64(object, offset as usize, little_endian != 0)
            .unwrap_or(f64::NAN)
    })
}

/// Store a u32 in a managed DataView without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_data_view_set_u32(
    address: i64,
    offset: i64,
    value: i32,
    little_endian: i32,
) -> i32 {
    if offset < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.data_view_set_u32(object, offset as usize, value as u32, little_endian != 0)
            .unwrap_or(false) as i32
    })
}

/// Store a u16 in a managed DataView without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_data_view_set_u16(
    address: i64,
    offset: i64,
    value: i16,
    little_endian: i32,
) -> i32 {
    if offset < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.data_view_set_u16(object, offset as usize, value as u16, little_endian != 0)
            .unwrap_or(false) as i32
    })
}

/// Store an f64 in a managed DataView without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_data_view_set_f64(
    address: i64,
    offset: i64,
    value: f64,
    little_endian: i32,
) -> i32 {
    if offset < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.data_view_set_f64(object, offset as usize, value, little_endian != 0)
            .unwrap_or(false) as i32
    })
}

/// Read one element from a managed Uint8Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_u8_array_get(address: i64, index: i64) -> i8 {
    if index < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.u8_array_get(object, index as usize)
            .map(|value| value as i8)
            .unwrap_or(-1)
    })
}

/// Write one element to a managed Uint8Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_u8_array_set(address: i64, index: i64, value: i8) -> i32 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.u8_array_set(object, index as usize, value as u8)
            .unwrap_or(false) as i32
    })
}

/// Read one element from a managed Uint16Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_u16_array_get(address: i64, index: i64) -> i16 {
    if index < 0 { return -1; }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.u16_array_get(object, index as usize).map(|value| value as i16).unwrap_or(-1)
    })
}

/// Write one element to a managed Uint16Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_u16_array_set(address: i64, index: i64, value: i16) -> i32 {
    if index < 0 { return 0; }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.u16_array_set(object, index as usize, value as u16).unwrap_or(false) as i32
    })
}

/// Read one element from a managed Uint32Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_u32_array_get(address: i64, index: i64) -> i32 {
    if index < 0 { return -1; }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.u32_array_get(object, index as usize).map(|value| value as i32).unwrap_or(-1)
    })
}

/// Write one element to a managed Uint32Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_u32_array_set(address: i64, index: i64, value: i32) -> i32 {
    if index < 0 { return 0; }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.u32_array_set(object, index as usize, value as u32).unwrap_or(false) as i32
    })
}

/// Read one element from a managed Float64Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_f64_array_get(address: i64, index: i64) -> f64 {
    if index < 0 { return f64::NAN; }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.f64_array_get(object, index as usize).unwrap_or(f64::NAN))
}

/// Write one element to a managed Float64Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_f64_array_set(address: i64, index: i64, value: f64) -> i32 {
    if index < 0 { return 0; }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.f64_array_set(object, index as usize, value).unwrap_or(false) as i32
    })
}

/// Read a managed Map length without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_map_size(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.map_len(object).map(|len| len as i64).unwrap_or(-1))
}

/// Read a managed Set length without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_set_size(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.set_len(object).map(|len| len as i64).unwrap_or(-1))
}

/// Test a numeric key in a managed Map without materializing an `Any` carrier.
#[inline]
pub extern "C" fn zun_jit_map_has_f64(address: i64, key: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.map_has(object, zun_core::Any::float(key)).unwrap_or(false) as i32
    })
}

/// Test a numeric value in a managed Set without materializing an `Any` carrier.
#[inline]
pub extern "C" fn zun_jit_set_has_f64(address: i64, value: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.set_has(object, zun_core::Any::float(value)).unwrap_or(false) as i32
    })
}

/// Insert a numeric value into a managed Set without materializing an `Any` carrier.
#[inline]
pub extern "C" fn zun_jit_set_add_f64(address: i64, value: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.set_add(object, zun_core::Any::float(value)).unwrap_or(false) as i32
    })
}

/// Delete a numeric key from a managed Map without materializing an `Any` carrier.
#[inline]
pub extern "C" fn zun_jit_map_delete_f64(address: i64, key: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.map_remove(object, zun_core::Any::float(key)).is_some() as i32
    })
}

/// Delete a numeric value from a managed Set without materializing an `Any` carrier.
#[inline]
pub extern "C" fn zun_jit_set_delete_f64(address: i64, value: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.set_remove(object, zun_core::Any::float(value)).unwrap_or(false) as i32
    })
}

/// Insert a numeric key/value pair into a managed Map without an `Any` carrier.
/// Returns one when the key was new and zero when an existing value was replaced.
#[inline]
pub extern "C" fn zun_jit_map_set_f64(address: i64, key: f64, value: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.map_insert(object, zun_core::Any::float(key), zun_core::Any::float(value))
            .is_none() as i32
    })
}

/// Read a numeric Map value, returning the caller-supplied fallback when the
/// key is absent or stores a non-floating value.
#[inline]
pub extern "C" fn zun_jit_map_get_f64_or(address: i64, key: f64, fallback: f64) -> f64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.map_get(object, zun_core::Any::float(key))
            .and_then(|value| value.as_float())
            .unwrap_or(fallback)
    })
}

#[inline]
pub extern "C" fn zun_jit_map_has_i64(address: i64, key: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.map_has(object, zun_core::Any::int(key)).unwrap_or(false) as i32)
}

#[inline]
pub extern "C" fn zun_jit_set_has_i64(address: i64, value: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.set_has(object, zun_core::Any::int(value)).unwrap_or(false) as i32)
}

#[inline]
pub extern "C" fn zun_jit_set_add_i64(address: i64, value: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| heap.set_add(object, zun_core::Any::int(value)).unwrap_or(false) as i32)
}

#[inline]
pub extern "C" fn zun_jit_map_delete_i64(address: i64, key: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| heap.map_remove(object, zun_core::Any::int(key)).is_some() as i32)
}

#[inline]
pub extern "C" fn zun_jit_set_delete_i64(address: i64, value: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| heap.set_remove(object, zun_core::Any::int(value)).unwrap_or(false) as i32)
}

#[inline]
pub extern "C" fn zun_jit_map_set_i64(address: i64, key: i64, value: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| heap.map_insert(object, zun_core::Any::int(key), zun_core::Any::int(value)).is_none() as i32)
}

#[inline]
pub extern "C" fn zun_jit_map_get_i64_or(address: i64, key: i64, fallback: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.map_get(object, zun_core::Any::int(key)).and_then(|value| value.as_int()).unwrap_or(fallback))
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

#[inline]
pub extern "C" fn zun_jit_array_push_u64(address: i64, value: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.array_push(object, zun_core::Any::int(value))
            .map(|length| length as i64)
            .unwrap_or(-1)
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

use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::{types, AbiParam};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::FunctionBuilderContext;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module as ClifModule};
use cranelift_object::{ObjectBuilder, ObjectModule};
use zun_module::{read_body, Module};

use translate::clif_ty;

/// Errors from building or invoking JIT code.
#[derive(Debug)]
pub enum JitError {
    /// The host architecture is not supported by Cranelift.
    UnsupportedHost(String),
    /// A Cranelift module operation failed.
    Module(String),
    /// Compilation of a specific method failed.
    Compile {
        /// The method RID.
        method_rid: u32,
        /// The failure detail.
        detail: String,
    },
    /// The requested method is not JIT-eligible.
    Ineligible(u32),
    /// The method's arity does not match the arguments provided.
    Arity {
        /// Parameters the method declares.
        expected: usize,
        /// Arguments supplied.
        found: usize,
    },
    /// An `Any` carrier contains an unknown runtime tag.
    InvalidAnyTag(u64),
}

impl core::fmt::Display for JitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            JitError::UnsupportedHost(e) => write!(f, "unsupported host: {e}"),
            JitError::Module(e) => write!(f, "module error: {e}"),
            JitError::Compile { method_rid, detail } => {
                write!(f, "compiling method #{method_rid}: {detail}")
            }
            JitError::Ineligible(rid) => write!(f, "method #{rid} is not JIT-eligible"),
            JitError::Arity { expected, found } => {
                write!(f, "arity mismatch: expected {expected}, found {found}")
            }
            JitError::InvalidAnyTag(tag) => write!(f, "invalid Any tag: {tag}"),
        }
    }
}

impl std::error::Error for JitError {}

/// A JIT that compiles a module's eligible numeric methods to native code.
/// The result of calling a compiled method, carrying its numeric kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RetVal {
    /// A float result (f32 widened to f64).
    Float(f64),
    /// An integer result (any width, in an i64; unsigned zero-extended).
    Int(i64),
    /// A boolean result.
    Bool(bool),
    /// No value (void).
    Void,
}

/// A machine-code safepoint carrying a user stack map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackMapInfo {
    /// Offset of the safepoint from the method's native entry point.
    pub code_offset: u32,
    /// Number of bytes covered by the map in the native frame.
    pub span: u32,
    /// Frame offsets, relative to the native stack pointer, containing managed
    /// references at this safepoint.
    pub reference_slots: Vec<u32>,
    /// Absolute address of the safepoint after JIT finalization.
    pub pc: usize,
}

/// A compiled method's call shape.
#[derive(Clone)]
struct Sig {
    param_kinds: Vec<JitKind>,
    returns: Option<JitKind>,
}

pub struct Jit {
    module: JITModule,
    func_ids: HashMap<u32, FuncId>,
    shim_ids: HashMap<u32, FuncId>,
    eligible: HashSet<u32>,
    /// Eligible callees of each eligible method (for the compile closure).
    callees: HashMap<u32, Vec<u32>>,
    /// Methods whose bodies are defined and finalized.
    defined: HashSet<u32>,
    sigs: HashMap<u32, Sig>,
    ctx: cranelift_codegen::Context,
    fb_ctx: FunctionBuilderContext,
    types: zun_types::TypeSystem,
    virtual_tables: Vec<Box<[u64]>>,
    virtual_table_ptrs: Box<[u64]>,
    stack_maps: HashMap<u32, Vec<StackMapInfo>>,
    stack_map_registry: zun_gc::StackMapRegistry,
}

impl core::fmt::Debug for Jit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Jit")
            .field("compiled", &self.func_ids.len())
            .finish()
    }
}

impl Jit {
    fn register_heap_layouts(&self, heap: &mut zun_gc::Heap) {
        for type_rid in 1.. {
            if self.types.get(type_rid).is_none() {
                break;
            }
            heap.register_reference_slots(type_rid, self.types.reference_slots(type_rid));
        }
    }

    /// Create a JIT for `zm`: determine eligible methods and declare their
    /// Cranelift functions, without compiling any bodies yet. Bodies are
    /// defined on demand by [`Jit::ensure_compiled`] (lazy tiering) or all at
    /// once by [`Jit::compile`].
    pub fn new(zm: &Module) -> Result<Self, JitError> {
        let mut flags = settings::builder();
        // Optimize for speed; the e-graph optimizer is on by default in this
        // Cranelift version, so there is no flag to set for it.
        flags.set("opt_level", "speed").unwrap();
        // Keep a stable frame chain so a future GC safepoint can capture the
        // JIT frame pointer and return address alongside its stack map.
        flags.set("preserve_frame_pointers", "true").unwrap();
        let isa = cranelift_native::builder()
            .map_err(|e| JitError::UnsupportedHost(e.to_string()))?
            .finish(settings::Flags::new(flags))
            .map_err(|e| JitError::UnsupportedHost(e.to_string()))?;

        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        // Make the runtime allocator callable from native `newobj`.
        builder.symbol("zun_rt_alloc", zun_gc::zun_rt_alloc as *const u8);
        builder.symbol("zun_jit_array_length", zun_jit_array_length as *const u8);
        builder.symbol("zun_jit_string_length", zun_jit_string_length as *const u8);
        builder.symbol("zun_jit_string_contains", zun_jit_string_contains as *const u8);
        builder.symbol("zun_jit_string_char_code_at", zun_jit_string_char_code_at as *const u8);
        builder.symbol("zun_jit_string_starts_with", zun_jit_string_starts_with as *const u8);
        builder.symbol("zun_jit_string_ends_with", zun_jit_string_ends_with as *const u8);
        builder.symbol("zun_jit_string_index_of", zun_jit_string_index_of as *const u8);
        builder.symbol("zun_jit_string_last_index_of", zun_jit_string_last_index_of as *const u8);
        builder.symbol("zun_jit_string_count", zun_jit_string_count as *const u8);
        builder.symbol("zun_jit_string_compare", zun_jit_string_compare as *const u8);
        builder.symbol("zun_jit_string_concat", zun_jit_string_concat as *const u8);
        builder.symbol("zun_jit_string_substring", zun_jit_string_substring as *const u8);
        builder.symbol("zun_jit_string_trim", zun_jit_string_trim as *const u8);
        builder.symbol("zun_jit_string_trim_start", zun_jit_string_trim_start as *const u8);
        builder.symbol("zun_jit_string_trim_end", zun_jit_string_trim_end as *const u8);
        builder.symbol("zun_jit_string_to_lower_case", zun_jit_string_to_lower_case as *const u8);
        builder.symbol("zun_jit_string_to_upper_case", zun_jit_string_to_upper_case as *const u8);
        builder.symbol("zun_jit_string_repeat", zun_jit_string_repeat as *const u8);
        builder.symbol("zun_jit_string_reverse", zun_jit_string_reverse as *const u8);
        builder.symbol("zun_jit_string_pad_start", zun_jit_string_pad_start as *const u8);
        builder.symbol("zun_jit_string_pad_end", zun_jit_string_pad_end as *const u8);
        builder.symbol("zun_jit_string_replace", zun_jit_string_replace as *const u8);
        builder.symbol("zun_jit_string_to_int", zun_jit_string_to_int as *const u8);
        builder.symbol("zun_jit_string_to_float", zun_jit_string_to_float as *const u8);
        builder.symbol("zun_jit_string_to_boolean", zun_jit_string_to_boolean as *const u8);
    builder.symbol("zun_jit_buffer_length", zun_jit_buffer_length as *const u8);
    builder.symbol("zun_jit_data_view_length", zun_jit_data_view_length as *const u8);
    builder.symbol("zun_jit_typed_array_length", zun_jit_typed_array_length as *const u8);
    builder.symbol("zun_jit_data_view_byte_length", zun_jit_data_view_byte_length as *const u8);
    builder.symbol("zun_jit_typed_array_byte_length", zun_jit_typed_array_byte_length as *const u8);
    builder.symbol("zun_jit_buffer_get_u8", zun_jit_buffer_get_u8 as *const u8);
    builder.symbol("zun_jit_buffer_read_u32", zun_jit_buffer_read_u32 as *const u8);
    builder.symbol("zun_jit_buffer_read_f64", zun_jit_buffer_read_f64 as *const u8);
    builder.symbol("zun_jit_buffer_set_u8", zun_jit_buffer_set_u8 as *const u8);
    builder.symbol("zun_jit_buffer_write_u32", zun_jit_buffer_write_u32 as *const u8);
    builder.symbol("zun_jit_buffer_write_f64", zun_jit_buffer_write_f64 as *const u8);
    builder.symbol("zun_jit_data_view_get_u32", zun_jit_data_view_get_u32 as *const u8);
    builder.symbol("zun_jit_data_view_get_u16", zun_jit_data_view_get_u16 as *const u8);
    builder.symbol("zun_jit_data_view_get_f64", zun_jit_data_view_get_f64 as *const u8);
    builder.symbol("zun_jit_data_view_set_u32", zun_jit_data_view_set_u32 as *const u8);
    builder.symbol("zun_jit_data_view_set_u16", zun_jit_data_view_set_u16 as *const u8);
    builder.symbol("zun_jit_data_view_set_f64", zun_jit_data_view_set_f64 as *const u8);
    builder.symbol("zun_jit_u8_array_get", zun_jit_u8_array_get as *const u8);
    builder.symbol("zun_jit_u8_array_set", zun_jit_u8_array_set as *const u8);
    builder.symbol("zun_jit_u16_array_get", zun_jit_u16_array_get as *const u8);
    builder.symbol("zun_jit_u16_array_set", zun_jit_u16_array_set as *const u8);
    builder.symbol("zun_jit_u32_array_get", zun_jit_u32_array_get as *const u8);
    builder.symbol("zun_jit_u32_array_set", zun_jit_u32_array_set as *const u8);
    builder.symbol("zun_jit_f64_array_get", zun_jit_f64_array_get as *const u8);
    builder.symbol("zun_jit_f64_array_set", zun_jit_f64_array_set as *const u8);
    builder.symbol("zun_jit_map_size", zun_jit_map_size as *const u8);
    builder.symbol("zun_jit_set_size", zun_jit_set_size as *const u8);
    builder.symbol("zun_jit_map_has_f64", zun_jit_map_has_f64 as *const u8);
    builder.symbol("zun_jit_set_has_f64", zun_jit_set_has_f64 as *const u8);
    builder.symbol("zun_jit_set_add_f64", zun_jit_set_add_f64 as *const u8);
    builder.symbol("zun_jit_map_delete_f64", zun_jit_map_delete_f64 as *const u8);
    builder.symbol("zun_jit_set_delete_f64", zun_jit_set_delete_f64 as *const u8);
    builder.symbol("zun_jit_map_set_f64", zun_jit_map_set_f64 as *const u8);
    builder.symbol("zun_jit_map_get_f64_or", zun_jit_map_get_f64_or as *const u8);
    builder.symbol("zun_jit_map_has_i64", zun_jit_map_has_i64 as *const u8);
    builder.symbol("zun_jit_set_has_i64", zun_jit_set_has_i64 as *const u8);
    builder.symbol("zun_jit_set_add_i64", zun_jit_set_add_i64 as *const u8);
    builder.symbol("zun_jit_map_delete_i64", zun_jit_map_delete_i64 as *const u8);
    builder.symbol("zun_jit_set_delete_i64", zun_jit_set_delete_i64 as *const u8);
    builder.symbol("zun_jit_map_set_i64", zun_jit_map_set_i64 as *const u8);
    builder.symbol("zun_jit_map_get_i64_or", zun_jit_map_get_i64_or as *const u8);
    builder.symbol("zun_jit_array_new", zun_jit_array_new as *const u8);
    builder.symbol("zun_jit_osr_poll", zun_jit_osr_poll as *const u8);
    builder.symbol("zun_jit_array_load_f64", zun_jit_array_load_f64 as *const u8);
    builder.symbol("zun_jit_array_load_u64", zun_jit_array_load_u64 as *const u8);
    builder.symbol("zun_jit_array_load_bool", zun_jit_array_load_bool as *const u8);
    builder.symbol("zun_jit_array_load_object", zun_jit_array_load_object as *const u8);
    builder.symbol("zun_jit_array_store_f64", zun_jit_array_store_f64 as *const u8);
    builder.symbol("zun_jit_array_store_u64", zun_jit_array_store_u64 as *const u8);
    builder.symbol("zun_jit_array_store_bool", zun_jit_array_store_bool as *const u8);
    builder.symbol("zun_jit_array_store_object", zun_jit_array_store_object as *const u8);
        builder.symbol("zun_jit_store_ref", zun_jit_store_ref as *const u8);
    builder.symbol("zun_jit_array_pop_f64", zun_jit_array_pop_f64 as *const u8);
    builder.symbol("zun_jit_array_push_f64", zun_jit_array_push_f64 as *const u8);
    builder.symbol("zun_jit_array_pop_u64", zun_jit_array_pop_u64 as *const u8);
    builder.symbol("zun_jit_array_push_u64", zun_jit_array_push_u64 as *const u8);
    builder.symbol("zun_jit_array_pop_object", zun_jit_array_pop_object as *const u8);
    builder.symbol("zun_jit_array_push_object", zun_jit_array_push_object as *const u8);
        builder.symbol(
            "zun_jit_array_load_any",
            zun_jit_array_load_any as *const u8,
        );
    builder.symbol(
        "zun_jit_array_store_any",
        zun_jit_array_store_any as *const u8,
    );
        builder.symbol("zun_jit_date_year", zun_jit_date_year as *const u8);
        builder.symbol("zun_jit_date_is_valid", zun_jit_date_is_valid as *const u8);
        builder.symbol("zun_jit_date_month", zun_jit_date_month as *const u8);
        builder.symbol("zun_jit_date_day", zun_jit_date_day as *const u8);
        builder.symbol("zun_jit_date_hour", zun_jit_date_hour as *const u8);
        builder.symbol("zun_jit_date_minute", zun_jit_date_minute as *const u8);
        builder.symbol("zun_jit_date_second", zun_jit_date_second as *const u8);
        builder.symbol(
            "zun_jit_date_millisecond",
            zun_jit_date_millisecond as *const u8,
        );
        builder.symbol("zun_jit_date_weekday", zun_jit_date_weekday as *const u8);
        builder.symbol(
            "zun_jit_date_day_of_year",
            zun_jit_date_day_of_year as *const u8,
        );
        builder.symbol("zun_jit_math_abs", zun_jit_math_abs as *const u8);
        builder.symbol("zun_jit_math_sqrt", zun_jit_math_sqrt as *const u8);
        builder.symbol("zun_jit_math_cbrt", zun_jit_math_cbrt as *const u8);
        builder.symbol("zun_jit_math_sign", zun_jit_math_sign as *const u8);
        builder.symbol("zun_jit_math_floor", zun_jit_math_floor as *const u8);
        builder.symbol("zun_jit_math_ceil", zun_jit_math_ceil as *const u8);
        builder.symbol("zun_jit_math_round", zun_jit_math_round as *const u8);
        builder.symbol("zun_jit_math_trunc", zun_jit_math_trunc as *const u8);
        builder.symbol("zun_jit_math_sin", zun_jit_math_sin as *const u8);
        builder.symbol("zun_jit_math_cos", zun_jit_math_cos as *const u8);
        builder.symbol("zun_jit_math_exp", zun_jit_math_exp as *const u8);
        builder.symbol("zun_jit_math_log", zun_jit_math_log as *const u8);
        builder.symbol("zun_jit_math_log10", zun_jit_math_log10 as *const u8);
        builder.symbol("zun_jit_math_log2", zun_jit_math_log2 as *const u8);
        builder.symbol("zun_jit_math_expm1", zun_jit_math_expm1 as *const u8);
        builder.symbol("zun_jit_math_log1p", zun_jit_math_log1p as *const u8);
        builder.symbol("zun_jit_math_sinh", zun_jit_math_sinh as *const u8);
        builder.symbol("zun_jit_math_cosh", zun_jit_math_cosh as *const u8);
        builder.symbol("zun_jit_math_tanh", zun_jit_math_tanh as *const u8);
        builder.symbol("zun_jit_math_asinh", zun_jit_math_asinh as *const u8);
        builder.symbol("zun_jit_math_acosh", zun_jit_math_acosh as *const u8);
        builder.symbol("zun_jit_math_atanh", zun_jit_math_atanh as *const u8);
        builder.symbol("zun_jit_math_tan", zun_jit_math_tan as *const u8);
        builder.symbol("zun_jit_math_min", zun_jit_math_min as *const u8);
        builder.symbol("zun_jit_math_max", zun_jit_math_max as *const u8);
        builder.symbol("zun_jit_math_pow", zun_jit_math_pow as *const u8);
        builder.symbol("zun_jit_math_atan2", zun_jit_math_atan2 as *const u8);
        builder.symbol("zun_jit_math_hypot", zun_jit_math_hypot as *const u8);
        builder.symbol(
            "zun_jit_number_is_finite",
            zun_jit_number_is_finite as *const u8,
        );
        builder.symbol("zun_jit_number_is_nan", zun_jit_number_is_nan as *const u8);
        builder.symbol(
            "zun_jit_number_is_negative_zero",
            zun_jit_number_is_negative_zero as *const u8,
        );
        builder.symbol(
            "zun_jit_number_is_safe_integer",
            zun_jit_number_is_safe_integer as *const u8,
        );
        let module = JITModule::new(builder);

        let eligible = eligible_methods(zm);
        let types =
            zun_types::TypeSystem::from_module(zm).map_err(|e| JitError::Module(e.to_string()))?;
        let virtual_tables: Vec<Box<[u64]>> = (1..=types.len() as u32)
            .map(|type_rid| {
                types
                    .get(type_rid)
                    .map(|ty| vec![0; ty.vtable.len()].into_boxed_slice())
                    .unwrap_or_default()
            })
            .collect();
        let mut virtual_table_ptrs = vec![0u64; types.len() + 1];
        for (index, table) in virtual_tables.iter().enumerate() {
            virtual_table_ptrs[index + 1] = table.as_ptr() as usize as u64;
        }

        // Declare every eligible function and shim up front (signatures only) so
        // calls can reference any of them; bodies are defined later, eagerly by
        // `compile` or lazily by `ensure_compiled`.
        let mut jit = Self {
            module,
            func_ids: HashMap::new(),
            shim_ids: HashMap::new(),
            eligible: eligible.clone(),
            callees: HashMap::new(),
            defined: HashSet::new(),
            sigs: HashMap::new(),
            ctx: cranelift_codegen::Context::new(),
            fb_ctx: FunctionBuilderContext::new(),
            types,
            virtual_tables,
            virtual_table_ptrs: virtual_table_ptrs.into_boxed_slice(),
            stack_maps: HashMap::new(),
            stack_map_registry: zun_gc::StackMapRegistry::new(),
        };

        let mut methods_by_name: HashMap<_, Vec<u32>> = HashMap::new();
        for (index, method) in zm.tables.method_defs.iter().enumerate() {
            methods_by_name
                .entry(method.name)
                .or_default()
                .push(index as u32 + 1);
        }

        for &rid in &eligible {
            let body = read_body(
                &zm.code,
                zm.tables.method_defs[rid as usize - 1].body_offset,
            )
            .map_err(|e| JitError::Compile {
                method_rid: rid,
                detail: e.to_string(),
            })?;
            let sig = body_signature(&jit.module, &body);
            let fid = jit
                .module
                .declare_function(&format!("m{rid}"), Linkage::Local, &sig)
                .map_err(|e| JitError::Module(e.to_string()))?;
            jit.func_ids.insert(rid, fid);

            let param_kinds: Vec<JitKind> = body.local_kinds[..body.param_count as usize]
                .iter()
                .map(|&b| JitKind::from_byte(b))
                .collect();
            jit.sigs.insert(
                rid,
                Sig {
                    param_kinds,
                    returns: body
                        .returns_value
                        .then(|| JitKind::from_byte(body.return_kind)),
                },
            );
            let instrs = zun_il::decode_all(body.code).map_err(|e| JitError::Compile {
                method_rid: rid,
                detail: e.to_string(),
            })?;
            let mut callees = Vec::new();
            for instr in instrs
                .iter()
                .filter(|i| matches!(i.opcode, zun_il::Opcode::Call | zun_il::Opcode::CallVirt))
            {
                let zun_il::Operand::Token(token) = instr.operand else {
                    continue;
                };
                if instr.opcode == zun_il::Opcode::CallVirt {
                    let name = zm.tables.method_defs[token.rid() as usize - 1].name;
                    callees.extend(
                        methods_by_name
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .copied()
                            .filter(|rid| eligible.contains(rid)),
                    );
                } else if eligible.contains(&token.rid()) {
                    callees.push(token.rid());
                }
            }
            jit.callees.insert(rid, callees);
        }
        Ok(jit)
    }

    /// Eagerly compile every eligible method. Used where all compiled code is
    /// wanted up front (e.g. equivalence tests).
    pub fn compile(zm: &Module) -> Result<Self, JitError> {
        let mut jit = Self::new(zm)?;
        let all: Vec<u32> = jit.eligible.iter().copied().collect();
        for rid in all {
            jit.ensure_compiled(zm, rid)?;
        }
        Ok(jit)
    }

    /// Ensure `rid` and every eligible method reachable from it are defined and
    /// finalized, so calling `rid` cannot jump into an undefined function.
    /// Returns whether `rid` is JIT-executable (i.e. eligible).
    pub fn ensure_compiled(&mut self, zm: &Module, rid: u32) -> Result<bool, JitError> {
        if !self.eligible.contains(&rid) {
            return Ok(false);
        }
        if self.defined.contains(&rid) {
            return Ok(true);
        }
        // Collect the transitive eligible-callee closure that still needs
        // defining, then define and finalize it as one batch.
        let mut worklist = vec![rid];
        let mut batch = Vec::new();
        let mut seen = HashSet::new();
        while let Some(m) = worklist.pop() {
            if !self.defined.contains(&m) && seen.insert(m) {
                batch.push(m);
                worklist.extend(self.callees[&m].iter().copied());
            }
        }
        for &m in &batch {
            self.define_method(zm, m)?;
        }
        self.module
            .finalize_definitions()
            .map_err(|e| JitError::Module(e.to_string()))?;
        for m in batch {
            self.defined.insert(m);
        }
        self.refresh_stack_map_pcs();
        self.refresh_virtual_table();
        Ok(true)
    }

    /// Translate and define one method body and its shim (no finalize).
    fn define_method(&mut self, zm: &Module, rid: u32) -> Result<(), JitError> {
        let body = read_body(
            &zm.code,
            zm.tables.method_defs[rid as usize - 1].body_offset,
        )
        .map_err(|e| JitError::Compile {
            method_rid: rid,
            detail: e.to_string(),
        })?;
        self.ctx.func.signature = body_signature(&self.module, &body);
        translate::translate_body(
            &mut self.module,
            zm,
            &mut self.ctx,
            &mut self.fb_ctx,
            &self.func_ids,
            &self.types,
            self.virtual_table_ptrs.as_ptr(),
            &body,
        )
        .map_err(|detail| JitError::Compile {
            method_rid: rid,
            detail,
        })?;
        self.module
            .define_function(self.func_ids[&rid], &mut self.ctx)
            .map_err(|e| JitError::Compile {
                method_rid: rid,
                detail: e.to_string(),
            })?;
        let maps = self
            .ctx
            .compiled_code()
            .map(|code| {
                code.buffer
                    .user_stack_maps()
                    .iter()
                    .map(|&(code_offset, span, ref stack_map)| StackMapInfo {
                        code_offset,
                        span,
                        pc: 0,
                        reference_slots: stack_map
                            .entries()
                            .map(|(_, offset)| offset)
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.stack_maps.insert(rid, maps);
        self.module.clear_context(&mut self.ctx);

        let sig = self.sigs[&rid].clone();
        let shim_id = if sig.returns == Some(JitKind::Other) {
            shim::define_any_return_shim(
                &mut self.module,
                &mut self.ctx,
                &mut self.fb_ctx,
                &format!("m{rid}_any_shim"),
                self.func_ids[&rid],
                &sig.param_kinds,
            )
        } else {
            shim::define_shim(
                &mut self.module,
                &mut self.ctx,
                &mut self.fb_ctx,
                &format!("m{rid}_shim"),
                self.func_ids[&rid],
                &sig.param_kinds,
                sig.returns,
            )
        }
        .map_err(|detail| JitError::Compile {
            method_rid: rid,
            detail,
        })?;
        self.shim_ids.insert(rid, shim_id);
        Ok(())
    }

    fn refresh_virtual_table(&mut self) {
        for type_rid in 1..=self.types.len() as u32 {
            let Some(ty) = self.types.get(type_rid) else {
                continue;
            };
            for (slot, &method_rid) in ty.vtable.iter().enumerate() {
                if method_rid == 0 {
                    continue;
                }
                let Some(&func_id) = self.func_ids.get(&method_rid) else {
                    continue;
                };
                if !self.defined.contains(&method_rid) {
                    continue;
                }
                let ptr = self.module.get_finalized_function(func_id);
                self.virtual_tables[type_rid as usize - 1][slot] = ptr as usize as u64;
            }
        }
    }

    fn refresh_stack_map_pcs(&mut self) {
        for (&rid, maps) in &mut self.stack_maps {
            let Some(&func_id) = self.func_ids.get(&rid) else {
                continue;
            };
            let base = self.module.get_finalized_function(func_id) as usize;
            for map in maps {
                map.pc = base + map.code_offset as usize;
            }
        }
        self.stack_map_registry
            .replace(self.stack_maps.values().flat_map(|maps| {
                maps.iter().map(|map| zun_gc::NativeStackMap {
                    pc: map.pc,
                    span: map.span,
                    reference_slots: map.reference_slots.clone(),
                })
            }));
    }

    /// Whether `method_rid` is eligible for JIT compilation.
    pub fn is_compiled(&self, method_rid: u32) -> bool {
        self.eligible.contains(&method_rid)
    }

    /// Whether `method_rid`'s body has actually been compiled and finalized.
    pub fn is_defined(&self, method_rid: u32) -> bool {
        self.defined.contains(&method_rid)
    }

    /// Return the native safepoint maps emitted for a method, if it is JIT-defined.
    pub fn stack_maps(&self, method_rid: u32) -> Option<&[StackMapInfo]> {
        self.stack_maps.get(&method_rid).map(Vec::as_slice)
    }

    /// Access the GC-facing registry of absolute native safepoint maps.
    pub fn stack_map_registry(&self) -> &zun_gc::StackMapRegistry {
        &self.stack_map_registry
    }

    /// The parameter kinds of a compiled method (for marshalling arguments).
    pub fn param_kinds(&self, method_rid: u32) -> Option<&[JitKind]> {
        self.sigs.get(&method_rid).map(|s| s.param_kinds.as_slice())
    }

    /// The return kind of a compiled method, or `None` for void / unknown.
    pub fn return_kind(&self, method_rid: u32) -> Option<JitKind> {
        self.sigs.get(&method_rid).and_then(|s| s.returns)
    }

    /// Call a compiled method through its shim with arguments already packed
    /// into `u64` slots (ints as their i64 bits, floats as float bits), and
    /// return the packed result. zun_vm marshals `Value`s to and from this.
    ///
    /// # Safety contract
    /// The module was verified before compilation, and `args` must match the
    /// method's parameter count and kinds. Object arguments must be valid
    /// managed pointers for the current GC heap.
    pub fn call_packed(&self, method_rid: u32, args: &[u64]) -> Result<u64, JitError> {
        let signature = self
            .sigs
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?
            .clone();
        if signature.returns == Some(JitKind::Other) {
            return Err(JitError::Ineligible(method_rid));
        }
        if signature.param_kinds.iter().any(|&kind| kind == JitKind::Other) {
            return Err(JitError::Ineligible(method_rid));
        }
        let param_kinds = signature.param_kinds;
        if param_kinds.len() != args.len() {
            return Err(JitError::Arity {
                expected: param_kinds.len(),
                found: args.len(),
            });
        }
        let native_roots: Vec<zun_gc::Obj> = args
            .iter()
            .zip(param_kinds)
            .filter_map(|(&arg, kind)| {
                (kind == JitKind::Object)
                    .then(|| unsafe { zun_gc::Obj::from_addr(arg as *mut u64) })
            })
            .collect();
        // Keep managed arguments visible for the complete native call. The VM
        // hook also registers its Value-level roots; this scope protects direct
        // callers of the public packed entry point as well.
        let _roots = unsafe { zun_gc::NativeRootScope::enter(&native_roots) };
        let _stack_maps = unsafe {
            zun_gc::ActiveStackMapScope::enter(&self.stack_map_registry)
        };
        let id = *self
            .shim_ids
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?;
        let ptr = self.module.get_finalized_function(id);
        let f: extern "C" fn(*const u64) -> u64 = unsafe { std::mem::transmute(ptr) };
        Ok(f(args.as_ptr()))
    }

    /// Call a compiled method using the explicit two-word `Any` return ABI.
    ///
    /// This is intentionally separate from [`Self::call_packed`], whose
    /// scalar return contract is exactly one `u64`. `Any` methods remain
    /// outside the eligibility set until their parameters and internal calls
    /// are lowered to the same carrier ABI.
    pub fn call_packed_any(
        &self,
        method_rid: u32,
        args: &[u64],
    ) -> Result<JitAnyAbi, JitError> {
        let signature = self
            .sigs
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?
            .clone();
        if signature.returns != Some(JitKind::Other) {
            return Err(JitError::Ineligible(method_rid));
        }
        let expected_words: usize = signature
            .param_kinds
            .iter()
            .map(|&kind| any_abi::abi_word_count(kind))
            .sum();
        if expected_words != args.len() {
            return Err(JitError::Arity {
                expected: expected_words,
                found: args.len(),
            });
        }
        let mut native_roots = Vec::new();
        let mut word = 0;
        for kind in signature.param_kinds {
            if kind == JitKind::Object {
                native_roots.push(unsafe { zun_gc::Obj::from_addr(args[word] as *mut u64) });
            } else if kind == JitKind::Other {
                let tag = args[word];
                let payload = args[word + 1];
                let Some(value) = zun_core::Any::from_raw_parts(tag as u8, payload) else {
                    return Err(JitError::InvalidAnyTag(tag));
                };
                if value.tag() == zun_core::AnyTag::Object {
                    native_roots.push(unsafe { zun_gc::Obj::from_addr(payload as *mut u64) });
                }
            }
            word += any_abi::abi_word_count(kind);
        }
        let _roots = unsafe { zun_gc::NativeRootScope::enter(&native_roots) };
        let _stack_maps = unsafe { zun_gc::ActiveStackMapScope::enter(&self.stack_map_registry) };
        let id = *self
            .shim_ids
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?;
        let ptr = self.module.get_finalized_function(id);
        let f: extern "C" fn(*const u64) -> JitAnyAbi = unsafe { std::mem::transmute(ptr) };
        Ok(f(args.as_ptr()))
    }

    /// Call a compiled method while registering `heap` as the current managed
    /// heap for runtime allocation and helper calls.
    ///
    /// This is the explicit host boundary for JIT code that can allocate or
    /// otherwise access managed storage. The scope is restored before return,
    /// so nested hosts keep ownership of their previous heap registration.
    pub fn call_packed_with_heap(
        &self,
        method_rid: u32,
        args: &[u64],
        heap: &mut zun_gc::Heap,
    ) -> Result<u64, JitError> {
        self.register_heap_layouts(heap);
        let _scope = unsafe { zun_gc::HeapScope::enter(heap as *mut _) };
        self.call_packed(method_rid, args)
    }

    /// Call a compiled method while publishing the caller-supplied native
    /// safepoint coordinates to the GC boundary.
    ///
    /// The host must provide a stopped frame stack pointer and a program
    /// counter covered by [`Self::stack_map_registry`]. This is the explicit
    /// coordinator path; automatic machine-context discovery remains a host
    /// responsibility.
    ///
    /// # Safety
    /// `pc` and `sp` must identify a live native frame whose stack memory stays
    /// valid for the complete call and remains stopped whenever the GC scans
    /// it. The packed arguments must satisfy the same contract as
    /// [`Self::call_packed`].
    pub unsafe fn call_packed_with_heap_and_native_frame(
        &self,
        method_rid: u32,
        args: &[u64],
        heap: &mut zun_gc::Heap,
        pc: usize,
        sp: *mut u8,
    ) -> Result<u64, JitError> {
        let _frame = unsafe {
            zun_gc::ActiveNativeFrameScope::enter(self.stack_map_registry(), pc, sp)
        };
        let _scope = unsafe { zun_gc::HeapScope::enter(heap as *mut _) };
        self.call_packed(method_rid, args)
    }

    /// Call a compiled method while publishing caller-supplied native
    /// safepoint coordinates, assuming the caller already registered the
    /// managed heap with [`zun_gc::HeapScope`].
    ///
    /// # Safety
    /// The frame coordinates and packed arguments must satisfy the same
    /// contract as [`Self::call_packed_with_heap_and_native_frame`], and a
    /// current heap must already be installed for any runtime helper calls.
    pub unsafe fn call_packed_with_native_frame(
        &self,
        method_rid: u32,
        args: &[u64],
        pc: usize,
        sp: *mut u8,
    ) -> Result<u64, JitError> {
        let _frame = unsafe {
            zun_gc::ActiveNativeFrameScope::enter(self.stack_map_registry(), pc, sp)
        };
        self.call_packed(method_rid, args)
    }

    /// Call a compiled zero-argument method (an entry point), returning its
    /// result tagged by kind.
    ///
    /// Only nullary methods are callable from Rust for now; methods that take
    /// arguments are still reached through Cranelift-to-Cranelift calls inside
    /// compiled code. The compiled code is trusted because the module was
    /// verified before compilation.
    pub fn call_entry(&self, method_rid: u32) -> Result<RetVal, JitError> {
        let sig = self
            .sigs
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?
            .clone();
        if !sig.param_kinds.is_empty() {
            return Err(JitError::Arity {
                expected: 0,
                found: sig.param_kinds.len(),
            });
        }
        let id = self.func_ids[&method_rid];
        let ptr = self.module.get_finalized_function(id);

        // Publish the finalized stack-map registry for the complete native
        // entry call. This keeps direct JIT entry points on the same precise
        // GC boundary as packed calls when a runtime safepoint is reached.
        let _stack_maps = unsafe {
            zun_gc::ActiveStackMapScope::enter(&self.stack_map_registry)
        };

        // Transmute the entry point to a nullary fn of its concrete return type,
        // then box the native result into a kind-tagged RetVal.
        use zun_il::NumKind::*;
        unsafe {
            Ok(match sig.returns {
                None => {
                    let f: extern "C" fn() = std::mem::transmute(ptr);
                    f();
                    RetVal::Void
                }
                Some(JitKind::Bool) => {
                    let f: extern "C" fn() -> i32 = std::mem::transmute(ptr);
                    RetVal::Bool(f() != 0)
                }
                Some(JitKind::Num(F64)) => {
                    let f: extern "C" fn() -> f64 = std::mem::transmute(ptr);
                    RetVal::Float(f())
                }
                Some(JitKind::Num(F32)) => {
                    let f: extern "C" fn() -> f32 = std::mem::transmute(ptr);
                    RetVal::Float(f() as f64)
                }
                Some(JitKind::Num(k)) => {
                    let (bits, signed) = k.int_parts().unwrap();
                    RetVal::Int(read_int(ptr, bits, signed))
                }
                Some(JitKind::Object) | Some(JitKind::String) => {
                    let f: extern "C" fn() -> i64 = std::mem::transmute(ptr);
                    RetVal::Int(f())
                }
                Some(JitKind::Other) => return Err(JitError::Ineligible(method_rid)),
            })
        }
    }

    /// Call a zero-argument entry point with an explicitly registered managed
    /// heap. This is the entry-point counterpart to [`Self::call_packed_with_heap`].
    pub fn call_entry_with_heap(
        &self,
        method_rid: u32,
        heap: &mut zun_gc::Heap,
    ) -> Result<RetVal, JitError> {
        self.register_heap_layouts(heap);
        let _scope = unsafe { zun_gc::HeapScope::enter(heap as *mut _) };
        self.call_entry(method_rid)
    }

    /// Call a zero-argument entry point while publishing caller-supplied
    /// native safepoint coordinates to the GC boundary.
    ///
    /// # Safety
    /// `pc` and `sp` must identify a live frame covered by this JIT's stack-map
    /// registry, and that frame must remain valid for the complete call.
    pub unsafe fn call_entry_with_heap_and_native_frame(
        &self,
        method_rid: u32,
        heap: &mut zun_gc::Heap,
        pc: usize,
        sp: *mut u8,
    ) -> Result<RetVal, JitError> {
        let _frame = unsafe {
            zun_gc::ActiveNativeFrameScope::enter(self.stack_map_registry(), pc, sp)
        };
        let _scope = unsafe { zun_gc::HeapScope::enter(heap as *mut _) };
        self.call_entry(method_rid)
    }
}

/// Emit the JIT-eligible portion of a ZIL module as a relocatable native
/// object. The `.zm` module remains the fallback for methods that are not in
/// the typed Cranelift subset; linking and runtime entry-point selection are
/// deliberately left to the host build step.
pub fn compile_object(zm: &Module) -> Result<Vec<u8>, JitError> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    flags.set("preserve_frame_pointers", "true").unwrap();
    let isa = cranelift_native::builder()
        .map_err(|e| JitError::UnsupportedHost(e.to_string()))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| JitError::UnsupportedHost(e.to_string()))?;
    let builder = ObjectBuilder::new(isa, "zun-aot", cranelift_module::default_libcall_names())
        .map_err(|e| JitError::Module(e.to_string()))?;
    let mut module = ObjectModule::new(builder);
    let eligible = eligible_methods(zm);
    let types =
        zun_types::TypeSystem::from_module(zm).map_err(|e| JitError::Module(e.to_string()))?;
    let mut func_ids = HashMap::new();
    let mut ctx = cranelift_codegen::Context::new();
    let mut fb_ctx = FunctionBuilderContext::new();

    for &rid in &eligible {
        let body = read_body(
            &zm.code,
            zm.tables.method_defs[rid as usize - 1].body_offset,
        )
        .map_err(|e| JitError::Compile {
            method_rid: rid,
            detail: e.to_string(),
        })?;
        let sig = body_signature(&module, &body);
        let linkage = zm
            .entry_point
            .is_some_and(|token| token.rid() == rid)
            .then_some(Linkage::Export)
            .unwrap_or(Linkage::Local);
        let fid = module
            .declare_function(&format!("m{rid}"), linkage, &sig)
            .map_err(|e| JitError::Module(e.to_string()))?;
        func_ids.insert(rid, fid);
    }

    for &rid in &eligible {
        let body = read_body(
            &zm.code,
            zm.tables.method_defs[rid as usize - 1].body_offset,
        )
        .map_err(|e| JitError::Compile {
            method_rid: rid,
            detail: e.to_string(),
        })?;
        ctx.func.signature = body_signature(&module, &body);
        translate::translate_body(
            &mut module,
            zm,
            &mut ctx,
            &mut fb_ctx,
            &func_ids,
            &types,
            core::ptr::null(),
            &body,
        )
        .map_err(|detail| JitError::Compile {
            method_rid: rid,
            detail,
        })?;
        module
            .define_function(func_ids[&rid], &mut ctx)
            .map_err(|e| JitError::Compile {
                method_rid: rid,
                detail: e.to_string(),
            })?;
    }
    module
        .finish()
        .emit()
        .map_err(|e| JitError::Module(e.to_string()))
}

/// Read a nullary integer-returning entry point, sign/zero-extending to i64.
///
/// # Safety
/// `ptr` must be a finalized nullary function returning an integer of the given
/// width and signedness.
unsafe fn read_int(ptr: *const u8, bits: u8, signed: bool) -> i64 {
    unsafe {
        match bits {
            8 if signed => (std::mem::transmute::<_, extern "C" fn() -> i8>(ptr))() as i64,
            8 => (std::mem::transmute::<_, extern "C" fn() -> u8>(ptr))() as i64,
            16 if signed => (std::mem::transmute::<_, extern "C" fn() -> i16>(ptr))() as i64,
            16 => (std::mem::transmute::<_, extern "C" fn() -> u16>(ptr))() as i64,
            32 if signed => (std::mem::transmute::<_, extern "C" fn() -> i32>(ptr))() as i64,
            32 => (std::mem::transmute::<_, extern "C" fn() -> u32>(ptr))() as i64,
            _ if signed => (std::mem::transmute::<_, extern "C" fn() -> i64>(ptr))(),
            _ => (std::mem::transmute::<_, extern "C" fn() -> u64>(ptr))() as i64,
        }
    }
}

/// Build a method's Cranelift signature from its recorded value-type codes: one
/// parameter per leading local kind, and a result when it returns a value.
///
/// It uses the host's native (C) calling convention so entry points are callable
/// through `extern "C"` function pointers and calls between compiled methods
/// match. Switching managed calls to `CallConv::Tail` is a later refinement.
fn body_signature<M: ClifModule>(
    module: &M,
    body: &zun_module::MethodBody<'_>,
) -> cranelift_codegen::ir::Signature {
    let mut sig = cranelift_codegen::ir::Signature::new(module.isa().default_call_conv());
    for &k in &body.local_kinds[..body.param_count as usize] {
        let kind = JitKind::from_byte(k);
        if kind == JitKind::Other {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        } else {
            sig.params.push(AbiParam::new(clif_ty(kind)));
        }
    }
    if body.returns_value {
        let return_kind = JitKind::from_byte(body.return_kind);
        if return_kind == JitKind::Other {
            // `Any` is a stable two-word carrier: tag followed by payload.
            sig.returns.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        } else {
            sig.returns.push(AbiParam::new(clif_ty(return_kind)));
        }
    }
    sig
}

#[cfg(test)]
mod tests {
    use super::{
        zun_jit_array_load_any, zun_jit_array_load_object, zun_jit_array_store_any,
        zun_jit_array_store_object, zun_jit_map_size, zun_jit_set_size, zun_jit_store_ref,
        JitAnyAbi,
    };

    #[test]
    fn dynamic_array_load_helper_preserves_both_abi_words() {
        let mut heap = zun_gc::Heap::new();
        let array = heap.alloc_array(1);
        heap.array_set(array, 0, zun_core::Any::object(0x1234));
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap) };
        let mut carrier = JitAnyAbi { tag: 5, payload: 0 };
        let loaded = unsafe {
            zun_jit_array_load_any(array.addr() as i64, 0, &mut carrier)
        };
        assert_eq!(loaded, 1);
        assert_eq!(carrier, JitAnyAbi { tag: 3, payload: 0x1234 });
        assert_eq!(unsafe { zun_jit_array_load_any(array.addr() as i64, -1, &mut carrier) }, 0);
        assert_eq!(unsafe { zun_jit_array_load_any(array.addr() as i64, 0, std::ptr::null_mut()) }, 0);
    }

    #[test]
    fn dynamic_array_store_helper_validates_and_preserves_both_abi_words() {
        let mut heap = zun_gc::Heap::new();
        let array = heap.alloc_array(1);
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        assert_eq!(
            zun_jit_array_store_any(array.addr() as i64, 0, 3, 0x1234),
            1
        );
        assert_eq!(
            heap.array_get(array, 0),
            Some(zun_core::Any::object(0x1234))
        );
        assert_eq!(zun_jit_array_store_any(array.addr() as i64, 0, 255, 0), 0);
        assert_eq!(zun_jit_array_store_any(array.addr() as i64, -1, 0, 0), 0);
    }

    #[test]
    fn dynamic_array_store_helper_keeps_object_payload_traced() {
        let mut heap = zun_gc::Heap::new();
        let child = heap.alloc(1, 1);
        let array = heap.alloc_array(1);
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        assert_eq!(
            zun_jit_array_store_any(array.addr() as i64, 0, 3, child.addr() as u64),
            1
        );
        assert_eq!(unsafe { heap.collect(&[array], &|_| Vec::new()) }, 0);
        assert_eq!(
            heap.array_get(array, 0).and_then(|value| value.as_object()),
            Some(child.addr() as usize)
        );
        assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 2);
    }

    #[test]
    fn object_array_helpers_round_trip_managed_pointer() {
        let mut heap = zun_gc::Heap::new();
        let array = heap.alloc_array(1);
        let child = heap.alloc(1, 0);
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };

        assert_eq!(zun_jit_array_store_object(array.addr() as i64, 0, child.addr() as i64), 1);
        assert_eq!(zun_jit_array_load_object(array.addr() as i64, 0), child.addr() as i64);
    }

    #[test]
    fn collection_size_helpers_read_managed_lengths() {
        let mut heap = zun_gc::Heap::new();
        let map = heap.alloc_map();
        let set = heap.alloc_set();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        assert_eq!(zun_jit_map_size(map.addr() as i64), 0);
        assert_eq!(zun_jit_set_size(set.addr() as i64), 0);
        heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::int(2));
        heap.set_add(set, zun_core::Any::int(3));
        assert_eq!(zun_jit_map_size(map.addr() as i64), 1);
        assert_eq!(zun_jit_set_size(set.addr() as i64), 1);
    }

    #[test]
    fn jit_reference_store_preserves_young_target_through_minor_collection() {
        let mut heap = zun_gc::Heap::new();
        let owner = heap.alloc(1, 1);
        unsafe { heap.collect(&[owner], &|_| vec![0]) };
        let child = heap.alloc(2, 0);
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };

        assert_eq!(zun_jit_store_ref(owner.addr() as i64, 0, child.addr() as i64), 1);
        assert_eq!(unsafe { heap.collect_young(&[owner], &|_| vec![0]) }, 0);
        assert_eq!(owner.field(0), child.addr() as u64);
    }
}
