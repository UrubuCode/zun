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
    RegExp,
    Dynamic,
    /// A typed class instance, carrying its metadata type token.
    Instance(u32),
    /// A kind the ser/de primitive does not cover (Promise, RegExp, Proxy,
    /// WeakMap/WeakSet, Error, DataView, a bound-function record, …).
    Unsupported,
}

/// An element parked across a child walk: either a plain value (no heap
/// reference to keep current) or an index into the walk's rooted scratch slots.
enum Item {
    Plain(Value),
    Slot(usize),
}

/// The serialize walk's state. Everything that names a heap object is held
/// in registry-owned [`zun_gc::RootArray`]s and addressed by index, because a
/// getter (user code) can run mid-walk and a collection can move every object.
struct SerState {
    out: Vec<u8>,
    /// Every object met so far, by memo index. The collector keeps these slots
    /// current, and they keep the objects alive.
    nodes: zun_gc::RootArray,
    count: usize,
    /// Address → memo index. Only valid while no user code has run since the
    /// last [`SerState::rebuild`]; a collection invalidates every key.
    by_addr: HashMap<usize, u32>,
    /// A stack of parked snapshot elements (see [`Item`]).
    scratch: zun_gc::RootArray,
    scratch_top: usize,
}

impl SerState {
    fn new() -> Self {
        Self {
            out: vec![SERDE_VERSION],
            nodes: zun_gc::RootArray::new(0),
            count: 0,
            by_addr: HashMap::new(),
            scratch: zun_gc::RootArray::new(0),
            scratch_top: 0,
        }
    }

    fn lookup(&self, obj: Obj) -> Option<u32> {
        self.by_addr.get(&(obj.addr() as usize)).copied()
    }

    /// Give `obj` the next memo index and root it; answers the index.
    fn register(&mut self, obj: Obj) -> usize {
        let index = self.count;
        self.count += 1;
        self.nodes.set(index, obj);
        self.by_addr.insert(obj.addr() as usize, index as u32);
        index
    }

    /// The object with memo index `index`, at its current address.
    fn node(&self, index: usize) -> Obj {
        self.nodes
            .get(index)
            .expect("every registered memo index holds an object")
    }

    /// Rebuild the address memo from the rooted slots after user code ran.
    fn rebuild(&mut self) {
        self.by_addr.clear();
        for index in 0..self.count {
            self.by_addr
                .insert(self.node(index).addr() as usize, index as u32);
        }
    }

    fn mark(&self) -> usize {
        self.scratch_top
    }

    fn stash(&mut self, value: Value) -> Item {
        match value {
            Value::Object(obj) => {
                self.scratch.set(self.scratch_top, obj);
                self.scratch_top += 1;
                Item::Slot(self.scratch_top - 1)
            }
            other => Item::Plain(other),
        }
    }

    fn unstash(&self, item: &Item) -> Value {
        match item {
            Item::Plain(value) => value.clone(),
            Item::Slot(slot) => Value::Object(
                self.scratch
                    .get(*slot)
                    .expect("a stashed slot is filled until released"),
            ),
        }
    }

    fn release(&mut self, mark: usize) {
        for slot in mark..self.scratch_top {
            self.scratch.clear(slot);
        }
        self.scratch_top = mark;
    }
}

impl Interp<'_> {
    // ---- serialize ---------------------------------------------------------

    /// `serialize(value)` → a fresh `Uint8Array` of the encoded bytes.
    pub(crate) fn serialize_value(&mut self, value: &Value) -> Result<Value, RunError> {
        let mut state = SerState::new();
        self.ser_value(value, &mut state)?;
        let out = state.out;
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

    /// Encode one value. A `Value::Object` handed in is registered in the walk's
    /// rooted tables before anything else can run, so the caller may drop its
    /// own copy of the reference as soon as this call starts.
    fn ser_value(&mut self, value: &Value, st: &mut SerState) -> Result<(), RunError> {
        match value {
            Value::Null => st.out.push(Tag::Null.to_u8()),
            Value::Void => st.out.push(Tag::Undefined.to_u8()),
            Value::Bool(false) => st.out.push(Tag::False.to_u8()),
            Value::Bool(true) => st.out.push(Tag::True.to_u8()),
            Value::Int(v, kind) => {
                st.out.push(Tag::Int.to_u8());
                st.out.push(kind.to_imm() as u8);
                st.out.extend_from_slice(&v.to_le_bytes());
            }
            Value::Float(v, NumKind::F32) => {
                st.out.push(Tag::Float32.to_u8());
                st.out
                    .extend_from_slice(&(*v as f32).to_bits().to_le_bytes());
            }
            Value::Float(v, _) => {
                st.out.push(Tag::Float64.to_u8());
                st.out.extend_from_slice(&v.to_bits().to_le_bytes());
            }
            Value::Str(s) => write_str(&mut st.out, s),
            Value::Function(_) => {
                return Err(RunError::Native(
                    "DOMException: DataCloneError: function could not be cloned.".into(),
                ))
            }
            Value::Object(obj) => self.ser_object(*obj, st)?,
            Value::Any(any) => match any.tag() {
                zun_core::AnyTag::Object => {
                    let address = any.as_object().ok_or(RunError::BadToken)?;
                    let obj = unsafe { Obj::from_addr(address as *mut u64) };
                    self.ser_object(obj, st)?;
                }
                zun_core::AnyTag::Symbol | zun_core::AnyTag::ClassDescriptor => {
                    return Err(RunError::Native(
                        "DOMException: DataCloneError: Symbol() could not be cloned.".into(),
                    ))
                }
                // A primitive carrier: unbox to its concrete Value and encode.
                _ => {
                    let v = zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .ok_or(RunError::BadToken)?;
                    self.ser_value(&v, st)?;
                }
            },
        }
        Ok(())
    }

    /// Encode a heap object.
    ///
    /// **Reentrancy contract.** An accessor property is read by calling its
    /// getter, which is user code and can collect (and move) any object. So the
    /// walk never keeps a heap reference in a Rust local across a child walk or
    /// a getter call: `obj` is registered in `st.nodes` on entry, and every
    /// container re-reads itself from that rooted slot (`st.node(node)`) and
    /// re-reads each child from the container just before descending into it.
    /// Snapshots that must outlive a child walk (a `Map`'s entries) live in
    /// `st.scratch`, also registry-owned. The `obj` parameter is only used
    /// before the first child walk.
    fn ser_object(&mut self, obj: Obj, st: &mut SerState) -> Result<(), RunError> {
        if let Some(index) = st.lookup(obj) {
            st.out.push(Tag::Ref.to_u8());
            write_uvarint(&mut st.out, u64::from(index));
            return Ok(());
        }
        // Assign this object's memo index BEFORE encoding its children, so a
        // back-reference inside them resolves to this object.
        let node = st.register(obj);

        match self.classify(obj) {
            ObjKind::BigInt => {
                let text = zun_gc::with_current_heap(|heap| {
                    heap.bigint_value(obj).map(|value| value.to_string())
                })
                .ok_or(RunError::BadToken)?;
                st.out.push(Tag::BigInt.to_u8());
                write_bytes(&mut st.out, text.as_bytes());
            }
            ObjKind::TypedArray => {
                let (element, buffer, offset, length, tracking) =
                    zun_gc::with_current_heap(|heap| {
                        Some((
                            heap.typed_array_element(obj)?,
                            heap.typed_array_buffer(obj)?,
                            heap.typed_array_byte_offset(obj)?,
                            heap.typed_array_len(obj)?,
                            heap.typed_array_is_tracking(obj)?,
                        ))
                    })
                    .ok_or(RunError::BadToken)?;
                st.out.push(Tag::TypedArray.to_u8());
                st.out.push(element.index() as u8);
                // The backing buffer is a value of its own: views of one buffer
                // meet at the same memo entry and keep aliasing after the clone.
                self.ser_object(buffer, st)?;
                write_uvarint(&mut st.out, offset as u64);
                write_uvarint(&mut st.out, length as u64);
                st.out.push(u8::from(tracking));
            }
            ObjKind::DataView => {
                let (buffer, offset, length, tracking) = zun_gc::with_current_heap(|heap| {
                    Some((
                        heap.data_view_buffer(obj)?,
                        heap.data_view_byte_offset(obj)?,
                        heap.data_view_byte_len(obj)?,
                        heap.data_view_is_tracking(obj)?,
                    ))
                })
                .ok_or(RunError::BadToken)?;
                st.out.push(Tag::DataView.to_u8());
                self.ser_object(buffer, st)?;
                write_uvarint(&mut st.out, offset as u64);
                write_uvarint(&mut st.out, length as u64);
                st.out.push(u8::from(tracking));
            }
            ObjKind::ArrayBuffer => {
                let (bytes, max) = zun_gc::with_current_heap(|heap| {
                    let len = heap.buffer_len(obj)?;
                    let mut bytes = Vec::with_capacity(len);
                    for i in 0..len {
                        bytes.push(heap.buffer_get(obj, i)?);
                    }
                    let max = heap
                        .buffer_resizable(obj)?
                        .then(|| heap.buffer_max_byte_length(obj))
                        .flatten();
                    Some((bytes, max))
                })
                .ok_or(RunError::BadToken)?;
                match max {
                    Some(max) => {
                        st.out.push(Tag::ResizableBuffer.to_u8());
                        write_bytes(&mut st.out, &bytes);
                        write_uvarint(&mut st.out, max as u64);
                    }
                    None => {
                        st.out.push(Tag::ArrayBuffer.to_u8());
                        write_bytes(&mut st.out, &bytes);
                    }
                }
            }
            ObjKind::Array => {
                let len = zun_gc::with_current_heap(|heap| heap.array_len(obj))
                    .ok_or(RunError::BadToken)?;
                // Own enumerable named (non-index) data properties are cloned
                // with the array; they used to be dropped.
                let named: Vec<String> = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_keys(obj)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|key| {
                            key != "length"
                                && key.parse::<u32>().is_err()
                                && heap.dynamic_is_enumerable(obj, key)
                                && !heap.dynamic_is_accessor(obj, key)
                                && heap.dynamic_get(obj, key).is_some()
                        })
                        .collect()
                });
                let tag = if named.is_empty() { Tag::Array } else { Tag::ArrayProps };
                st.out.push(tag.to_u8());
                write_uvarint(&mut st.out, len as u64);
                for i in 0..len {
                    // Re-read the array and the element: a child walk may have
                    // moved the array, and shrunk it.
                    let array = st.node(node);
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(array, i));
                    match element {
                        Some(element) if !element.is_hole() => {
                            let value = self.any_to_value(element)?;
                            self.ser_value(&value, st)?;
                        }
                        _ => st.out.push(Tag::Hole.to_u8()),
                    }
                }
                if !named.is_empty() {
                    write_uvarint(&mut st.out, named.len() as u64);
                    for key in named {
                        write_bytes(&mut st.out, key.as_bytes());
                        let holder = st.node(node);
                        let value = match zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get(holder, &key)
                        }) {
                            Some(any) => self.any_to_value(any)?,
                            None => Value::Void,
                        };
                        self.ser_value(&value, st)?;
                    }
                }
            }
            ObjKind::Map => {
                let entries = zun_gc::with_current_heap(|heap| heap.map_entries(obj))
                    .ok_or(RunError::BadToken)?;
                st.out.push(Tag::Map.to_u8());
                write_uvarint(&mut st.out, entries.len() as u64);
                let mark = st.mark();
                let mut items = Vec::with_capacity(entries.len());
                for (key, value) in entries {
                    let key = self.stash(st, key)?;
                    let value = self.stash(st, value)?;
                    items.push((key, value));
                }
                for (key, value) in &items {
                    let key = st.unstash(key);
                    self.ser_value(&key, st)?;
                    let value = st.unstash(value);
                    self.ser_value(&value, st)?;
                }
                st.release(mark);
            }
            ObjKind::Set => {
                let values = zun_gc::with_current_heap(|heap| heap.set_values(obj))
                    .ok_or(RunError::BadToken)?;
                st.out.push(Tag::Set.to_u8());
                write_uvarint(&mut st.out, values.len() as u64);
                let mark = st.mark();
                let mut items = Vec::with_capacity(values.len());
                for value in values {
                    items.push(self.stash(st, value)?);
                }
                for item in &items {
                    let value = st.unstash(item);
                    self.ser_value(&value, st)?;
                }
                st.release(mark);
            }
            ObjKind::RegExp => {
                let (source, flags) = zun_gc::with_current_heap(|heap| {
                    (
                        heap.regex_source(obj).map(str::to_string),
                        heap.regex_flags(obj).map(str::to_string),
                    )
                });
                let (Some(source), Some(flags)) = (source, flags) else {
                    return Err(RunError::BadToken);
                };
                st.out.push(Tag::RegExp.to_u8());
                write_bytes(&mut st.out, source.as_bytes());
                write_bytes(&mut st.out, flags.as_bytes());
            }
            ObjKind::Date => {
                let (valid, millis) = zun_gc::with_current_heap(|heap| {
                    (heap.date_is_valid(obj), heap.date_millis(obj))
                });
                st.out.push(Tag::Date.to_u8());
                match (valid, millis) {
                    (Some(true), Some(millis)) => {
                        st.out.push(1);
                        st.out.extend_from_slice(&millis.to_le_bytes());
                    }
                    _ => st.out.push(0),
                }
            }
            ObjKind::Dynamic => {
                // Own enumerable string keys in property order. A data property
                // is read when its turn comes; an accessor property is read by
                // calling its getter, and the result is cloned as a plain data
                // property. A function-valued property cannot be cloned.
                let keys: Vec<(String, bool)> = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_keys(obj)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|key| heap.dynamic_is_enumerable(obj, key))
                        .map(|key| {
                            let accessor = heap.dynamic_is_accessor(obj, &key);
                            (key, accessor)
                        })
                        .collect()
                });
                // Symbol-keyed entries: only registered (`Symbol.for`) and
                // well-known symbols have an identity that survives a
                // deserialize. Unique `Symbol()` keys cannot be reconstructed, so
                // they are silently omitted (the rest of the graph still
                // round-trips). Only the keys are collected here; each value is
                // re-read from the rooted object when its turn comes.
                let symbol_entries: Vec<(u8, Option<String>, u64, zun_core::SymbolId)> =
                    zun_gc::with_current_heap(|heap| {
                        let mut found = Vec::new();
                        for symbol in heap.dynamic_symbol_keys(obj) {
                            if heap.dynamic_get_symbol(obj, symbol).is_none() {
                                continue;
                            }
                            if let Some(key) = heap.symbol_key_for(symbol) {
                                found.push((0u8, Some(key.to_string()), 0, symbol));
                            } else if zun_core::WellKnownSymbol::ALL
                                .iter()
                                .any(|w| w.id() == symbol)
                            {
                                found.push((1u8, None, symbol.get(), symbol));
                            }
                        }
                        found
                    });
                let tag = if symbol_entries.is_empty() {
                    Tag::Object
                } else {
                    Tag::ObjectSym
                };
                st.out.push(tag.to_u8());
                write_uvarint(&mut st.out, keys.len() as u64);
                for (key, accessor) in keys {
                    // The key is a bare length-prefixed string (no value tag);
                    // deserialize reads it with `read_str`.
                    write_bytes(&mut st.out, key.as_bytes());
                    let value = if accessor {
                        self.ser_read_accessor(st, node, &key)?
                    } else {
                        let holder = st.node(node);
                        if zun_gc::with_current_heap(|heap| heap.dynamic_get_function(holder, &key))
                            .is_some()
                        {
                            return Err(RunError::Native(
                                "DOMException: DataCloneError: function could not be cloned.".into(),
                            ));
                        }
                        // A key a getter deleted meanwhile reads as undefined.
                        match zun_gc::with_current_heap(|heap| heap.dynamic_get(holder, &key)) {
                            Some(any) => self.any_to_value(any)?,
                            None => Value::Void,
                        }
                    };
                    self.ser_value(&value, st)?;
                }
                if !symbol_entries.is_empty() {
                    write_uvarint(&mut st.out, symbol_entries.len() as u64);
                    for (kind, key, id, symbol) in symbol_entries {
                        st.out.push(kind);
                        match key {
                            Some(key) => write_bytes(&mut st.out, key.as_bytes()),
                            None => write_uvarint(&mut st.out, id),
                        }
                        let holder = st.node(node);
                        let value = match zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get_symbol(holder, symbol)
                        }) {
                            Some(any) => self.any_to_value(any)?,
                            None => Value::Void,
                        };
                        self.ser_value(&value, st)?;
                    }
                }
            }
            ObjKind::Instance(type_rid) => {
                // Metadata shape-walk: the declared instance fields in layout
                // order, from the SAME `(slot, kind)` table deserialize rebuilds
                // from, so the two sides agree field for field. Reflection such
                // as `object_own_entries` hides a wrapper's `value` and an
                // `Error`'s record slots and adds expandos, so it cannot define
                // the wire shape.
                // An error clones as the ES error type its name denotes (a
                // subclass or `AggregateError` flattens to `Error`); own extras
                // are not part of the layout, so they are dropped.
                // An instance of a class the *program* declares clones as a
                // plain object holding its public fields: the structured clone
                // algorithm has a fixed list of platform types it preserves,
                // and a user class is not one, so the prototype (and with it
                // every method) is dropped. Platform classes -- the prelude's
                // -- keep the instance encoding below.
                if self.error_clone_rid(type_rid).is_none() && self.is_program_class(type_rid) {
                    let fields = self.instance_named_fields(type_rid)?;
                    st.out.push(Tag::Object.to_u8());
                    write_uvarint(&mut st.out, fields.len() as u64);
                    for (name, slot, kind) in fields {
                        write_bytes(&mut st.out, name.as_bytes());
                        let holder = st.node(node);
                        let value = zun_gc::with_current_heap(|heap| {
                            value::load_field(kind, holder, slot, heap)
                        });
                        if matches!(value, Value::Function(_)) {
                            return Err(RunError::Native(
                                "DOMException: DataCloneError: function could not be cloned.".into(),
                            ));
                        }
                        self.ser_value(&value, st)?;
                    }
                    return Ok(());
                }
                let type_rid = self.error_clone_rid(type_rid).unwrap_or(type_rid);
                let layout = self.instance_serialized_fields(type_rid)?;
                st.out.push(Tag::Instance.to_u8());
                write_uvarint(&mut st.out, u64::from(type_rid));
                write_uvarint(&mut st.out, layout.len() as u64);
                for (slot, kind) in layout {
                    let holder = st.node(node);
                    let value = zun_gc::with_current_heap(|heap| {
                        value::load_field(kind, holder, slot, heap)
                    });
                    self.ser_value(&value, st)?;
                }
            }
            ObjKind::Unsupported => {
                return Err(RunError::Native(
                    "DOMException: DataCloneError: #<Object> could not be cloned.".into(),
                ))
            }
        }
        Ok(())
    }

    /// Read an accessor property of the rooted object `node` by calling its
    /// getter with the object as receiver (an accessor without a getter reads as
    /// `undefined`). The getter is user code: afterwards the address memo is
    /// rebuilt from the rooted slots, since a collection may have moved
    /// everything. The returned value is not rooted; the caller must hand it to
    /// `ser_value` immediately.
    fn ser_read_accessor(
        &mut self,
        st: &mut SerState,
        node: usize,
        key: &str,
    ) -> Result<Value, RunError> {
        let holder = st.node(node);
        let Some(getter) =
            zun_gc::with_current_heap(|heap| heap.dynamic_accessor_getter(holder, key))
        else {
            return Ok(Value::Void);
        };
        let args = self.with_this_arg(getter, holder, Vec::new());
        let result = self.invoke_function_value(getter, args);
        st.rebuild();
        Ok(match result? {
            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                .ok_or(RunError::BadToken)?,
            other => other,
        })
    }

    /// Park an element of a snapshot: a heap object goes into a rooted scratch
    /// slot, anything else is kept as a plain value.
    fn stash(&self, st: &mut SerState, any: Any) -> Result<Item, RunError> {
        let value = self.any_to_value(any)?;
        Ok(st.stash(value))
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
            } else if heap.regex_source(obj).is_some() {
                Some(ObjKind::RegExp)
            } else if heap.date_is_valid(obj).is_some() {
                Some(ObjKind::Date)
            } else if heap.is_dynamic_object(obj) && !heap.is_instance_bag(obj) {
                // (A typed instance carrying expandos is an instance bag: it
                // falls through to the metadata shape-walk below.)
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
            Tag::Undefined | Tag::Hole => Ok(Value::Void),
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
            Tag::Str => Ok(Value::Str(zun_core::ZStr::from(
                reader.read_str().ok_or_else(bad)?.as_str(),
            ))),
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
            Tag::ArrayBuffer | Tag::ResizableBuffer => {
                let bytes = reader.read_bytes().ok_or_else(bad)?.to_vec();
                let max = if tag == Tag::ResizableBuffer {
                    Some(reader.read_uvarint().ok_or_else(bad)? as usize)
                } else {
                    None
                };
                let obj = zun_gc::with_current_heap_mut(|heap| {
                    let buffer = match max {
                        Some(max) => heap.alloc_resizable_buffer(bytes.len(), max),
                        None => Some(heap.alloc_buffer(bytes.len())),
                    }?;
                    for (i, byte) in bytes.iter().enumerate() {
                        heap.buffer_set(buffer, i, *byte);
                    }
                    Some(buffer)
                })
                .ok_or_else(bad)?;
                memo.push(Value::Object(obj));
                Ok(Value::Object(obj))
            }
            Tag::DataView => {
                // Reserve the memo index first: the buffer is a child value.
                let slot = memo.len();
                memo.push(Value::Void);
                let buffer = self.de_buffer(reader, memo)?;
                let offset = reader.read_uvarint().ok_or_else(bad)? as usize;
                let length = reader.read_uvarint().ok_or_else(bad)? as usize;
                let tracking = de_u8(reader)? == 1;
                let obj = zun_gc::with_current_heap_mut(|heap| {
                    if tracking {
                        heap.alloc_data_view_tracking(buffer, offset)
                    } else {
                        heap.alloc_data_view(buffer, offset, length)
                    }
                })
                .ok_or_else(bad)?;
                memo[slot] = Value::Object(obj);
                Ok(Value::Object(obj))
            }
            Tag::TypedArray => {
                let element = zun_core::TypedElement::from_index(u32::from(de_u8(reader)?))
                    .ok_or_else(|| {
                        RunError::Native("deserialize: unknown typed-array kind".into())
                    })?;
                let slot = memo.len();
                memo.push(Value::Void);
                let buffer = self.de_buffer(reader, memo)?;
                let offset = reader.read_uvarint().ok_or_else(bad)? as usize;
                let length = reader.read_uvarint().ok_or_else(bad)? as usize;
                let tracking = de_u8(reader)? == 1;
                let obj = zun_gc::with_current_heap_mut(|heap| {
                    if tracking {
                        heap.alloc_typed_array_tracking(buffer, offset, element)
                    } else {
                        heap.alloc_typed_array(buffer, offset, length, element)
                    }
                })
                .ok_or_else(bad)?;
                memo[slot] = Value::Object(obj);
                Ok(Value::Object(obj))
            }
            Tag::RegExp => {
                let source = reader.read_str().ok_or_else(bad)?;
                let flags = reader.read_str().ok_or_else(bad)?;
                let obj = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_regex_with_flags(source, flags)
                })
                .map_err(|_| bad())?;
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
            Tag::Array | Tag::ArrayProps => {
                let len = reader.read_uvarint().ok_or_else(bad)? as usize;
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(len));
                // Record the shell before filling so a self/back reference resolves.
                memo.push(Value::Object(obj));
                for i in 0..len {
                    if reader.take_hole() {
                        continue;
                    }
                    let value = self.de_value(reader, memo)?;
                    let any = self.value_to_any(&value)?;
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(obj, i, any));
                }
                if tag == Tag::ArrayProps {
                    let count = reader.read_uvarint().ok_or_else(bad)? as usize;
                    for _ in 0..count {
                        let key = reader.read_str().ok_or_else(bad)?;
                        let value = self.de_value(reader, memo)?;
                        let any = self.value_to_any(&value)?;
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.ensure_dynamic_entry(obj);
                            heap.dynamic_set(obj, key, any);
                        });
                    }
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

    /// Read the backing buffer of a view: a full value that must decode to an
    /// `ArrayBuffer` (a back-reference to one shared with another view is fine).
    fn de_buffer(&mut self, reader: &mut Reader, memo: &mut Vec<Value>) -> Result<Obj, RunError> {
        match self.de_value(reader, memo)? {
            Value::Object(buffer)
                if zun_gc::with_current_heap(|heap| heap.buffer_len(buffer).is_some()) =>
            {
                Ok(buffer)
            }
            _ => Err(bad()),
        }
    }

    /// For an error instance of type `type_rid`: the TypeDef the structured
    /// clone carries it as. The nearest ancestor (or the type itself) that is one
    /// of the ES error types or `DOMException` wins; anything else falls back to
    /// `Error`. `None` when `type_rid` is not an error type.
    fn error_clone_rid(&self, type_rid: u32) -> Option<u32> {
        const CLONED: [&str; 8] = [
            "Error",
            "EvalError",
            "RangeError",
            "ReferenceError",
            "SyntaxError",
            "TypeError",
            "URIError",
            "DOMException",
        ];
        let mut found = None;
        let mut current = type_rid;
        while current != 0 {
            let ty = self.module.tables.type_defs.get(current as usize - 1)?;
            let name = self.module.strings.get(ty.name);
            if found.is_none() && CLONED.contains(&name) {
                found = Some(current);
            }
            if name == "Error" {
                return found.or(Some(current));
            }
            if ty.base.is_null() || ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                return None;
            }
            current = ty.base.rid();
        }
        None
    }

    /// The `(slot, kind)` of each instance field serialized for `type_rid`, in
    /// declaration order (declared instance fields, base classes first,
    /// `static`/`private` excluded), so serialize and deserialize agree
    /// field-for-field.
    /// Whether class `type_rid` is declared by the program rather than by the
    /// prelude. The prelude is compiled as source *after* the user's, so its
    /// classes take the higher TypeDef rids, and `Error` is the first of them.
    fn is_program_class(&self, type_rid: u32) -> bool {
        self.type_rid_of("Error").is_some_and(|first_prelude| type_rid < first_prelude)
    }

    /// The public instance fields of `type_rid` as `(name, slot, kind)`, in
    /// layout order (inherited first).
    fn instance_named_fields(&self, type_rid: u32) -> Result<Vec<(String, usize, u8)>, RunError> {
        let ty = self
            .module
            .tables
            .type_defs
            .get(type_rid as usize - 1)
            .ok_or(RunError::BadToken)?;
        let mut out = Vec::new();
        for field_rid in ty.field_start..ty.field_end {
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
            out.push((self.module.strings.get(field.name).to_string(), layout.slot, layout.kind));
        }
        Ok(out)
    }

    fn instance_serialized_fields(&self, type_rid: u32) -> Result<Vec<(usize, u8)>, RunError> {
        // A `TypeDef`'s field range already includes the inherited fields (base
        // first), so only the concrete type is walked.
        let mut out = Vec::new();
        for type_rid in [type_rid] {
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
