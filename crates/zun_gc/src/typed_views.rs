use super::{Heap, Obj};

impl Heap {
    pub fn data_view_len(&self, object: Obj) -> Option<usize> {
        self.data_views.get(&(object.addr() as usize)).map(|view| view.len())
    }

    pub fn typed_array_len(&self, object: Obj) -> Option<usize> {
        self.typed_views.get(&(object.addr() as usize)).map(|(_, view)| view.len())
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

    pub fn f64_array_get(&self, object: Obj, index: usize) -> Option<f64> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize))?;
        view.read_f64(self.buffers.get(&(buffer.addr() as usize))?, index)
    }

    pub fn f64_array_set(&mut self, object: Obj, index: usize, value: f64) -> Option<bool> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize)).copied()?;
        Some(view.write_f64(self.buffers.get_mut(&(buffer.addr() as usize))?, index, value))
    }

    pub fn u16_array_get(&self, object: Obj, index: usize) -> Option<u16> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize))?;
        view.read_u16(self.buffers.get(&(buffer.addr() as usize))?, index)
    }

    pub fn u16_array_set(&mut self, object: Obj, index: usize, value: u16) -> Option<bool> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize)).copied()?;
        Some(view.write_u16(self.buffers.get_mut(&(buffer.addr() as usize))?, index, value))
    }

    pub fn u32_array_get(&self, object: Obj, index: usize) -> Option<u32> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize))?;
        view.read_u32(self.buffers.get(&(buffer.addr() as usize))?, index)
    }

    pub fn u32_array_set(&mut self, object: Obj, index: usize, value: u32) -> Option<bool> {
        let (buffer, view) = self.typed_views.get(&(object.addr() as usize)).copied()?;
        Some(view.write_u32(self.buffers.get_mut(&(buffer.addr() as usize))?, index, value))
    }
}
