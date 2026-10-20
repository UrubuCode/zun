//! Frozen contracts shared by the Zun runtime and its standard-library intrinsics.
//!
//! This crate deliberately has no dependency on the interpreter, compiler, or
//! heap implementation. It is the small Rust equivalent of the runtime-owned
//! portion of `System.Private.CoreLib`.

use std::collections::{BTreeMap, HashMap};

/// Size of one raw object slot in bytes.
pub const SLOT_SIZE: usize = 8;

/// Number of slots reserved before an object's first field.
pub const OBJECT_HEADER_SLOTS: usize = 2;

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

/// Runtime identity for a Symbol. Symbols with the same description are still
/// distinct unless obtained through the global registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SymbolId(u64);

impl SymbolId {
    pub const fn get(self) -> u64 { self.0 }
}

/// Per-isolate symbol storage. The registry is deliberately separate from
/// ordinary string interning: Symbol descriptions do not define identity.
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
        if let Some(&id) = self.global.get(&key) {
            return id;
        }
        let id = self.create(Some(key.clone()));
        self.global.insert(key, id);
        id
    }

    pub fn description(&self, id: SymbolId) -> Option<&str> {
        self.descriptions.get(id.0 as usize).and_then(Option::as_deref)
    }

    pub fn len(&self) -> usize { self.descriptions.len() }
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

/// An insertion-ordered hash table used by the runtime-backed Map/Set types.
///
/// Hashing and SameValueZero equality are supplied by the runtime because
/// object keys need the GC-owned identity hash. Entries remain in insertion
/// order independently of bucket layout.
#[derive(Debug, Clone)]
pub struct OrderedMap {
    entries: Vec<Option<(Any, Any, u64)>>,
    buckets: Vec<Vec<usize>>,
    live_len: usize,
    dead_len: usize,
}

impl Default for OrderedMap {
    fn default() -> Self {
        Self::new()
    }
}

impl OrderedMap {
    pub fn new() -> Self {
        Self { entries: Vec::new(), buckets: vec![Vec::new(); 8], live_len: 0, dead_len: 0 }
    }

    pub fn len(&self) -> usize { self.live_len }
    pub fn is_empty(&self) -> bool { self.live_len == 0 }

    pub fn get(&self, hash: u64, equal: impl Fn(Any) -> bool) -> Option<Any> {
        let bucket = &self.buckets[(hash as usize) % self.buckets.len()];
        bucket.iter().find_map(|&index| {
            self.entries[index].and_then(|(key, value, _)| equal(key).then_some(value))
        })
    }

    pub fn contains(&self, hash: u64, equal: impl Fn(Any) -> bool) -> bool {
        self.get(hash, equal).is_some()
    }

    pub fn insert(&mut self, key: Any, value: Any, hash: u64, equal: impl Fn(Any) -> bool) -> Option<Any> {
        if self.dead_len >= 8 && self.dead_len > self.live_len {
            self.compact();
        }
        let bucket_index = (hash as usize) % self.buckets.len();
        if let Some(&index) = self.buckets[bucket_index].iter().find(|&&index| {
            self.entries[index].is_some_and(|(key, _, _)| equal(key))
        }) {
            return Some(self.entries[index].as_mut().map(|entry| std::mem::replace(&mut entry.1, value)).unwrap());
        }
        let index = self.entries.len();
        self.entries.push(Some((key, value, hash)));
        self.buckets[bucket_index].push(index);
        self.live_len += 1;
        if self.live_len * 4 > self.buckets.len() * 3 {
            self.rehash();
        }
        None
    }

    pub fn remove(&mut self, hash: u64, equal: impl Fn(Any) -> bool) -> Option<(Any, Any)> {
        let bucket_index = (hash as usize) % self.buckets.len();
        let &index = self.buckets[bucket_index].iter().find(|&&index| {
            self.entries[index].is_some_and(|(key, _, _)| equal(key))
        })?;
        let removed = self.entries[index].take()?;
        self.live_len -= 1;
        self.dead_len += 1;
        Some((removed.0, removed.1))
    }

    pub fn iter(&self) -> impl Iterator<Item = (Any, Any)> + '_ {
        self.entries.iter().filter_map(|entry| entry.map(|(key, value, _)| (key, value)))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.buckets = vec![Vec::new(); 8];
        self.live_len = 0;
        self.dead_len = 0;
    }

    fn rehash(&mut self) {
        let mut buckets = vec![Vec::new(); self.buckets.len() * 2];
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some((_, _, hash)) = entry {
                let bucket = (*hash as usize) % buckets.len();
                buckets[bucket].push(index);
            }
        }
        self.buckets = buckets;
    }

    fn compact(&mut self) {
        let entries: Vec<_> = self.entries.iter().filter_map(|entry| *entry).map(Some).collect();
        self.entries = entries;
        self.dead_len = 0;
        self.rehash();
    }
}

/// Contiguous storage contract for a managed array.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArrayStorage {
    elements: Vec<Any>,
}

/// Contiguous byte storage for ArrayBuffer-like runtime objects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ByteBuffer {
    bytes: Vec<u8>,
}

/// Bounds-checked view into a shared byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteView {
    offset: usize,
    length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedElement {
    U8,
    U16,
    U32,
    F64,
}

impl TypedElement {
    pub const fn size(self) -> usize {
        match self { Self::U8 => 1, Self::U16 => 2, Self::U32 => 4, Self::F64 => 8 }
    }
}

/// Typed-array view metadata over a bounded byte window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedArrayView {
    view: ByteView,
    element: TypedElement,
}

impl TypedArrayView {
    pub fn new(buffer_length: usize, offset: usize, length: usize, element: TypedElement) -> Option<Self> {
        let view = ByteView::new(buffer_length, offset, length)?;
        (offset % element.size() == 0 && length % element.size() == 0).then_some(Self { view, element })
    }

    pub fn len(self) -> usize { self.view.len() / self.element.size() }
    pub fn element(self) -> TypedElement { self.element }

    pub fn read_u8(self, buffer: &ByteBuffer, index: usize) -> Option<u8> {
        (self.element == TypedElement::U8).then(|| buffer.get(self.view.offset + index)).flatten()
    }

    pub fn write_u8(self, buffer: &mut ByteBuffer, index: usize, value: u8) -> bool {
        self.element == TypedElement::U8 && index < self.len() && buffer.set(self.view.offset + index, value)
    }

    pub fn read_u16(self, buffer: &ByteBuffer, index: usize) -> Option<u16> {
        (self.element == TypedElement::U16).then(|| buffer.read_u16(self.view.offset + index.checked_mul(2)?)).flatten()
    }

    pub fn write_u16(self, buffer: &mut ByteBuffer, index: usize, value: u16) -> bool {
        self.element == TypedElement::U16 && index.checked_mul(2).is_some_and(|offset| self.view.write_u16(buffer, offset, value))
    }

    pub fn read_u32(self, buffer: &ByteBuffer, index: usize) -> Option<u32> {
        if self.element != TypedElement::U32 { return None; }
        self.view.read_u32(buffer, index.checked_mul(4)?)
    }

    pub fn write_u32(self, buffer: &mut ByteBuffer, index: usize, value: u32) -> bool {
        self.element == TypedElement::U32 && index.checked_mul(4).is_some_and(|offset| self.view.write_u32(buffer, offset, value))
    }

    pub fn read_f64(self, buffer: &ByteBuffer, index: usize) -> Option<f64> {
        if self.element != TypedElement::F64 { return None; }
        let offset = index.checked_mul(8)?;
        (offset.checked_add(8)? <= self.view.len()).then(|| buffer.read_f64(self.view.offset() + offset)).flatten()
    }

    pub fn write_f64(self, buffer: &mut ByteBuffer, index: usize, value: f64) -> bool {
        if self.element != TypedElement::F64 { return false; }
        let Some(offset) = index.checked_mul(8) else { return false; };
        offset.checked_add(8).is_some_and(|end| end <= self.view.len() && buffer.write_f64(self.view.offset() + offset, value))
    }
}

impl ByteView {
    pub fn new(buffer_length: usize, offset: usize, length: usize) -> Option<Self> {
        offset.checked_add(length).filter(|&end| end <= buffer_length).map(|_| Self { offset, length })
    }

    pub fn offset(self) -> usize { self.offset }
    pub fn len(self) -> usize { self.length }

    pub fn read_u16(self, buffer: &ByteBuffer, relative: usize) -> Option<u16> {
        let end = relative.checked_add(2)?;
        (end <= self.length).then(|| buffer.read_u16(self.offset + relative)).flatten()
    }

    pub fn write_u16(self, buffer: &mut ByteBuffer, relative: usize, value: u16) -> bool {
        relative.checked_add(2).is_some_and(|end| end <= self.length && buffer.write_u16(self.offset + relative, value))
    }

    pub fn read_u32(self, buffer: &ByteBuffer, relative: usize) -> Option<u32> {
        let end = relative.checked_add(4)?;
        if end > self.length { return None; }
        buffer.read_u32(self.offset + relative)
    }

    pub fn write_u32(self, buffer: &mut ByteBuffer, relative: usize, value: u32) -> bool {
        let Some(end) = relative.checked_add(4) else { return false; };
        end <= self.length && buffer.write_u32(self.offset + relative, value)
    }
}

impl ByteBuffer {
    pub fn new(length: usize) -> Self { Self { bytes: vec![0; length] } }
    pub fn len(&self) -> usize { self.bytes.len() }
    pub fn is_empty(&self) -> bool { self.bytes.is_empty() }
    pub fn get(&self, index: usize) -> Option<u8> { self.bytes.get(index).copied() }
    pub fn set(&mut self, index: usize, value: u8) -> bool {
        let Some(slot) = self.bytes.get_mut(index) else { return false; };
        *slot = value;
        true
    }
    pub fn as_slice(&self) -> &[u8] { &self.bytes }

    pub fn read_u16(&self, offset: usize) -> Option<u16> {
        let bytes = self.bytes.get(offset..offset.checked_add(2)?)?;
        Some(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn write_u16(&mut self, offset: usize, value: u16) -> bool {
        let Some(bytes) = self.bytes.get_mut(offset..offset.saturating_add(2)) else { return false; };
        if bytes.len() != 2 { return false; }
        bytes.copy_from_slice(&value.to_le_bytes());
        true
    }

    pub fn read_u32(&self, offset: usize) -> Option<u32> {
        let bytes = self.bytes.get(offset..offset.checked_add(4)?)?;
        Some(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    pub fn write_u32(&mut self, offset: usize, value: u32) -> bool {
        let Some(bytes) = self.bytes.get_mut(offset..offset.saturating_add(4)) else { return false; };
        if bytes.len() != 4 { return false; }
        bytes.copy_from_slice(&value.to_le_bytes());
        true
    }

    pub fn read_f64(&self, offset: usize) -> Option<f64> {
        let bytes = self.bytes.get(offset..offset.checked_add(8)?)?;
        Some(f64::from_le_bytes(bytes.try_into().unwrap()))
    }

    pub fn write_f64(&mut self, offset: usize, value: f64) -> bool {
        let Some(bytes) = self.bytes.get_mut(offset..offset.saturating_add(8)) else { return false; };
        if bytes.len() != 8 { return false; }
        bytes.copy_from_slice(&value.to_le_bytes());
        true
    }
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
        assert_eq!(field_offset(0), 16);
        assert_eq!(field_offset(3), 40);
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
    fn symbols_are_unique_but_global_keys_are_canonical() {
        let mut symbols = SymbolRegistry::new();
        let first = symbols.create(Some("iterator"));
        let second = symbols.create(Some("iterator"));
        let global_a = symbols.for_key("iterator");
        let global_b = symbols.for_key("iterator");
        assert_ne!(first, second);
        assert_eq!(global_a, global_b);
        assert_eq!(symbols.description(first), Some("iterator"));
        assert_eq!(symbols.len(), 3);
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

    #[test]
    fn byte_buffer_is_contiguous_and_bounds_checked() {
        let mut buffer = ByteBuffer::new(3);
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.get(0), Some(0));
        assert!(buffer.set(1, 0xab));
        assert_eq!(buffer.get(1), Some(0xab));
        assert!(!buffer.set(3, 1));
        assert_eq!(buffer.get(3), None);
        assert_eq!(buffer.as_slice(), &[0, 0xab, 0]);
        let mut typed = ByteBuffer::new(16);
        assert!(typed.write_u16(0, 0x1234));
        assert_eq!(typed.read_u16(0), Some(0x1234));
        assert!(typed.write_u32(2, 0xdead_beef));
        assert_eq!(typed.read_u32(2), Some(0xdead_beef));
        assert!(typed.write_f64(8, 3.5));
        assert_eq!(typed.read_f64(8), Some(3.5));
        assert!(!typed.write_u32(14, 1));
        let view = ByteView::new(typed.len(), 2, 8).unwrap();
        assert_eq!(view.read_u32(&typed, 0), Some(0xdead_beef));
        assert!(view.write_u32(&mut typed, 4, 7));
        assert_eq!(typed.read_u32(6), Some(7));
        assert!(!view.write_u32(&mut typed, 5, 8));
        assert!(ByteView::new(typed.len(), 15, 2).is_none());
        let array = TypedArrayView::new(typed.len(), 4, 8, TypedElement::U32).unwrap();
        assert_eq!(array.len(), 2);
        assert!(array.write_u32(&mut typed, 1, 0xcafe));
        assert_eq!(array.read_u32(&typed, 1), Some(0xcafe));
        assert!(!array.write_u32(&mut typed, 2, 1));
        assert!(TypedArrayView::new(typed.len(), 1, 4, TypedElement::U32).is_none());
        assert!(TypedArrayView::new(typed.len(), 0, 6, TypedElement::U32).is_none());
        let bytes = TypedArrayView::new(typed.len(), 0, 4, TypedElement::U8).unwrap();
        assert!(bytes.write_u8(&mut typed, 0, 0x7f));
        assert_eq!(bytes.read_u8(&typed, 0), Some(0x7f));
        let shorts = TypedArrayView::new(typed.len(), 0, 4, TypedElement::U16).unwrap();
        assert!(shorts.write_u16(&mut typed, 1, 0xbeef));
        assert_eq!(shorts.read_u16(&typed, 1), Some(0xbeef));
        let floats = TypedArrayView::new(typed.len(), 8, 8, TypedElement::F64).unwrap();
        assert!(floats.write_f64(&mut typed, 0, 1.25));
        assert_eq!(floats.read_f64(&typed, 0), Some(1.25));
    }

    #[test]
    fn ordered_map_preserves_insertion_order_and_updates_in_place() {
        let mut map = OrderedMap::new();
        let key = |candidate: Any, wanted: i64| candidate.as_int() == Some(wanted);
        assert_eq!(map.insert(Any::int(2), Any::int(20), 2, |k| key(k, 2)), None);
        assert_eq!(map.insert(Any::int(1), Any::int(10), 1, |k| key(k, 1)), None);
        assert_eq!(map.insert(Any::int(2), Any::int(22), 2, |k| key(k, 2)).and_then(Any::as_int), Some(20));
        assert_eq!(map.get(2, |k| key(k, 2)).and_then(Any::as_int), Some(22));
        let values: Vec<i64> = map.iter().filter_map(|(_, value)| value.as_int()).collect();
        assert_eq!(values, vec![22, 10]);
        assert_eq!(map.remove(1, |k| key(k, 1)).map(|(_, value)| value.as_int()), Some(Some(10)));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn ordered_map_compacts_tombstones_without_changing_order() {
        let mut map = OrderedMap::new();
        for key in 0..12 {
            map.insert(Any::int(key), Any::int(key * 10), key as u64, |candidate| candidate.as_int() == Some(key));
        }
        for key in 0..10 {
            assert!(map.remove(key as u64, |candidate| candidate.as_int() == Some(key)).is_some());
        }
        map.insert(Any::int(12), Any::int(120), 12, |candidate| candidate.as_int() == Some(12));
        let keys: Vec<i64> = map.iter().filter_map(|(key, _)| key.as_int()).collect();
        assert_eq!(keys, vec![10, 11, 12]);
    }
}
