//! The binary serialize/deserialize ENGINE — the Tier-0 shape-walk and object
//! reconstruction behind `serialize`/`deserialize` (imported from `"zun:core"`).
//!
//! The wire format is defined once in [`zun_core::serde`]; this is the half
//! that touches the managed heap (so it rides on the GC/metadata and stays on
//! the always-correct interpreter floor, not the JIT).
//!
//! - **Shape-walk.** A typed class instance is walked by metadata: its declared
//!   instance fields in layout order (base classes first, `static`/`private`
//!   excluded), reusing [`Interp::object_own_entries`] — the same enumeration
//!   `Object.keys`/`getOwnPropertyDescriptor` use. A dynamic object is walked by
//!   its own entry map. Nothing reflects on Rust types.
//! - **Cycles + sharing.** A memo keyed by object identity gives every heap
//!   object one index at first sight; a later sight emits a back-reference.
//!   Deserialize keeps a parallel memo and, for a container, allocates the
//!   shell and records it BEFORE filling its children, so a back-reference
//!   (including a self-cycle) resolves to the object under construction.
//! - **Safe by construction.** Deserialize reconstructs only data and
//!   metadata-known types; it never runs code or a constructor body. An unknown
//!   tag, a truncated stream, or a type token that does not resolve in this
//!   program is a clean error.

use super::value::{self, Value};
use super::{Interp, RunError};
use std::collections::HashMap;
use std::rc::Rc;
use zun_core::serde::{write_uvarint, Reader, Tag, SERDE_VERSION};
use zun_core::Any;
use zun_gc::Obj;
use zun_il::NumKind;

/// The serializable kind of a heap object, determined by side-table probes
/// (the only reliable discriminator: arrays/maps/sets/dates/buffers/views all
/// share `type_rid == 0`). A typed class instance is the metadata fallback.
enum ObjKind {
    BigInt,
    TypedArray,
    ArrayBuffer,
    DataView,
    Array,
    Map,
    Set,
    Date,
    Dynamic,
    /// A typed class instance, carrying its metadata type token.
    Instance(u32),
    /// A kind the ser/de primitive does not cover (Promise, RegExp, Proxy,
    /// WeakMap/WeakSet, Error, DataView, a bound-function record, …).
    Unsupported,
}

impl Interp<'_> {
    // ---- serialize ---------------------------------------------------------

    /// `serialize(value)` → a fresh `Uint8Array` of the encoded bytes.
    pub(crate) fn serialize_value(&self, value: &Value) -> Result<Value, RunError> {
        let mut out = vec![SERDE_VERSION];
        let mut memo: HashMap<usize, u32> = HashMap::new();
        self.ser_value(value, &mut out, &mut memo)?;
        // Allocate the output only after the walk, so no allocation interleaves
        // with the read walk (and no live handle needs rooting across it).
        let view = zun_gc::with_current_heap_mut(|heap| {
            let buffer = heap.alloc_buffer(out.len());
            for (index, byte) in out.iter().enumerate() {
                heap.buffer_set(buffer, index, *byte);
            }
            heap.alloc_typed_array(buffer, 0, out.len(), zun_core::TypedElement::U8)
        })
        .ok_or(RunError::BadToken)?;
        Ok(Value::Object(view))
    }

    fn ser_value(
        &self,
        value: &Value,
        out: &mut Vec<u8>,
        memo: &mut HashMap<usize, u32>,
    ) -> Result<(), RunError> {
        match value {
            Value::Null => out.push(Tag::Null.to_u8()),
            Value::Void => out.push(Tag::Undefined.to_u8()),
            Value::Bool(false) => out.push(Tag::False.to_u8()),
            Value::Bool(true) => out.push(Tag::True.to_u8()),
            Value::Int(v, kind) => {
                out.push(Tag::Int.to_u8());
                out.push(kind.to_imm() as u8);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Value::Float(v, NumKind::F32) => {
                out.push(Tag::Float32.to_u8());
                out.extend_from_slice(&(*v as f32).to_bits().to_le_bytes());
            }
            Value::Float(v, _) => {
                out.push(Tag::Float64.to_u8());
                out.extend_from_slice(&v.to_bits().to_le_bytes());
            }
            Value::Str(s) => write_str(out, s),
            Value::Function(_) => {
                return Err(RunError::Native(
                    "serialize: a function value is not serializable".into(),
                ))
            }
            Value::Object(obj) => self.ser_object(*obj, out, memo)?,
            Value::Any(any) => match any.tag() {
                zun_core::AnyTag::Object => {
                    let address = any.as_object().ok_or(RunError::BadToken)?;
                    let obj = unsafe { Obj::from_addr(address as *mut u64) };
                    self.ser_object(obj, out, memo)?;
                }
                zun_core::AnyTag::Symbol | zun_core::AnyTag::ClassDescriptor => {
                    return Err(RunError::Native(
                        "serialize: a symbol value is not serializable".into(),
                    ))
                }
                // A primitive carrier: unbox to its concrete Value and encode.
                _ => {
                    let v = zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .ok_or(RunError::BadToken)?;
                    self.ser_value(&v, out, memo)?;
                }
            },
        }
        Ok(())
    }

    fn ser_object(
        &self,
        obj: Obj,
        out: &mut Vec<u8>,
        memo: &mut HashMap<usize, u32>,
    ) -> Result<(), RunError> {
        let address = obj.addr() as usize;
        if let Some(&index) = memo.get(&address) {
            out.push(Tag::Ref.to_u8());
            write_uvarint(out, u64::from(index));
            return Ok(());
        }
        // Assign this object's memo index BEFORE encoding its children, so a
        // back-reference inside them resolves to this object.
        let index = memo.len() as u32;
        memo.insert(address, index);

        match self.classify(obj) {
            ObjKind::BigInt => {
                let text = zun_gc::with_current_heap(|heap| {
                    heap.bigint_value(obj).map(|value| value.to_string())
                })
                .ok_or(RunError::BadToken)?;
                out.push(Tag::BigInt.to_u8());
                write_bytes(out, text.as_bytes());
            }
            ObjKind::TypedArray => {
                let (element, bytes) = zun_gc::with_current_heap(|heap| {
                    let element = heap.typed_array_element(obj)?;
                    let buffer = heap.typed_array_buffer(obj)?;
                    let offset = heap.typed_array_byte_offset(obj)?;
                    let byte_len = heap.typed_array_byte_len(obj)?;
                    let mut bytes = Vec::with_capacity(byte_len);
                    for i in 0..byte_len {
                        bytes.push(heap.buffer_get(buffer, offset + i)?);
                    }
                    Some((element.index(), bytes))
                })
                .ok_or(RunError::BadToken)?;
                out.push(Tag::TypedArray.to_u8());
                out.push(element as u8);
                write_bytes(out, &bytes);
            }
            ObjKind::DataView => {
                let (bytes, offset, length, tracking) = zun_gc::with_current_heap(|heap| {
                    let buffer = heap.data_view_buffer(obj)?;
                    let len = heap.buffer_len(buffer)?;
                    let mut bytes = Vec::with_capacity(len);
                    for i in 0..len {
                        bytes.push(heap.buffer_get(buffer, i)?);
                    }
                    Some((
                        bytes,
                        heap.data_view_byte_offset(obj)?,
                        heap.data_view_byte_len(obj)?,
                        heap.data_view_is_tracking(obj)?,
                    ))
                })
                .ok_or(RunError::BadToken)?;
                out.push(Tag::DataView.to_u8());
                write_bytes(out, &bytes);
                write_uvarint(out, offset as u64);
                write_uvarint(out, length as u64);
                out.push(u8::from(tracking));
            }
            ObjKind::ArrayBuffer => {
                let bytes = zun_gc::with_current_heap(|heap| {
                    let len = heap.buffer_len(obj)?;
                    let mut bytes = Vec::with_capacity(len);
                    for i in 0..len {
                        bytes.push(heap.buffer_get(obj, i)?);
                    }
                    Some(bytes)
                })
                .ok_or(RunError::BadToken)?;
                out.push(Tag::ArrayBuffer.to_u8());
                write_bytes(out, &bytes);
            }
            ObjKind::Array => {
                let elements = zun_gc::with_current_heap(|heap| {
                    let len = heap.array_len(obj)?;
                    let mut elements = Vec::with_capacity(len);
                    for i in 0..len {
                        elements.push(heap.array_get(obj, i)?);
                    }
                    Some(elements)
                })
                .ok_or(RunError::BadToken)?;
                out.push(Tag::Array.to_u8());
                write_uvarint(out, elements.len() as u64);
                for element in elements {
                    let value = self.any_to_value(element)?;
                    self.ser_value(&value, out, memo)?;
                }
            }
            ObjKind::Map => {
                let entries =
                    zun_gc::with_current_heap(|heap| heap.map_entries(obj)).ok_or(RunError::BadToken)?;
                out.push(Tag::Map.to_u8());
                write_uvarint(out, entries.len() as u64);
                for (key, value) in entries {
                    let key = self.any_to_value(key)?;
                    let value = self.any_to_value(value)?;
                    self.ser_value(&key, out, memo)?;
                    self.ser_value(&value, out, memo)?;
                }
            }
            ObjKind::Set => {
                let values =
                    zun_gc::with_current_heap(|heap| heap.set_values(obj)).ok_or(RunError::BadToken)?;
                out.push(Tag::Set.to_u8());
                write_uvarint(out, values.len() as u64);
                for value in values {
                    let value = self.any_to_value(value)?;
                    self.ser_value(&value, out, memo)?;
                }
            }
            ObjKind::Date => {
                let (valid, millis) = zun_gc::with_current_heap(|heap| {
                    (heap.date_is_valid(obj), heap.date_millis(obj))
                });
                out.push(Tag::Date.to_u8());
                match (valid, millis) {
                    (Some(true), Some(millis)) => {
                        out.push(1);
                        out.extend_from_slice(&millis.to_le_bytes());
                    }
                    _ => out.push(0),
                }
            }
            ObjKind::Dynamic => {
                let entries = zun_gc::with_current_heap(|heap| heap.dynamic_entries(obj))
                    .ok_or(RunError::BadToken)?;
                // Symbol-keyed entries: only registered (`Symbol.for`) and
                // well-known symbols have an identity that survives a
                // deserialize. Unique `Symbol()` keys cannot be reconstructed, so
                // they are silently omitted (the rest of the graph still
                // round-trips).
                let symbol_entries: Vec<(u8, Option<String>, u64, zun_core::Any)> =
                    zun_gc::with_current_heap(|heap| {
                        let mut found = Vec::new();
                        for symbol in heap.dynamic_symbol_keys(obj) {
                            let Some(value) = heap.dynamic_get_symbol(obj, symbol) else { continue };
                            if let Some(key) = heap.symbol_key_for(symbol) {
                                found.push((0u8, Some(key.to_string()), 0, value));
                            } else if zun_core::WellKnownSymbol::ALL
                                .iter()
                                .any(|w| w.id() == symbol)
                            {
                                found.push((1u8, None, symbol.get(), value));
                            }
                        }
                        found
                    });
                let tag = if symbol_entries.is_empty() { Tag::Object } else { Tag::ObjectSym };
                out.push(tag.to_u8());
                write_uvarint(out, entries.len() as u64);
                for (key, value) in entries {
                    // The key is a bare length-prefixed string (no value tag);
                    // deserialize reads it with `read_str`.
                    write_bytes(out, key.as_bytes());
                    let value = self.any_to_value(value)?;
                    self.ser_value(&value, out, memo)?;
                }
                if !symbol_entries.is_empty() {
                    write_uvarint(out, symbol_entries.len() as u64);
                    for (kind, key, id, value) in symbol_entries {
                        out.push(kind);
                        match key {
                            Some(key) => write_bytes(out, key.as_bytes()),
                            None => write_uvarint(out, id),
                        }
                        let value = self.any_to_value(value)?;
                        self.ser_value(&value, out, memo)?;
                    }
                }
            }
            ObjKind::Instance(type_rid) => {
                // Metadata shape-walk: declared instance fields, layout order,
                // static/private excluded — the exact helper `Object.keys` uses.
                let entries = self.object_own_entries(obj)?;
                out.push(Tag::Instance.to_u8());
                write_uvarint(out, u64::from(type_rid));
                write_uvarint(out, entries.len() as u64);
                for (_name, value) in entries {
                    self.ser_value(&value, out, memo)?;
                }
            }
            ObjKind::Unsupported => {
                return Err(RunError::Native(
                    "serialize: unsupported object kind (functions, promises, \
                     proxies, weak collections, errors and regexps are not data)"
                        .into(),
                ))
            }
        }
        Ok(())
    }

    /// Classify a heap object into its serializable kind. Side-table probes
    /// first (they share `type_rid == 0`), then the typed-instance fallback.
    fn classify(&self, obj: Obj) -> ObjKind {
        let probed = zun_gc::with_current_heap(|heap| {
            if heap.bigint_value(obj).is_some() {
                Some(ObjKind::BigInt)
            } else if heap.is_view(obj) {
                Some(if heap.typed_array_len(obj).is_some() {
                    ObjKind::TypedArray
                } else if heap.data_view_buffer(obj).is_some() {
                    ObjKind::DataView
                } else {
                    ObjKind::Unsupported
                })
            } else if heap.buffer_len(obj).is_some() {
                Some(ObjKind::ArrayBuffer)
            } else if heap.array_len(obj).is_some() {
                Some(ObjKind::Array)
            } else if heap.map_len(obj).is_some() {
                Some(ObjKind::Map)
            } else if heap.set_len(obj).is_some() {
                Some(ObjKind::Set)
            } else if heap.date_is_valid(obj).is_some() {
                Some(ObjKind::Date)
            } else if heap.is_dynamic_object(obj) {
                Some(ObjKind::Dynamic)
            } else {
                None
            }
        });
        if let Some(kind) = probed {
            return kind;
        }
        // Not in any side table: a typed class instance iff its type token
        // resolves in this program's metadata (`get` returns None for rid 0).
        let type_rid = obj.type_rid();
        if self.types.get(type_rid).is_some() {
            ObjKind::Instance(type_rid)
        } else {
            ObjKind::Unsupported
        }
    }

    // ---- deserialize -------------------------------------------------------

    /// `deserialize(bytes)` → the reconstructed value. `view` is the input
    /// `Uint8Array`/`Buffer` byte view.
    pub(crate) fn deserialize_value(&mut self, view: Obj) -> Result<Value, RunError> {
        let bytes = zun_gc::with_current_heap(|heap| {
            let buffer = heap.typed_array_buffer(view)?;
            let offset = heap.typed_array_byte_offset(view)?;
            let byte_len = heap.typed_array_byte_len(view)?;
            let mut bytes = Vec::with_capacity(byte_len);
            for i in 0..byte_len {
                bytes.push(heap.buffer_get(buffer, offset + i)?);
            }
            Some(bytes)
        })
        .ok_or_else(|| RunError::Native("deserialize: input is not a byte view".into()))?;

        let mut reader = Reader::new(&bytes);
        match reader.read_u8() {
            Some(SERDE_VERSION) => {}
            _ => {
                return Err(RunError::Native(
                    "deserialize: unrecognized format version".into(),
                ))
            }
        }
        let mut memo: Vec<Value> = Vec::new();
        self.de_value(&mut reader, &mut memo)
    }

    fn de_value(&mut self, reader: &mut Reader, memo: &mut Vec<Value>) -> Result<Value, RunError> {
        let tag = reader
            .read_tag()
            .ok_or_else(|| RunError::Native("deserialize: truncated or unknown tag".into()))?;
        match tag {
            Tag::Null => Ok(Value::Null),
            Tag::Undefined => Ok(Value::Void),
            Tag::False => Ok(Value::Bool(false)),
            Tag::True => Ok(Value::Bool(true)),
            Tag::Int => {
                let kind = NumKind::from_imm(i32::from(de_u8(reader)?)).ok_or_else(bad)?;
                let value = reader.read_i64().ok_or_else(bad)?;
                Ok(Value::Int(value, kind))
            }
            Tag::Float64 => {
                let mut bits = [0u8; 8];
                for slot in &mut bits {
                    *slot = de_u8(reader)?;
                }
                Ok(Value::Float(f64::from_le_bytes(bits), NumKind::F64))
            }
            Tag::Float32 => {
                let bits = reader.read_u32().ok_or_else(bad)?;
                Ok(Value::Float(f64::from(f32::from_bits(bits)), NumKind::F32))
            }
            Tag::Str => Ok(Value::Str(Rc::from(reader.read_str().ok_or_else(bad)?.as_str()))),
            Tag::Ref => {
                let index = reader.read_uvarint().ok_or_else(bad)? as usize;
                memo.get(index).cloned().ok_or_else(|| {
                    RunError::Native("deserialize: back-reference out of range".into())
                })
            }
            Tag::BigInt => {
                let text = reader.read_str().ok_or_else(bad)?;
                let value = zun_core::BigInt::parse(&text)
                    .map_err(|_| RunError::Native("deserialize: malformed bigint".into()))?;
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_bigint(value));
                memo.push(Value::Object(obj));
                Ok(Value::Object(obj))
            }
            Tag::ArrayBuffer => {
                let bytes = reader.read_bytes().ok_or_else(bad)?.to_vec();
                let obj = zun_gc::with_current_heap_mut(|heap| {
                    let buffer = heap.alloc_buffer(bytes.len());
                    for (i, byte) in bytes.iter().enumerate() {
                        heap.buffer_set(buffer, i, *byte);
                    }
                    buffer
                });
                memo.push(Value::Object(obj));
                Ok(Value::Object(obj))
            }
            Tag::DataView => {
                let bytes = reader.read_bytes().ok_or_else(bad)?.to_vec();
                let offset = reader.read_uvarint().ok_or_else(bad)? as usize;
                let length = reader.read_uvarint().ok_or_else(bad)? as usize;
                let tracking = de_u8(reader)? == 1;
                let obj = zun_gc::with_current_heap_mut(|heap| {
                    let buffer = heap.alloc_buffer(bytes.len());
                    for (i, byte) in bytes.iter().enumerate() {
                        heap.buffer_set(buffer, i, *byte);
                    }
                    if tracking {
                        heap.alloc_data_view_tracking(buffer, offset)
                    } else {
                        heap.alloc_data_view(buffer, offset, length)
                    }
                })
                .ok_or_else(bad)?;
                memo.push(Value::Object(obj));
                Ok(Value::Object(obj))
            }
            Tag::TypedArray => {
                let element = zun_core::TypedElement::from_index(u32::from(de_u8(reader)?))
                    .ok_or_else(|| RunError::Native("deserialize: unknown typed-array kind".into()))?;
                let bytes = reader.read_bytes().ok_or_else(bad)?.to_vec();
                let length = bytes.len() / element.size().max(1);
                let obj = zun_gc::with_current_heap_mut(|heap| {
                    let buffer = heap.alloc_buffer(bytes.len());
                    for (i, byte) in bytes.iter().enumerate() {
                        heap.buffer_set(buffer, i, *byte);
                    }
                    heap.alloc_typed_array(buffer, 0, length, element)
                })
                .ok_or_else(bad)?;
                memo.push(Value::Object(obj));
                Ok(Value::Object(obj))
            }
            Tag::Date => {
                let valid = de_u8(reader)?;
                let millis = if valid == 1 {
                    Some(reader.read_i64().ok_or_else(bad)?)
                } else {
                    None
                };
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_date(millis));
                memo.push(Value::Object(obj));
                Ok(Value::Object(obj))
            }
            Tag::Array => {
                let len = reader.read_uvarint().ok_or_else(bad)? as usize;
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(len));
                // Record the shell before filling so a self/back reference resolves.
                memo.push(Value::Object(obj));
                for i in 0..len {
                    let value = self.de_value(reader, memo)?;
                    let any = self.value_to_any(&value)?;
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(obj, i, any));
                }
                Ok(Value::Object(obj))
            }
            Tag::Set => {
                let len = reader.read_uvarint().ok_or_else(bad)? as usize;
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_set());
                memo.push(Value::Object(obj));
                for _ in 0..len {
                    let value = self.de_value(reader, memo)?;
                    let any = self.value_to_any(&value)?;
                    zun_gc::with_current_heap_mut(|heap| heap.set_add(obj, any));
                }
                Ok(Value::Object(obj))
            }
            Tag::Map => {
                let len = reader.read_uvarint().ok_or_else(bad)? as usize;
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_map());
                memo.push(Value::Object(obj));
                for _ in 0..len {
                    let key = self.de_value(reader, memo)?;
                    let value = self.de_value(reader, memo)?;
                    let key = self.value_to_any(&key)?;
                    let value = self.value_to_any(&value)?;
                    zun_gc::with_current_heap_mut(|heap| heap.map_insert(obj, key, value));
                }
                Ok(Value::Object(obj))
            }
            Tag::Object | Tag::ObjectSym => {
                let len = reader.read_uvarint().ok_or_else(bad)? as usize;
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                memo.push(Value::Object(obj));
                for _ in 0..len {
                    let key = reader.read_str().ok_or_else(bad)?;
                    let value = self.de_value(reader, memo)?;
                    let any = self.value_to_any(&value)?;
                    zun_gc::with_current_heap_mut(|heap| heap.dynamic_set(obj, key, any));
                }
                if tag == Tag::ObjectSym {
                    let count = reader.read_uvarint().ok_or_else(bad)? as usize;
                    for _ in 0..count {
                        let kind = reader.read_u8().ok_or_else(bad)?;
                        let symbol = match kind {
                            0 => {
                                let key = reader.read_str().ok_or_else(bad)?;
                                zun_gc::with_current_heap_mut(|heap| heap.symbol_for(key))
                            }
                            1 => {
                                let id = reader.read_uvarint().ok_or_else(bad)?;
                                let known = zun_core::WellKnownSymbol::ALL
                                    .iter()
                                    .find(|w| w.id().get() == id)
                                    .ok_or_else(bad)?;
                                known.id()
                            }
                            _ => return Err(bad()),
                        };
                        let value = self.de_value(reader, memo)?;
                        let any = self.value_to_any(&value)?;
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set_symbol(obj, symbol, any)
                        });
                    }
                }
                Ok(Value::Object(obj))
            }
            Tag::Instance => {
                let type_rid = reader.read_uvarint().ok_or_else(bad)? as u32;
                let field_count = reader.read_uvarint().ok_or_else(bad)? as usize;
                // Token resolution is the safety gate: an unknown/invalid token
                // in this program is a clean error, never a fabricated object.
                let descriptor_fields = self
                    .types
                    .get(type_rid)
                    .ok_or_else(|| {
                        RunError::Native(
                            "deserialize: type token does not resolve in this program".into(),
                        )
                    })?
                    .fields
                    .len();
                let layout = self.instance_serialized_fields(type_rid)?;
                if layout.len() != field_count {
                    return Err(RunError::Native(
                        "deserialize: type field count does not match the stream".into(),
                    ));
                }
                let obj =
                    zun_gc::with_current_heap_mut(|heap| heap.alloc(type_rid, descriptor_fields));
                memo.push(Value::Object(obj));
                for (slot, kind) in layout {
                    let value = self.de_value(reader, memo)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        value::store_field(kind, obj, slot, &value, heap)
                    });
                }
                Ok(Value::Object(obj))
            }
        }
    }

    /// The `(slot, kind)` of each instance field serialized for `type_rid`, in
    /// the exact order and with the exact filter [`Interp::object_own_entries`]
    /// uses (declared instance fields, base classes first, `static`/`private`
    /// excluded), so serialize and deserialize agree field-for-field.
    fn instance_serialized_fields(&self, type_rid: u32) -> Result<Vec<(usize, u8)>, RunError> {
        let mut chain = Vec::new();
        let mut current = type_rid;
        loop {
            chain.push(current);
            let ty = self
                .module
                .tables
                .type_defs
                .get(current as usize - 1)
                .ok_or(RunError::BadToken)?;
            if ty.base.is_null() || ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                break;
            }
            current = ty.base.rid();
        }
        chain.reverse();
        let mut out = Vec::new();
        for type_rid in chain {
            let (field_start, field_end) = {
                let ty = self
                    .module
                    .tables
                    .type_defs
                    .get(type_rid as usize - 1)
                    .ok_or(RunError::BadToken)?;
                (ty.field_start, ty.field_end)
            };
            for field_rid in field_start..field_end {
                let field = self
                    .module
                    .tables
                    .field_defs
                    .get(field_rid as usize - 1)
                    .ok_or(RunError::BadToken)?;
                if field.is_static || field.is_private {
                    continue;
                }
                let layout = self.types.get_field(field_rid).ok_or(RunError::BadToken)?;
                out.push((layout.slot, layout.kind));
            }
        }
        Ok(out)
    }

    /// Unbox a managed `Any` element into a concrete `Value` for the walk.
    fn any_to_value(&self, any: Any) -> Result<Value, RunError> {
        zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)).ok_or(RunError::BadToken)
    }

    /// Box a reconstructed `Value` back into an `Any` for a container slot.
    fn value_to_any(&self, value: &Value) -> Result<Any, RunError> {
        zun_gc::with_current_heap_mut(|heap| value.box_any(heap)).ok_or_else(|| {
            RunError::Native("deserialize: value cannot be stored in a container".into())
        })
    }
}

fn write_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    write_uvarint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn write_str(out: &mut Vec<u8>, text: &str) {
    out.push(Tag::Str.to_u8());
    write_bytes(out, text.as_bytes());
}

fn de_u8(reader: &mut Reader) -> Result<u8, RunError> {
    reader.read_u8().ok_or_else(bad)
}

fn bad() -> RunError {
    RunError::Native("deserialize: truncated or malformed byte stream".into())
}
