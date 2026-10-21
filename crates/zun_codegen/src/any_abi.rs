//! Two-word carrier used when managed `Any` values cross the JIT boundary.

use zun_il::JitKind;

/// Number of machine words occupied by one logical JIT argument.
#[inline]
pub const fn abi_word_count(kind: JitKind) -> usize {
    if matches!(kind, JitKind::Other) { 2 } else { 1 }
}

/// C-ABI representation of [`zun_core::Any`].
///
/// The tag and payload are intentionally separate machine words. Keeping this
/// carrier explicit prevents the typed JIT from treating the 16-byte dynamic
/// value as a single `i64` and losing object-reference metadata.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitAnyAbi {
    /// Stable [`zun_core::AnyTag`] discriminant in the low byte.
    pub tag: u64,
    /// Payload word (integer bits, string handle, or managed address).
    pub payload: u64,
}

impl JitAnyAbi {
    /// Return the carrier as the two machine words used by the JIT boundary.
    pub const fn raw_words(self) -> (u64, u64) {
        (self.tag, self.payload)
    }

    /// Reconstruct a carrier from its two machine words without validating the
    /// dynamic tag. Validation remains available through [`into_any`].
    pub const fn from_raw_words(tag: u64, payload: u64) -> Self {
        Self { tag, payload }
    }

    /// Encode a runtime `Any` value for a native call boundary.
    pub const fn from_any(value: zun_core::Any) -> Self {
        let (tag, payload) = value.raw_parts();
        Self { tag: tag as u64, payload }
    }

    /// Validate and decode a carrier returned by native/JIT code.
    pub const fn into_any(self) -> Option<zun_core::Any> {
        zun_core::Any::from_raw_parts(self.tag as u8, self.payload)
    }

    /// Append the payload as a GC root when this carrier contains an object.
    /// The tag must be inspected before the payload is interpreted as an
    /// address; integers, strings, and null are not managed object pointers.
    pub fn append_managed_root(self, roots: &mut Vec<zun_gc::Obj>) {
        if self.tag == zun_core::AnyTag::Object as u64 && self.payload != 0 {
            roots.push(unsafe { zun_gc::Obj::from_addr(self.payload as *mut u64) });
        }
    }

    /// Rewrite an object payload after a moving collection.
    pub fn rewrite_managed_payload(&mut self, relocation: &zun_gc::RelocationMap) -> bool {
        if self.tag != zun_core::AnyTag::Object as u64 || self.payload == 0 {
            return false;
        }
        let mapped = relocation.relocate_address(self.payload as usize);
        if mapped == self.payload as usize {
            return false;
        }
        self.payload = mapped as u64;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{abi_word_count, JitAnyAbi};
    use zun_il::{JitKind, NumKind};

    #[test]
    fn any_carrier_width_is_two_words() {
        assert_eq!(abi_word_count(JitKind::Other), 2);
        assert_eq!(abi_word_count(JitKind::Num(NumKind::I64)), 1);
    }

    #[test]
    fn carrier_roundtrips_any_without_collapsing_two_words() {
        let value = zun_core::Any::object(0x1234);
        let carrier = JitAnyAbi::from_any(value);
        assert_eq!(carrier.into_any(), Some(value));
        assert_eq!(core::mem::size_of::<JitAnyAbi>(), 16);
        assert_eq!(core::mem::align_of::<JitAnyAbi>(), 8);
        assert_eq!(JitAnyAbi { tag: 255, payload: 0 }.into_any(), None);
        let words = carrier.raw_words();
        assert_eq!(JitAnyAbi::from_raw_words(words.0, words.1), carrier);
    }

    #[test]
    fn carrier_exposes_and_rewrites_managed_object_payloads() {
        let old = 0x1000usize;
        let new = 0x2000usize;
        let mut carrier = JitAnyAbi {
            tag: zun_core::AnyTag::Object as u64,
            payload: old as u64,
        };
        let mut roots = Vec::new();
        carrier.append_managed_root(&mut roots);
        assert_eq!(roots[0].addr() as usize, old);

        let mut relocation = zun_gc::RelocationMap::new();
        relocation.insert_addresses(old, new);
        assert!(carrier.rewrite_managed_payload(&relocation));
        assert_eq!(carrier.payload, new as u64);
    }
}
