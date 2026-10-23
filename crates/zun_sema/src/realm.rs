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

use crate::hir::Ty;

/// The declarative seed source. Its directives are the source of truth for which
/// base types are realm globals.
const REALM_SOURCE: &str = include_str!("../../../builtin/realm.zun");

/// How `new <Name>()` lowers for a registered base type. Captures the existing
/// lowering shapes so the registry only redirects the *name* over an unchanged
/// frozen representation.
#[derive(Debug, Clone)]
pub(crate) enum CtorLowering {
    /// A runtime-managed collection allocated by a zero-argument intrinsic
    /// (`mapNew`, `setNew`, ...), yielding a value of type `ty`.
    ManagedCollection { native: &'static str, ty: Ty },
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
/// For this slice only `Map` is wired; rolling out the rest is a matter of
/// adding its `(Ty, CtorLowering)` here and listing it in `builtin/realm.zun`.
fn base_type_facets(name: &str) -> Option<(Ty, CtorLowering)> {
    match name {
        "Map" => Some((
            Ty::Map,
            CtorLowering::ManagedCollection { native: "mapNew", ty: Ty::Map },
        )),
        _ => None,
    }
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
    fn registry_seeds_map_only() {
        let registry = registry();
        assert_eq!(registry.len(), 1);
        assert_eq!(registry[0].0, "Map");
        assert_eq!(registry[0].1.ty, Ty::Map);
    }
}
