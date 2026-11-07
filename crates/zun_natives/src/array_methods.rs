//! The single authoritative `Array` method table.
//!
//! Both dispatch surfaces for `Array.prototype` methods consult this one
//! table so the member-name → native-name mapping is defined exactly once:
//!
//! * the static, typed path in `zun_sema` (the `Array` arm of
//!   `managed_method_descriptor`), which reads [`ArrayMethod::native`] for the
//!   native each simple fixed-arity method routes to; and
//! * the dynamic `any`-receiver path in `zun_interp`
//!   (`collections::dynamic_call`), which reads both the native name and the
//!   [`ArrayHandling`] describing how that method normalizes its arguments at
//!   run time, in place of the previous hand-coded `array{Method}` string
//!   munging and `match member` switch.
//!
//! This table only names the mapping; it does not change the `.zm` format,
//! the object/string/array layout, or any opcode. Methods a surface lowers to
//! IL instructions rather than a native (`push`/`pop` on the typed path) still
//! appear here (with [`ArrayHandling::Push`]/[`ArrayHandling::Pop`] and an
//! empty native) so the mapping has no ad-hoc, out-of-table knowledge.

/// How the dynamic `any`-array path normalizes a method's arguments before
/// handing them to the native. [`ArrayHandling::Plain`] forwards the call
/// arguments unchanged after the receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayHandling {
    /// Forward the arguments unchanged (`array{Method}` with the receiver
    /// prepended).
    Plain,
    /// `push`: no native — the typed path lowers it to an IL instruction and
    /// the dynamic path appends to the heap array directly.
    Push,
    /// `pop`: no native — handled like `push`.
    Pop,
    /// `sort`: the native is chosen by the comparator's presence
    /// (`arraySortDefault` with no argument, `arraySortBy` otherwise).
    Sort,
    /// `slice`: the dynamic path uses `arraySliceRange` with the two bounds
    /// resized to exactly two slots and non-numeric bounds coerced.
    SliceRange,
    /// `join`: a missing or non-string separator defaults to `","`.
    JoinDefault,
    /// `toString`: `arrayJoin` with a `","` separator.
    ToStringJoin,
    /// `fill` / `copyWithin`: the argument list is resized to the native's
    /// four defaulted slots.
    FillResize,
    /// `concat`: the call arguments are gathered into a managed list argument.
    ConcatList,
    /// `splice`: the call arguments are gathered into a managed list and routed
    /// to `arraySpliceArgs`.
    SpliceList,
}

/// One row of the authoritative `Array` method table.
#[derive(Debug, Clone, Copy)]
pub struct ArrayMethod {
    /// The `Array.prototype` member name.
    pub member: &'static str,
    /// The native this method routes to, or `""` when the method has no native
    /// (`push`/`pop`). For [`ArrayHandling::Sort`]/[`ArrayHandling::ToStringJoin`]/
    /// [`ArrayHandling::SpliceList`] this is the common/base native; the dynamic
    /// path may refine it per the handling.
    pub native: &'static str,
    /// How the dynamic path normalizes this method's arguments.
    pub handling: ArrayHandling,
}

use ArrayHandling::*;

/// The authoritative table. Simple methods forward their arguments unchanged
/// ([`ArrayHandling::Plain`]) and map to the conventional `array{Method}`
/// native; the remainder carry the exact native and run-time argument
/// normalization both surfaces must agree on.
static ARRAY_METHODS: &[ArrayMethod] = &[
    // Fixed-arity callback / query methods also used by the typed descriptor.
    row("forEach", "arrayForEach", Plain),
    row("map", "arrayMap", Plain),
    row("filter", "arrayFilter", Plain),
    row("find", "arrayFind", Plain),
    row("findIndex", "arrayFindIndex", Plain),
    row("some", "arraySome", Plain),
    row("every", "arrayEvery", Plain),
    row("shift", "arrayShift", Plain),
    row("unshift", "arrayUnshift", Plain),
    row("at", "arrayAt", Plain),
    row("findLast", "arrayFindLast", Plain),
    row("findLastIndex", "arrayFindLastIndex", Plain),
    row("flatMap", "arrayFlatMap", Plain),
    row("group", "arrayGroup", Plain),
    row("groupToMap", "arrayGroupToMap", Plain),
    row("keys", "arrayKeys", Plain),
    row("values", "arrayValues", Plain),
    row("entries", "arrayEntries", Plain),
    // Further methods reached by the dynamic path's default `array{Method}`
    // rule (the typed path lowers these through its own interceptors).
    row("flat", "arrayFlat", Plain),
    row("reverse", "arrayReverse", Plain),
    row("indexOf", "arrayIndexOf", Plain),
    row("lastIndexOf", "arrayLastIndexOf", Plain),
    row("includes", "arrayIncludes", Plain),
    row("reduce", "arrayReduce", Plain),
    row("reduceRight", "arrayReduceRight", Plain),
    row("toReversed", "arrayToReversed", Plain),
    row("toSorted", "arrayToSorted", Plain),
    row("with", "arrayWith", Plain),
    row("toSpliced", "arrayToSpliced", Plain),
    row("has", "arrayHas", Plain),
    // Methods whose dynamic path refines the native or reshapes its arguments.
    row("sort", "arraySortDefault", Sort),
    row("slice", "arraySliceRange", SliceRange),
    row("join", "arrayJoin", JoinDefault),
    row("toString", "arrayJoin", ToStringJoin),
    row("fill", "arrayFill", FillResize),
    row("copyWithin", "arrayCopyWithin", FillResize),
    row("concat", "arrayConcat", ConcatList),
    row("splice", "arraySpliceArgs", SpliceList),
    // No native: lowered to IL (typed) / handled directly on the heap (dynamic).
    row("push", "", Push),
    row("pop", "", Pop),
];

const fn row(member: &'static str, native: &'static str, handling: ArrayHandling) -> ArrayMethod {
    ArrayMethod {
        member,
        native,
        handling,
    }
}

/// Look the authoritative table up by `Array.prototype` member name. Returns
/// `None` for a member the table does not name, so the dynamic path can keep
/// its open-set `array{Method}` fallback for any other member.
pub fn array_method(member: &str) -> Option<&'static ArrayMethod> {
    ARRAY_METHODS.iter().find(|row| row.member == member)
}

#[cfg(test)]
mod key_uniqueness {
    use super::ARRAY_METHODS;

    #[test]
    fn array_method_keys_are_unique() {
        for (i, a) in ARRAY_METHODS.iter().enumerate() {
            for b in &ARRAY_METHODS[i + 1..] {
                assert_ne!(
                    a.member, b.member,
                    "duplicate ARRAY_METHODS key `{}`",
                    a.member
                );
            }
        }
    }
}
