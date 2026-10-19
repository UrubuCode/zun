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

/// A parameter or return type in a native signature. The compiler maps these
/// onto its own `Ty`.
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
        f.debug_struct("NativeFn").field("name", &self.name).finish()
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

/// The global id of module index `m`'s local function `i`.
fn global_id(m: usize, i: usize) -> u32 {
    let base: usize = MODULES[..m].iter().map(|md| md.functions.len()).sum();
    (base + i) as u32
}

/// Look up a function by dispatch id (its registry position).
pub fn by_id(id: u32) -> Option<&'static NativeFn> {
    let mut n = id as usize;
    for m in MODULES {
        if n < m.functions.len() {
            return Some(&m.functions[n]);
        }
        n -= m.functions.len();
    }
    None
}

/// Resolve `module::name` to its id and signature (for `import` checking).
pub fn resolve(module: &str, name: &str) -> Option<(u32, &'static NativeFn)> {
    let m = MODULES.iter().position(|md| md.name == module)?;
    let i = MODULES[m].functions.iter().position(|f| f.name == name)?;
    Some((global_id(m, i), &MODULES[m].functions[i]))
}

/// Look up a module by its `zun:`-prefixed name.
pub fn module(name: &str) -> Option<&'static Module> {
    MODULES.iter().find(|m| m.name == name)
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
}
