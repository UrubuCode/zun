//! The intrinsic registry.
//!
//! An **intrinsic** is a function whose signature is declared in Zun but whose
//! body lives inside the runtime (in Rust) — the analogue of .NET's
//! InternalCall/FCall and Rust's `std::sys`/intrinsics. This crate holds only
//! the *signatures and stable ids*; the implementations live in the runtime
//! (`zun_interp`), dispatched by id. This keeps the compiler (which needs
//! signatures to type-check) decoupled from the runtime (which needs bodies).
//!
//! Intrinsics are distinct from **FFI/P-Invoke**: an intrinsic is implemented
//! by us; FFI (future) calls an *external* native library. See PLAN.md §4.4.
//!
//! Each intrinsic is reached by the fast indexed `call.builtin <id>` dispatch.
//! Ids are stable and shared with the runtime's dispatch table — never renumber.

/// A parameter or return type in an intrinsic signature. A deliberately small
/// enum the compiler maps onto its own `Ty`, so this crate needs no dependency
/// on the type checker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTy {
    /// `int` (i32).
    Int,
    /// `i64`.
    I64,
    /// `number`/`float` (f64).
    Float,
    /// `boolean`.
    Bool,
    /// `string`.
    Str,
    /// `void` (return only).
    Void,
}

/// One intrinsic function: its name, signature, and stable dispatch id.
#[derive(Debug, Clone, Copy)]
pub struct Intrinsic {
    /// The name as imported in Zun.
    pub name: &'static str,
    /// Parameter types, in order.
    pub params: &'static [NativeTy],
    /// Return type.
    pub ret: NativeTy,
    /// Stable id for `call.builtin` dispatch.
    pub id: u32,
}

/// A built-in module: a `zun:`-prefixed name and its intrinsics.
#[derive(Debug, Clone, Copy)]
pub struct Module {
    /// The module name, e.g. `"zun:fs"`.
    pub name: &'static str,
    /// The intrinsics the module exports.
    pub functions: &'static [Intrinsic],
}

// --- stable intrinsic ids (shared with the runtime dispatch table) ---

/// `print(value)` — the implicit builtin (no import needed). Id 0 for back-compat.
pub const PRINT: u32 = 0;
/// `zun:io` `println(value)`.
pub const IO_PRINTLN: u32 = 1;
/// `zun:fs` `readText(path)`.
pub const FS_READ_TEXT: u32 = 10;
/// `zun:fs` `writeText(path, content)`.
pub const FS_WRITE_TEXT: u32 = 11;
/// `zun:fs` `exists(path)`.
pub const FS_EXISTS: u32 = 12;
/// `zun:fs` `removeFile(path)`.
pub const FS_REMOVE: u32 = 13;
/// `zun:env` `get(name)`.
pub const ENV_GET: u32 = 20;
/// `zun:time` `nowMillis()`.
pub const TIME_NOW_MILLIS: u32 = 30;

use NativeTy::*;

const IO: Module = Module {
    name: "zun:io",
    functions: &[Intrinsic { name: "println", params: &[Str], ret: Void, id: IO_PRINTLN }],
};

const FS: Module = Module {
    name: "zun:fs",
    functions: &[
        Intrinsic { name: "readText", params: &[Str], ret: Str, id: FS_READ_TEXT },
        Intrinsic { name: "writeText", params: &[Str, Str], ret: Void, id: FS_WRITE_TEXT },
        Intrinsic { name: "exists", params: &[Str], ret: Bool, id: FS_EXISTS },
        Intrinsic { name: "removeFile", params: &[Str], ret: Void, id: FS_REMOVE },
    ],
};

const ENV: Module = Module {
    name: "zun:env",
    functions: &[Intrinsic { name: "get", params: &[Str], ret: Str, id: ENV_GET }],
};

const TIME: Module = Module {
    name: "zun:time",
    functions: &[Intrinsic { name: "nowMillis", params: &[], ret: I64, id: TIME_NOW_MILLIS }],
};

/// All built-in intrinsic modules.
pub const MODULES: &[Module] = &[IO, FS, ENV, TIME];

/// Look up a module by its `zun:`-prefixed name.
pub fn module(name: &str) -> Option<&'static Module> {
    MODULES.iter().find(|m| m.name == name)
}

/// Look up an intrinsic by its dispatch id (for the runtime's arity/dispatch).
pub fn by_id(id: u32) -> Option<&'static Intrinsic> {
    MODULES
        .iter()
        .flat_map(|m| m.functions.iter())
        .find(|f| f.id == id)
}

/// The number of parameters an intrinsic id takes (`print`/id 0 takes one).
pub fn param_count(id: u32) -> usize {
    if id == PRINT {
        1
    } else {
        by_id(id).map(|f| f.params.len()).unwrap_or(0)
    }
}

impl Module {
    /// Find an exported intrinsic by name.
    pub fn function(&self, name: &str) -> Option<&'static Intrinsic> {
        self.functions.iter().find(|f| f.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_resolve_and_expose_functions() {
        let fs = module("zun:fs").expect("zun:fs exists");
        assert_eq!(fs.function("readText").unwrap().id, FS_READ_TEXT);
        assert_eq!(fs.function("readText").unwrap().ret, NativeTy::Str);
        assert!(module("zun:nope").is_none());
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<u32> = MODULES
            .iter()
            .flat_map(|m| m.functions.iter().map(|f| f.id))
            .collect();
        ids.push(PRINT);
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate intrinsic id");
    }
}
