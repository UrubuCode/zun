//! Two-word carrier used when managed `Any` values cross the JIT boundary.

use zun_il::JitKind;

/// Native layout of the two-word managed `Any` carrier.
pub const TAG_OFFSET: i32 = 0;
pub const PAYLOAD_OFFSET: i32 = 8;
pub const CARRIER_SIZE: u32 = TAG_OFFSET as u32 + 16;
pub const CARRIER_ALIGN_SHIFT: u8 = 3;

/// Number of machine words occupied by one logical JIT argument.
#[inline]
pub const fn abi_word_count(kind: JitKind) -> usize {
    if matches!(kind, JitKind::Other) {
        2
    } else {
        1
    }
}

/// Whether a kind's packed word is the address of a GC-managed heap object that
/// must be rooted (via `zun_gc::Obj::from_addr`) while native code runs.
///
/// This is the single source of truth for "is this `JitKind` a GC reference" in
/// the Rust entry points (`call_packed*`, `call_osr*`). Per kind:
/// - `Object`, `Array`: heap object addresses, moved/freed by the collector.
/// - `String`: an index into the append-only `Heap::strings` table, which is
///   neither collected nor moved, so no rooting is needed.
/// - `Function`: a one-word method rid (closures are excluded by eligibility).
/// - `Num`, `Bool`: scalars.
/// - `Other`: the two-word `Any` carrier, handled by [`collect_native_roots`].
///
/// When adding a `JitKind`, decide it here; the match is deliberately exhaustive.
#[inline]
pub const fn is_gc_reference(kind: JitKind) -> bool {
    match kind {
        JitKind::Object | JitKind::Array => true,
        JitKind::Num(_)
        | JitKind::Bool
        | JitKind::String
        | JitKind::Function
        | JitKind::Other => false,
    }
}

/// Collect the GC roots held by packed argument words for a native call.
///
/// With `carrier_any == true`, `words` use the ABI layout (`Other` occupies two
/// words, see [`abi_word_count`]) and an `Any` carrier with an invalid tag is
/// rejected with [`crate::JitError::InvalidAnyTag`]; an `Object`-tagged carrier
/// contributes its payload. With `carrier_any == false` every kind occupies one
/// word (OSR local slots and `call_packed`) and `Other` carries nothing.
/// Null (zero) addresses are never rooted.
pub(crate) fn collect_native_roots(
    kinds: &[JitKind],
    words: &[u64],
    carrier_any: bool,
) -> Result<Vec<zun_gc::Obj>, crate::JitError> {
    let mut roots = Vec::new();
    let mut cursor = 0usize;
    for &kind in kinds {
        let Some(&word) = words.get(cursor) else {
            break;
        };
        if is_gc_reference(kind) {
            if word != 0 {
                roots.push(unsafe { zun_gc::Obj::from_addr(word as *mut u64) });
            }
        } else if carrier_any && matches!(kind, JitKind::Other) {
            let payload = words.get(cursor + 1).copied().unwrap_or(0);
            let carrier = JitAnyAbi::from_raw_words(word, payload);
            if carrier.into_any().is_none() {
                return Err(crate::JitError::InvalidAnyTag(word));
            }
            carrier.append_managed_root(&mut roots);
        }
        cursor += if carrier_any { abi_word_count(kind) } else { 1 };
    }
    Ok(roots)
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
        Self {
            tag: tag as u64,
            payload,
        }
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
    use super::{
        abi_word_count, JitAnyAbi, CARRIER_ALIGN_SHIFT, CARRIER_SIZE, PAYLOAD_OFFSET, TAG_OFFSET,
    };
    use zun_il::{JitKind, NumKind};

    #[test]
    fn any_carrier_width_is_two_words() {
        assert_eq!(abi_word_count(JitKind::Other), 2);
        assert_eq!(abi_word_count(JitKind::Num(NumKind::I64)), 1);
    }

    #[test]
    fn gc_reference_predicate_covers_object_and_array_only() {
        use super::is_gc_reference;
        assert!(is_gc_reference(JitKind::Object));
        assert!(is_gc_reference(JitKind::Array));
        for kind in [
            JitKind::String,
            JitKind::Function,
            JitKind::Other,
            JitKind::Bool,
            JitKind::Num(NumKind::F64),
        ] {
            assert!(!is_gc_reference(kind));
        }
    }

    #[test]
    fn root_set_includes_arrays_and_object_tagged_carriers() {
        use super::collect_native_roots;
        let kinds = [
            JitKind::Num(NumKind::I64),
            JitKind::Array,
            JitKind::String,
            JitKind::Other,
            JitKind::Object,
        ];
        let carrier = JitAnyAbi::from_any(zun_core::Any::object(0x3000));
        let words = [7, 0x1000, 0x55, carrier.tag, carrier.payload, 0x2000];
        let roots = collect_native_roots(&kinds, &words, true).unwrap();
        let addrs: Vec<usize> = roots.iter().map(|o| o.addr() as usize).collect();
        assert_eq!(addrs, vec![0x1000, 0x3000, 0x2000]);

        // One word per kind (OSR locals): `Other` carries nothing.
        let packed = [7, 0x1000, 0x55, 0, 0x2000];
        let roots = collect_native_roots(&kinds, &packed, false).unwrap();
        let addrs: Vec<usize> = roots.iter().map(|o| o.addr() as usize).collect();
        assert_eq!(addrs, vec![0x1000, 0x2000]);

        // Invalid carrier tag is an error on the carrier layout.
        let bad = [7, 0x1000, 0x55, 255, 0, 0x2000];
        assert!(matches!(
            collect_native_roots(&kinds, &bad, true),
            Err(crate::JitError::InvalidAnyTag(255))
        ));
    }

    #[test]
    fn collection_keeps_rooted_array_argument_alive() {
        use super::collect_native_roots;
        let mut heap = zun_gc::Heap::new();
        let array = heap.alloc_array(0);
        heap.array_push(array, zun_core::Any::int(41));
        heap.array_push(array, zun_core::Any::int(42));
        let garbage_a = heap.alloc(1, 0);
        let garbage_b = heap.alloc(1, 0);
        let _ = (garbage_a, garbage_b);
        let handle = heap.intern_string("kept");

        let kinds = [JitKind::Array, JitKind::String];
        let words = [array.addr() as u64, u64::from(handle)];
        let roots = collect_native_roots(&kinds, &words, false).unwrap();
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let freed = unsafe {
            heap.collect_with_native_roots(&[], &|_| std::rc::Rc::from(Vec::new()))
        };
        assert_eq!(freed, 2, "only the two unrooted objects are reclaimed");
        assert_eq!(heap.live_count(), 1);
        assert_eq!(heap.array_len(array), Some(2));
        assert_eq!(heap.array_get(array, 1), Some(zun_core::Any::int(42)));
        assert_eq!(heap.try_string(handle).map(|s| s.to_string()), Some("kept".into()));
    }

    #[test]
    fn carrier_roundtrips_any_without_collapsing_two_words() {
        assert_eq!(TAG_OFFSET, 0);
        assert_eq!(PAYLOAD_OFFSET, 8);
        assert_eq!(CARRIER_SIZE, 16);
        assert_eq!(CARRIER_ALIGN_SHIFT, 3);
        let value = zun_core::Any::object(0x1234);
        let carrier = JitAnyAbi::from_any(value);
        assert_eq!(carrier.into_any(), Some(value));
        assert_eq!(core::mem::size_of::<JitAnyAbi>(), 16);
        assert_eq!(core::mem::align_of::<JitAnyAbi>(), 8);
        assert_eq!(
            JitAnyAbi {
                tag: 255,
                payload: 0
            }
            .into_any(),
            None
        );
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
