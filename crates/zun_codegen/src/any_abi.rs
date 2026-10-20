//! Two-word carrier used when managed `Any` values cross the JIT boundary.

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
    /// Encode a runtime `Any` value for a native call boundary.
    pub const fn from_any(value: zun_core::Any) -> Self {
        let (tag, payload) = value.raw_parts();
        Self { tag: tag as u64, payload }
    }

    /// Validate and decode a carrier returned by native/JIT code.
    pub const fn into_any(self) -> Option<zun_core::Any> {
        zun_core::Any::from_raw_parts(self.tag as u8, self.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::JitAnyAbi;

    #[test]
    fn carrier_roundtrips_any_without_collapsing_two_words() {
        let value = zun_core::Any::object(0x1234);
        let carrier = JitAnyAbi::from_any(value);
        assert_eq!(carrier.into_any(), Some(value));
        assert_eq!(core::mem::size_of::<JitAnyAbi>(), 16);
        assert_eq!(core::mem::align_of::<JitAnyAbi>(), 8);
        assert_eq!(JitAnyAbi { tag: 255, payload: 0 }.into_any(), None);
    }
}
