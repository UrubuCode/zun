use std::rc::Rc;

pub(super) fn hash_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(1469598103934665603, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
    })
}

pub(super) fn any_same_value_zero(
    a: zun_core::Any,
    b: zun_core::Any,
    strings: &[Rc<str>],
) -> bool {
    if a.tag() != b.tag() {
        return false;
    }
    match a.tag() {
        zun_core::AnyTag::Float => {
            let left = a.as_float().unwrap();
            let right = b.as_float().unwrap();
            left == right || (left.is_nan() && right.is_nan())
        }
        zun_core::AnyTag::String => {
            strings.get(a.as_string().unwrap() as usize)
                == strings.get(b.as_string().unwrap() as usize)
        }
        zun_core::AnyTag::Int => a.as_int() == b.as_int(),
        zun_core::AnyTag::Bool => a.as_bool() == b.as_bool(),
        zun_core::AnyTag::Object => a.as_object() == b.as_object(),
        zun_core::AnyTag::Null => true,
        zun_core::AnyTag::Void => true,
        zun_core::AnyTag::ClassDescriptor => a.as_class_descriptor() == b.as_class_descriptor(),
        zun_core::AnyTag::Symbol => a.as_symbol() == b.as_symbol(),
    }
}
