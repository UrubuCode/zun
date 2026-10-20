use super::{Heap, Obj};

impl Heap {
    pub fn error_name(&self, object: Obj) -> Option<&str> {
        self.errors.get(&(object.addr() as usize)).map(zun_core::ErrorValue::name)
    }

    pub fn error_message(&self, object: Obj) -> Option<&str> {
        self.errors.get(&(object.addr() as usize)).map(zun_core::ErrorValue::message)
    }

    pub fn error_set_name(&mut self, object: Obj, name: impl Into<String>) -> Option<()> {
        self.errors.get_mut(&(object.addr() as usize))?.set_name(name);
        Some(())
    }

    pub fn error_set_message(&mut self, object: Obj, message: impl Into<String>) -> Option<()> {
        self.errors.get_mut(&(object.addr() as usize))?.set_message(message);
        Some(())
    }
}
