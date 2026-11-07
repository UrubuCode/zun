//! JIT runtime thunks for managed Map and Set collections.
//!
//! These `extern "C"` symbols are linked into generated native code by address
//! (see `Jit::build_*_builder` in `lib.rs`). Signatures, visibility and
//! attributes are the FFI contract and must not change.

use crate::JitAnyAbi;

/// Read a managed Map length without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_map_size(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.map_len(object).map(|len| len as i64).unwrap_or(-1))
}

pub extern "C" fn zun_jit_map_new() -> i64 {
    zun_gc::with_current_heap_mut(|heap| heap.alloc_map().addr() as i64)
}

pub extern "C" fn zun_jit_map_clear(address: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| heap.map_clear(object).is_some() as i32)
}

pub extern "C" fn zun_jit_map_values(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        let values = heap
            .map_entries(object)?
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        let array = heap.alloc_array(values.len());
        for (index, value) in values.into_iter().enumerate() {
            heap.array_set(array, index, value)?;
        }
        Some(array.addr() as i64)
    })
    .unwrap_or(0)
}

pub extern "C" fn zun_jit_map_keys(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        let values = heap
            .map_entries(object)?
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>();
        let array = heap.alloc_array(values.len());
        for (index, value) in values.into_iter().enumerate() {
            heap.array_set(array, index, value)?;
        }
        Some(array.addr() as i64)
    })
    .unwrap_or(0)
}

pub extern "C" fn zun_jit_map_entries(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        let entries = heap.map_entries(object)?;
        let array = heap.alloc_array(entries.len());
        for (index, (key, value)) in entries.into_iter().enumerate() {
            let pair = heap.alloc_array(2);
            heap.array_set(pair, 0, key)?;
            heap.array_set(pair, 1, value)?;
            heap.array_set(array, index, zun_core::Any::object(pair.addr() as usize))?;
        }
        Some(array.addr() as i64)
    })
    .unwrap_or(0)
}

pub extern "C" fn zun_jit_set_values(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        let values = heap.set_values(object)?;
        let array = heap.alloc_array(values.len());
        for (index, value) in values.into_iter().enumerate() {
            heap.array_set(array, index, value)?;
        }
        Some(array.addr() as i64)
    })
    .unwrap_or(0)
}

pub extern "C" fn zun_jit_set_entries(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        let values = heap.set_values(object)?;
        let array = heap.alloc_array(values.len());
        for (index, value) in values.into_iter().enumerate() {
            let pair = heap.alloc_array(2);
            heap.array_set(pair, 0, value)?;
            heap.array_set(pair, 1, value)?;
            heap.array_set(array, index, zun_core::Any::object(pair.addr() as usize))?;
        }
        Some(array.addr() as i64)
    })
    .unwrap_or(0)
}

/// Read a managed Set length without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_set_size(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.set_len(object).map(|len| len as i64).unwrap_or(-1))
}

pub extern "C" fn zun_jit_set_new() -> i64 {
    zun_gc::with_current_heap_mut(|heap| heap.alloc_set().addr() as i64)
}

pub extern "C" fn zun_jit_set_clear(address: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| heap.set_clear(object).is_some() as i32)
}

/// Test a numeric key in a managed Map without materializing an `Any` carrier.
#[inline]
pub extern "C" fn zun_jit_map_has_f64(address: i64, key: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.map_has(object, zun_core::Any::float(key))
            .unwrap_or(false) as i32
    })
}

/// Test a numeric value in a managed Set without materializing an `Any` carrier.
#[inline]
pub extern "C" fn zun_jit_set_has_f64(address: i64, value: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.set_has(object, zun_core::Any::float(value))
            .unwrap_or(false) as i32
    })
}

/// Insert a numeric value into a managed Set without materializing an `Any` carrier.
#[inline]
pub extern "C" fn zun_jit_set_add_f64(address: i64, value: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.set_add(object, zun_core::Any::float(value))
            .unwrap_or(false) as i32
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
        heap.set_remove(object, zun_core::Any::float(value))
            .unwrap_or(false) as i32
    })
}

/// Insert a numeric key/value pair into a managed Map without an `Any` carrier.
/// Returns one when the key was new and zero when an existing value was replaced.
#[inline]
pub extern "C" fn zun_jit_map_set_f64(address: i64, key: f64, value: f64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.map_insert(
            object,
            zun_core::Any::float(key),
            zun_core::Any::float(value),
        )
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

/// Read a generic Map value into the stable two-word Any carrier.
#[inline]
pub unsafe extern "C" fn zun_jit_map_get_any(
    address: i64,
    key_tag: u64,
    key_payload: u64,
    out: *mut JitAnyAbi,
) -> i32 {
    if out.is_null() {
        return 0;
    }
    let Some(key) = zun_core::Any::from_raw_parts(key_tag as u8, key_payload) else {
        return 0;
    };
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    let value = zun_gc::with_current_heap(|heap| heap.map_get(object, key))
        .unwrap_or_else(zun_core::Any::void);
    unsafe { out.write(JitAnyAbi::from_any(value)) };
    1
}

#[inline]
pub extern "C" fn zun_jit_map_has_i64(address: i64, key: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.map_has(object, zun_core::Any::int(key))
            .unwrap_or(false) as i32
    })
}

pub extern "C" fn zun_jit_map_has_i32(address: i64, key: i32) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.map_has(object, zun_core::Any::int(key as i64))
            .unwrap_or(false) as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_set_has_i64(address: i64, value: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.set_has(object, zun_core::Any::int(value))
            .unwrap_or(false) as i32
    })
}

pub extern "C" fn zun_jit_set_has_i32(address: i64, value: i32) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.set_has(object, zun_core::Any::int(value as i64))
            .unwrap_or(false) as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_set_add_i64(address: i64, value: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.set_add(object, zun_core::Any::int(value))
            .unwrap_or(false) as i32
    })
}

pub extern "C" fn zun_jit_set_add_i32(address: i64, value: i32) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.set_add(object, zun_core::Any::int(value as i64))
            .unwrap_or(false) as i32
    })
}

pub extern "C" fn zun_jit_map_delete_i32(address: i64, key: i32) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.map_remove(object, zun_core::Any::int(key as i64))
            .is_some() as i32
    })
}

pub extern "C" fn zun_jit_set_delete_i32(address: i64, value: i32) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.set_remove(object, zun_core::Any::int(value as i64))
            .unwrap_or(false) as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_map_delete_i64(address: i64, key: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.map_remove(object, zun_core::Any::int(key)).is_some() as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_set_delete_i64(address: i64, value: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.set_remove(object, zun_core::Any::int(value))
            .unwrap_or(false) as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_map_set_i64(address: i64, key: i64, value: i64) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.map_insert(object, zun_core::Any::int(key), zun_core::Any::int(value))
            .is_none() as i32
    })
}

#[inline]
pub extern "C" fn zun_jit_map_get_i64_or(address: i64, key: i64, fallback: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.map_get(object, zun_core::Any::int(key))
            .and_then(|value| value.as_int())
            .unwrap_or(fallback)
    })
}

pub extern "C" fn zun_jit_map_get_i32_or(address: i64, key: i32, fallback: i32) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.map_get(object, zun_core::Any::int(key as i64))
            .and_then(|value| value.as_int())
            .map(|value| value as i32)
            .unwrap_or(fallback)
    })
}

pub extern "C" fn zun_jit_map_set_i32(address: i64, key: i32, value: i32) -> i32 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.map_insert(
            object,
            zun_core::Any::int(key as i64),
            zun_core::Any::int(value as i64),
        )
        .is_none() as i32
    })
}
