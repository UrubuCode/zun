pub(super) fn hash_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(1469598103934665603, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
    })
}

pub(super) fn any_same_value_zero(
    a: zun_core::Any,
    b: zun_core::Any,
    strings: &[zun_core::ZStr],
    bigints: &std::collections::HashMap<usize, zun_core::BigInt>,
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
        // A BigInt is a primitive carried as a heap payload: two of them are
        // the same key when their values are equal, not when they are the
        // same allocation. (`new Map().set(1n, x).get(1n)` was `undefined`.)
        zun_core::AnyTag::Object => {
            let (left, right) = (a.as_object(), b.as_object());
            if left == right {
                return true;
            }
            if bigints.is_empty() {
                return false;
            }
            match (
                left.and_then(|addr| bigints.get(&addr)),
                right.and_then(|addr| bigints.get(&addr)),
            ) {
                (Some(x), Some(y)) => x.cmp(y) == std::cmp::Ordering::Equal,
                _ => false,
            }
        }
        zun_core::AnyTag::Null => true,
        zun_core::AnyTag::Void => true,
        zun_core::AnyTag::ClassDescriptor => a.as_class_descriptor() == b.as_class_descriptor(),
        zun_core::AnyTag::Symbol => a.as_symbol() == b.as_symbol(),
    }
}
