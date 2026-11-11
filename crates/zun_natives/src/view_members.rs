//! The member -> native table for byte views reached by name at run time.
//!
//! A statically typed `buf.toString(..)` is lowered by the checker to a direct
//! `CallBuiltin`; a receiver typed `any` has no such lowering, so the
//! interpreter must map the member name to the same native and apply the same
//! optional-argument defaulting. This module is that mapping, kept next to the
//! native declarations so the interpreter and the checker's accessor naming
//! (`node_buffer_accessor_native`) share ONE spelling of it.
//!
//! The checker still lowers typed calls by hand (it also type-checks each
//! argument); the tests below prove every native named here is a declared
//! intrinsic, so this table cannot drift away from the declarations.

/// Largest length-like default: a value past any real length that `f64`
/// holds exactly; the natives clamp it down (matches the checker's sentinel).
pub const LENGTH_SENTINEL: i64 = 1i64 << 53;

/// One user-supplied argument slot of a view member, with what to use when the
/// caller omitted it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewArg {
    /// Must be supplied; passed through (numbers are coerced to the native's
    /// declared integer type by the interpreter).
    Required,
    /// Must be supplied and is passed boxed as `any` (search needles, seeds).
    RequiredAny,
    /// Unsigned integer default.
    U64(u64),
    /// Signed integer default.
    I64(i64),
    /// String default.
    Str(&'static str),
}

/// How a member name maps onto a native.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewMember {
    /// The intrinsic name (e.g. `nodeBufferToString`).
    pub native: String,
    /// The `zun_natives` module that declares it (`nodeBuffer` or `buffer`).
    pub module: &'static str,
    /// Argument slots after the receiver, in JS call order.
    pub args: Vec<ViewArg>,
    /// Node's `write<Kind>(value, offset, ..)` puts the value first while the
    /// native takes `(view, offset, value, ..)`: swap the first two arguments.
    pub value_first: bool,
}

/// The `nodeBuffer` native for a Buffer reader/writer method, e.g.
/// `readUInt16LE` -> `nodeBufferReadUInt16LE`.
pub fn node_buffer_accessor_native(method: &str) -> String {
    let mut chars = method.chars();
    let first = chars.next().map(|c| c.to_ascii_uppercase()).unwrap_or_default();
    format!("nodeBuffer{first}{}", chars.as_str())
}

fn node(native: String, args: Vec<ViewArg>, value_first: bool) -> Option<ViewMember> {
    // Only names that are really declared count as members.
    crate::resolve("nodeBuffer", &native)?;
    Some(ViewMember { native, module: "nodeBuffer", args, value_first })
}

fn generic(native: &str, args: Vec<ViewArg>) -> Option<ViewMember> {
    crate::resolve("buffer", native)?;
    Some(ViewMember { native: native.to_string(), module: "buffer", args, value_first: false })
}

/// Resolve `member` called with `argc` arguments on a view. `is_u8` selects
/// the Node `Buffer` surface (a `Uint8Array` view IS a Buffer, as in Node).
/// `None` when the member is not a view method (callers fall through to
/// "is not a function").
pub fn view_member(member: &str, is_u8: bool, argc: usize) -> Option<ViewMember> {
    use ViewArg::*;
    if is_u8 {
        let buffer = match member {
            "toString" => node("nodeBufferToString".into(), vec![Str("utf8"), U64(0), U64(u64::MAX)], false),
            "write" => node("nodeBufferWrite".into(), vec![Required, U64(0), Str("utf8")], false),
            "slice" => node("nodeBufferSlice".into(), vec![I64(0), I64(LENGTH_SENTINEL)], false),
            "fill" if argc == 1 => node("nodeBufferFill".into(), vec![Required], false),
            "equals" => node("nodeBufferEquals".into(), vec![Required], false),
            "copy" => node(
                "nodeBufferCopy".into(),
                vec![Required, I64(0), I64(0), I64(LENGTH_SENTINEL)],
                false,
            ),
            "swap16" | "swap32" | "swap64" => node(node_buffer_accessor_native(member), vec![], false),
            "indexOf" | "includes" => node(
                node_buffer_accessor_native(member),
                vec![RequiredAny, I64(0), Str("utf8")],
                false,
            ),
            "lastIndexOf" => node(
                node_buffer_accessor_native(member),
                vec![RequiredAny, I64(LENGTH_SENTINEL), Str("utf8")],
                false,
            ),
            "readUIntLE" | "readUIntBE" | "readIntLE" | "readIntBE" => {
                node(node_buffer_accessor_native(member), vec![U64(0), U64(1)], false)
            }
            "writeUIntLE" | "writeUIntBE" | "writeIntLE" | "writeIntBE" => {
                node(node_buffer_accessor_native(member), vec![Required, U64(0), U64(1)], true)
            }
            m if m.starts_with("read") => node(node_buffer_accessor_native(m), vec![U64(0)], false),
            m if m.starts_with("write") => {
                node(node_buffer_accessor_native(m), vec![Required, U64(0)], true)
            }
            _ => None,
        };
        if buffer.is_some() {
            return buffer;
        }
    }
    match member {
        "set" => generic("typedArraySetFrom", vec![Required, U64(0)]),
        "slice" => generic("typedArraySlice", vec![I64(0), I64(LENGTH_SENTINEL)]),
        "copyWithin" => generic("typedArrayCopyWithin", vec![Required, I64(0), I64(LENGTH_SENTINEL)]),
        "indexOf" => generic("typedArrayIndexOf", vec![RequiredAny, I64(0)]),
        "includes" => generic("typedArrayIncludes", vec![RequiredAny, I64(0)]),
        "join" => generic("typedArrayJoin", vec![Str(",")]),
        "forEach" => generic("typedArrayForEach", vec![Required]),
        "map" => generic("typedArrayMap", vec![Required]),
        "filter" => generic("typedArrayFilter", vec![Required]),
        "find" => generic("typedArrayFind", vec![Required]),
        "findIndex" => generic("typedArrayFindIndex", vec![Required]),
        "some" => generic("typedArraySome", vec![Required]),
        "every" => generic("typedArrayEvery", vec![Required]),
        "reduce" if argc >= 2 => generic("typedArrayReduce", vec![Required, RequiredAny]),
        "reduce" => generic("typedArrayReduceNoInit", vec![Required]),
        "reduceRight" if argc >= 2 => generic("typedArrayReduceRight", vec![Required, RequiredAny]),
        "reduceRight" => generic("typedArrayReduceRightNoInit", vec![Required]),
        // Kind-agnostic: `u8Array*` serves every view (the view carries its kind).
        "subarray" => generic("u8ArraySubarray", vec![I64(0), I64(LENGTH_SENTINEL)]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_members_resolve() {
        for member in [
            "toString", "write", "slice", "fill", "equals", "copy", "swap16", "swap32", "swap64",
            "indexOf", "includes", "lastIndexOf", "readUIntLE", "writeIntBE", "readUInt8",
            "readUInt16LE", "writeUInt32BE", "readDoubleLE", "writeFloatBE",
        ] {
            assert!(view_member(member, true, 1).is_some(), "Buffer member `{member}` unresolved");
        }
        for member in [
            "set", "slice", "copyWithin", "indexOf", "includes", "join", "forEach", "map", "filter",
            "find", "findIndex", "some", "every", "reduce", "reduceRight", "subarray",
        ] {
            for argc in [1, 2] {
                assert!(view_member(member, false, argc).is_some(), "view member `{member}` unresolved");
            }
        }
    }

    #[test]
    fn buffer_slice_wins_over_generic_for_u8_only() {
        assert_eq!(view_member("slice", true, 0).unwrap().native, "nodeBufferSlice");
        assert_eq!(view_member("slice", false, 0).unwrap().native, "typedArraySlice");
    }

    #[test]
    fn unknown_member_is_none() {
        assert!(view_member("nope", true, 0).is_none());
        assert!(view_member("readNothingAtAll", true, 0).is_none());
    }

    #[test]
    fn write_family_is_value_first() {
        assert!(view_member("writeUInt16LE", true, 1).unwrap().value_first);
        assert!(!view_member("readUInt16LE", true, 1).unwrap().value_first);
    }
}
