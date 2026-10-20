//! Frozen contracts shared by the Zun runtime and its standard-library intrinsics.
//!
//! This crate deliberately has no dependency on the interpreter, compiler, or
//! heap implementation. It is the small Rust equivalent of the runtime-owned
//! portion of `System.Private.CoreLib`.

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
    fn any_roundtrips_each_dynamic_tag() {
        assert_eq!(Any::int(-7).as_int(), Some(-7));
        assert_eq!(Any::float(3.5).as_float(), Some(3.5));
        assert_eq!(Any::bool(true).as_bool(), Some(true));
        assert_eq!(Any::object(0x1234).as_object(), Some(0x1234));
        assert_eq!(Any::string(9).as_string(), Some(9));
        assert_eq!(Any::int(1).as_float(), None);
    }
}
