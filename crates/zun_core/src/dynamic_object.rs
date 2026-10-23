use super::{Any, FunctionHandle, ModuleBoundFunction, SymbolId};
use std::collections::BTreeMap;

/// A dictionary-backed object for opt-in dynamic property access.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DynamicObject {
    properties: BTreeMap<String, Any>,
    symbol_properties: BTreeMap<SymbolId, Any>,
    symbol_functions: BTreeMap<SymbolId, FunctionHandle>,
    string_functions: BTreeMap<String, FunctionHandle>,
    module_functions: BTreeMap<String, ModuleBoundFunction>,
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
    pub fn get_function(&self, name: &str) -> Option<FunctionHandle> {
        self.string_functions.get(name).copied()
    }
    pub fn set_function(&mut self, name: impl Into<String>, function: FunctionHandle) -> Option<FunctionHandle> {
        self.string_functions.insert(name.into(), function)
    }
    pub fn set_symbol_function(&mut self, symbol: SymbolId, function: FunctionHandle) -> Option<FunctionHandle> {
        self.symbol_functions.insert(symbol, function)
    }
    pub fn get_module_function(&self, name: &str) -> Option<&ModuleBoundFunction> {
        self.module_functions.get(name)
    }
    pub fn set_module_function(&mut self, name: impl Into<String>, function: ModuleBoundFunction) -> Option<ModuleBoundFunction> {
        self.module_functions.insert(name.into(), function)
    }
    pub fn module_function_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.module_functions.keys().map(String::as_str)
    }
    pub fn remove(&mut self, name: &str) -> Option<Any> { self.properties.remove(name) }
    pub fn contains(&self, name: &str) -> bool { self.properties.contains_key(name) }
    pub fn len(&self) -> usize { self.properties.len() }
    pub fn is_empty(&self) -> bool { self.properties.is_empty() }
    pub fn object_references(&self) -> impl Iterator<Item = usize> + '_ {
        self.properties.values().filter_map(|value| value.as_object())
            .chain(self.symbol_properties.values().filter_map(|value| value.as_object()))
            .chain(self.symbol_functions.values().filter_map(|value| value.environment()))
            .chain(self.string_functions.values().filter_map(|value| value.environment()))
    }

    pub fn rewrite_object_references(&mut self, map: impl Fn(usize) -> usize + Copy) -> usize {
        let mut rewritten = 0;
        for value in self.properties.values_mut().chain(self.symbol_properties.values_mut()) {
            let mapped = value.map_object_address(map);
            if mapped != *value { rewritten += 1; *value = mapped; }
        }
        rewritten
    }
}
