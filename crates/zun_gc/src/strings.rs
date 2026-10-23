use std::rc::Rc;

use super::Heap;

impl Heap {
    pub fn intern_string(&mut self, s: Rc<str>) -> u32 {
        let handle = self.strings.len() as u32;
        self.strings.push(s);
        handle
    }

    pub fn string(&self, handle: u32) -> &Rc<str> {
        &self.strings[handle as usize]
    }

    pub fn try_string(&self, handle: u32) -> Option<&Rc<str>> {
        self.strings.get(handle as usize)
    }

    pub fn symbol_create(&mut self, description: Option<impl Into<String>>) -> zun_core::SymbolId {
        self.symbols.create(description)
    }

    pub fn symbol_for(&mut self, key: impl Into<String>) -> zun_core::SymbolId {
        self.symbols.for_key(key)
    }

    pub fn symbol_description(&self, symbol: zun_core::SymbolId) -> Option<&str> {
        self.symbols.description(symbol)
    }

    pub fn symbol_key_for(&self, symbol: zun_core::SymbolId) -> Option<&str> {
        self.symbols.key_for(symbol)
    }

    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }
}
