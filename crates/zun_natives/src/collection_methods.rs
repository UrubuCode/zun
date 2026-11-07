//! The single authoritative `Map` / `Set` method tables.
//!
//! Both dispatch surfaces for the managed collections consult these tables so
//! the member-name → native-name mapping (and, for the ES2025 Set operations,
//! the member-name → prelude-helper mapping) is defined exactly once:
//!
//! * the static, typed path in `zun_sema` (the `Map`/`Set` arm of
//!   `managed_method_descriptor`, and the Set-operation lowering in
//!   `mcall_buffer_set`), which reads the native/helper each method routes to
//!   while keeping its own sema-side return and argument types; and
//! * the dynamic `any`-receiver path in `zun_interp`
//!   (`collections::dynamic_call`), which reads both the native name and the
//!   [`CollectionHandling`] describing how that method normalizes its arguments
//!   at run time, in place of the previous hand-coded `match member` switches.
//!
//! This mirrors the `Array` table in `array_methods`. It only names the
//! mapping; it does not change the `.zm` format, the Map/Set layout, or any
//! opcode.
//!
//! `size` is deliberately *not* in these tables: it is a property on both
//! surfaces (the typed path answers it through its own descriptor, the dynamic
//! path through property access), never a method the method-call dispatcher
//! routes, so it is not shared knowledge and stays in the sema descriptor.

/// How the dynamic `any`-collection path normalizes a method's arguments
/// before handing them to the native.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionHandling {
    /// Forward the call arguments unchanged (the native with the receiver
    /// prepended).
    Plain,
    /// A chaining setter whose argument list is resized to the receiver plus
    /// two defaulted slots (`Map.prototype.set` → `mapSetChain`).
    SetChain,
}

/// One row of a `Map`/`Set` method table.
#[derive(Debug, Clone, Copy)]
pub struct CollectionMethod {
    /// The `Map.prototype` / `Set.prototype` member name.
    pub member: &'static str,
    /// The native this method routes to.
    pub native: &'static str,
    /// How the dynamic path normalizes this method's arguments.
    pub handling: CollectionHandling,
}

use CollectionHandling::*;

const fn row(
    member: &'static str,
    native: &'static str,
    handling: CollectionHandling,
) -> CollectionMethod {
    CollectionMethod {
        member,
        native,
        handling,
    }
}

/// The authoritative `Map.prototype` method table.
static MAP_METHODS: &[CollectionMethod] = &[
    row("set", "mapSetChain", SetChain),
    row("get", "mapGet", Plain),
    row("has", "mapHas", Plain),
    row("delete", "mapDelete", Plain),
    row("clear", "mapClear", Plain),
    row("keys", "mapKeys", Plain),
    row("values", "mapValues", Plain),
    row("entries", "mapEntries", Plain),
    row("forEach", "mapForEach", Plain),
];

/// The authoritative `Set.prototype` method table (the collection methods;
/// the ES2025 Set *operations* are in [`SET_OPS`]). `keys` and `values` share
/// the one `setValues` native, matching the ECMAScript aliasing.
static SET_METHODS: &[CollectionMethod] = &[
    row("add", "setAddChain", Plain),
    row("has", "setHas", Plain),
    row("delete", "setDelete", Plain),
    row("clear", "setClear", Plain),
    row("keys", "setValues", Plain),
    row("values", "setValues", Plain),
    row("entries", "setEntries", Plain),
    row("forEach", "setForEach", Plain),
];

/// Look up the authoritative `Map.prototype` method table by member name.
/// Returns `None` for a member the table does not name, so each surface keeps
/// its own fallback for other members.
pub fn map_method(member: &str) -> Option<&'static CollectionMethod> {
    MAP_METHODS.iter().find(|row| row.member == member)
}

/// Look up the authoritative `Set.prototype` method table by member name.
pub fn set_method(member: &str) -> Option<&'static CollectionMethod> {
    SET_METHODS.iter().find(|row| row.member == member)
}

/// One ES2025 Set-operation row. These seven methods do not route to a native;
/// both surfaces lower them to the prelude helpers `__zunSetOpSet` /
/// `__zunSetOpBool`, which follow the set-like protocol on the argument. The
/// row names which helper and the result shape.
#[derive(Debug, Clone, Copy)]
pub struct SetOp {
    /// The `Set.prototype` member name.
    pub member: &'static str,
    /// Whether the operation is a predicate (`isSubsetOf` / `isSupersetOf` /
    /// `isDisjointFrom`) returning a boolean via `__zunSetOpBool`, versus a
    /// set-producing operation returning a new `Set` via `__zunSetOpSet`.
    pub predicate: bool,
}

/// The prelude helper that produces a new `Set` from a Set operation.
pub const SET_OP_SET_HELPER: &str = "__zunSetOpSet";
/// The prelude helper that produces a boolean from a Set predicate operation.
pub const SET_OP_BOOL_HELPER: &str = "__zunSetOpBool";

impl SetOp {
    /// The prelude helper this operation lowers to.
    pub fn helper(&self) -> &'static str {
        if self.predicate {
            SET_OP_BOOL_HELPER
        } else {
            SET_OP_SET_HELPER
        }
    }
}

const fn set_op(member: &'static str, predicate: bool) -> SetOp {
    SetOp { member, predicate }
}

/// The seven ES2025 Set operations, in specification order.
static SET_OPS: &[SetOp] = &[
    set_op("union", false),
    set_op("intersection", false),
    set_op("difference", false),
    set_op("symmetricDifference", false),
    set_op("isSubsetOf", true),
    set_op("isSupersetOf", true),
    set_op("isDisjointFrom", true),
];

/// Look up the ES2025 Set-operation table by member name. Returns `None` for a
/// member that is not one of the seven set operations.
pub fn set_operation(member: &str) -> Option<&'static SetOp> {
    SET_OPS.iter().find(|row| row.member == member)
}
