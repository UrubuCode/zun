//! Declarative realm globals for base types (PLAN §3.1).
//!
//! The list of base-type globals lives in `builtin/realm.zun` as a sequence of
//! `global.set("<BaseType>", <BaseType>, true)` directives. This module reads
//! that file at compile time, recognizes the directives, and produces the
//! [`RealmBinding`] registry the checker consults — the single source of truth
//! for how a registered name resolves as a value (an opaque class descriptor),
//! as a type annotation, and as a `new <Name>()` constructor.
//!
//! Resolution is **fallback-shaped**: only names that appear here AND have known
//! facets ([`base_type_facets`]) flow through the registry; every other base
//! type falls through to the checker's existing hardcoded matches unchanged.

use crate::hir;
use crate::hir::Ty;

/// The declarative seed source. Its directives are the source of truth for which
/// base types are realm globals.
const REALM_SOURCE: &str = zun_prelude::REALM;

/// How `new <Name>()` lowers for a registered base type. Captures the existing
/// lowering shapes so the registry only redirects the *name* over an unchanged
/// frozen representation.
#[derive(Debug, Clone)]
pub(crate) enum CtorLowering {
    /// A runtime-managed value allocated by a zero-argument intrinsic
    /// (`mapNew`, `setNew`, `promiseNew`, ...) in `module`, yielding `ty`.
    Intrinsic0 { module: &'static str, native: &'static str, ty: Ty },
    /// An argument-taking intrinsic (`regexpNew`, `bigIntNew`, ...) in `module`.
    /// Accepts `min_args..=max_args` arguments and yields `ty`. When
    /// `pad_empty_flags` is set and exactly one argument is supplied, an empty
    /// string is appended (RegExp's optional `flags`).
    IntrinsicN {
        module: &'static str,
        native: &'static str,
        ty: Ty,
        min_args: usize,
        max_args: usize,
        pad_empty_flags: bool,
    },
    /// A managed collection with a zero-argument allocator (`empty`), an
    /// array-argument constructor (`from`), and a general iterable constructor
    /// (`from_iterable`) in `module`, yielding `ty`. `new Map()`/`new Set()`
    /// lower through `empty`; `new Map(array)`/`new Set(array)` take the
    /// `from` fast path; any other built-in iterable argument (another Map/Set,
    /// a string) lowers through `from_iterable`, which dispatches on the
    /// runtime kind. Arbitrary user `[Symbol.iterator]` objects are deferred.
    CollectionFrom {
        module: &'static str,
        empty: &'static str,
        from: &'static str,
        from_iterable: &'static str,
        ty: Ty,
    },
    /// `new Promise()` → `promiseNew` (an empty pending promise); `new
    /// Promise(executor)` → `promiseNewExecutor`, which creates the promise plus
    /// its resolve/reject settlers (the `withResolvers` mechanism) and
    /// synchronously invokes the executor with them (an executor throw rejects
    /// the promise). Yields `Ty::Promise`.
    Promise,
    /// `new Array(len)` → an `ArrayNew` node (`Ty::Array`).
    ArrayNew,
    /// `new ArrayBuffer(len)` → `bufferNew` (`Ty::Buffer`).
    ArrayBuffer,
    /// A buffer view built from `(buffer, offset, length)` — `DataView` and the
    /// typed arrays — yielding the view `kind`.
    BufferView { native: &'static str, kind: hir::BufferViewKind },
    /// The registry owns the name's annotation type and descriptor value, but
    /// `new <Name>()` falls through to the checker's existing resolution. Used
    /// for value types (`String`/`Number`/`Boolean`) and surface-only globals
    /// (`Object`/`Symbol`) that have no intrinsic constructor of their own.
    FallThrough,
    /// A class-backed base type whose annotation is `Ty::Instance(<class
    /// index>)`. The class index is program-assigned (these classes ship via
    /// the prelude), so it cannot be named by a static facet; the checker
    /// resolves the name through `classes_by_name` at check time, both for the
    /// annotation type and for `new <Name>(...)` (which uses the ordinary
    /// class-construction path). Used for `Error` and its subclasses. The
    /// registry still owns the realm descriptor value, so `globalThis.<Name>`
    /// resolves like every other base type.
    ClassFallThrough,
}

/// A base type registered as a realm global.
#[derive(Debug, Clone)]
pub(crate) struct RealmBinding {
    /// Opaque metadata token identifying the base type, boxed into the
    /// class-descriptor value registered on the realm global.
    pub token: u32,
    /// The type this name denotes as an annotation (`: Map`).
    pub ty: Ty,
    /// How `new <Name>()` lowers.
    pub ctor: CtorLowering,
}

/// The compile-time facets of a known base type: the type it denotes and how its
/// constructor lowers. Returns `None` for names not yet migrated to the
/// declarative path, so they fall through to the hardcoded resolution.
///
/// Rolling out a base type is a matter of adding its `(Ty, CtorLowering)` here
/// and listing it in `builtin/realm.zun`.
///
/// Class-backed base types whose annotation is `Ty::Instance(<class index>)`
/// (`Date`, `Error` and its subclasses) are *not* expressible here — a static
/// facet cannot name a program-assigned class index — so they stay on the
/// checker's hardcoded class-constructor path.
fn base_type_facets(name: &str) -> Option<(Ty, CtorLowering)> {
    use hir::BufferViewKind as Bv;
    // Every typed-array constructor comes from the master table, so a kind
    // cannot be annotatable without being constructible.
    if let Some(facets) = crate::typed_arrays::constructor(name) {
        return Some(facets);
    }
    let collection0 = |native, ty| CtorLowering::Intrinsic0 { module: "collections", native, ty };
    Some(match name {
        // Managed collections with an optional iterable constructor argument.
        "Map" => (
            Ty::Map,
            CtorLowering::CollectionFrom {
                module: "collections",
                empty: "mapNew",
                from: "mapFrom",
                from_iterable: "mapFromIterable",
                ty: Ty::Map,
            },
        ),
        "Set" => (
            Ty::Set,
            CtorLowering::CollectionFrom {
                module: "collections",
                empty: "setNew",
                from: "setFrom",
                from_iterable: "setFromIterable",
                ty: Ty::Set,
            },
        ),
        // Zero-argument managed collections.
        "WeakMap" => (Ty::WeakMap, collection0("weakMapNew", Ty::WeakMap)),
        "WeakSet" => (Ty::WeakSet, collection0("weakSetNew", Ty::WeakSet)),
        "WeakRef" => (
            Ty::WeakRef,
            CtorLowering::IntrinsicN {
                module: "collections",
                native: "weakRefNew",
                ty: Ty::WeakRef,
                min_args: 1,
                max_args: 1,
                pad_empty_flags: false,
            },
        ),
        "FinalizationRegistry" => (
            Ty::FinalizationRegistry,
            CtorLowering::IntrinsicN {
                module: "collections",
                native: "finalizationRegistryNew",
                ty: Ty::FinalizationRegistry,
                min_args: 1,
                max_args: 1,
                pad_empty_flags: false,
            },
        ),
        "Promise" => (Ty::Promise, CtorLowering::Promise),
        // Argument-taking intrinsics.
        "RegExp" => (
            Ty::RegExp,
            CtorLowering::IntrinsicN {
                module: "collections",
                native: "regexpNew",
                ty: Ty::RegExp,
                min_args: 1,
                max_args: 2,
                pad_empty_flags: true,
            },
        ),
        "BigInt" => (
            Ty::BigInt,
            CtorLowering::IntrinsicN {
                module: "collections",
                native: "bigIntNew",
                ty: Ty::BigInt,
                min_args: 1,
                max_args: 1,
                pad_empty_flags: false,
            },
        ),
        // Array and the buffer family.
        "Array" => (Ty::Array, CtorLowering::ArrayNew),
        "ArrayBuffer" => (Ty::Buffer, CtorLowering::ArrayBuffer),
        "DataView" => (
            Ty::BufferView(Bv::DataView),
            CtorLowering::BufferView { native: "dataViewNew", kind: Bv::DataView },
        ),
        // Value types and surface-only globals: annotation + descriptor only,
        // `new` (where it exists) stays on the existing class path.
        "String" => (Ty::String, CtorLowering::FallThrough),
        "Number" => (Ty::F64, CtorLowering::FallThrough),
        "Boolean" => (Ty::Boolean, CtorLowering::FallThrough),
        "Object" => (Ty::DynamicObject, CtorLowering::FallThrough),
        "Symbol" => (Ty::Symbol, CtorLowering::FallThrough),
        // Surface-only reflection namespace. Like `Object`, it has no `new`
        // constructor and is never used as a type annotation; the descriptor
        // value is what `globalThis.Reflect` resolves to, and
        // `Reflect.<method>(...)` static calls are recognized by receiver name.
        "Reflect" => (Ty::DynamicObject, CtorLowering::FallThrough),
        // `Buffer` is a nominal alias for the `Uint8Array` byte view (Node's
        // Buffer IS a Uint8Array). The annotation denotes a U8 view and the
        // descriptor value is what `globalThis.Buffer` resolves to;
        // `Buffer.from`/`alloc`/… statics are recognized by receiver name, so
        // there is no `new Buffer()` constructor lowering (deprecated in Node).
        "Buffer" => (Ty::BufferView(Bv::U8), CtorLowering::FallThrough),
        // Class-backed base types (`Error` + subclasses). Their annotation type
        // is resolved from `classes_by_name` at check time (the class index is
        // program-assigned via the prelude), so the `Ty` here is a placeholder
        // the checker never reads for a `ClassFallThrough` binding. The registry
        // owns only the realm descriptor value so `globalThis.Error` resolves.
        "Error" | "EvalError" | "RangeError" | "ReferenceError" | "SyntaxError"
        | "TypeError" | "URIError" => (Ty::Error, CtorLowering::ClassFallThrough),
        _ => return None,
    })
}

/// Recognize a single `global.set("<Name>", <Name>, true)` directive on `line`,
/// returning the base-type name. The quoted name and the bare identifier must
/// match and the readonly flag must be `true`.
fn recognize_directive(line: &str) -> Option<String> {
    let line = line.split("//").next().unwrap_or(line).trim();
    let inner = line
        .strip_prefix("global.set(")?
        .strip_suffix(");")
        .or_else(|| line.strip_prefix("global.set(").and_then(|_| line.strip_suffix(")")))?;
    // `inner` is `"Name", Name, true`.
    let mut parts = inner.split(',').map(str::trim);
    let quoted = parts.next()?;
    let ident = parts.next()?;
    let readonly = parts.next()?;
    if parts.next().is_some() || readonly != "true" {
        return None;
    }
    let name = quoted.strip_prefix('"')?.strip_suffix('"')?;
    if name != ident {
        return None;
    }
    Some(name.to_string())
}

/// Build the realm registry from the declarative seed. Each recognized directive
/// whose name has known facets becomes a [`RealmBinding`]; tokens are assigned in
/// declaration order (opaque, stable within a build).
pub(crate) fn registry() -> Vec<(String, RealmBinding)> {
    let mut bindings = Vec::new();
    let mut token: u32 = 0;
    for line in REALM_SOURCE.lines() {
        let Some(name) = recognize_directive(line) else {
            continue;
        };
        let Some((ty, ctor)) = base_type_facets(&name) else {
            continue;
        };
        bindings.push((name, RealmBinding { token, ty, ctor }));
        token += 1;
    }
    bindings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_map_directive() {
        assert_eq!(
            recognize_directive(r#"global.set("Map", Map, true);"#),
            Some("Map".to_string())
        );
    }

    #[test]
    fn rejects_mismatched_or_writable_directives() {
        assert_eq!(recognize_directive(r#"global.set("Map", Set, true);"#), None);
        assert_eq!(recognize_directive(r#"global.set("Map", Map, false);"#), None);
        assert_eq!(recognize_directive("// a comment"), None);
    }

    #[test]
    fn registry_seeds_map_first() {
        let registry = registry();
        // `Map` must stay first so its descriptor keeps token 0 (end-to-end
        // tests observe `classDescriptorToken(globalThis.Map) == 0`).
        assert_eq!(registry[0].0, "Map");
        assert_eq!(registry[0].1.token, 0);
        assert_eq!(registry[0].1.ty, Ty::Map);
    }

    #[test]
    fn registry_covers_the_declared_base_types() {
        let names: Vec<String> = registry().into_iter().map(|(n, _)| n).collect();
        for expected in ["Set", "WeakMap", "Promise", "RegExp", "Array", "String", "Number"] {
            assert!(names.iter().any(|n| n == expected), "missing {expected}");
        }
    }
}
