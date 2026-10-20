//! Native modules — the runtime-implemented part of the standard library
//! (`zun:fs`, `zun:io`, `zun:env`, `zun:time`), the analogue of .NET
//! InternalCall bodies and Rust's `std::sys`. Distinct from FFI/P-Invoke, which
//! calls *external* native libraries (PLAN.md §4.4).
//!
//! Each module lives in its own directory and is **self-contained**: its
//! signatures *and* implementations sit together, as a list of [`NativeFn`]s.
//! Modules speak in **neutral value types** ([`NativeArg`]/[`NativeValue`]) so
//! this crate depends on no other Zun crate — the compiler can read signatures
//! without pulling in the heap or the interpreter.
//!
//! **Dispatch ids are positions, not hand-written numbers.** A function's id is
//! its index in the flattened registry ([`MODULES`] in order). Appending a
//! function keeps every existing id stable; nothing is numbered by hand, so ids
//! can never collide. `zun_node` and `zun_napi` will be organized the same way.

mod env;
mod fs;
mod io;
mod time;

use std::collections::HashMap;
use std::sync::OnceLock;

/// A parameter or return type in a native signature.
///
/// Native modules are machine-facing (not JS-flavored), so signatures use the
/// exact fixed-width numeric types — `u8` for bytes, `u64` for sizes/counts,
/// `i64` for timestamps — rather than the JS `number`/f64. The compiler maps
/// each onto its own `Ty`; the full lattice matches `zun_sema`'s `Ty::Int`/
/// `Ty::Float`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTy {
    /// Signed 8-bit.
    I8,
    /// Signed 16-bit.
    I16,
    /// Signed 32-bit (`int`).
    I32,
    /// Signed 64-bit.
    I64,
    /// Unsigned 8-bit (a byte).
    U8,
    /// Unsigned 16-bit.
    U16,
    /// Unsigned 32-bit.
    U32,
    /// Unsigned 64-bit (sizes, counts, handles).
    U64,
    /// 32-bit float.
    F32,
    /// 64-bit float (`number`/`float`).
    F64,
    /// `boolean`.
    Bool,
    /// `string`.
    Str,
    /// `void` (return only).
    Void,
}

/// A neutral argument value passed to a native function.
#[derive(Debug, Clone, Copy)]
pub enum NativeArg<'a> {
    /// An integer (any width, in an i64).
    Int(i64),
    /// A float.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// A string.
    Str(&'a str),
    /// An opaque reference (e.g. an object) — only its presence is known.
    Opaque,
    /// No value.
    Void,
}

impl NativeArg<'_> {
    /// Interpret as a string, or `""` if it is not one.
    pub fn as_str(&self) -> &str {
        match self {
            NativeArg::Str(s) => s,
            _ => "",
        }
    }

    /// Render for `print`/`println`, JS-style (integral floats without `.0`).
    pub fn display(&self) -> String {
        match self {
            NativeArg::Int(n) => n.to_string(),
            NativeArg::Float(n) => format_float(*n),
            NativeArg::Bool(b) => b.to_string(),
            NativeArg::Str(s) => s.to_string(),
            NativeArg::Opaque => "[object]".to_string(),
            NativeArg::Void => "undefined".to_string(),
        }
    }
}

/// A neutral result value returned by a native function.
#[derive(Debug, Clone)]
pub enum NativeValue {
    /// An integer result.
    Int(i64),
    /// A float result.
    Float(f64),
    /// A boolean result.
    Bool(bool),
    /// A string result.
    Str(String),
    /// No value.
    Void,
}

/// A native-call failure (e.g. a filesystem error).
#[derive(Debug, Clone)]
pub struct NativeError(pub String);

/// Output sink for `print`/`println`, so the host can capture program output.
pub trait Sink {
    /// Write one already-formatted line.
    fn println(&mut self, line: &str);
}

/// The implementation of a native function: neutral args in, neutral value out.
pub type Imp = fn(&[NativeArg], &mut dyn Sink) -> Result<NativeValue, NativeError>;

/// One native function: its name, signature, and implementation, together. Its
/// dispatch id is assigned by position in the registry, not stored here.
pub struct NativeFn {
    /// The name as imported in Zun.
    pub name: &'static str,
    /// Parameter types, in order.
    pub params: &'static [NativeTy],
    /// Return type.
    pub ret: NativeTy,
    /// The runtime implementation.
    pub imp: Imp,
}

impl core::fmt::Debug for NativeFn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NativeFn")
            .field("name", &self.name)
            .finish()
    }
}

/// A built-in module: a `zun:`-prefixed name and its functions.
#[derive(Debug)]
pub struct Module {
    /// The module name, e.g. `"zun:fs"`.
    pub name: &'static str,
    /// The functions the module exports (ids are their registry positions).
    pub functions: &'static [NativeFn],
}

/// Every built-in native module, in id order. A function's id is its global
/// index across this list; `print` is the very first entry, so its id is 0.
pub static MODULES: &[Module] = &[io::MODULE, fs::MODULE, env::MODULE, time::MODULE];

/// `print(value)` — the implicit builtin (no import). Id 0 by construction.
pub const PRINT: u32 = 0;

/// Convert a registry position to the stable core-library identifier used by
/// runtime call sites.
pub const fn intrinsic_id(id: u32) -> zun_core::IntrinsicId {
    zun_core::IntrinsicId::new(id)
}

/// The global id of module index `m`'s local function `i`.
fn global_id(m: usize, i: usize) -> u32 {
    let base: usize = MODULES[..m].iter().map(|md| md.functions.len()).sum();
    (base + i) as u32
}

/// Flattened dispatch registry, built once and then indexed directly by id.
/// The order is exactly the historical module/function order, so ids remain
/// stable when functions are appended.
fn registry() -> &'static [&'static NativeFn] {
    static REGISTRY: OnceLock<Vec<&'static NativeFn>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        MODULES
            .iter()
            .flat_map(|module| module.functions.iter())
            .collect()
    })
}

/// Name indexes for module and intrinsic resolution. They are built once from
/// the positional tables, so adding an export cannot accidentally change an
/// existing dispatch id.
fn name_index() -> &'static HashMap<(&'static str, &'static str), u32> {
    static INDEX: OnceLock<HashMap<(&'static str, &'static str), u32>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        for (module_index, module) in MODULES.iter().enumerate() {
            for (function_index, function) in module.functions.iter().enumerate() {
                index.insert(
                    (module.name, function.name),
                    global_id(module_index, function_index),
                );
            }
        }
        index
    })
}

fn module_index() -> &'static HashMap<&'static str, &'static Module> {
    static INDEX: OnceLock<HashMap<&'static str, &'static Module>> = OnceLock::new();
    INDEX.get_or_init(|| MODULES.iter().map(|module| (module.name, module)).collect())
}

/// Look up a function by dispatch id (its registry position).
pub fn by_id(id: u32) -> Option<&'static NativeFn> {
    registry().get(id as usize).copied()
}

/// Resolve `module::name` to its id and signature (for `import` checking).
pub fn resolve(module: &str, name: &str) -> Option<(u32, &'static NativeFn)> {
    let id = *name_index().get(&(module, name))?;
    Some((id, by_id(id)?))
}

/// Look up a module by its `zun:`-prefixed name.
pub fn module(name: &str) -> Option<&'static Module> {
    module_index().get(name).copied()
}

/// Parameter count of a function id.
pub fn param_count(id: u32) -> usize {
    by_id(id).map(|f| f.params.len()).unwrap_or(0)
}

/// Run the native function `id` with `args`, writing any output to `sink`.
pub fn dispatch(
    id: u32,
    args: &[NativeArg],
    sink: &mut dyn Sink,
) -> Result<NativeValue, NativeError> {
    match by_id(id) {
        Some(f) => (f.imp)(args, sink),
        None => Err(NativeError(format!("unknown intrinsic {id}"))),
    }
}

/// JS-style float formatting: integral values print without a trailing `.0`.
pub(crate) fn format_float(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_is_id_zero_and_resolves() {
        // io's first function is `print`, so PRINT == 0.
        assert_eq!(by_id(PRINT).unwrap().name, "print");
        assert_eq!(intrinsic_id(PRINT).get(), 0);
        let (id, f) = resolve("zun:fs", "readText").unwrap();
        assert_eq!(f.name, "readText");
        // Ids are positional and round-trip through by_id.
        assert_eq!(by_id(id).unwrap().name, "readText");
    }

    #[test]
    fn param_counts_are_right() {
        let (write_id, _) = resolve("zun:fs", "writeText").unwrap();
        assert_eq!(param_count(write_id), 2);
    }

    #[test]
    fn name_indexes_preserve_positional_ids() {
        for (module_index, module) in MODULES.iter().enumerate() {
            for (function_index, function) in module.functions.iter().enumerate() {
                let (id, resolved) = resolve(module.name, function.name).expect("indexed export");
                assert_eq!(id, global_id(module_index, function_index));
                assert_eq!(resolved.name, function.name);
            }
        }
    }
}
