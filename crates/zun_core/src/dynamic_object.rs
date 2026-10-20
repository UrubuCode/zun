use super::{Any, FunctionHandle, SymbolId};
use std::collections::BTreeMap;

/// A dictionary-backed object for opt-in dynamic property access.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DynamicObject {
    properties: BTreeMap<String, Any>,
    symbol_properties: BTreeMap<SymbolId, Any>,
    symbol_functions: BTreeMap<SymbolId, FunctionHandle>,
}

impl DynamicObject {
    pub fn new() -> Self { Self::default() }
    pub fn get(&self, name: &str) -> Option<Any> { self.properties.get(name).copied() }
    pub fn set(&mut self, name: impl Into<String>, value: Any) -> Option<Any> {
        self.properties.insert(name.into(), value)
    }
    pub fn get_symbol(&self, symbol: SymbolId) -> Option<Any> {
        self.symbol_properties.get(&symbol).copied()
    }
    pub fn set_symbol(&mut self, symbol: SymbolId, value: Any) -> Option<Any> {
        self.symbol_properties.insert(symbol, value)
    }
    pub fn get_symbol_function(&self, symbol: SymbolId) -> Option<FunctionHandle> {
        self.symbol_functions.get(&symbol).copied()
    }
    pub fn set_symbol_function(&mut self, symbol: SymbolId, function: FunctionHandle) -> Option<FunctionHandle> {
        self.symbol_functions.insert(symbol, function)
    }
    pub fn remove(&mut self, name: &str) -> Option<Any> { self.properties.remove(name) }
    pub fn contains(&self, name: &str) -> bool { self.properties.contains_key(name) }
    pub fn len(&self) -> usize { self.properties.len() }
    pub fn is_empty(&self) -> bool { self.properties.is_empty() }
    pub fn object_references(&self) -> impl Iterator<Item = usize> + '_ {
        self.properties.values().filter_map(|value| value.as_object())
            .chain(self.symbol_properties.values().filter_map(|value| value.as_object()))
            .chain(self.symbol_functions.values().filter_map(|value| value.environment()))
    }
}
