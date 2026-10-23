use super::{Heap, Obj};

impl Heap {
    pub fn dynamic_get(&self, object: Obj, name: &str) -> Option<zun_core::Any> {
        self.dynamic_objects.get(&(object.addr() as usize)).and_then(|properties| properties.get(name))
    }

    pub fn dynamic_has(&self, object: Obj, name: &str) -> bool {
        self.dynamic_objects
            .get(&(object.addr() as usize))
            .is_some_and(|properties| properties.get(name).is_some())
    }

    pub fn dynamic_get_symbol(&self, object: Obj, symbol: zun_core::SymbolId) -> Option<zun_core::Any> {
        self.dynamic_objects.get(&(object.addr() as usize)).and_then(|properties| properties.get_symbol(symbol))
    }

    pub fn dynamic_set(&mut self, object: Obj, name: impl Into<String>, value: zun_core::Any) -> Option<zun_core::Any> {
        let result = self.dynamic_objects.get_mut(&(object.addr() as usize))?.set(name, value);
        if let Some(child) = value.as_object() {
            self.write_barrier(object, child as u64);
        }
        result
    }

    pub fn dynamic_set_symbol(&mut self, object: Obj, symbol: zun_core::SymbolId, value: zun_core::Any) -> Option<zun_core::Any> {
        let result = self.dynamic_objects.get_mut(&(object.addr() as usize))?.set_symbol(symbol, value);
        if let Some(child) = value.as_object() {
            self.write_barrier(object, child as u64);
        }
        result
    }

    pub fn dynamic_get_symbol_function(&self, object: Obj, symbol: zun_core::SymbolId) -> Option<zun_core::FunctionHandle> {
        self.dynamic_objects.get(&(object.addr() as usize)).and_then(|properties| properties.get_symbol_function(symbol))
    }
    pub fn dynamic_get_function(&self, object: Obj, name: &str) -> Option<zun_core::FunctionHandle> {
        self.dynamic_objects
            .get(&(object.addr() as usize))
            .and_then(|properties| properties.get_function(name))
    }

    pub fn dynamic_set_symbol_function(&mut self, object: Obj, symbol: zun_core::SymbolId, function: zun_core::FunctionHandle) -> Option<zun_core::FunctionHandle> {
        let result = self.dynamic_objects.get_mut(&(object.addr() as usize))?.set_symbol_function(symbol, function);
        if let Some(environment) = function.environment() {
            self.write_barrier(object, environment as u64);
        }
        result
    }
    pub fn dynamic_set_function(&mut self, object: Obj, name: impl Into<String>, function: zun_core::FunctionHandle) -> Option<zun_core::FunctionHandle> {
        let result = self.dynamic_objects
            .get_mut(&(object.addr() as usize))
            .and_then(|properties| properties.set_function(name, function));
        if let Some(environment) = function.environment() {
            self.write_barrier(object, environment as u64);
        }
        result
    }

    pub fn dynamic_get_module_function(&self, object: Obj, name: &str) -> Option<zun_core::ModuleBoundFunction> {
        self.dynamic_objects
            .get(&(object.addr() as usize))
            .and_then(|properties| properties.get_module_function(name).cloned())
    }

    pub fn dynamic_set_module_function(&mut self, object: Obj, name: impl Into<String>, function: zun_core::ModuleBoundFunction) -> Option<zun_core::ModuleBoundFunction> {
        self.dynamic_objects
            .get_mut(&(object.addr() as usize))
            .and_then(|properties| properties.set_module_function(name, function))
    }

    pub fn dynamic_remove(&mut self, object: Obj, name: &str) -> Option<zun_core::Any> {
        self.dynamic_objects.get_mut(&(object.addr() as usize))?.remove(name)
    }

    pub fn dynamic_delete(&mut self, object: Obj, name: &str) -> bool {
        self.dynamic_remove(object, name).is_some()
    }

    /// Whether `object` is a dictionary-backed dynamic object (as opposed to a
    /// fixed-layout typed object).
    pub fn is_dynamic_object(&self, object: Obj) -> bool {
        self.dynamic_objects.contains_key(&(object.addr() as usize))
    }

    /// Own string data-property keys of a dynamic object, in insertion order.
    pub fn dynamic_keys(&self, object: Obj) -> Option<Vec<String>> {
        self.dynamic_objects
            .get(&(object.addr() as usize))
            .map(|properties| properties.keys().map(str::to_owned).collect())
    }

    /// Own string data-property entries of a dynamic object, in insertion order.
    pub fn dynamic_entries(&self, object: Obj) -> Option<Vec<(String, zun_core::Any)>> {
        self.dynamic_objects
            .get(&(object.addr() as usize))
            .map(|properties| {
                properties
                    .entries()
                    .map(|(name, value)| (name.to_owned(), value))
                    .collect()
            })
    }

    /// Freeze a dynamic object: later adds/mutations through the dynamic path
    /// are rejected. Returns `false` if `object` is not a dynamic object.
    pub fn dynamic_freeze(&mut self, object: Obj) -> bool {
        match self.dynamic_objects.get_mut(&(object.addr() as usize)) {
            Some(properties) => {
                properties.freeze();
                true
            }
            None => false,
        }
    }

    /// Whether a dynamic object has been frozen.
    pub fn dynamic_is_frozen(&self, object: Obj) -> bool {
        self.dynamic_objects
            .get(&(object.addr() as usize))
            .is_some_and(|properties| properties.is_frozen())
    }

    /// The `[[Prototype]]` link of a dynamic object, if any.
    pub fn dynamic_prototype(&self, object: Obj) -> Option<zun_core::Any> {
        self.dynamic_objects
            .get(&(object.addr() as usize))
            .and_then(|properties| properties.prototype())
    }

    /// Set the `[[Prototype]]` link of a dynamic object. Returns `false` if
    /// `object` is not a dynamic object.
    pub fn dynamic_set_prototype(&mut self, object: Obj, prototype: Option<zun_core::Any>) -> bool {
        let Some(properties) = self.dynamic_objects.get_mut(&(object.addr() as usize)) else {
            return false;
        };
        properties.set_prototype(prototype);
        if let Some(child) = prototype.and_then(|value| value.as_object()) {
            self.write_barrier(object, child as u64);
        }
        true
    }
}
