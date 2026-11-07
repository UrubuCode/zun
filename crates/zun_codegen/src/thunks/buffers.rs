//! JIT runtime thunks for ArrayBuffer, DataView and typed-array views.
//!
//! These `extern "C"` symbols are linked into generated native code by address
//! (see `Jit::build_*_builder` in `lib.rs`). Signatures, visibility and
//! attributes are the FFI contract and must not change.

/// Read a managed ArrayBuffer length without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_buffer_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.buffer_len(object).map(|len| len as i64).unwrap_or(-1))
}

#[inline]
pub extern "C" fn zun_jit_data_view_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.data_view_len(object)
            .map(|len| len as i64)
            .unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_typed_array_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.typed_array_len(object)
            .map(|len| len as i64)
            .unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_data_view_byte_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.data_view_byte_len(object)
            .map(|len| len as i64)
            .unwrap_or(-1)
    })
}

#[inline]
pub extern "C" fn zun_jit_typed_array_byte_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.typed_array_byte_len(object)
            .map(|len| len as i64)
            .unwrap_or(-1)
    })
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
        heap.buffer_read_f64(object, offset as usize)
            .unwrap_or(f64::NAN)
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
        heap.buffer_set(object, index as usize, value as u8)
            .unwrap_or(false) as i32
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
        heap.buffer_write_u32(object, offset as usize, value as u32)
            .unwrap_or(false) as i32
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
        heap.buffer_write_f64(object, offset as usize, value)
            .unwrap_or(false) as i32
    })
}

/// Read a u32 from a managed DataView without allocating or collecting.
#[inline]
pub extern "C" fn zun_jit_data_view_get_u32(address: i64, offset: i64, little_endian: i32) -> i32 {
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
pub extern "C" fn zun_jit_data_view_get_u16(address: i64, offset: i64, little_endian: i32) -> i16 {
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
pub extern "C" fn zun_jit_data_view_get_f64(address: i64, offset: i64, little_endian: i32) -> f64 {
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

// DataView typed accessors beyond the u16/u32/f64 fast paths. Each mirrors the
// pattern above: a negative offset or an out-of-range access yields the `-1` /
// NaN sentinel (matching `get_u16`), stores return the success flag as i32. The
// 8-bit forms take no endianness argument.
#[inline]
pub extern "C" fn zun_jit_data_view_get_i8(address: i64, offset: i64) -> i8 {
    if offset < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.data_view_get_i8(object, offset as usize).unwrap_or(-1))
}
#[inline]
pub extern "C" fn zun_jit_data_view_get_u8(address: i64, offset: i64) -> i8 {
    if offset < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.data_view_get_u8(object, offset as usize)
            .map(|value| value as i8)
            .unwrap_or(-1)
    })
}
#[inline]
pub extern "C" fn zun_jit_data_view_get_i16(address: i64, offset: i64, little_endian: i32) -> i16 {
    if offset < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.data_view_get_i16(object, offset as usize, little_endian != 0)
            .unwrap_or(-1)
    })
}
#[inline]
pub extern "C" fn zun_jit_data_view_get_i32(address: i64, offset: i64, little_endian: i32) -> i32 {
    if offset < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.data_view_get_i32(object, offset as usize, little_endian != 0)
            .unwrap_or(-1)
    })
}
#[inline]
pub extern "C" fn zun_jit_data_view_get_f32(address: i64, offset: i64, little_endian: i32) -> f32 {
    if offset < 0 {
        return f32::NAN;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.data_view_get_f32(object, offset as usize, little_endian != 0)
            .unwrap_or(f32::NAN)
    })
}
#[inline]
pub extern "C" fn zun_jit_data_view_set_i8(address: i64, offset: i64, value: i8) -> i32 {
    if offset < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.data_view_set_i8(object, offset as usize, value)
            .unwrap_or(false) as i32
    })
}
#[inline]
pub extern "C" fn zun_jit_data_view_set_u8(address: i64, offset: i64, value: i8) -> i32 {
    if offset < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.data_view_set_u8(object, offset as usize, value as u8)
            .unwrap_or(false) as i32
    })
}
#[inline]
pub extern "C" fn zun_jit_data_view_set_i16(
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
        heap.data_view_set_i16(object, offset as usize, value, little_endian != 0)
            .unwrap_or(false) as i32
    })
}
#[inline]
pub extern "C" fn zun_jit_data_view_set_i32(
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
        heap.data_view_set_i32(object, offset as usize, value, little_endian != 0)
            .unwrap_or(false) as i32
    })
}
#[inline]
pub extern "C" fn zun_jit_data_view_set_f32(
    address: i64,
    offset: i64,
    value: f32,
    little_endian: i32,
) -> i32 {
    if offset < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.data_view_set_f32(object, offset as usize, value, little_endian != 0)
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
    if index < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.u16_array_get(object, index as usize)
            .map(|value| value as i16)
            .unwrap_or(-1)
    })
}

/// Write one element to a managed Uint16Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_u16_array_set(address: i64, index: i64, value: i16) -> i32 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.u16_array_set(object, index as usize, value as u16)
            .unwrap_or(false) as i32
    })
}

/// Read one element from a managed Uint32Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_u32_array_get(address: i64, index: i64) -> i32 {
    if index < 0 {
        return -1;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.u32_array_get(object, index as usize)
            .map(|value| value as i32)
            .unwrap_or(-1)
    })
}

/// Write one element to a managed Uint32Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_u32_array_set(address: i64, index: i64, value: i32) -> i32 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.u32_array_set(object, index as usize, value as u32)
            .unwrap_or(false) as i32
    })
}

/// Read one element from a managed Float64Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_f64_array_get(address: i64, index: i64) -> f64 {
    if index < 0 {
        return f64::NAN;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| {
        heap.f64_array_get(object, index as usize)
            .unwrap_or(f64::NAN)
    })
}

/// Write one element to a managed Float64Array view without allocation.
#[inline]
pub extern "C" fn zun_jit_f64_array_set(address: i64, index: i64, value: f64) -> i32 {
    if index < 0 {
        return 0;
    }
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap_mut(|heap| {
        heap.f64_array_set(object, index as usize, value)
            .unwrap_or(false) as i32
    })
}
