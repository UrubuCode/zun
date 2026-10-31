use super::{Any, FunctionHandle, ModuleBoundClass, ModuleBoundFunction, SymbolId};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A dictionary-backed object for opt-in dynamic property access.
///
/// String-keyed properties keep **insertion order** (the ECMAScript own-key
/// order for string keys), so `Object.keys`/`values`/`entries` and
/// `getOwnPropertyNames` observe the order in which properties were first
/// added. Lookups stay O(1) through a side index. Symbol-keyed slots and
/// function slots are auxiliary and are not part of the string-key enumeration.
///
/// Each own string key carries the full ECMAScript property-attribute set
/// (NUCLEUS §6):
/// - a **data property** = `{ value, writable, enumerable, configurable }`;
/// - an **accessor property** = `{ get?, set?, enumerable, configurable }`
///   whose `get`/`set` are function values held in the dedicated accessor slots
///   (function values do not box into `Any`).
///
/// The default attributes depend on the creation path: an ordinary assignment
/// (`obj.k = v`, via [`set`](Self::set)) creates an enumerable + writable +
/// configurable data property, while `Object.defineProperty` defaults every
/// omitted flag to `false`. Attributes are stored as "cleared" sets
/// (`non_writable`/`non_enumerable`/`non_configurable`), so a key absent from a
/// set has that attribute set to its default-true value (writable / enumerable
/// / configurable). `non_extensible` blocks adding new keys.
#[derive(Debug, Clone, Default)]
pub struct DynamicObject {
    /// String properties, in insertion order. For an accessor property the
    /// stored `Any` is an unused placeholder (the key is listed in `accessors`
    /// and the functions live in `accessor_get`/`accessor_set`); it is kept in
    /// the vector only so the key participates in own-key ordering.
    properties: Vec<(String, Any)>,
    /// Name → position in `properties`, kept in sync for O(1) access.
    index: HashMap<String, usize>,
    /// Optional `[[Prototype]]` link (`Object.create`/`setPrototypeOf`).
    prototype: Option<Any>,
    /// Fast flag set by `Object.freeze`; also reconciled with the granular
    /// attribute state below (a frozen object is non-extensible and every key
    /// is non-writable + non-configurable).
    frozen: bool,
    /// When set, adding a **new** string key is rejected (`preventExtensions`,
    /// `seal`, `freeze`).
    non_extensible: bool,
    /// Data-property keys whose **writable** attribute is cleared: a plain
    /// assignment (`set`) is rejected for these; the definition path
    /// (`define`/`define_data`) may still redefine a writable or configurable
    /// one. A key absent from the set is writable (the default).
    non_writable: HashSet<String>,
    /// Keys whose **enumerable** attribute is cleared: skipped by
    /// `keys`/`values`/`entries` (and `Object.assign`), still reported by
    /// `getOwnPropertyNames`/`getOwnPropertyDescriptor`.
    non_enumerable: HashSet<String>,
    /// Keys whose **configurable** attribute is cleared: re-`defineProperty`
    /// and `delete` are rejected for these.
    non_configurable: HashSet<String>,
    /// Keys that are **accessor** properties (not data properties). Their entry
    /// in `properties` is a placeholder; the functions are in the maps below.
    accessors: HashSet<String>,
    /// Getter functions for accessor properties.
    accessor_get: BTreeMap<String, FunctionHandle>,
    /// Setter functions for accessor properties.
    accessor_set: BTreeMap<String, FunctionHandle>,
    symbol_properties: BTreeMap<SymbolId, Any>,
    /// Symbol keys in first-insertion order (the order own-key enumeration reports).
    symbol_order: Vec<SymbolId>,
    /// Symbol keys whose **enumerable** attribute is cleared (skipped by spread
    /// and `Object.assign`).
    non_enumerable_symbols: HashSet<SymbolId>,
    symbol_functions: BTreeMap<SymbolId, FunctionHandle>,
    /// Getter functions of symbol-keyed accessor properties.
    symbol_getters: BTreeMap<SymbolId, FunctionHandle>,
    string_functions: BTreeMap<String, FunctionHandle>,
    module_functions: BTreeMap<String, ModuleBoundFunction>,
    module_classes: BTreeMap<String, ModuleBoundClass>,
}

impl PartialEq for DynamicObject {
    fn eq(&self, other: &Self) -> bool {
        self.properties == other.properties
            && self.prototype == other.prototype
            && self.frozen == other.frozen
            && self.non_extensible == other.non_extensible
            && self.non_writable == other.non_writable
            && self.non_enumerable == other.non_enumerable
            && self.non_configurable == other.non_configurable
            && self.accessors == other.accessors
            && self.accessor_get == other.accessor_get
            && self.accessor_set == other.accessor_set
            && self.symbol_properties == other.symbol_properties
            && self.non_enumerable_symbols == other.non_enumerable_symbols
            && self.symbol_functions == other.symbol_functions
            && self.symbol_getters == other.symbol_getters
            && self.string_functions == other.string_functions
            && self.module_functions == other.module_functions
            && self.module_classes == other.module_classes
    }
}

impl Eq for DynamicObject {}

/// A read-only view of one own string property's attributes, for
/// `Object.getOwnPropertyDescriptor`/`getOwnPropertyDescriptors`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyDescriptor {
    Data { value: Any, writable: bool, enumerable: bool, configurable: bool },
    Accessor {
        get: Option<FunctionHandle>,
        set: Option<FunctionHandle>,
        enumerable: bool,
        configurable: bool,
    },
}

/// The canonical array index of `name` (`"0"`, `"17"`, no leading zeros, below
/// 2^32 - 1), if it is one. Such keys enumerate first, in ascending order.
fn array_index(name: &str) -> Option<u32> {
    if name.is_empty() || name.len() > 10 || (name.len() > 1 && name.starts_with('0')) {
        return None;
    }
    if !name.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    name.parse::<u64>().ok().filter(|&n| n < u32::MAX as u64).map(|n| n as u32)
}

impl DynamicObject {
    pub fn new() -> Self { Self::default() }
    /// Append a brand-new string key in ECMAScript own-key order: canonical
    /// array indices first (ascending), then other strings by insertion.
    fn insert_new(&mut self, name: String, value: Any) {
        let position = match array_index(&name) {
            Some(new_index) => self
                .properties
                .iter()
                .position(|(key, _)| array_index(key).is_none_or(|existing| existing > new_index))
                .unwrap_or(self.properties.len()),
            None => self.properties.len(),
        };
        if position < self.properties.len() {
            for slot in self.index.values_mut() {
                if *slot >= position {
                    *slot += 1;
                }
            }
        }
        self.index.insert(name.clone(), position);
        self.properties.insert(position, (name, value));
    }
    /// Read a data property's value. Returns `None` for an absent key **or an
    /// accessor property** (reading an accessor requires invoking its getter,
    /// which happens a layer up in the interpreter).
    pub fn get(&self, name: &str) -> Option<Any> {
        if self.accessors.contains(name) {
            return None;
        }
        self.index.get(name).map(|&i| self.properties[i].1)
    }
    /// Insert or update a string data property through the ordinary assignment
    /// path (`obj.k = v`). Rejection returns `None` (non-throwing, matching
    /// `Object.freeze`): a new key is rejected on a non-extensible/frozen
    /// object; an existing non-writable data key (or any key on a frozen
    /// object) rejects the overwrite; an accessor key is left to the
    /// interpreter's setter path (no data value is written here).
    pub fn set(&mut self, name: impl Into<String>, value: Any) -> Option<Any> {
        let name = name.into();
        if let Some(&i) = self.index.get(&name) {
            if self.accessors.contains(&name) {
                // Accessor property: the data store does not apply; the
                // interpreter invokes the setter (if any) separately.
                return None;
            }
            if self.frozen || self.non_writable.contains(&name) {
                return None;
            }
            return Some(std::mem::replace(&mut self.properties[i].1, value));
        }
        if self.frozen || self.non_extensible {
            return None;
        }
        self.insert_new(name, value);
        None
    }
    /// Define or redefine a string data property with an explicit `writable`
    /// attribute, leaving `enumerable`/`configurable` at their defaults (true).
    /// This is the legacy definition path backing `global.set`. A frozen object
    /// rejects it (returns `false`); a non-extensible object rejects adding a
    /// new key. Otherwise it inserts or overwrites and records writability.
    pub fn define(&mut self, name: impl Into<String>, value: Any, writable: bool) -> bool {
        let name = name.into();
        if self.frozen {
            return false;
        }
        let exists = self.index.contains_key(&name);
        if !exists && self.non_extensible {
            return false;
        }
        self.define_data(name, value, writable, true, true)
    }
    /// The full-descriptor definition path for `Object.defineProperty` with a
    /// **data** descriptor. Enforces configurability (an existing
    /// non-configurable key is rejected) and extensibility (a new key on a
    /// non-extensible object is rejected). Returns `true` on success.
    pub fn define_data(
        &mut self,
        name: impl Into<String>,
        value: Any,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    ) -> bool {
        let name = name.into();
        let exists = self.index.contains_key(&name);
        if exists {
            if self.non_configurable.contains(&name) {
                return false;
            }
        } else if self.non_extensible {
            return false;
        }
        self.accessors.remove(&name);
        self.accessor_get.remove(&name);
        self.accessor_set.remove(&name);
        if let Some(&i) = self.index.get(&name) {
            self.properties[i].1 = value;
        } else {
            self.insert_new(name.clone(), value);
        }
        self.apply_flags(&name, Some(writable), enumerable, configurable);
        true
    }
    /// The full-descriptor definition path for `Object.defineProperty` with an
    /// **accessor** descriptor. Same configurability/extensibility rules as
    /// [`define_data`](Self::define_data). A `None` getter/setter means the
    /// attribute is absent.
    pub fn define_accessor(
        &mut self,
        name: impl Into<String>,
        get: Option<FunctionHandle>,
        set: Option<FunctionHandle>,
        enumerable: bool,
        configurable: bool,
    ) -> bool {
        let name = name.into();
        let exists = self.index.contains_key(&name);
        if exists {
            if self.non_configurable.contains(&name) {
                return false;
            }
        } else if self.non_extensible {
            return false;
        }
        if !self.index.contains_key(&name) {
            self.insert_new(name.clone(), Any::null());
        }
        self.accessors.insert(name.clone());
        match get {
            Some(handle) => { self.accessor_get.insert(name.clone(), handle); }
            None => { self.accessor_get.remove(&name); }
        }
        match set {
            Some(handle) => { self.accessor_set.insert(name.clone(), handle); }
            None => { self.accessor_set.remove(&name); }
        }
        self.apply_flags(&name, None, enumerable, configurable);
        true
    }
    /// Redefine a property after the caller validated the change against the
    /// current descriptor (skips the non-configurable guard of
    /// [`define_data`](Self::define_data)).
    pub fn force_define_data(
        &mut self,
        name: impl Into<String>,
        value: Any,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    ) -> bool {
        let name = name.into();
        self.non_configurable.remove(&name);
        self.define_data(name, value, writable, enumerable, configurable)
    }
    /// Accessor counterpart of [`force_define_data`](Self::force_define_data).
    pub fn force_define_accessor(
        &mut self,
        name: impl Into<String>,
        get: Option<FunctionHandle>,
        set: Option<FunctionHandle>,
        enumerable: bool,
        configurable: bool,
    ) -> bool {
        let name = name.into();
        self.non_configurable.remove(&name);
        self.define_accessor(name, get, set, enumerable, configurable)
    }
    /// Record the `writable`/`enumerable`/`configurable` cleared-attribute sets
    /// for `name`. `writable` is `None` for an accessor property (writability
    /// does not apply).
    fn apply_flags(&mut self, name: &str, writable: Option<bool>, enumerable: bool, configurable: bool) {
        match writable {
            Some(true) | None => { self.non_writable.remove(name); }
            Some(false) => { self.non_writable.insert(name.to_owned()); }
        }
        if enumerable { self.non_enumerable.remove(name); } else { self.non_enumerable.insert(name.to_owned()); }
        if configurable { self.non_configurable.remove(name); } else { self.non_configurable.insert(name.to_owned()); }
    }
    /// Whether `name` may be reassigned through the ordinary [`set`](Self::set)
    /// path. An absent key is writable (a plain assignment would create it).
    pub fn is_writable(&self, name: &str) -> bool {
        !self.non_writable.contains(name)
    }
    /// Whether `name` is an enumerable own property (default true). An absent
    /// key reports `false`.
    pub fn is_enumerable(&self, name: &str) -> bool {
        self.index.contains_key(name) && !self.non_enumerable.contains(name)
    }
    /// Whether `name` is configurable (default true). An absent key reports
    /// `false`.
    pub fn is_configurable(&self, name: &str) -> bool {
        self.index.contains_key(name) && !self.non_configurable.contains(name)
    }
    /// Whether `name` is an accessor (get/set) property.
    pub fn is_accessor(&self, name: &str) -> bool { self.accessors.contains(name) }
    pub fn accessor_getter(&self, name: &str) -> Option<FunctionHandle> {
        self.accessor_get.get(name).copied()
    }
    pub fn accessor_setter(&self, name: &str) -> Option<FunctionHandle> {
        self.accessor_set.get(name).copied()
    }
    /// The own descriptor for `name`, or `None` if there is no own property.
    pub fn descriptor(&self, name: &str) -> Option<PropertyDescriptor> {
        let &i = self.index.get(name)?;
        let enumerable = !self.non_enumerable.contains(name);
        let configurable = !self.non_configurable.contains(name);
        if self.accessors.contains(name) {
            Some(PropertyDescriptor::Accessor {
                get: self.accessor_get.get(name).copied(),
                set: self.accessor_set.get(name).copied(),
                enumerable,
                configurable,
            })
        } else {
            Some(PropertyDescriptor::Data {
                value: self.properties[i].1,
                writable: !self.non_writable.contains(name),
                enumerable,
                configurable,
            })
        }
    }
    /// All own string-data-property keys, in insertion order
    /// (`getOwnPropertyNames`): both enumerable and non-enumerable, data and
    /// accessor.
    pub fn keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.properties.iter().map(|(name, _)| name.as_str())
    }
    /// Own **enumerable** string-data-property keys, in insertion order
    /// (`Object.keys`).
    pub fn enumerable_keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.properties
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(move |name| !self.non_enumerable.contains(*name))
    }
    /// Own **enumerable** data entries, in insertion order
    /// (`Object.values`/`entries`/`assign`). Accessor properties are skipped
    /// (their value is produced by invoking the getter, a layer up).
    pub fn entries(&self) -> impl Iterator<Item = (&str, Any)> + '_ {
        self.properties
            .iter()
            .filter(move |(name, _)| {
                !self.non_enumerable.contains(name) && !self.accessors.contains(name)
            })
            .map(|(name, value)| (name.as_str(), *value))
    }
    pub fn is_frozen(&self) -> bool {
        if self.frozen {
            return true;
        }
        // Granular freeze: non-extensible and every own key non-configurable,
        // with every data key non-writable.
        self.non_extensible
            && self.properties.iter().all(|(name, _)| {
                self.non_configurable.contains(name)
                    && (self.accessors.contains(name) || self.non_writable.contains(name))
            })
    }
    /// `Object.freeze`: non-extensible, every own key non-writable (data) and
    /// non-configurable.
    pub fn freeze(&mut self) {
        self.frozen = true;
        self.non_extensible = true;
        let names: Vec<String> = self.properties.iter().map(|(n, _)| n.clone()).collect();
        for name in names {
            if !self.accessors.contains(&name) {
                self.non_writable.insert(name.clone());
            }
            self.non_configurable.insert(name);
        }
    }
    /// `Object.seal`: non-extensible, every own key non-configurable (values of
    /// data properties stay writable).
    pub fn seal(&mut self) {
        self.non_extensible = true;
        let names: Vec<String> = self.properties.iter().map(|(n, _)| n.clone()).collect();
        for name in names {
            self.non_configurable.insert(name);
        }
    }
    /// Whether every own key is non-configurable and the object is
    /// non-extensible (`Object.isSealed`).
    pub fn is_sealed(&self) -> bool {
        self.non_extensible
            && self.properties.iter().all(|(name, _)| self.non_configurable.contains(name))
    }
    /// `Object.preventExtensions`.
    pub fn prevent_extensions(&mut self) { self.non_extensible = true; }
    /// Whether new keys may be added (`Object.isExtensible`).
    pub fn is_extensible(&self) -> bool { !self.non_extensible && !self.frozen }
    pub fn prototype(&self) -> Option<Any> { self.prototype }
    pub fn set_prototype(&mut self, prototype: Option<Any>) { self.prototype = prototype; }
    pub fn get_symbol(&self, symbol: SymbolId) -> Option<Any> {
        self.symbol_properties.get(&symbol).copied()
    }
    /// The object's own symbol keys in a stable order (the id order of the
    /// backing store). Backs `Object.getOwnPropertySymbols` and, later,
    /// symbol-key copying for object spread and serialization.
    pub fn symbol_keys(&self) -> impl Iterator<Item = SymbolId> + '_ {
        self.symbol_order.iter().copied().filter(|symbol| self.symbol_properties.contains_key(symbol))
    }
    /// Define a symbol-keyed data property with an explicit enumerable flag
    /// (`Object.defineProperty(o, sym, ...)`). Rejected on a non-extensible
    /// object when the key is new.
    pub fn define_symbol(&mut self, symbol: SymbolId, value: Any, enumerable: bool) -> bool {
        if !self.symbol_properties.contains_key(&symbol) && (self.non_extensible || self.frozen) {
            return false;
        }
        if !self.symbol_properties.contains_key(&symbol) { self.symbol_order.push(symbol); }
        self.symbol_properties.insert(symbol, value);
        if enumerable { self.non_enumerable_symbols.remove(&symbol); } else { self.non_enumerable_symbols.insert(symbol); }
        true
    }
    /// Whether the symbol key is an own enumerable property.
    pub fn is_symbol_enumerable(&self, symbol: SymbolId) -> bool {
        self.symbol_properties.contains_key(&symbol) && !self.non_enumerable_symbols.contains(&symbol)
    }
    /// Whether a plain assignment `obj.name = v` would be accepted by the data
    /// path (strict-mode callers throw when this is `false`). An accessor key
    /// is accepted here (its setter is dispatched separately).
    pub fn can_set(&self, name: &str) -> bool {
        if self.index.contains_key(name) {
            self.accessors.contains(name) || !(self.frozen || self.non_writable.contains(name))
        } else {
            !(self.frozen || self.non_extensible)
        }
    }
    /// Whether `delete obj.name` succeeds (absent keys succeed vacuously).
    pub fn can_delete(&self, name: &str) -> bool {
        !self.index.contains_key(name) || !(self.frozen || self.non_configurable.contains(name))
    }
    pub fn set_symbol(&mut self, symbol: SymbolId, value: Any) -> Option<Any> {
        if !self.symbol_properties.contains_key(&symbol) { self.symbol_order.push(symbol); }
        self.symbol_properties.insert(symbol, value)
    }
    pub fn get_symbol_getter(&self, symbol: SymbolId) -> Option<FunctionHandle> {
        self.symbol_getters.get(&symbol).copied()
    }
    pub fn set_symbol_getter(&mut self, symbol: SymbolId, function: FunctionHandle) -> Option<FunctionHandle> {
        self.symbol_getters.insert(symbol, function)
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
    pub fn get_module_class(&self, name: &str) -> Option<&ModuleBoundClass> {
        self.module_classes.get(name)
    }
    pub fn set_module_class(&mut self, name: impl Into<String>, class: ModuleBoundClass) -> Option<ModuleBoundClass> {
        self.module_classes.insert(name.into(), class)
    }
    /// Remove a string property. A frozen object, or a **non-configurable**
    /// property, rejects removal (returns `None`).
    pub fn remove(&mut self, name: &str) -> Option<Any> {
        if self.frozen || self.non_configurable.contains(name) {
            return None;
        }
        let position = self.index.remove(name)?;
        self.non_writable.remove(name);
        self.non_enumerable.remove(name);
        self.non_configurable.remove(name);
        self.accessors.remove(name);
        self.accessor_get.remove(name);
        self.accessor_set.remove(name);
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
            .chain(self.symbol_getters.values().filter_map(|value| value.environment()))
            .chain(self.string_functions.values().filter_map(|value| value.environment()))
            .chain(self.accessor_get.values().filter_map(|value| value.environment()))
            .chain(self.accessor_set.values().filter_map(|value| value.environment()))
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
