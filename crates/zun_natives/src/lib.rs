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
//! can never collide. Extension modules such as `zun_node` will be organized
//! the same way.

mod buffer;
mod collections;
mod date;
mod env;
mod fs;
mod io;
mod math;
mod number;
mod string;
mod time;
mod promise;

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
    /// An opaque managed reference, used for an extern method receiver.
    Opaque,
    /// A managed Array reference.
    Array,
    /// A managed ArrayBuffer reference.
    Buffer,
    /// A managed DataView reference.
    DataView,
    // One variant per row of the typed-element master table in `zun_core`.
    // The declarations in `buffer.rs` are generated from that table and name
    // these, so a missing one is a build error rather than a kind that
    // quietly has no intrinsics.
    /// A managed Int8Array view.
    I8Array,
    /// A managed Uint8Array view.
    U8Array,
    /// A managed Uint8ClampedArray view.
    U8ClampedArray,
    /// A managed Int16Array view.
    I16Array,
    /// A managed Uint16Array view.
    U16Array,
    /// A managed Int32Array view.
    I32Array,
    /// A managed Uint32Array view.
    U32Array,
    /// A managed Float32Array view.
    F32Array,
    /// A managed Float64Array view.
    F64Array,
    /// A managed BigInt64Array view.
    I64Array,
    /// A managed BigUint64Array view.
    U64Array,
    /// A managed Map reference.
    Map,
    /// A managed Set reference.
    Set,
    /// A managed WeakMap reference.
    WeakMap,
    /// A managed WeakSet reference.
    WeakSet,
    /// A managed WeakRef reference.
    WeakRef,
    /// A managed Promise object.
    Promise,
    /// A managed compiled regular expression.
    RegExp,
    /// A managed arbitrary-precision integer.
    BigInt,
    /// A managed finalization callback registry.
    FinalizationRegistry,
    /// A dictionary-backed dynamic object.
    DynamicObject,
    /// A managed target/handler proxy pair.
    Proxy,
    /// A callable Zun function, including its closure environment.
    Function,
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
    /// A managed value preserved across an intrinsic boundary.
    Any(zun_core::Any),
    /// A callable value preserved across an intrinsic boundary.
    Function(zun_core::FunctionHandle),
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
            NativeArg::Any(value) => format!("[any:{:?}]", value.tag()),
            NativeArg::Function(function) => {
                format!("[function:{}]", function.method_rid())
            }
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
    /// A managed value returned by a collection intrinsic.
    Any(zun_core::Any),
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
pub static MODULES: &[Module] = &[
    io::MODULE,
    fs::MODULE,
    env::MODULE,
    time::MODULE,
    math::MODULE,
    number::MODULE,
    string::MODULE,
    date::MODULE,
    collections::MODULE,
    buffer::MODULE,
    promise::MODULE,
];

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
///
/// This addresses functions by their historical per-type module name
/// (`zun:string`, `zun:collections`, …). It is retained as an internal,
/// back-compatible alias while call sites migrate to the consolidated
/// [`CORE_MODULE`] + `(namespace, name)` surface via [`resolve_core`].
pub fn resolve(module: &str, name: &str) -> Option<(u32, &'static NativeFn)> {
    let id = *name_index().get(&(module, name))?;
    Some((id, by_id(id)?))
}

/// The single consolidated native module. All intrinsics are exposed here as
/// per-type namespaces (PLAN.md §3.4); the historical `zun:<type>` module names
/// remain resolvable internally through [`resolve`] during migration.
pub const CORE_MODULE: &str = "zun:core";

/// Classify a `zun:collections` function into its per-type `zun:core`
/// namespace by name prefix. Every collections function carries an
/// unambiguous type prefix, so this is a total, order-independent mapping.
fn collections_namespace(name: &str) -> &'static str {
    const PREFIXES: &[(&str, &str)] = &[
        ("weakMap", "weakMap"),
        ("weakSet", "weakSet"),
        ("weakRef", "weakRef"),
        ("array", "array"),
        ("map", "map"),
        ("set", "set"),
        ("regexp", "regexp"),
        ("finalization", "finalizationRegistry"),
        ("reflect", "reflect"),
        ("proxy", "proxy"),
        ("dynamic", "dynamic"),
        ("bigInt", "bigint"),
        ("object", "object"),
        ("global", "global"),
    ];
    for (prefix, namespace) in PREFIXES {
        if name.starts_with(prefix) {
            return namespace;
        }
    }
    "collections"
}

/// The `zun:core` namespace a historical module's function belongs to. The
/// single-type modules map wholesale; `zun:collections` splits per type and
/// `zun:buffer` groups under `buffer`.
fn module_namespace(module_name: &str, fn_name: &str) -> &'static str {
    match module_name {
        "zun:string" => "string",
        "zun:number" => "number",
        "zun:math" => "math",
        "zun:date" => "date",
        "zun:promise" => "promise",
        "zun:fs" => "fs",
        "zun:io" => "io",
        "zun:env" => "env",
        "zun:time" => "time",
        "zun:buffer" => "buffer",
        "zun:collections" => collections_namespace(fn_name),
        _ => "core",
    }
}

/// `(namespace, name) -> id` index for the consolidated `zun:core` surface.
fn core_namespace_index() -> &'static HashMap<(&'static str, &'static str), u32> {
    static INDEX: OnceLock<HashMap<(&'static str, &'static str), u32>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        for (module_index, module) in MODULES.iter().enumerate() {
            for (function_index, function) in module.functions.iter().enumerate() {
                index.insert(
                    (module_namespace(module.name, function.name), function.name),
                    global_id(module_index, function_index),
                );
            }
        }
        index
    })
}

/// `name -> id` index. Native function names are globally unique across every
/// module, so an intrinsic can be resolved by name alone.
fn flat_name_index() -> &'static HashMap<&'static str, u32> {
    static INDEX: OnceLock<HashMap<&'static str, u32>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        for (module_index, module) in MODULES.iter().enumerate() {
            for (function_index, function) in module.functions.iter().enumerate() {
                index.insert(function.name, global_id(module_index, function_index));
            }
        }
        index
    })
}

fn namespace_set() -> &'static HashMap<&'static str, ()> {
    static SET: OnceLock<HashMap<&'static str, ()>> = OnceLock::new();
    SET.get_or_init(|| {
        core_namespace_index()
            .keys()
            .map(|(namespace, _)| (*namespace, ()))
            .collect()
    })
}

/// The redundant type-prefix carried by every native name inside a given
/// `zun:core` namespace, or `None` for namespaces whose natives are already
/// bare (`string`, `number`, `math`, `date`, `buffer`, …). A member's
/// JS-idiomatic SHORT name is the native name with this prefix stripped and its
/// first remaining character lowercased (`weakMapSet` → `set`, `objectKeys` →
/// `keys`, `finalizationRegister` → `register`).
///
/// Namespaces that are already idiomatic (no shared prefix) return `None`, so
/// `math.sqrt`, `date.year`, `number.isNaN` resolve directly through the
/// existing full-name surface — no alias is synthesized and no collision with
/// the typed `*Number` scaffolding (`sqrtNumber`, …) can arise.
fn namespace_prefix(namespace: &str) -> Option<&'static str> {
    Some(match namespace {
        "weakMap" => "weakMap",
        "weakSet" => "weakSet",
        "weakRef" => "weakRef",
        "map" => "map",
        "set" => "set",
        "regexp" => "regexp",
        "finalizationRegistry" => "finalization",
        "reflect" => "reflect",
        "proxy" => "proxy",
        "dynamic" => "dynamic",
        "bigint" => "bigInt",
        "object" => "object",
        "global" => "global",
        "promise" => "promise",
        _ => return None,
    })
}

/// The JS-idiomatic short member name for `name` within a namespace whose
/// redundant `prefix` is given, or `None` if `name` does not carry the prefix
/// (nothing to strip) or is exactly the prefix.
fn short_member(prefix: &str, name: &str) -> Option<String> {
    let rest = name.strip_prefix(prefix)?;
    let mut chars = rest.chars();
    let first = chars.next()?;
    Some(core::iter::once(first.to_ascii_lowercase()).chain(chars).collect())
}

/// `namespace -> (short member name -> id)` index. The public member surface of
/// each namespace is the JS-idiomatic short name (`weakMap.set`, `map.get`,
/// `object.keys`), addressed as (namespace, short) because short names are only
/// unique *within* a namespace (`map.get` and `weakMap.get` both exist). The
/// underlying native's unique name/id is unchanged; this is purely the public
/// alias layer. Built from the positional tables, so ids never drift.
fn core_short_index() -> &'static HashMap<&'static str, HashMap<String, u32>> {
    static INDEX: OnceLock<HashMap<&'static str, HashMap<String, u32>>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index: HashMap<&'static str, HashMap<String, u32>> = HashMap::new();
        for (module_index, module) in MODULES.iter().enumerate() {
            for (function_index, function) in module.functions.iter().enumerate() {
                let namespace = module_namespace(module.name, function.name);
                let Some(prefix) = namespace_prefix(namespace) else {
                    continue;
                };
                let Some(short) = short_member(prefix, function.name) else {
                    continue;
                };
                let id = global_id(module_index, function_index);
                if let Some(previous) = index.entry(namespace).or_default().insert(short.clone(), id)
                {
                    // Two distinct natives collapsing onto one (namespace, short)
                    // name would silently shadow one another; refuse to build.
                    assert_eq!(
                        previous, id,
                        "short-member collision in `{namespace}`: `{short}` maps to both id \
                         {previous} and id {id}",
                    );
                }
            }
        }
        index
    })
}

/// Resolve a `zun:core` namespaced export `namespace.member` to its id and
/// signature. `member` is matched first against the native's full unique name
/// (the historical surface) and then against the JS-idiomatic SHORT member
/// alias (`weakMap.set` → `weakMapSet`), so both spellings resolve to the same
/// id. Ids are identical to those returned by [`resolve`] for the historical
/// module that owned the function.
pub fn resolve_core(namespace: &str, name: &str) -> Option<(u32, &'static NativeFn)> {
    let id = core_namespace_index()
        .get(&(namespace, name))
        .copied()
        .or_else(|| core_short_index().get(namespace)?.get(name).copied())?;
    Some((id, by_id(id)?))
}

/// Resolve an intrinsic by its (globally unique) name, ignoring the namespace.
pub fn resolve_name(name: &str) -> Option<(u32, &'static NativeFn)> {
    let id = *flat_name_index().get(name)?;
    Some((id, by_id(id)?))
}

/// Return the interned name of a `zun:core` namespace, or `None` if `name` is
/// not a namespace exported by `zun:core`.
pub fn namespace(name: &str) -> Option<&'static str> {
    namespace_set().get_key_value(name).map(|(key, _)| *key)
}

/// Whether `name` is a namespace exported by `zun:core`.
pub fn is_namespace(name: &str) -> bool {
    namespace_set().contains_key(name)
}

/// Whether `name` is a valid `zun:core` export: a namespace, or (for direct
/// intrinsic imports such as `extern`-method bindings) a function name.
pub fn is_core_export(name: &str) -> bool {
    is_namespace(name) || flat_name_index().contains_key(name)
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
    fn core_namespaces_preserve_historical_ids() {
        // Every function is reachable through the consolidated `zun:core`
        // surface at exactly the id its historical module assigns, so the
        // consolidation is a pure re-addressing (no id drift).
        for module in MODULES {
            for function in module.functions {
                let (old_id, _) = resolve(module.name, function.name).expect("historical export");
                let namespace = module_namespace(module.name, function.name);
                let (core_id, core_fn) =
                    resolve_core(namespace, function.name).expect("core export");
                assert_eq!(old_id, core_id, "id drift for {}", function.name);
                assert_eq!(core_fn.name, function.name);
                assert_eq!(resolve_name(function.name).unwrap().0, old_id);
                assert!(is_namespace(namespace));
            }
        }
        // Spot-check the per-type split of the former `zun:collections`.
        assert_eq!(
            resolve_core("weakMap", "weakMapSet").unwrap().0,
            resolve("zun:collections", "weakMapSet").unwrap().0
        );
        assert_eq!(module_namespace("zun:collections", "objectKeys"), "object");
        assert_eq!(module_namespace("zun:collections", "bigIntAdd"), "bigint");
        assert!(is_core_export("string"));
        assert!(is_core_export("length"));
        assert!(!is_core_export("definitely-not-a-thing"));
    }

    #[test]
    fn short_members_alias_to_the_same_native_ids() {
        // The JS-idiomatic short member resolves to exactly the same id as the
        // native's historical full name, per namespace. Building the index also
        // asserts no (namespace, short) collision, so this exercises that too.
        let cases = [
            ("weakMap", "set", "weakMapSet"),
            ("weakMap", "get", "weakMapGet"),
            ("weakMap", "has", "weakMapHas"),
            ("weakMap", "delete", "weakMapDelete"),
            ("map", "set", "mapSet"),
            ("map", "get", "mapGet"),
            ("map", "entries", "mapEntries"),
            ("set", "add", "setAdd"),
            ("set", "values", "setValues"),
            ("object", "keys", "objectKeys"),
            ("object", "fromEntries", "objectFromEntries"),
            ("object", "is", "objectIs"),
            ("regexp", "isMatch", "regexpIsMatch"),
            ("regexp", "source", "regexpSource"),
            ("finalizationRegistry", "register", "finalizationRegister"),
            ("finalizationRegistry", "unregister", "finalizationUnregister"),
            ("bigint", "add", "bigIntAdd"),
            ("promise", "resolve", "promiseResolve"),
            ("reflect", "has", "reflectHas"),
            ("proxy", "get", "proxyGet"),
        ];
        for (namespace, short, full) in cases {
            let short_id = resolve_core(namespace, short).expect("short member").0;
            let full_id = resolve_core(namespace, full).expect("full member").0;
            assert_eq!(short_id, full_id, "{namespace}.{short} != {namespace}.{full}");
        }
        // Short names are namespace-relative, never global: `get` is ambiguous
        // across namespaces and resolves differently per namespace.
        assert_ne!(
            resolve_core("map", "get").unwrap().0,
            resolve_core("weakMap", "get").unwrap().0
        );
        // Already-idiomatic namespaces keep resolving by their bare names and
        // synthesize no colliding alias for the typed `*Number` scaffolding.
        assert!(resolve_core("math", "sqrt").is_some());
        assert_eq!(
            resolve_core("math", "sqrt").unwrap().0,
            resolve("zun:math", "sqrt").unwrap().0
        );
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

    #[test]
    fn math_intrinsics_resolve_after_existing_ids() {
        let (id, function) = resolve("zun:math", "sqrt").expect("math intrinsic");
        assert_eq!(function.ret, NativeTy::F64);
        assert_eq!(
            id as usize,
            MODULES[..4]
                .iter()
                .map(|m| m.functions.len())
                .sum::<usize>()
                + 1
        );
    }

    #[test]
    fn math_intrinsics_execute_with_numeric_values() {
        let (id, _) = resolve("zun:math", "sqrt").unwrap();
        let mut sink = TestSink;
        let value = dispatch(id, &[NativeArg::Float(81.0)], &mut sink).unwrap();
        assert!(matches!(value, NativeValue::Float(value) if value == 9.0));
    }

    #[test]
    fn extern_receiver_is_passed_as_the_first_native_argument() {
        let (id, _) = resolve("zun:math", "receiverValue").unwrap();
        let mut sink = TestSink;
        let value = dispatch(id, &[NativeArg::Opaque, NativeArg::Float(7.5)], &mut sink).unwrap();
        assert!(matches!(value, NativeValue::Float(value) if value == 7.5));
    }

    #[test]
    fn date_intrinsics_decompose_and_format_utc() {
        let (minute_id, _) = resolve("zun:date", "minute").unwrap();
        let (second_id, _) = resolve("zun:date", "second").unwrap();
        let (millisecond_id, _) = resolve("zun:date", "millisecond").unwrap();
        let (ordinal_id, _) = resolve("zun:date", "dayOfYear").unwrap();
        let (iso_id, _) = resolve("zun:date", "isoDateTime").unwrap();
        let mut sink = TestSink;
        let timestamp = 3_723_004_i64;
        assert!(matches!(
            dispatch(minute_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(),
            NativeValue::Int(2)
        ));
        assert!(matches!(
            dispatch(second_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(),
            NativeValue::Int(3)
        ));
        assert!(matches!(
            dispatch(millisecond_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(),
            NativeValue::Int(4)
        ));
        assert!(matches!(
            dispatch(ordinal_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(),
            NativeValue::Int(1)
        ));
        assert!(
            matches!(dispatch(iso_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(), NativeValue::Str(value) if value == "1970-01-01T01:02:03.004Z")
        );
        let (parse_id, _) = resolve("zun:date", "parseIsoDateTime").unwrap();
        assert!(
            matches!(dispatch(parse_id, &[NativeArg::Str("1970-01-01T01:02:03.004Z")], &mut sink).unwrap(), NativeValue::Int(value) if value == timestamp)
        );
        let (parse_date_id, _) = resolve("zun:date", "parseIsoDate").unwrap();
        assert!(matches!(
            dispatch(parse_date_id, &[NativeArg::Str("1970-01-01")], &mut sink).unwrap(),
            NativeValue::Int(0)
        ));
        assert!(dispatch(parse_date_id, &[NativeArg::Str("1970-02-30")], &mut sink).is_err());
        assert!(dispatch(parse_id, &[NativeArg::Str("not-a-date")], &mut sink).is_err());
        let leap_day =
            match dispatch(parse_date_id, &[NativeArg::Str("2020-02-29")], &mut sink).unwrap() {
                NativeValue::Int(value) => value,
                other => panic!("unexpected parsed leap-day value: {other:?}"),
            };
        assert!(matches!(
            dispatch(ordinal_id, &[NativeArg::Int(leap_day)], &mut sink).unwrap(),
            NativeValue::Int(60)
        ));
    }

    #[test]
    fn string_primitives_use_machine_facing_signatures() {
        let (length_id, length_fn) = resolve("zun:string", "length").unwrap();
        assert_eq!(length_fn.params, &[NativeTy::Str]);
        assert_eq!(length_fn.ret, NativeTy::U64);
        let (concat_id, _) = resolve("zun:string", "concat").unwrap();
        let mut sink = TestSink;
        assert!(matches!(
            dispatch(length_id, &[NativeArg::Str("hé")], &mut sink).unwrap(),
            NativeValue::Int(2)
        ));
        let (compare_id, compare_fn) = resolve("zun:string", "compare").unwrap();
        assert_eq!(compare_fn.params, &[NativeTy::Str, NativeTy::Str]);
        assert!(matches!(
            dispatch(
                compare_id,
                &[NativeArg::Str("abc"), NativeArg::Str("abd")],
                &mut sink
            )
            .unwrap(),
            NativeValue::Int(-1)
        ));
        let (to_int_id, to_int_fn) = resolve("zun:string", "toInt").unwrap();
        assert_eq!(to_int_fn.params, &[NativeTy::Str]);
        assert!(matches!(
            dispatch(to_int_id, &[NativeArg::Str(" -42 ")], &mut sink).unwrap(),
            NativeValue::Int(-42)
        ));
        let (to_float_id, to_float_fn) = resolve("zun:string", "toFloat").unwrap();
        assert_eq!(to_float_fn.params, &[NativeTy::Str]);
        assert!(matches!(
            dispatch(to_float_id, &[NativeArg::Str(" 3.5 ")], &mut sink).unwrap(),
            NativeValue::Float(value) if value == 3.5
        ));
        let (to_boolean_id, to_boolean_fn) = resolve("zun:string", "toBoolean").unwrap();
        assert_eq!(to_boolean_fn.params, &[NativeTy::Str]);
        assert!(matches!(
            dispatch(to_boolean_id, &[NativeArg::Str(" true ")], &mut sink).unwrap(),
            NativeValue::Bool(true)
        ));
        assert!(matches!(
            dispatch(length_id, &[NativeArg::Str("😀")], &mut sink).unwrap(),
            NativeValue::Int(2)
        ));
        assert!(matches!(
            dispatch(concat_id, &[NativeArg::Str("a"), NativeArg::Str("b")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "ab"
        ));
        let (contains_id, _) = resolve("zun:string", "contains").unwrap();
        assert!(matches!(
            dispatch(
                contains_id,
                &[NativeArg::Str("abc"), NativeArg::Str("bc")],
                &mut sink
            )
            .unwrap(),
            NativeValue::Bool(true)
        ));
        let (count_id, count_fn) = resolve("zun:string", "count").unwrap();
        assert_eq!(count_fn.params, &[NativeTy::Str, NativeTy::Str]);
        assert!(matches!(
            dispatch(
                count_id,
                &[NativeArg::Str("aaaa"), NativeArg::Str("aa")],
                &mut sink
            )
            .unwrap(),
            NativeValue::Int(2)
        ));
        let (includes_id, includes_fn) = resolve("zun:string", "includes").unwrap();
        assert_eq!(includes_fn.params, &[NativeTy::Str, NativeTy::Str]);
        assert!(matches!(
            dispatch(
                includes_id,
                &[NativeArg::Str("abc"), NativeArg::Str("bc")],
                &mut sink
            )
            .unwrap(),
            NativeValue::Bool(true)
        ));
        let (index_id, index_fn) = resolve("zun:string", "indexOf").unwrap();
        assert_eq!(index_fn.params, &[NativeTy::Str, NativeTy::Str]);
        assert!(matches!(
            dispatch(
                index_id,
                &[NativeArg::Str("😀zun"), NativeArg::Str("zun")],
                &mut sink
            )
            .unwrap(),
            NativeValue::Int(2)
        ));
        let (last_index_id, _) = resolve("zun:string", "lastIndexOf").unwrap();
        assert!(matches!(
            dispatch(
                last_index_id,
                &[NativeArg::Str("zunzun"), NativeArg::Str("zun")],
                &mut sink
            )
            .unwrap(),
            NativeValue::Int(3)
        ));
        let (replace_id, replace_fn) = resolve("zun:string", "replace").unwrap();
        assert_eq!(
            replace_fn.params,
            &[NativeTy::Str, NativeTy::Str, NativeTy::Str]
        );
        assert!(matches!(
            dispatch(replace_id, &[NativeArg::Str("zun vm"), NativeArg::Str("vm"), NativeArg::Str("runtime")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "zun runtime"
        ));
        let (substring_id, substring_fn) = resolve("zun:string", "substring").unwrap();
        assert_eq!(
            substring_fn.params,
            &[NativeTy::Str, NativeTy::U64, NativeTy::U64]
        );
        assert!(matches!(
            dispatch(substring_id, &[NativeArg::Str("😀zun"), NativeArg::Int(2), NativeArg::Int(5)], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "zun"
        ));
        let (pad_id, pad_fn) = resolve("zun:string", "padStart").unwrap();
        assert_eq!(
            pad_fn.params,
            &[NativeTy::Str, NativeTy::U64, NativeTy::Str]
        );
        assert!(matches!(
            dispatch(pad_id, &[NativeArg::Str("zun"), NativeArg::Int(5), NativeArg::Str("0")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "00zun"
        ));
        let (pad_end_id, pad_end_fn) = resolve("zun:string", "padEnd").unwrap();
        assert_eq!(
            pad_end_fn.params,
            &[NativeTy::Str, NativeTy::U64, NativeTy::Str]
        );
        assert!(matches!(
            dispatch(pad_end_id, &[NativeArg::Str("zun"), NativeArg::Int(5), NativeArg::Str("0")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "zun00"
        ));
        let (lower_id, _) = resolve("zun:string", "toLowerCase").unwrap();
        assert!(matches!(
            dispatch(lower_id, &[NativeArg::Str("ZuN")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "zun"
        ));
        let (upper_id, _) = resolve("zun:string", "toUpperCase").unwrap();
        assert!(matches!(
            dispatch(upper_id, &[NativeArg::Str("ZuN")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "ZUN"
        ));
        let (trim_start_id, _) = resolve("zun:string", "trimStart").unwrap();
        assert!(matches!(
            dispatch(trim_start_id, &[NativeArg::Str("  zun  ")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "zun  "
        ));
        let (trim_end_id, _) = resolve("zun:string", "trimEnd").unwrap();
        assert!(matches!(
            dispatch(trim_end_id, &[NativeArg::Str("  zun  ")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "  zun"
        ));
        let (char_code_id, _) = resolve("zun:string", "charCodeAt").unwrap();
        assert!(matches!(
            dispatch(
                char_code_id,
                &[NativeArg::Str("hé"), NativeArg::Int(1)],
                &mut sink
            )
            .unwrap(),
            NativeValue::Int(0xE9)
        ));
        assert!(matches!(
            dispatch(
                char_code_id,
                &[NativeArg::Str("😀"), NativeArg::Int(0)],
                &mut sink
            )
            .unwrap(),
            NativeValue::Int(0xD83D)
        ));
        let (starts_id, _) = resolve("zun:string", "startsWith").unwrap();
        assert!(matches!(
            dispatch(
                starts_id,
                &[NativeArg::Str("zun"), NativeArg::Str("zu")],
                &mut sink
            )
            .unwrap(),
            NativeValue::Bool(true)
        ));
        let (ends_id, _) = resolve("zun:string", "endsWith").unwrap();
        assert!(matches!(
            dispatch(
                ends_id,
                &[NativeArg::Str("zun"), NativeArg::Str("un")],
                &mut sink
            )
            .unwrap(),
            NativeValue::Bool(true)
        ));
        let (trim_id, _) = resolve("zun:string", "trim").unwrap();
        assert!(matches!(
            dispatch(trim_id, &[NativeArg::Str("  zun  ")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "zun"
        ));
        let (repeat_id, repeat_fn) = resolve("zun:string", "repeat").unwrap();
        assert_eq!(repeat_fn.params, &[NativeTy::Str, NativeTy::U64]);
        assert!(matches!(
            dispatch(repeat_id, &[NativeArg::Str("zu"), NativeArg::Int(3)], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "zuzuzu"
        ));
        let (reverse_id, reverse_fn) = resolve("zun:string", "reverse").unwrap();
        assert_eq!(reverse_fn.params, &[NativeTy::Str]);
        assert!(matches!(
            dispatch(reverse_id, &[NativeArg::Str("zun")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "nuz"
        ));
    }

    #[test]
    fn string_at_slice_and_code_point_statics() {
        let mut sink = TestSink;
        let (at_id, _) = resolve("zun:string", "at").unwrap();
        // Negative index counts from the end; out of range is `undefined`.
        assert!(matches!(
            dispatch(at_id, &[NativeArg::Str("hello"), NativeArg::Float(-1.0)], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "o"
        ));
        assert!(matches!(
            dispatch(at_id, &[NativeArg::Str("hello"), NativeArg::Float(9.0)], &mut sink).unwrap(),
            NativeValue::Void
        ));
        let (slice_id, _) = resolve("zun:string", "slice").unwrap();
        assert!(matches!(
            dispatch(slice_id, &[NativeArg::Str("hello"), NativeArg::Float(1.0), NativeArg::Float(3.0)], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "el"
        ));
        assert!(matches!(
            dispatch(slice_id, &[NativeArg::Str("hello"), NativeArg::Float(-2.0), NativeArg::Float(9e15)], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "lo"
        ));
        let (cp_id, _) = resolve("zun:string", "codePointAt").unwrap();
        // "😀" is a surrogate pair at index 0: the combined astral code point.
        assert!(matches!(
            dispatch(cp_id, &[NativeArg::Str("😀"), NativeArg::Int(0)], &mut sink).unwrap(),
            NativeValue::Int(0x1F600)
        ));
        assert!(matches!(
            dispatch(cp_id, &[NativeArg::Str("héllo"), NativeArg::Int(1)], &mut sink).unwrap(),
            NativeValue::Int(0xE9)
        ));
        let (replace_all_id, _) = resolve("zun:string", "replaceAll").unwrap();
        assert!(matches!(
            dispatch(replace_all_id, &[NativeArg::Str("aXbXc"), NativeArg::Str("X"), NativeArg::Str("-")], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "a-b-c"
        ));
        let (char_code_id, _) = resolve("zun:string", "fromCharCode1").unwrap();
        assert!(matches!(
            dispatch(char_code_id, &[NativeArg::Int(104)], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "h"
        ));
        let (code_point_id, _) = resolve("zun:string", "fromCodePoint1").unwrap();
        assert!(matches!(
            dispatch(code_point_id, &[NativeArg::Int(0x1F600)], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "😀"
        ));
        assert!(dispatch(code_point_id, &[NativeArg::Int(0x110000)], &mut sink).is_err());
    }

    #[test]
    fn number_primitives_parse_and_format() {
        let (parse_id, _) = resolve("zun:number", "parseFloat").unwrap();
        let (fixed_id, _) = resolve("zun:number", "toFixed").unwrap();
        let mut sink = TestSink;
        let (string_id, string_fn) = resolve("zun:number", "toString").unwrap();
        assert_eq!(string_fn.params, &[NativeTy::F64]);
        assert!(matches!(
            dispatch(string_id, &[NativeArg::Float(12.5)], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "12.5"
        ));
        assert!(matches!(
            dispatch(parse_id, &[NativeArg::Str(" 3.50 ")], &mut sink).unwrap(),
            NativeValue::Float(value) if value == 3.5
        ));
        assert!(matches!(
            dispatch(fixed_id, &[NativeArg::Float(3.5), NativeArg::Int(2)], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "3.50"
        ));
        let (finite_id, finite_fn) = resolve("zun:number", "isFinite").unwrap();
        let (nan_id, nan_fn) = resolve("zun:number", "isNaN").unwrap();
        assert_eq!(finite_fn.params, &[NativeTy::F64]);
        assert_eq!(nan_fn.params, &[NativeTy::F64]);
        assert!(matches!(
            dispatch(finite_id, &[NativeArg::Float(12.5)], &mut sink).unwrap(),
            NativeValue::Bool(true)
        ));
        assert!(matches!(
            dispatch(finite_id, &[NativeArg::Float(f64::INFINITY)], &mut sink).unwrap(),
            NativeValue::Bool(false)
        ));
        assert!(matches!(
            dispatch(nan_id, &[NativeArg::Float(f64::NAN)], &mut sink).unwrap(),
            NativeValue::Bool(true)
        ));
        let (negative_zero_id, _) = resolve("zun:number", "isNegativeZero").unwrap();
        let (safe_integer_id, _) = resolve("zun:number", "isSafeInteger").unwrap();
        assert!(matches!(
            dispatch(negative_zero_id, &[NativeArg::Float(-0.0)], &mut sink).unwrap(),
            NativeValue::Bool(true)
        ));
        assert!(matches!(
            dispatch(negative_zero_id, &[NativeArg::Float(0.0)], &mut sink).unwrap(),
            NativeValue::Bool(false)
        ));
        assert!(matches!(
            dispatch(
                safe_integer_id,
                &[NativeArg::Float(9_007_199_254_740_991.0)],
                &mut sink
            )
            .unwrap(),
            NativeValue::Bool(true)
        ));
        assert!(matches!(
            dispatch(safe_integer_id, &[NativeArg::Float(1.5)], &mut sink).unwrap(),
            NativeValue::Bool(false)
        ));
        assert!(matches!(
            dispatch(
                safe_integer_id,
                &[NativeArg::Float(f64::INFINITY)],
                &mut sink
            )
            .unwrap(),
            NativeValue::Bool(false)
        ));
    }

    #[test]
    fn number_strict_statics_do_not_coerce() {
        let mut sink = TestSink;
        let (is_nan, _) = resolve("zun:number", "isNaNStrict").unwrap();
        let (is_finite, _) = resolve("zun:number", "isFiniteStrict").unwrap();
        let (is_integer, _) = resolve("zun:number", "isIntegerStrict").unwrap();
        let (is_safe, _) = resolve("zun:number", "isSafeIntegerStrict").unwrap();
        let string_any = zun_core::Any::string(0);
        assert!(matches!(
            dispatch(is_nan, &[NativeArg::Float(f64::NAN)], &mut sink).unwrap(),
            NativeValue::Bool(true)
        ));
        // A non-number answers `false` without coercion.
        assert!(matches!(
            dispatch(is_nan, &[NativeArg::Any(string_any)], &mut sink).unwrap(),
            NativeValue::Bool(false)
        ));
        assert!(matches!(
            dispatch(is_finite, &[NativeArg::Float(f64::INFINITY)], &mut sink).unwrap(),
            NativeValue::Bool(false)
        ));
        assert!(matches!(
            dispatch(is_integer, &[NativeArg::Float(4.0)], &mut sink).unwrap(),
            NativeValue::Bool(true)
        ));
        assert!(matches!(
            dispatch(is_integer, &[NativeArg::Float(4.5)], &mut sink).unwrap(),
            NativeValue::Bool(false)
        ));
        assert!(matches!(
            dispatch(is_safe, &[NativeArg::Float(9_007_199_254_740_992.0)], &mut sink).unwrap(),
            NativeValue::Bool(false)
        ));
    }

    #[test]
    fn number_parse_statics_follow_js_semantics() {
        let mut sink = TestSink;
        let (parse_float, _) = resolve("zun:number", "parseFloatPrefix").unwrap();
        let (parse_int, _) = resolve("zun:number", "parseIntRadix").unwrap();
        assert!(matches!(
            dispatch(parse_float, &[NativeArg::Str("3.14abc")], &mut sink).unwrap(),
            NativeValue::Float(v) if v == 3.14
        ));
        assert!(matches!(
            dispatch(parse_float, &[NativeArg::Str("nope")], &mut sink).unwrap(),
            NativeValue::Float(v) if v.is_nan()
        ));
        assert!(matches!(
            dispatch(parse_int, &[NativeArg::Str("42"), NativeArg::Int(0)], &mut sink).unwrap(),
            NativeValue::Float(v) if v == 42.0
        ));
        assert!(matches!(
            dispatch(parse_int, &[NativeArg::Str("ff"), NativeArg::Int(16)], &mut sink).unwrap(),
            NativeValue::Float(v) if v == 255.0
        ));
        assert!(matches!(
            dispatch(parse_int, &[NativeArg::Str("0x1F"), NativeArg::Int(0)], &mut sink).unwrap(),
            NativeValue::Float(v) if v == 31.0
        ));
        assert!(matches!(
            dispatch(parse_int, &[NativeArg::Str("-10"), NativeArg::Int(2)], &mut sink).unwrap(),
            NativeValue::Float(v) if v == -2.0
        ));
    }

    #[test]
    fn number_to_string_radix_renders_bases() {
        let mut sink = TestSink;
        let (to_string, _) = resolve("zun:number", "toStringRadix").unwrap();
        assert!(matches!(
            dispatch(to_string, &[NativeArg::Float(255.0), NativeArg::Int(16)], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "ff"
        ));
        assert!(matches!(
            dispatch(to_string, &[NativeArg::Float(10.0), NativeArg::Int(2)], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "1010"
        ));
        assert!(matches!(
            dispatch(to_string, &[NativeArg::Float(255.0), NativeArg::Int(10)], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "255"
        ));
        assert!(matches!(
            dispatch(to_string, &[NativeArg::Float(f64::INFINITY), NativeArg::Int(2)], &mut sink).unwrap(),
            NativeValue::Str(v) if v == "Infinity"
        ));
        assert!(dispatch(to_string, &[NativeArg::Float(1.0), NativeArg::Int(40)], &mut sink).is_err());
    }

    #[test]
    fn math_rounding_matches_numeric_expectations() {
        let mut sink = TestSink;
        let (floor_id, _) = resolve("zun:math", "floor").unwrap();
        let (ceil_id, _) = resolve("zun:math", "ceil").unwrap();
        let (round_id, _) = resolve("zun:math", "round").unwrap();
        assert!(
            matches!(dispatch(floor_id, &[NativeArg::Float(-1.2)], &mut sink).unwrap(), NativeValue::Float(v) if v == -2.0)
        );
        assert!(
            matches!(dispatch(ceil_id, &[NativeArg::Float(-1.2)], &mut sink).unwrap(), NativeValue::Float(v) if v == -1.0)
        );
        assert!(
            matches!(dispatch(round_id, &[NativeArg::Float(1.6)], &mut sink).unwrap(), NativeValue::Float(v) if v == 2.0)
        );
    }

    #[test]
    fn math_power_matches_numeric_expectations() {
        let (id, _) = resolve("zun:math", "powNumber").expect("powNumber intrinsic");
        let mut sink = TestSink;
        let value = dispatch(
            id,
            &[NativeArg::Float(2.0), NativeArg::Float(3.0)],
            &mut sink,
        )
        .unwrap();
        assert!(matches!(value, NativeValue::Float(value) if value == 8.0));
    }

    #[test]
    fn math_trigonometry_matches_numeric_expectations() {
        let mut sink = TestSink;
        let (sin_id, _) = resolve("zun:math", "sinNumber").unwrap();
        let (cos_id, _) = resolve("zun:math", "cosNumber").unwrap();
        assert!(
            matches!(dispatch(sin_id, &[NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 0.0)
        );
        assert!(
            matches!(dispatch(cos_id, &[NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 1.0)
        );
    }

    #[test]
    fn math_exponential_and_logarithm_match_numeric_expectations() {
        let mut sink = TestSink;
        let (exp_id, _) = resolve("zun:math", "expNumber").unwrap();
        let (log_id, _) = resolve("zun:math", "logNumber").unwrap();
        assert!(
            matches!(dispatch(exp_id, &[NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 1.0)
        );
        assert!(
            matches!(dispatch(log_id, &[NativeArg::Float(1.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 0.0)
        );
    }

    #[test]
    fn math_tangent_and_atan2_match_numeric_expectations() {
        let mut sink = TestSink;
        let (tan_id, _) = resolve("zun:math", "tanNumber").unwrap();
        let (atan2_id, _) = resolve("zun:math", "atan2Number").unwrap();
        assert!(
            matches!(dispatch(tan_id, &[NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 0.0)
        );
        assert!(
            matches!(dispatch(atan2_id, &[NativeArg::Float(1.0), NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if (value - std::f64::consts::FRAC_PI_2).abs() < 1e-12)
        );
    }

    #[test]
    fn math_hypot_matches_numeric_expectations() {
        let (id, _) = resolve("zun:math", "hypotNumber").unwrap();
        let mut sink = TestSink;
        assert!(
            matches!(dispatch(id, &[NativeArg::Float(3.0), NativeArg::Float(4.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 5.0)
        );
    }

    struct TestSink;

    impl Sink for TestSink {
        fn println(&mut self, _: &str) {}
    }
}
