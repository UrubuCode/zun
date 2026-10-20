use super::{Heap, Obj};

impl Heap {
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

    pub fn u8_array_get(&self, object: Obj, index: usize) -> Option<u8> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize))?;
        view.read_u8(self.buffers.get(&(buffer.addr() as usize))?, index)
    }

    pub fn u8_array_set(&mut self, object: Obj, index: usize, value: u8) -> Option<bool> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize)).copied()?;
        Some(view.write_u8(self.buffers.get_mut(&(buffer.addr() as usize))?, index, value))
    }
}
