/// Runtime tag carried by a dynamically typed `Any` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AnyTag { Int = 0, Float = 1, Bool = 2, Object = 3, String = 4, Null = 5 }

/// Explicit tagged-union representation for opt-in dynamic values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Any { tag: AnyTag, payload: u64 }

impl Any {
    pub const fn null() -> Self { Self { tag: AnyTag::Null, payload: 0 } }
    pub const fn int(value: i64) -> Self { Self { tag: AnyTag::Int, payload: value as u64 } }
    pub const fn float(value: f64) -> Self { Self { tag: AnyTag::Float, payload: value.to_bits() } }
    pub const fn bool(value: bool) -> Self { Self { tag: AnyTag::Bool, payload: value as u64 } }
    pub const fn object(address: usize) -> Self { Self { tag: AnyTag::Object, payload: address as u64 } }
    pub const fn string(handle: u32) -> Self { Self { tag: AnyTag::String, payload: handle as u64 } }
    pub const fn tag(self) -> AnyTag { self.tag }
    pub const fn as_int(self) -> Option<i64> { match self.tag { AnyTag::Int => Some(self.payload as i64), _ => None } }
    pub const fn as_float(self) -> Option<f64> { match self.tag { AnyTag::Float => Some(f64::from_bits(self.payload)), _ => None } }
    pub const fn as_bool(self) -> Option<bool> { match self.tag { AnyTag::Bool => Some(self.payload != 0), _ => None } }
    pub const fn as_object(self) -> Option<usize> { match self.tag { AnyTag::Object => Some(self.payload as usize), _ => None } }
    pub const fn as_string(self) -> Option<u32> { match self.tag { AnyTag::String => Some(self.payload as u32), _ => None } }
}
