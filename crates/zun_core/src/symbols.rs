use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SymbolId(u64);

impl SymbolId {
    pub const fn get(self) -> u64 { self.0 }
}

#[derive(Debug, Default)]
pub struct SymbolRegistry {
    descriptions: Vec<Option<String>>,
    global: HashMap<String, SymbolId>,
}

impl SymbolRegistry {
    pub fn new() -> Self { Self::default() }
    pub fn create(&mut self, description: Option<impl Into<String>>) -> SymbolId {
        let id = SymbolId(self.descriptions.len() as u64);
        self.descriptions.push(description.map(Into::into));
        id
    }
    pub fn for_key(&mut self, key: impl Into<String>) -> SymbolId {
        let key = key.into();
        if let Some(&id) = self.global.get(&key) { return id; }
        let id = self.create(Some(key.clone()));
        self.global.insert(key, id);
        id
    }
    pub fn iterator(&mut self) -> SymbolId { self.for_key("Symbol.iterator") }
    pub fn description(&self, id: SymbolId) -> Option<&str> {
        self.descriptions.get(id.0 as usize).and_then(Option::as_deref)
    }
    pub fn len(&self) -> usize { self.descriptions.len() }
}
