use super::{Heap, Obj};

impl Heap {
    pub fn buffer_len(&self, object: Obj) -> Option<usize> {
        self.buffers.get(&(object.addr() as usize)).map(zun_core::ByteBuffer::len)
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
