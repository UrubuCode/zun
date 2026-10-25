use super::{Heap, Obj};

impl Heap {
    pub fn buffer_len(&self, object: Obj) -> Option<usize> {
        self.buffers.get(&(object.addr() as usize)).map(zun_core::ByteBuffer::len)
    }

    /// Allocates a resizable `ArrayBuffer` of `length` bytes growable to
    /// `max`. `None` when `length > max` (the constructor's `TypeError`).
    pub fn alloc_resizable_buffer(&mut self, length: usize, max: usize) -> Option<Obj> {
        let buffer = zun_core::ByteBuffer::new_resizable(length, max)?;
        let object = self.alloc(0, 0);
        self.buffers.insert(object.addr() as usize, buffer);
        Some(object)
    }

    /// `ArrayBuffer.prototype.maxByteLength`.
    pub fn buffer_max_byte_length(&self, object: Obj) -> Option<usize> {
        self.buffers.get(&(object.addr() as usize)).map(zun_core::ByteBuffer::max_byte_length)
    }

    /// `ArrayBuffer.prototype.resizable`.
    pub fn buffer_resizable(&self, object: Obj) -> Option<bool> {
        self.buffers.get(&(object.addr() as usize)).map(zun_core::ByteBuffer::resizable)
    }

    /// `ArrayBuffer.prototype.resize(newLength)`. Outstanding typed-array and
    /// `DataView` windows are not walked: each re-resolves against the new
    /// length on its next access, so nothing here has to touch them.
    pub fn buffer_resize(&mut self, object: Obj, new_length: usize) -> Option<bool> {
        Some(self.buffers.get_mut(&(object.addr() as usize))?.resize(new_length))
    }

    /// `ArrayBuffer.prototype.detached`.
    pub fn buffer_detached(&self, object: Obj) -> Option<bool> {
        self.buffers.get(&(object.addr() as usize)).map(zun_core::ByteBuffer::detached)
    }

    /// `ArrayBuffer.prototype.transfer(newLength?)` /
    /// `transferToFixedLength(newLength?)`. Detaches `source` and moves its
    /// bytes into a freshly allocated buffer of `new_length` bytes (the source
    /// length when `new_length` is `None`), preserving the surviving prefix.
    /// `fixed` forces a fixed-length destination (`transferToFixedLength`);
    /// otherwise the destination keeps the source's resizability and growth
    /// cap. Returns `None` (a runtime error) when the source is already
    /// detached, is not a buffer, or the destination would exceed its cap.
    pub fn buffer_transfer(&mut self, source: Obj, new_length: Option<usize>, fixed: bool) -> Option<Obj> {
        let (old_bytes, old_max) = self
            .buffers
            .get_mut(&(source.addr() as usize))?
            .detach()?;
        let target_len = new_length.unwrap_or(old_bytes.len());
        let max = if fixed { None } else { old_max };
        let mut buffer = zun_core::ByteBuffer::with_capacity(target_len, max)?;
        let copy = target_len.min(old_bytes.len());
        for (index, byte) in old_bytes.iter().take(copy).enumerate() {
            buffer.set(index, *byte);
        }
        // Allocation may move objects, so `buffer` holds no managed address and
        // is inserted only after the allocation settles.
        let object = self.alloc(0, 0);
        self.buffers.insert(object.addr() as usize, buffer);
        Some(object)
    }

    pub fn buffer_get(&self, object: Obj, index: usize) -> Option<u8> {
        self.buffers.get(&(object.addr() as usize))?.get(index)
    }

    pub fn buffer_set(&mut self, object: Obj, index: usize, value: u8) -> Option<bool> {
        Some(self.buffers.get_mut(&(object.addr() as usize))?.set(index, value))
    }

    pub fn buffer_read_u32(&self, object: Obj, offset: usize) -> Option<u32> {
        self.buffers.get(&(object.addr() as usize))?.read_u32(offset)
    }

    pub fn buffer_write_u32(&mut self, object: Obj, offset: usize, value: u32) -> Option<bool> {
        Some(self.buffers.get_mut(&(object.addr() as usize))?.write_u32(offset, value))
    }

    pub fn buffer_read_f64(&self, object: Obj, offset: usize) -> Option<f64> {
        self.buffers.get(&(object.addr() as usize))?.read_f64(offset)
    }

    pub fn buffer_write_f64(&mut self, object: Obj, offset: usize, value: f64) -> Option<bool> {
        Some(self.buffers.get_mut(&(object.addr() as usize))?.write_f64(offset, value))
    }

    /// Copy `[begin, end)` of `object` into a freshly allocated buffer.
    ///
    /// `ArrayBuffer.prototype.slice` copies rather than aliasing, so the two
    /// buffers are independent afterwards. Out-of-range and inverted ranges
    /// clamp to an empty result, as they do in JS, rather than failing.
    pub fn buffer_slice(&mut self, object: Obj, begin: usize, end: usize) -> Option<Obj> {
        let length = self.buffer_len(object)?;
        let begin = begin.min(length);
        let end = end.min(length).max(begin);
        let bytes: Vec<u8> = (begin..end)
            .map(|index| self.buffer_get(object, index).unwrap_or(0))
            .collect();

        // Allocating may move objects, so nothing read above may be held
        // across this call as an address.
        let copy = self.alloc_buffer(bytes.len());
        for (index, byte) in bytes.into_iter().enumerate() {
            self.buffer_set(copy, index, byte);
        }
        Some(copy)
    }

    pub fn buffer_view(&self, object: Obj, offset: usize, length: usize) -> Option<zun_core::ByteView> {
        zun_core::ByteView::new(self.buffer_len(object)?, offset, length)
    }

    pub fn data_view(&self, object: Obj, offset: usize, length: usize) -> Option<zun_core::DataView> {
        zun_core::DataView::new(self.buffer_len(object)?, offset, length)
    }

    pub fn data_view_read_u16(&self, object: Obj, view: zun_core::DataView, offset: usize, little_endian: bool) -> Option<u16> {
        view.get_u16(self.buffers.get(&(object.addr() as usize))?, offset, little_endian)
    }

    pub fn data_view_write_u16(&mut self, object: Obj, view: zun_core::DataView, offset: usize, value: u16, little_endian: bool) -> Option<bool> {
        Some(view.set_u16(self.buffers.get_mut(&(object.addr() as usize))?, offset, value, little_endian))
    }

    pub fn data_view_read_u32(&self, object: Obj, view: zun_core::DataView, offset: usize, little_endian: bool) -> Option<u32> {
        view.get_u32(self.buffers.get(&(object.addr() as usize))?, offset, little_endian)
    }

    pub fn data_view_write_u32(&mut self, object: Obj, view: zun_core::DataView, offset: usize, value: u32, little_endian: bool) -> Option<bool> {
        Some(view.set_u32(self.buffers.get_mut(&(object.addr() as usize))?, offset, value, little_endian))
    }

    pub fn data_view_read_f64(&self, object: Obj, view: zun_core::DataView, offset: usize, little_endian: bool) -> Option<f64> {
        view.get_f64(self.buffers.get(&(object.addr() as usize))?, offset, little_endian)
    }

    pub fn data_view_write_f64(&mut self, object: Obj, view: zun_core::DataView, offset: usize, value: f64, little_endian: bool) -> Option<bool> {
        Some(view.set_f64(self.buffers.get_mut(&(object.addr() as usize))?, offset, value, little_endian))
    }

    pub fn buffer_view_read_u32(&self, object: Obj, view: zun_core::ByteView, offset: usize) -> Option<u32> {
        view.read_u32(self.buffers.get(&(object.addr() as usize))?, offset)
    }

    pub fn buffer_view_write_u32(&mut self, object: Obj, view: zun_core::ByteView, offset: usize, value: u32) -> Option<bool> {
        Some(view.write_u32(self.buffers.get_mut(&(object.addr() as usize))?, offset, value))
    }

    pub fn buffer_view_read_u8(&self, object: Obj, view: zun_core::TypedArrayView, index: usize) -> Option<u8> {
        view.read_u8(self.buffers.get(&(object.addr() as usize))?, index)
    }

    pub fn buffer_view_write_u8(&mut self, object: Obj, view: zun_core::TypedArrayView, index: usize, value: u8) -> Option<bool> {
        Some(view.write_u8(self.buffers.get_mut(&(object.addr() as usize))?, index, value))
    }

    pub fn buffer_view_read_u16(&self, object: Obj, view: zun_core::TypedArrayView, index: usize) -> Option<u16> {
        view.read_u16(self.buffers.get(&(object.addr() as usize))?, index)
    }

    pub fn buffer_view_write_u16(&mut self, object: Obj, view: zun_core::TypedArrayView, index: usize, value: u16) -> Option<bool> {
        Some(view.write_u16(self.buffers.get_mut(&(object.addr() as usize))?, index, value))
    }

    pub fn buffer_view_read_f64(&self, object: Obj, view: zun_core::TypedArrayView, index: usize) -> Option<f64> {
        view.read_f64(self.buffers.get(&(object.addr() as usize))?, index)
    }

    pub fn buffer_view_write_f64(&mut self, object: Obj, view: zun_core::TypedArrayView, index: usize, value: f64) -> Option<bool> {
        Some(view.write_f64(self.buffers.get_mut(&(object.addr() as usize))?, index, value))
    }
}
