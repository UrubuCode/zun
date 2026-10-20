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
}
