//! Frozen contracts shared by the Zun runtime and its standard-library intrinsics.
//!
//! This crate deliberately has no dependency on the interpreter, compiler, or
//! heap implementation. It is the small Rust equivalent of the runtime-owned
//! portion of `System.Private.CoreLib`.

use std::collections::{BTreeMap, HashMap};

#[path = "views.rs"]
mod views;
mod ordered_map;
mod collection_iterators;

pub use ordered_map::OrderedMap;
pub use collection_iterators::{MapIterator, SetIterator};
pub use views::ByteView;
pub use views::DataView;
pub use views::TypedElement;
pub use views::TypedArrayView;

/// Size of one raw object slot in bytes.
pub const SLOT_SIZE: usize = 8;

/// Number of slots reserved before an object's first field.
pub const OBJECT_HEADER_SLOTS: usize = 2;

/// Convert an object field slot to its byte offset in the frozen layout.
pub const fn field_offset(slot: usize) -> usize {
    (OBJECT_HEADER_SLOTS + slot) * SLOT_SIZE
}

/// Stable identity of a callable Zun function value.
///
/// The method RID identifies the code/signature metadata. `environment` is a
/// managed object address containing captured locals, or zero for a non-
/// capturing function. The handle is intentionally a plain frozen value;
/// allocation and tracing belong to the GC/runtime layers built on top of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub struct FunctionHandle {
    method_rid: u32,
    reserved: u32,
    environment: usize,
}

impl FunctionHandle {
    /// Construct a function value for a method and optional closure object.
    pub const fn new(method_rid: u32, environment: Option<usize>) -> Self {
        Self {
            method_rid,
            reserved: 0,
            environment: match environment {
                Some(address) => address,
                None => 0,
            },
        }
    }

    /// Metadata method RID targeted by this function value.
    pub const fn method_rid(self) -> u32 {
        self.method_rid
    }

    /// Captured environment address, if this value is a closure.
    pub const fn environment(self) -> Option<usize> {
        if self.environment == 0 {
            None
        } else {
            Some(self.environment)
        }
    }
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
    /// The explicit JavaScript `null` value.
    Null = 5,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SymbolId(u64);

impl SymbolId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Per-isolate symbol storage. The registry is deliberately separate from
/// ordinary string interning: Symbol descriptions do not define identity.
#[derive(Debug, Default)]
pub struct SymbolRegistry {
    descriptions: Vec<Option<String>>,
    global: HashMap<String, SymbolId>,
}

impl SymbolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

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

    /// Return the isolate-local well-known symbol used by the iteration protocol.
    pub fn iterator(&mut self) -> SymbolId {
        self.for_key("Symbol.iterator")
    }

    pub fn description(&self, id: SymbolId) -> Option<&str> {
        self.descriptions
            .get(id.0 as usize)
            .and_then(Option::as_deref)
    }

    pub fn len(&self) -> usize {
        self.descriptions.len()
    }
}

impl Any {
    /// The explicit JavaScript `null` value.
    pub const fn null() -> Self {
        Self {
            tag: AnyTag::Null,
            payload: 0,
        }
    }
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
    symbol_properties: BTreeMap<SymbolId, Any>,
    symbol_functions: BTreeMap<SymbolId, FunctionHandle>,
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

/// Millisecond-precision date value. Invalid dates carry no usable timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateValue {
    millis: i64,
    valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorValue {
    name: String,
    message: String,
}

impl ErrorValue {
    pub fn new(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            message: message.into(),
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }
}

impl DateValue {
    pub const fn from_millis(millis: i64) -> Self {
        Self {
            millis,
            valid: true,
        }
    }
    pub const fn invalid() -> Self {
        Self {
            millis: 0,
            valid: false,
        }
    }
    pub const fn is_valid(self) -> bool {
        self.valid
    }
    pub const fn millis(self) -> Option<i64> {
        if self.valid {
            Some(self.millis)
        } else {
            None
        }
    }
    pub fn compare(self, other: Self) -> Option<std::cmp::Ordering> {
        Some(self.millis()?.cmp(&other.millis()?))
    }

    /// Return UTC calendar components using the proleptic Gregorian calendar.
    pub fn utc_date(self) -> Option<(i32, u32, u32)> {
        let days = self.millis()?.div_euclid(86_400_000);
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
        let day_of_era = z - era * 146_097;
        let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
            - day_of_era / 146_096)
            .div_euclid(365);
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_part = (5 * day_of_year + 2).div_euclid(153);
        let day = day_of_year - (153 * month_part + 2).div_euclid(5) + 1;
        let month = month_part + if month_part < 10 { 3 } else { -9 };
        let year = year + if month <= 2 { 1 } else { 0 };
        Some((year as i32, month as u32, day as u32))
    }

    pub fn iso_date(self) -> Option<String> {
        let (year, month, day) = self.utc_date()?;
        Some(format!("{year:04}-{month:02}-{day:02}"))
    }

    pub fn utc_time(self) -> Option<(u32, u32, u32, u32)> {
        let millis = self.millis()?.rem_euclid(86_400_000);
        let hour = (millis / 3_600_000) as u32;
        let minute = ((millis % 3_600_000) / 60_000) as u32;
        let second = ((millis % 60_000) / 1_000) as u32;
        let millisecond = (millis % 1_000) as u32;
        Some((hour, minute, second, millisecond))
    }

    /// ISO weekday where Monday is 1 and Sunday is 7.
    pub fn weekday(self) -> Option<u32> {
        let days = self.millis()?.div_euclid(86_400_000);
        Some((days + 3).rem_euclid(7) as u32 + 1)
    }

    pub fn ordinal(self) -> Option<u32> {
        let (year, month, day) = self.utc_date()?;
        let before = (1..month).map(|m| days_in_month(year, m)).sum::<u32>();
        Some(before + day)
    }

    pub fn iso_datetime(self) -> Option<String> {
        let (year, month, day) = self.utc_date()?;
        let (hour, minute, second, millisecond) = self.utc_time()?;
        Some(format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}Z"
        ))
    }

    pub fn parse_iso_datetime(value: &str) -> Option<Self> {
        let (date, time) = value.strip_suffix('Z')?.split_once('T')?;
        let date = Self::parse_iso_date(date)?;
        let mut fields = time.split([':', '.']);
        let hour: i64 = fields.next()?.parse().ok()?;
        let minute: i64 = fields.next()?.parse().ok()?;
        let second: i64 = fields.next()?.parse().ok()?;
        let millisecond: i64 = fields.next()?.parse().ok()?;
        if fields.next().is_some()
            || hour < 0
            || hour > 23
            || minute < 0
            || minute > 59
            || second < 0
            || second > 59
            || millisecond < 0
            || millisecond > 999
        {
            return None;
        }
        let day = date.millis()?;
        let time = hour * 3_600_000 + minute * 60_000 + second * 1_000 + millisecond;
        Some(Self::from_millis(day.checked_add(time)?))
    }

    pub fn parse_iso_date(value: &str) -> Option<Self> {
        let mut parts = value.split('-');
        let year: i32 = parts.next()?.parse().ok()?;
        let month: u32 = parts.next()?.parse().ok()?;
        let day: u32 = parts.next()?.parse().ok()?;
        if parts.next().is_some()
            || !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
        {
            return None;
        }
        let days = days_from_civil(year, month, day)?;
        Some(Self::from_millis(days.checked_mul(86_400_000)?))
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 }.div_euclid(400);
    let year_of_era = y - era * 400;
    let month = i64::from(month);
    let day_of_year =
        (153 * (month + if month > 2 { -3 } else { 9 }) + 2).div_euclid(5) + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

impl ByteBuffer {
    pub fn new(length: usize) -> Self {
        Self {
            bytes: vec![0; length],
        }
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
    pub fn get(&self, index: usize) -> Option<u8> {
        self.bytes.get(index).copied()
    }
    pub fn set(&mut self, index: usize, value: u8) -> bool {
        let Some(slot) = self.bytes.get_mut(index) else {
            return false;
        };
        *slot = value;
        true
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn read_u16(&self, offset: usize) -> Option<u16> {
        let bytes = self.bytes.get(offset..offset.checked_add(2)?)?;
        Some(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn write_u16(&mut self, offset: usize, value: u16) -> bool {
        let Some(bytes) = self.bytes.get_mut(offset..offset.saturating_add(2)) else {
            return false;
        };
        if bytes.len() != 2 {
            return false;
        }
        bytes.copy_from_slice(&value.to_le_bytes());
        true
    }

    pub fn read_u32(&self, offset: usize) -> Option<u32> {
        let bytes = self.bytes.get(offset..offset.checked_add(4)?)?;
        Some(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    pub fn write_u32(&mut self, offset: usize, value: u32) -> bool {
        let Some(bytes) = self.bytes.get_mut(offset..offset.saturating_add(4)) else {
            return false;
        };
        if bytes.len() != 4 {
            return false;
        }
        bytes.copy_from_slice(&value.to_le_bytes());
        true
    }

    pub fn read_f64(&self, offset: usize) -> Option<f64> {
        let bytes = self.bytes.get(offset..offset.checked_add(8)?)?;
        Some(f64::from_le_bytes(bytes.try_into().unwrap()))
    }

    pub fn write_f64(&mut self, offset: usize, value: f64) -> bool {
        let Some(bytes) = self.bytes.get_mut(offset..offset.saturating_add(8)) else {
            return false;
        };
        if bytes.len() != 8 {
            return false;
        }
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

/// State for the default `Array[Symbol.iterator]()` protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayIterator {
    next_index: usize,
}

impl ArrayIterator {
    /// Create an iterator positioned before the first array element.
    pub fn new() -> Self {
        Self { next_index: 0 }
    }

    /// Advance in insertion/index order, returning `None` at the end.
    pub fn next(&mut self, array: &ArrayStorage) -> Option<Any> {
        let value = array.get(self.next_index)?;
        self.next_index += 1;
        Some(value)
    }

    /// Whether the iterator has consumed the current array length.
    pub fn is_done(&self, array: &ArrayStorage) -> bool {
        self.next_index >= array.len()
    }
}

impl Default for ArrayIterator {
    fn default() -> Self {
        Self::new()
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

    /// Read a property keyed by a runtime symbol.
    pub fn get_symbol(&self, symbol: SymbolId) -> Option<Any> {
        self.symbol_properties.get(&symbol).copied()
    }

    /// Insert or replace a symbol-keyed property.
    pub fn set_symbol(&mut self, symbol: SymbolId, value: Any) -> Option<Any> {
        self.symbol_properties.insert(symbol, value)
    }

    /// Read a callable property keyed by a runtime symbol.
    pub fn get_symbol_function(&self, symbol: SymbolId) -> Option<FunctionHandle> {
        self.symbol_functions.get(&symbol).copied()
    }

    /// Insert or replace a callable symbol-keyed property.
    pub fn set_symbol_function(
        &mut self,
        symbol: SymbolId,
        function: FunctionHandle,
    ) -> Option<FunctionHandle> {
        self.symbol_functions.insert(symbol, function)
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
            .chain(
                self.symbol_properties
                    .values()
                    .filter_map(|value| value.as_object()),
            )
            .chain(
                self.symbol_functions
                    .values()
                    .filter_map(|value| value.environment()),
            )
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
    fn function_handle_keeps_code_and_environment_identity() {
        let plain = FunctionHandle::new(7, None);
        assert_eq!(plain.method_rid(), 7);
        assert_eq!(plain.environment(), None);

        let closure = FunctionHandle::new(9, Some(0x1000));
        assert_eq!(closure.method_rid(), 9);
        assert_eq!(closure.environment(), Some(0x1000));
        assert_eq!(FunctionHandle::new(9, Some(0x1000)), closure);
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
    fn iterator_symbol_is_canonical_within_an_isolate() {
        let mut symbols = SymbolRegistry::new();
        let first = symbols.iterator();
        let second = symbols.iterator();
        assert_eq!(first, second);
        assert_eq!(symbols.description(first), Some("Symbol.iterator"));
    }

    #[test]
    fn dynamic_object_stores_and_replaces_properties() {
        let mut object = DynamicObject::new();
        assert!(object.is_empty());
        assert_eq!(object.set("answer", Any::int(41)), None);
        assert_eq!(object.get("answer").and_then(Any::as_int), Some(41));
        assert_eq!(
            object.set("answer", Any::int(42)).and_then(Any::as_int),
            Some(41)
        );
        assert!(object.contains("answer"));
        assert_eq!(object.remove("answer").and_then(Any::as_int), Some(42));
        assert_eq!(object.len(), 0);
    }

    #[test]
    fn dynamic_object_keeps_symbol_properties_separate_from_string_properties() {
        let mut symbols = SymbolRegistry::new();
        let iterator = symbols.iterator();
        let mut object = DynamicObject::new();
        object.set("Symbol.iterator", Any::int(1));
        object.set_symbol(iterator, Any::int(2));

        assert_eq!(object.get("Symbol.iterator").and_then(Any::as_int), Some(1));
        assert_eq!(object.get_symbol(iterator).and_then(Any::as_int), Some(2));
        assert_eq!(object.len(), 1);
    }

    #[test]
    fn dynamic_object_stores_callable_symbol_properties() {
        let mut symbols = SymbolRegistry::new();
        let iterator = symbols.iterator();
        let mut object = DynamicObject::new();
        let function = FunctionHandle::new(17, Some(0x2000));

        assert_eq!(object.set_symbol_function(iterator, function), None);
        assert_eq!(object.get_symbol_function(iterator), Some(function));
        assert_eq!(object.object_references().collect::<Vec<_>>(), vec![0x2000]);
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
    fn array_iterator_preserves_index_order_and_end_state() {
        let mut array = ArrayStorage::with_length(3);
        array.set(0, Any::int(10));
        array.set(1, Any::int(20));
        array.set(2, Any::int(30));
        let mut iterator = ArrayIterator::new();
        assert_eq!(iterator.next(&array).and_then(Any::as_int), Some(10));
        assert_eq!(iterator.next(&array).and_then(Any::as_int), Some(20));
        assert_eq!(iterator.next(&array).and_then(Any::as_int), Some(30));
        assert!(iterator.is_done(&array));
        assert_eq!(iterator.next(&array), None);
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
        let date = DateValue::from_millis(1_700_000_000_123);
        assert!(date.is_valid());
        assert_eq!(date.millis(), Some(1_700_000_000_123));
        assert_eq!(
            date.compare(DateValue::from_millis(1_700_000_000_124)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(DateValue::from_millis(0).utc_date(), Some((1970, 1, 1)));
        assert_eq!(DateValue::from_millis(0).weekday(), Some(4));
        assert_eq!(DateValue::from_millis(0).ordinal(), Some(1));
        assert_eq!(
            DateValue::from_millis(1_700_000_000_000).utc_date(),
            Some((2023, 11, 14))
        );
        assert_eq!(
            DateValue::from_millis(1_700_000_000_000).ordinal(),
            Some(318)
        );
        assert_eq!(
            DateValue::from_millis(3_723_004).utc_time(),
            Some((1, 2, 3, 4))
        );
        assert_eq!(
            DateValue::from_millis(-1).utc_time(),
            Some((23, 59, 59, 999))
        );
        assert_eq!(
            DateValue::from_millis(3_723_004).iso_datetime().as_deref(),
            Some("1970-01-01T01:02:03.004Z")
        );
        assert_eq!(
            DateValue::parse_iso_datetime("1970-01-01T01:02:03.004Z")
                .and_then(DateValue::iso_datetime)
                .as_deref(),
            Some("1970-01-01T01:02:03.004Z")
        );
        assert!(DateValue::parse_iso_datetime("1970-01-01T24:00:00.000Z").is_none());
        assert_eq!(
            DateValue::from_millis(0).iso_date().as_deref(),
            Some("1970-01-01")
        );
        assert_eq!(
            DateValue::parse_iso_date("2023-11-14")
                .and_then(DateValue::iso_date)
                .as_deref(),
            Some("2023-11-14")
        );
        assert!(DateValue::parse_iso_date("2023-02-29").is_none());
        assert!(DateValue::parse_iso_date("bad-date").is_none());
        assert_eq!(DateValue::invalid().compare(date), None);
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
        let data = DataView::new(typed.len(), 1, 10).unwrap();
        assert!(data.set_u16(&mut typed, 1, 0x1234, true));
        assert_eq!(data.get_u16(&typed, 1, true), Some(0x1234));
        assert_eq!(data.get_u16(&typed, 1, false), Some(0x3412));
        assert!(data.set_u32(&mut typed, 3, 0xdead_beef, false));
        assert_eq!(data.get_u32(&typed, 3, false), Some(0xdead_beef));
        assert!(data.set_f64(&mut typed, 2, -3.5, true));
        assert_eq!(data.get_f64(&typed, 2, true), Some(-3.5));
        assert!(!data.set_u32(&mut typed, 7, 1, true));
    }

    #[test]
    fn ordered_map_preserves_insertion_order_and_updates_in_place() {
        let mut map = OrderedMap::new();
        let key = |candidate: Any, wanted: i64| candidate.as_int() == Some(wanted);
        assert_eq!(
            map.insert(Any::int(2), Any::int(20), 2, |k| key(k, 2)),
            None
        );
        assert_eq!(
            map.insert(Any::int(1), Any::int(10), 1, |k| key(k, 1)),
            None
        );
        assert_eq!(
            map.insert(Any::int(2), Any::int(22), 2, |k| key(k, 2))
                .and_then(Any::as_int),
            Some(20)
        );
        assert_eq!(map.get(2, |k| key(k, 2)).and_then(Any::as_int), Some(22));
        let values: Vec<i64> = map.iter().filter_map(|(_, value)| value.as_int()).collect();
        assert_eq!(values, vec![22, 10]);
        assert_eq!(
            map.remove(1, |k| key(k, 1))
                .map(|(_, value)| value.as_int()),
            Some(Some(10))
        );
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn ordered_map_compacts_tombstones_without_changing_order() {
        let mut map = OrderedMap::new();
        for key in 0..12 {
            map.insert(Any::int(key), Any::int(key * 10), key as u64, |candidate| {
                candidate.as_int() == Some(key)
            });
        }
        for key in 0..10 {
            assert!(map
                .remove(key as u64, |candidate| candidate.as_int() == Some(key))
                .is_some());
        }
        map.insert(Any::int(12), Any::int(120), 12, |candidate| {
            candidate.as_int() == Some(12)
        });
        let keys: Vec<i64> = map.iter().filter_map(|(key, _)| key.as_int()).collect();
        assert_eq!(keys, vec![10, 11, 12]);
    }

    #[test]
    fn map_and_set_iterators_follow_live_insertion_order() {
        let mut map = OrderedMap::new();
        for value in 1..=3 {
            map.insert(Any::int(value), Any::int(value * 10), value as u64, |key| {
                key.as_int() == Some(value)
            });
        }
        let mut map_iterator = MapIterator::new();
        assert_eq!(
            map_iterator.next(&map).map(|(_, value)| value.as_int()),
            Some(Some(10))
        );
        assert_eq!(
            map_iterator.next(&map).map(|(_, value)| value.as_int()),
            Some(Some(20))
        );

        let mut set_iterator = SetIterator::new();
        assert_eq!(
            set_iterator.next(&map).and_then(|value| value.as_int()),
            Some(1)
        );
        assert_eq!(
            set_iterator.next(&map).and_then(|value| value.as_int()),
            Some(2)
        );
        assert_eq!(
            set_iterator.next(&map).and_then(|value| value.as_int()),
            Some(3)
        );
        assert_eq!(set_iterator.next(&map), None);
    }
}
