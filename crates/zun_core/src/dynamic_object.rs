use super::{Any, FunctionHandle, ModuleBoundFunction, SymbolId};
use std::collections::{BTreeMap, HashMap};

/// A dictionary-backed object for opt-in dynamic property access.
///
/// String-keyed data properties keep **insertion order** (the ECMAScript
/// own-key order for string keys), so `Object.keys`/`values`/`entries` observe
/// the order in which properties were first added. Lookups stay O(1) through a
/// side index. Symbol-keyed slots and function slots are auxiliary and are not
/// part of the string-key enumeration.
#[derive(Debug, Clone, Default)]
pub struct DynamicObject {
    /// String data properties, in insertion order.
    properties: Vec<(String, Any)>,
    /// Name → position in `properties`, kept in sync for O(1) access.
    index: HashMap<String, usize>,
    /// Optional `[[Prototype]]` link (`Object.create`/`setPrototypeOf`).
    prototype: Option<Any>,
    /// When set, the dynamic path rejects adds and mutations (`Object.freeze`).
    frozen: bool,
    symbol_properties: BTreeMap<SymbolId, Any>,
    symbol_functions: BTreeMap<SymbolId, FunctionHandle>,
    string_functions: BTreeMap<String, FunctionHandle>,
    module_functions: BTreeMap<String, ModuleBoundFunction>,
}

impl PartialEq for DynamicObject {
    fn eq(&self, other: &Self) -> bool {
        self.properties == other.properties
            && self.prototype == other.prototype
            && self.frozen == other.frozen
            && self.symbol_properties == other.symbol_properties
            && self.symbol_functions == other.symbol_functions
            && self.string_functions == other.string_functions
            && self.module_functions == other.module_functions
    }
}

impl Eq for DynamicObject {}

impl DynamicObject {
    pub fn new() -> Self { Self::default() }
    pub fn get(&self, name: &str) -> Option<Any> {
        self.index.get(name).map(|&i| self.properties[i].1)
    }
    /// Insert or update a string data property. A frozen object rejects both
    /// adding a new key and overwriting an existing one, returning `None`.
    pub fn set(&mut self, name: impl Into<String>, value: Any) -> Option<Any> {
        let name = name.into();
        if let Some(&i) = self.index.get(&name) {
            if self.frozen {
                return None;
            }
            return Some(std::mem::replace(&mut self.properties[i].1, value));
        }
        if self.frozen {
            return None;
        }
        self.index.insert(name.clone(), self.properties.len());
        self.properties.push((name, value));
        None
    }
    /// String data-property keys, in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.properties.iter().map(|(name, _)| name.as_str())
    }
    /// String data-property entries, in insertion order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, Any)> + '_ {
        self.properties.iter().map(|(name, value)| (name.as_str(), *value))
    }
    pub fn is_frozen(&self) -> bool { self.frozen }
    pub fn freeze(&mut self) { self.frozen = true; }
    pub fn prototype(&self) -> Option<Any> { self.prototype }
    pub fn set_prototype(&mut self, prototype: Option<Any>) { self.prototype = prototype; }
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
    /// Remove a string data property. A frozen object rejects removal.
    pub fn remove(&mut self, name: &str) -> Option<Any> {
        if self.frozen {
            return None;
        }
        let position = self.index.remove(name)?;
        let (_, value) = self.properties.remove(position);
        for slot in self.index.values_mut() {
            if *slot > position {
                *slot -= 1;
            }
        }
        Some(value)
    }
    pub fn contains(&self, name: &str) -> bool { self.index.contains_key(name) }
    pub fn len(&self) -> usize { self.properties.len() }
    pub fn is_empty(&self) -> bool { self.properties.is_empty() }
    pub fn object_references(&self) -> impl Iterator<Item = usize> + '_ {
        self.properties.iter().filter_map(|(_, value)| value.as_object())
            .chain(self.prototype.iter().filter_map(|value| value.as_object()))
            .chain(self.symbol_properties.values().filter_map(|value| value.as_object()))
            .chain(self.symbol_functions.values().filter_map(|value| value.environment()))
            .chain(self.string_functions.values().filter_map(|value| value.environment()))
    }

    pub fn rewrite_object_references(&mut self, map: impl Fn(usize) -> usize + Copy) -> usize {
        let mut rewritten = 0;
        for (_, value) in self.properties.iter_mut() {
            let mapped = value.map_object_address(map);
            if mapped != *value { rewritten += 1; *value = mapped; }
        }
        if let Some(prototype) = self.prototype.as_mut() {
            let mapped = prototype.map_object_address(map);
            if mapped != *prototype { rewritten += 1; *prototype = mapped; }
        }
        for value in self.symbol_properties.values_mut() {
            let mapped = value.map_object_address(map);
            if mapped != *value { rewritten += 1; *value = mapped; }
        }
        rewritten
    }
}
