use super::{Heap, Obj};

impl Heap {
    /// The `DataView` window re-resolved against its buffer's current length
    /// (a resize may move a length-tracking view or push a fixed one out of
    /// bounds). `None` means out-of-bounds: the access fails.
    pub(crate) fn resolved_data_view(&self, object: Obj) -> Option<zun_core::DataView> {
        let view = *self.data_views.get(&(object.addr() as usize))?;
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        view.resolve(self.buffer_len(buffer)?)
    }

    pub fn data_view_len(&self, object: Obj) -> Option<usize> {
        let view = self.data_views.get(&(object.addr() as usize))?;
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let buffer_len = self.buffer_len(buffer)?;
        // Out-of-bounds (a fixed view on a shrunk buffer) reports length 0.
        Some(view.resolve(buffer_len).map_or(0, |view| view.len()))
    }

    pub fn typed_array_len(&self, object: Obj) -> Option<usize> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize))?;
        let buffer_len = self.buffer_len(*buffer)?;
        Some(view.resolve(buffer_len).map_or(0, |view| view.len()))
    }

    pub fn data_view_byte_len(&self, object: Obj) -> Option<usize> {
        self.data_view_len(object)
    }

    pub fn typed_array_byte_len(&self, object: Obj) -> Option<usize> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize))?;
        let buffer_len = self.buffer_len(*buffer)?;
        Some(view.resolve(buffer_len).map_or(0, |view| view.byte_len()))
    }

    /// The typed-array view's byte offset into its backing buffer
    /// (`typedArray.byteOffset`). Kind-independent: the view carries its own
    /// byte offset regardless of element width.
    pub fn typed_array_byte_offset(&self, object: Obj) -> Option<usize> {
        self.typed_views.get(&(object.addr() as usize)).map(|(_, view)| view.byte_offset())
    }

    /// The backing `ArrayBuffer` of a typed-array view (`typedArray.buffer`).
    pub fn typed_array_buffer(&self, object: Obj) -> Option<Obj> {
        self.typed_views.get(&(object.addr() as usize)).map(|(buffer, _)| *buffer)
    }

    /// The element kind of a typed-array view, or `None` if `object` is not a
    /// typed-array view. Backs Node's `Buffer.isBuffer`, which is true only for
    /// a `Uint8Array`-kind view.
    pub fn typed_array_element(&self, object: Obj) -> Option<zun_core::TypedElement> {
        self.typed_views.get(&(object.addr() as usize)).map(|(_, view)| view.element())
    }

    /// The byte offset of a `DataView` into its backing buffer
    /// (`dataView.byteOffset`).
    pub fn data_view_byte_offset(&self, object: Obj) -> Option<usize> {
        self.data_views.get(&(object.addr() as usize)).map(|view| view.offset())
    }

    /// Whether a `DataView` is length-tracking (no explicit length at
    /// construction); `None` if `object` is not a data view.
    pub fn data_view_is_tracking(&self, object: Obj) -> Option<bool> {
        self.data_views.get(&(object.addr() as usize)).map(|view| view.is_tracking())
    }

    /// The backing `ArrayBuffer` of a `DataView` (`dataView.buffer`). The
    /// handle is stored in field 0 at construction; it is only returned for an
    /// object that is actually a registered data view.
    pub fn data_view_buffer(&self, object: Obj) -> Option<Obj> {
        self.data_views
            .contains_key(&(object.addr() as usize))
            .then(|| unsafe { Obj::from_addr(object.field(0) as *mut u64) })
    }

    /// Whether `object` is a buffer view — a `DataView` or any typed array.
    /// Backs `ArrayBuffer.isView`; a non-view object (or any value that is not
    /// an object at all) answers `false` rather than failing.
    pub fn is_view(&self, object: Obj) -> bool {
        let key = object.addr() as usize;
        self.data_views.contains_key(&key) || self.typed_views.contains_key(&key)
    }

    pub fn data_view_get_u32(&self, object: Obj, relative: usize, little_endian: bool) -> Option<u32> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_u32(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_u32(&mut self, object: Obj, relative: usize, value: u32, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = self.resolved_data_view(object)?;
        Some(view.set_u32(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    // The remaining DataView element widths. Single bytes take no
    // endianness; the rest mirror the u16/u32/f64 pairs below.
    pub fn data_view_get_u8(&self, object: Obj, relative: usize) -> Option<u8> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_u8(
            self.buffers.get(&(buffer.addr() as usize))?, relative,
        )
    }

    pub fn data_view_set_u8(&mut self, object: Obj, relative: usize, value: u8) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = self.resolved_data_view(object)?;
        Some(view.set_u8(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value))
    }

    pub fn data_view_get_i8(&self, object: Obj, relative: usize) -> Option<i8> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_i8(
            self.buffers.get(&(buffer.addr() as usize))?, relative,
        )
    }

    pub fn data_view_set_i8(&mut self, object: Obj, relative: usize, value: i8) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = self.resolved_data_view(object)?;
        Some(view.set_i8(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value))
    }

    pub fn data_view_get_i16(&self, object: Obj, relative: usize, little_endian: bool) -> Option<i16> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_i16(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_i16(&mut self, object: Obj, relative: usize, value: i16, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = self.resolved_data_view(object)?;
        Some(view.set_i16(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_i32(&self, object: Obj, relative: usize, little_endian: bool) -> Option<i32> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_i32(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_i32(&mut self, object: Obj, relative: usize, value: i32, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = self.resolved_data_view(object)?;
        Some(view.set_i32(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_f32(&self, object: Obj, relative: usize, little_endian: bool) -> Option<f32> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_f32(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_f32(&mut self, object: Obj, relative: usize, value: f32, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = self.resolved_data_view(object)?;
        Some(view.set_f32(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_i64(&self, object: Obj, relative: usize, little_endian: bool) -> Option<i64> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_i64(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_i64(&mut self, object: Obj, relative: usize, value: i64, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = self.resolved_data_view(object)?;
        Some(view.set_i64(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_u64(&self, object: Obj, relative: usize, little_endian: bool) -> Option<u64> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_u64(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_u64(&mut self, object: Obj, relative: usize, value: u64, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = self.resolved_data_view(object)?;
        Some(view.set_u64(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_u16(&self, object: Obj, relative: usize, little_endian: bool) -> Option<u16> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_u16(
            self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian,
        )
    }

    pub fn data_view_set_u16(&mut self, object: Obj, relative: usize, value: u16, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = self.resolved_data_view(object)?;
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
                    let bytes = self.buffers.get(&(buffer.addr() as usize))?;
                    // Re-resolve the window against the buffer's current
                    // length: a resize may have shrunk it out of bounds or
                    // (for a length-tracking view) changed how many elements
                    // it covers. `None` here means out-of-bounds — the read
                    // fails, as ES2024 requires.
                    view.resolve(bytes.len())?.$read(bytes, index)
                }

                /// Writes one element, reporting whether it landed.
                pub fn $set(&mut self, object: Obj, index: usize, value: $arg) -> Option<bool> {
                    let (buffer, view) =
                        self.typed_views.get(&(object.addr() as usize)).copied()?;
                    let bytes = self.buffers.get_mut(&(buffer.addr() as usize))?;
                    let resolved = view.resolve(bytes.len())?;
                    Some(resolved.$write(bytes, index, value))
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

    /// Allocates a length-tracking typed-array view over `buffer` (`new
    /// <Kind>Array(resizableBuffer)` / `… (buffer, offset)`, with no explicit
    /// length). Its element count follows the buffer's current length.
    pub fn alloc_typed_array_tracking(
        &mut self,
        buffer: Obj,
        offset: usize,
        element: zun_core::TypedElement,
    ) -> Option<Obj> {
        let view =
            zun_core::TypedArrayView::new_tracking(self.buffer_len(buffer)?, offset, element)?;
        let object = self.alloc(0, 1);
        object.set_field(0, buffer.addr() as u64);
        self.typed_views.insert(object.addr() as usize, (buffer, view));
        Some(object)
    }
}

impl Heap {
    /// `subarray(begin, end)`: a second view of the same kind over the same
    /// buffer. The object it returns shares the bytes rather than copying
    /// them, so the two views see each other's writes.
    pub fn typed_array_subarray(&mut self, object: Obj, begin: usize, end: usize) -> Option<Obj> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize)).copied()?;
        // Resolve against the buffer's current length first, so a subarray of
        // a length-tracking (or resized) view steps over the window the view
        // actually covers now. The child is a fixed-length view of that range.
        let view = view.resolve(self.buffer_len(buffer)?)?;
        let subview = view.subview(begin, end);
        // The buffer's address is carried across this allocation, which the
        // comment on `buffer_slice` forbids. It is safe here, and the reason
        // is worth writing down because the shape looks like the bug it is
        // not: `alloc` only bump-allocates or reuses a free slot, and never
        // moves anything. Objects move in `compact`, which is an explicit
        // `unsafe fn` the VM calls at a safepoint — and which remaps both the
        // `typed_views` key and the buffer handle stored in the value, so a
        // view survives compaction with both ends still pointing at the right
        // objects. `buffer_slice`'s rule is about reading bytes across a
        // collection, not about this.
        let child = self.alloc(0, 1);
        child.set_field(0, buffer.addr() as u64);
        self.typed_views.insert(child.addr() as usize, (buffer, subview));
        Some(child)
    }

    /// The element range a `fill(value, begin, end)` covers on this view,
    /// clamped as JS clamps it. The filling is per kind, so the caller walks
    /// this range calling the kind's setter.
    pub fn typed_array_fill_range(
        &self,
        object: Obj,
        begin: usize,
        end: usize,
    ) -> Option<std::ops::Range<usize>> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize))?;
        let view = view.resolve(self.buffer_len(*buffer)?)?;
        Some(view.fill_range(begin, end))
    }
}
