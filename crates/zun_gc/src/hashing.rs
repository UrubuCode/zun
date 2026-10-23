use super::{Heap, Obj};
use super::value_ops::hash_bytes;

impl Heap {
    pub(super) fn any_hash(&self, value: zun_core::Any) -> u64 {
        match value.tag() {
            zun_core::AnyTag::Int => value.as_int().unwrap() as u64 ^ 0x11,
            zun_core::AnyTag::Float => value.as_float().unwrap().to_bits() ^ 0x22,
            zun_core::AnyTag::Bool => value.as_bool().unwrap() as u64 ^ 0x33,
            zun_core::AnyTag::Object => {
                unsafe { Obj::from_addr(value.as_object().unwrap() as *mut u64) }.identity_hash() ^ 0x44
            }
            zun_core::AnyTag::String => {
                self.try_string(value.as_string().unwrap()).map_or(0, |s| hash_bytes(s.as_bytes())) ^ 0x55
            }
            zun_core::AnyTag::Null => 0x66,
            zun_core::AnyTag::Void => 0x77,
            zun_core::AnyTag::ClassDescriptor => value.as_class_descriptor().unwrap() as u64 ^ 0x88,
            zun_core::AnyTag::Symbol => value.as_symbol().unwrap().get() ^ 0x99,
        }
    }
}
