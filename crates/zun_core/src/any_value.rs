/// Runtime tag carried by a dynamically typed `Any` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AnyTag { Int = 0, Float = 1, Bool = 2, Object = 3, String = 4, Null = 5, Void = 6 }

impl AnyTag {
    /// Decode the stable tag byte used by native/JIT ABI boundaries.
    pub const fn from_raw(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Int),
            1 => Some(Self::Float),
            2 => Some(Self::Bool),
            3 => Some(Self::Object),
            4 => Some(Self::String),
            5 => Some(Self::Null),
            6 => Some(Self::Void),
            _ => None,
        }
    }
}

/// Explicit tagged-union representation for opt-in dynamic values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Any { tag: AnyTag, payload: u64 }

impl Any {
    pub const fn void() -> Self { Self { tag: AnyTag::Void, payload: 0 } }
    pub const fn null() -> Self { Self { tag: AnyTag::Null, payload: 0 } }
    pub const fn int(value: i64) -> Self { Self { tag: AnyTag::Int, payload: value as u64 } }
    pub const fn float(value: f64) -> Self { Self { tag: AnyTag::Float, payload: value.to_bits() } }
    pub const fn bool(value: bool) -> Self { Self { tag: AnyTag::Bool, payload: value as u64 } }
    pub const fn object(address: usize) -> Self { Self { tag: AnyTag::Object, payload: address as u64 } }
    pub const fn string(handle: u32) -> Self { Self { tag: AnyTag::String, payload: handle as u64 } }
    /// Return the two machine-word components used by the native/JIT ABI.
    pub const fn raw_parts(self) -> (u8, u64) { (self.tag as u8, self.payload) }
    /// Reconstruct an `Any` value from validated ABI components.
    pub const fn from_raw_parts(tag: u8, payload: u64) -> Option<Self> {
        match AnyTag::from_raw(tag) {
            Some(tag) => Some(Self { tag, payload }),
            None => None,
        }
    }
    pub const fn tag(self) -> AnyTag { self.tag }
    pub const fn as_int(self) -> Option<i64> { match self.tag { AnyTag::Int => Some(self.payload as i64), _ => None } }
    pub const fn as_float(self) -> Option<f64> { match self.tag { AnyTag::Float => Some(f64::from_bits(self.payload)), _ => None } }
    pub const fn as_bool(self) -> Option<bool> { match self.tag { AnyTag::Bool => Some(self.payload != 0), _ => None } }
    pub const fn as_object(self) -> Option<usize> { match self.tag { AnyTag::Object => Some(self.payload as usize), _ => None } }
    pub fn map_object_address(self, map: impl FnOnce(usize) -> usize) -> Self {
        match self.as_object() {
            Some(address) => Self::object(map(address)),
            None => self,
        }
    }
    pub const fn as_string(self) -> Option<u32> { match self.tag { AnyTag::String => Some(self.payload as u32), _ => None } }
}

#[cfg(test)]
mod tests {
    use super::{Any, AnyTag};

    #[test]
    fn any_abi_roundtrips_tag_and_payload_words() {
        for value in [Any::int(-7), Any::float(2.5), Any::bool(true), Any::object(0x1234), Any::string(9), Any::null(), Any::void()] {
            let (tag, payload) = value.raw_parts();
            assert_eq!(Any::from_raw_parts(tag, payload), Some(value));
        }
        assert_eq!(Any::from_raw_parts(255, 0), None);
        assert_eq!(core::mem::size_of::<Any>(), 16);
        assert_eq!(core::mem::align_of::<Any>(), 8);
        assert_eq!(AnyTag::from_raw(5), Some(AnyTag::Null));
        assert_eq!(AnyTag::from_raw(6), Some(AnyTag::Void));
    }
}
