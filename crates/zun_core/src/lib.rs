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
}
