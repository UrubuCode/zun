use super::{Heap, Obj};

impl Heap {
    pub fn dynamic_get(&self, object: Obj, name: &str) -> Option<zun_core::Any> {
        self.dynamic_objects.get(&(object.addr() as usize)).and_then(|properties| properties.get(name))
    }

    pub fn dynamic_get_symbol(&self, object: Obj, symbol: zun_core::SymbolId) -> Option<zun_core::Any> {
        self.dynamic_objects.get(&(object.addr() as usize)).and_then(|properties| properties.get_symbol(symbol))
    }

    pub fn dynamic_set(&mut self, object: Obj, name: impl Into<String>, value: zun_core::Any) -> Option<zun_core::Any> {
        self.dynamic_objects.get_mut(&(object.addr() as usize))?.set(name, value)
    }

    pub fn dynamic_set_symbol(&mut self, object: Obj, symbol: zun_core::SymbolId, value: zun_core::Any) -> Option<zun_core::Any> {
        self.dynamic_objects.get_mut(&(object.addr() as usize))?.set_symbol(symbol, value)
    }

    pub fn dynamic_get_symbol_function(&self, object: Obj, symbol: zun_core::SymbolId) -> Option<zun_core::FunctionHandle> {
        self.dynamic_objects.get(&(object.addr() as usize)).and_then(|properties| properties.get_symbol_function(symbol))
    }

    pub fn dynamic_set_symbol_function(&mut self, object: Obj, symbol: zun_core::SymbolId, function: zun_core::FunctionHandle) -> Option<zun_core::FunctionHandle> {
        self.dynamic_objects.get_mut(&(object.addr() as usize))?.set_symbol_function(symbol, function)
    }

    pub fn dynamic_remove(&mut self, object: Obj, name: &str) -> Option<zun_core::Any> {
        self.dynamic_objects.get_mut(&(object.addr() as usize))?.remove(name)
    }
}
