//! Frozen contracts shared by the Zun runtime and its standard-library intrinsics.
//!
//! This crate deliberately has no dependency on the interpreter, compiler, or
//! heap implementation. It is the small Rust equivalent of the runtime-owned
//! portion of `System.Private.CoreLib`.

use std::collections::BTreeMap;

/// Size of one raw object slot in bytes.
pub const SLOT_SIZE: usize = 8;

/// Number of slots reserved before an object's first field.
pub const OBJECT_HEADER_SLOTS: usize = 1;

/// Convert an object field slot to its byte offset in the frozen layout.
pub const fn field_offset(slot: usize) -> usize {
    (OBJECT_HEADER_SLOTS + slot) * SLOT_SIZE
}

/// Runtime tag carried by a dynamically typed [`Any`] value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AnyTag {
    /// A signed small integer.
    Int = 0,
    /// An IEEE-754 f64 payload.
    Float = 1,
    /// A boolean encoded as zero or one.
    Bool = 2,
    /// A raw managed-object address.
    Object = 3,
    /// An index into the runtime string table.
    String = 4,
}

/// Explicit tagged-union representation for opt-in dynamic values.
///
/// The payload is deliberately opaque at this layer: the GC owns object
/// addresses and the heap owns string handles. This keeps `zun_core` free of
/// interpreter and collector dependencies while freezing the wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Any {
    tag: AnyTag,
    payload: u64,
}

impl Any {
    /// Box a signed integer.
    pub const fn int(value: i64) -> Self {
        Self {
            tag: AnyTag::Int,
            payload: value as u64,
        }
    }

    /// Box an f64 without changing its bits.
    pub const fn float(value: f64) -> Self {
        Self {
            tag: AnyTag::Float,
            payload: value.to_bits(),
        }
    }

    /// Box a boolean.
    pub const fn bool(value: bool) -> Self {
        Self {
            tag: AnyTag::Bool,
            payload: value as u64,
        }
    }

    /// Box an object address supplied by the GC boundary.
    pub const fn object(address: usize) -> Self {
        Self {
            tag: AnyTag::Object,
            payload: address as u64,
        }
    }

    /// Box a string-table handle supplied by the heap.
    pub const fn string(handle: u32) -> Self {
        Self {
            tag: AnyTag::String,
            payload: handle as u64,
        }
    }

    /// Return the dynamic tag.
    pub const fn tag(self) -> AnyTag {
        self.tag
    }

    /// Unbox an integer if the tag matches.
    pub const fn as_int(self) -> Option<i64> {
        match self.tag {
            AnyTag::Int => Some(self.payload as i64),
            _ => None,
        }
    }

    /// Unbox an f64 if the tag matches.
    pub const fn as_float(self) -> Option<f64> {
        match self.tag {
            AnyTag::Float => Some(f64::from_bits(self.payload)),
            _ => None,
        }
    }

    /// Unbox a boolean if the tag matches.
    pub const fn as_bool(self) -> Option<bool> {
        match self.tag {
            AnyTag::Bool => Some(self.payload != 0),
            _ => None,
        }
    }

    /// Unbox an object address if the tag matches.
    pub const fn as_object(self) -> Option<usize> {
        match self.tag {
            AnyTag::Object => Some(self.payload as usize),
            _ => None,
        }
    }

    /// Unbox a string handle if the tag matches.
    pub const fn as_string(self) -> Option<u32> {
        match self.tag {
            AnyTag::String => Some(self.payload as u32),
            _ => None,
        }
    }
}

/// A dictionary-backed object for opt-in dynamic property access.
///
/// Property storage is deliberately kept out of the static object layout:
/// typed Zun classes continue to use fixed field slots, while dynamic values
/// pay for name lookup only when they opt into this representation. A
/// `BTreeMap` keeps iteration deterministic for serialization and tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DynamicObject {
    properties: BTreeMap<String, Any>,
}

/// Contiguous storage contract for a managed array.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArrayStorage {
    elements: Vec<Any>,
}

impl ArrayStorage {
    /// Create an array with `length` initialized to `undefined`-like void values.
    pub fn with_length(length: usize) -> Self {
        Self {
            elements: vec![Any::int(0); length],
        }
    }

    /// Return the number of elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether the array has no elements.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Read an element with bounds checking.
    pub fn get(&self, index: usize) -> Option<Any> {
        self.elements.get(index).copied()
    }

    /// Replace an element with bounds checking and return its previous value.
    pub fn set(&mut self, index: usize, value: Any) -> Option<Any> {
        let slot = self.elements.get_mut(index)?;
        Some(std::mem::replace(slot, value))
    }

    /// Append an element and return its new zero-based index.
    pub fn push(&mut self, value: Any) -> usize {
        let index = self.elements.len();
        self.elements.push(value);
        index
    }

    /// Remove and return the last element, if any.
    pub fn pop(&mut self) -> Option<Any> {
        self.elements.pop()
    }

    /// Resize the array, filling new slots with the zero numeric sentinel.
    pub fn resize(&mut self, new_length: usize) {
        self.elements.resize(new_length, Any::int(0));
    }

    /// Insert an element at an existing index, or append at `len`.
    pub fn insert(&mut self, index: usize, value: Any) -> bool {
        if index > self.elements.len() {
            return false;
        }
        self.elements.insert(index, value);
        true
    }

    /// Remove an element with bounds checking.
    pub fn remove(&mut self, index: usize) -> Option<Any> {
        (index < self.elements.len()).then(|| self.elements.remove(index))
    }

    /// Iterate over elements without exposing the backing vector.
    pub fn iter(&self) -> impl Iterator<Item = Any> + '_ {
        self.elements.iter().copied()
    }

    /// Return object references stored in the array for precise GC tracing.
    pub fn object_references(&self) -> impl Iterator<Item = usize> + '_ {
        self.elements.iter().filter_map(|value| value.as_object())
    }
}

impl DynamicObject {
    /// Create an empty dynamic object.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a property without removing it.
    pub fn get(&self, name: &str) -> Option<Any> {
        self.properties.get(name).copied()
    }

    /// Insert or replace a property and return its previous value, if any.
    pub fn set(&mut self, name: impl Into<String>, value: Any) -> Option<Any> {
        self.properties.insert(name.into(), value)
    }

    /// Remove a property and return its value, if present.
    pub fn remove(&mut self, name: &str) -> Option<Any> {
        self.properties.remove(name)
    }

    /// Whether the object contains a property with this name.
    pub fn contains(&self, name: &str) -> bool {
        self.properties.contains_key(name)
    }

    /// Number of stored properties.
    pub fn len(&self) -> usize {
        self.properties.len()
    }

    /// Whether the object has no properties.
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }

    /// Return the addresses stored in object-tagged properties.
    pub fn object_references(&self) -> impl Iterator<Item = usize> + '_ {
        self.properties
            .values()
            .filter_map(|value| value.as_object())
    }
}

/// Stable identifier for a runtime intrinsic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct IntrinsicId(pub u32);

impl IntrinsicId {
    /// Construct an intrinsic identifier from its stable registry position.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the positional registry value.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable positional identity for the runtime's built-in types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CoreTypeId {
    /// Root object type.
    Object = 0,
    /// Text/string type.
    String = 1,
    /// JavaScript-compatible numeric type.
    Number = 2,
    /// Boolean type.
    Boolean = 3,
    /// Array type.
    Array = 4,
    /// Function type.
    Function = 5,
}

impl CoreTypeId {
    /// Convert a stable numeric id into a core type id.
    pub const fn new(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Object),
            1 => Some(Self::String),
            2 => Some(Self::Number),
            3 => Some(Self::Boolean),
            4 => Some(Self::Array),
            5 => Some(Self::Function),
            _ => None,
        }
    }

    /// Return the positional id used by the runtime contract.
    pub const fn get(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_layout_offsets_are_stable() {
        assert_eq!(field_offset(0), 8);
        assert_eq!(field_offset(3), 32);
    }

    #[test]
    fn intrinsic_ids_are_positional() {
        assert_eq!(IntrinsicId::new(7).get(), 7);
    }

    #[test]
    fn core_type_ids_are_stable() {
        assert_eq!(CoreTypeId::Object.get(), 0);
        assert_eq!(CoreTypeId::Function.get(), 5);
        assert_eq!(CoreTypeId::new(2), Some(CoreTypeId::Number));
        assert_eq!(CoreTypeId::new(6), None);
    }

    #[test]
    fn any_roundtrips_each_dynamic_tag() {
        assert_eq!(Any::int(-7).as_int(), Some(-7));
        assert_eq!(Any::float(3.5).as_float(), Some(3.5));
        assert_eq!(Any::bool(true).as_bool(), Some(true));
        assert_eq!(Any::object(0x1234).as_object(), Some(0x1234));
        assert_eq!(Any::string(9).as_string(), Some(9));
        assert_eq!(Any::int(1).as_float(), None);
    }

    #[test]
    fn dynamic_object_stores_and_replaces_properties() {
        let mut object = DynamicObject::new();
        assert!(object.is_empty());
        assert_eq!(object.set("answer", Any::int(41)), None);
        assert_eq!(object.get("answer").and_then(Any::as_int), Some(41));
        assert_eq!(object.set("answer", Any::int(42)).and_then(Any::as_int), Some(41));
        assert!(object.contains("answer"));
        assert_eq!(object.remove("answer").and_then(Any::as_int), Some(42));
        assert_eq!(object.len(), 0);
    }

    #[test]
    fn array_storage_is_contiguous_and_bounds_checked() {
        let mut array = ArrayStorage::with_length(2);
        assert_eq!(array.len(), 2);
        assert_eq!(array.get(0).and_then(Any::as_int), Some(0));
        assert_eq!(array.set(1, Any::int(7)).and_then(Any::as_int), Some(0));
        assert_eq!(array.get(1).and_then(Any::as_int), Some(7));
        assert_eq!(array.get(2), None);
        assert_eq!(array.set(2, Any::int(9)), None);
        assert_eq!(array.push(Any::int(11)), 2);
        assert_eq!(array.pop().and_then(Any::as_int), Some(11));
        array.resize(4);
        assert_eq!(array.len(), 4);
        assert_eq!(array.iter().count(), 4);
        array.resize(1);
        assert_eq!(array.len(), 1);
        assert!(array.insert(1, Any::int(13)));
        assert_eq!(array.remove(1).and_then(Any::as_int), Some(13));
        assert!(!array.insert(3, Any::int(14)));
    }
}
