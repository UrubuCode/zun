use super::{Heap, Obj};

impl Heap {
    pub fn data_view_len(&self, object: Obj) -> Option<usize> {
        self.data_views.get(&(object.addr() as usize)).map(|view| view.len())
    }

    pub fn typed_array_len(&self, object: Obj) -> Option<usize> {
        self.typed_views.get(&(object.addr() as usize)).map(|(_, view)| view.len())
    }

    pub fn data_view_byte_len(&self, object: Obj) -> Option<usize> {
        self.data_views.get(&(object.addr() as usize)).map(|view| view.len())
    }

    pub fn typed_array_byte_len(&self, object: Obj) -> Option<usize> {
        self.typed_views.get(&(object.addr() as usize)).map(|(_, view)| view.byte_len())
    }

    pub fn data_view_get_u32(&self, object: Obj, relative: usize, little_endian: bool) -> Option<u32> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_u32(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_u32(&mut self, object: Obj, relative: usize, value: u32, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_u32(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    // The remaining DataView element widths. Single bytes take no
    // endianness; the rest mirror the u16/u32/f64 pairs below.
    pub fn data_view_get_u8(&self, object: Obj, relative: usize) -> Option<u8> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_u8(
            self.buffers.get(&(buffer.addr() as usize))?, relative,
        )
    }

    pub fn data_view_set_u8(&mut self, object: Obj, relative: usize, value: u8) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_u8(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value))
    }

    pub fn data_view_get_i8(&self, object: Obj, relative: usize) -> Option<i8> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_i8(
            self.buffers.get(&(buffer.addr() as usize))?, relative,
        )
    }

    pub fn data_view_set_i8(&mut self, object: Obj, relative: usize, value: i8) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_i8(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value))
    }

    pub fn data_view_get_i16(&self, object: Obj, relative: usize, little_endian: bool) -> Option<i16> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_i16(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_i16(&mut self, object: Obj, relative: usize, value: i16, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_i16(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_i32(&self, object: Obj, relative: usize, little_endian: bool) -> Option<i32> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_i32(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_i32(&mut self, object: Obj, relative: usize, value: i32, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_i32(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_f32(&self, object: Obj, relative: usize, little_endian: bool) -> Option<f32> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_f32(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_f32(&mut self, object: Obj, relative: usize, value: f32, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_f32(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_i64(&self, object: Obj, relative: usize, little_endian: bool) -> Option<i64> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_i64(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_i64(&mut self, object: Obj, relative: usize, value: i64, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_i64(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_u64(&self, object: Obj, relative: usize, little_endian: bool) -> Option<u64> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_u64(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_u64(&mut self, object: Obj, relative: usize, value: u64, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_u64(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_u16(&self, object: Obj, relative: usize, little_endian: bool) -> Option<u16> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_u16(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_u16(&mut self, object: Obj, relative: usize, value: u16, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_u16(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }
}

/// Generates the heap side of every typed-array kind from the master table
/// in `zun_core`: one allocator, one getter and one setter each. Threading a
/// new kind through by hand is what left kinds half-wired before.
macro_rules! typed_array_heap_access {
    ($({
        variant: $variant:ident, repr: $repr:ty, arg: $arg:ty, width: $width:expr,
        js: $js:literal, native: $native:literal,
        elem_ty: $elem_ty:literal, arg_ty: $arg_ty:literal,
        view_ty: $view_ty:ident, elem_native: $elem_native:ident,
        arg_native: $arg_native:ident,
        encode: $encode:path,
        read: $read:ident, write: $write:ident,
        get: $get:ident, set: $set:ident
    }),+ $(,)?) => {
        impl Heap {
            $(
                /// Reads one element. `None` when the object is not a view,
                /// the index is outside it, or it is a view of another kind.
                pub fn $get(&self, object: Obj, index: usize) -> Option<$repr> {
                    let (buffer, view) = self.typed_views.get(&(object.addr() as usize))?;
                    view.$read(self.buffers.get(&(buffer.addr() as usize))?, index)
                }

                /// Writes one element, reporting whether it landed.
                pub fn $set(&mut self, object: Obj, index: usize, value: $arg) -> Option<bool> {
                    let (buffer, view) =
                        self.typed_views.get(&(object.addr() as usize)).copied()?;
                    Some(view.$write(
                        self.buffers.get_mut(&(buffer.addr() as usize))?,
                        index,
                        value,
                    ))
                }
            )+
        }
    };
}

zun_core::typed_element_table!(typed_array_heap_access);

impl Heap {
    /// Allocates a typed-array view over `buffer`. `length` counts
    /// **elements**, as the JavaScript constructor does.
    ///
    /// One function for every kind rather than one per kind: the element kind
    /// is a value here, and the view carries it.
    pub fn alloc_typed_array(
        &mut self,
        buffer: Obj,
        offset: usize,
        length: usize,
        element: zun_core::TypedElement,
    ) -> Option<Obj> {
        let view =
            zun_core::TypedArrayView::new(self.buffer_len(buffer)?, offset, length, element)?;
        let object = self.alloc(0, 1);
        object.set_field(0, buffer.addr() as u64);
        self.typed_views.insert(object.addr() as usize, (buffer, view));
        Some(object)
    }
}
