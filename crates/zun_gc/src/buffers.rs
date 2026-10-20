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
}
