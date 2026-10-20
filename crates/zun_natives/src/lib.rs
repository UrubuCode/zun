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
mod math;
mod number;
mod string;
mod time;
mod date;

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

    #[test]
    fn math_intrinsics_resolve_after_existing_ids() {
        let (id, function) = resolve("zun:math", "sqrt").expect("math intrinsic");
        assert_eq!(function.ret, NativeTy::F64);
        assert_eq!(id as usize, MODULES[..4].iter().map(|m| m.functions.len()).sum::<usize>() + 1);
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
        let value = dispatch(
            id,
            &[NativeArg::Opaque, NativeArg::Float(7.5)],
            &mut sink,
        )
        .unwrap();
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
        assert!(matches!(dispatch(minute_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(), NativeValue::Int(2)));
        assert!(matches!(dispatch(second_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(), NativeValue::Int(3)));
        assert!(matches!(dispatch(millisecond_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(), NativeValue::Int(4)));
        assert!(matches!(dispatch(ordinal_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(), NativeValue::Int(1)));
        assert!(matches!(dispatch(iso_id, &[NativeArg::Int(timestamp)], &mut sink).unwrap(), NativeValue::Str(value) if value == "1970-01-01T01:02:03.004Z"));
        let (parse_id, _) = resolve("zun:date", "parseIsoDateTime").unwrap();
        assert!(matches!(dispatch(parse_id, &[NativeArg::Str("1970-01-01T01:02:03.004Z")], &mut sink).unwrap(), NativeValue::Int(value) if value == timestamp));
        let (parse_date_id, _) = resolve("zun:date", "parseIsoDate").unwrap();
        assert!(matches!(dispatch(parse_date_id, &[NativeArg::Str("1970-01-01")], &mut sink).unwrap(), NativeValue::Int(0)));
        assert!(dispatch(parse_date_id, &[NativeArg::Str("1970-02-30")], &mut sink).is_err());
        assert!(dispatch(parse_id, &[NativeArg::Str("not-a-date")], &mut sink).is_err());
        let leap_day = match dispatch(parse_date_id, &[NativeArg::Str("2020-02-29")], &mut sink).unwrap() {
            NativeValue::Int(value) => value,
            other => panic!("unexpected parsed leap-day value: {other:?}"),
        };
        assert!(matches!(dispatch(ordinal_id, &[NativeArg::Int(leap_day)], &mut sink).unwrap(), NativeValue::Int(60)));
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
            dispatch(compare_id, &[NativeArg::Str("abc"), NativeArg::Str("abd")], &mut sink).unwrap(),
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
            dispatch(contains_id, &[NativeArg::Str("abc"), NativeArg::Str("bc")], &mut sink).unwrap(),
            NativeValue::Bool(true)
        ));
        let (count_id, count_fn) = resolve("zun:string", "count").unwrap();
        assert_eq!(count_fn.params, &[NativeTy::Str, NativeTy::Str]);
        assert!(matches!(
            dispatch(count_id, &[NativeArg::Str("aaaa"), NativeArg::Str("aa")], &mut sink).unwrap(),
            NativeValue::Int(2)
        ));
        let (includes_id, includes_fn) = resolve("zun:string", "includes").unwrap();
        assert_eq!(includes_fn.params, &[NativeTy::Str, NativeTy::Str]);
        assert!(matches!(
            dispatch(includes_id, &[NativeArg::Str("abc"), NativeArg::Str("bc")], &mut sink).unwrap(),
            NativeValue::Bool(true)
        ));
        let (index_id, index_fn) = resolve("zun:string", "indexOf").unwrap();
        assert_eq!(index_fn.params, &[NativeTy::Str, NativeTy::Str]);
        assert!(matches!(
            dispatch(index_id, &[NativeArg::Str("😀zun"), NativeArg::Str("zun")], &mut sink).unwrap(),
            NativeValue::Int(2)
        ));
        let (last_index_id, _) = resolve("zun:string", "lastIndexOf").unwrap();
        assert!(matches!(
            dispatch(last_index_id, &[NativeArg::Str("zunzun"), NativeArg::Str("zun")], &mut sink).unwrap(),
            NativeValue::Int(3)
        ));
        let (replace_id, replace_fn) = resolve("zun:string", "replace").unwrap();
        assert_eq!(replace_fn.params, &[NativeTy::Str, NativeTy::Str, NativeTy::Str]);
        assert!(matches!(
            dispatch(replace_id, &[NativeArg::Str("zun vm"), NativeArg::Str("vm"), NativeArg::Str("runtime")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "zun runtime"
        ));
        let (substring_id, substring_fn) = resolve("zun:string", "substring").unwrap();
        assert_eq!(substring_fn.params, &[NativeTy::Str, NativeTy::U64, NativeTy::U64]);
        assert!(matches!(
            dispatch(substring_id, &[NativeArg::Str("😀zun"), NativeArg::Int(2), NativeArg::Int(5)], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "zun"
        ));
        let (pad_id, pad_fn) = resolve("zun:string", "padStart").unwrap();
        assert_eq!(pad_fn.params, &[NativeTy::Str, NativeTy::U64, NativeTy::Str]);
        assert!(matches!(
            dispatch(pad_id, &[NativeArg::Str("zun"), NativeArg::Int(5), NativeArg::Str("0")], &mut sink).unwrap(),
            NativeValue::Str(value) if value == "00zun"
        ));
        let (pad_end_id, pad_end_fn) = resolve("zun:string", "padEnd").unwrap();
        assert_eq!(pad_end_fn.params, &[NativeTy::Str, NativeTy::U64, NativeTy::Str]);
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
            dispatch(char_code_id, &[NativeArg::Str("hé"), NativeArg::Int(1)], &mut sink).unwrap(),
            NativeValue::Int(0xE9)
        ));
        assert!(matches!(
            dispatch(char_code_id, &[NativeArg::Str("😀"), NativeArg::Int(0)], &mut sink).unwrap(),
            NativeValue::Int(0xD83D)
        ));
        let (starts_id, _) = resolve("zun:string", "startsWith").unwrap();
        assert!(matches!(
            dispatch(starts_id, &[NativeArg::Str("zun"), NativeArg::Str("zu")], &mut sink).unwrap(),
            NativeValue::Bool(true)
        ));
        let (ends_id, _) = resolve("zun:string", "endsWith").unwrap();
        assert!(matches!(
            dispatch(ends_id, &[NativeArg::Str("zun"), NativeArg::Str("un")], &mut sink).unwrap(),
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
    }

    #[test]
    fn math_rounding_matches_numeric_expectations() {
        let mut sink = TestSink;
        let (floor_id, _) = resolve("zun:math", "floor").unwrap();
        let (ceil_id, _) = resolve("zun:math", "ceil").unwrap();
        let (round_id, _) = resolve("zun:math", "round").unwrap();
        assert!(matches!(dispatch(floor_id, &[NativeArg::Float(-1.2)], &mut sink).unwrap(), NativeValue::Float(v) if v == -2.0));
        assert!(matches!(dispatch(ceil_id, &[NativeArg::Float(-1.2)], &mut sink).unwrap(), NativeValue::Float(v) if v == -1.0));
        assert!(matches!(dispatch(round_id, &[NativeArg::Float(1.6)], &mut sink).unwrap(), NativeValue::Float(v) if v == 2.0));
    }

    #[test]
    fn math_power_matches_numeric_expectations() {
        let (id, _) = resolve("zun:math", "powNumber").expect("powNumber intrinsic");
        let mut sink = TestSink;
        let value = dispatch(id, &[NativeArg::Float(2.0), NativeArg::Float(3.0)], &mut sink).unwrap();
        assert!(matches!(value, NativeValue::Float(value) if value == 8.0));
    }

    #[test]
    fn math_trigonometry_matches_numeric_expectations() {
        let mut sink = TestSink;
        let (sin_id, _) = resolve("zun:math", "sinNumber").unwrap();
        let (cos_id, _) = resolve("zun:math", "cosNumber").unwrap();
        assert!(matches!(dispatch(sin_id, &[NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 0.0));
        assert!(matches!(dispatch(cos_id, &[NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 1.0));
    }

    #[test]
    fn math_exponential_and_logarithm_match_numeric_expectations() {
        let mut sink = TestSink;
        let (exp_id, _) = resolve("zun:math", "expNumber").unwrap();
        let (log_id, _) = resolve("zun:math", "logNumber").unwrap();
        assert!(matches!(dispatch(exp_id, &[NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 1.0));
        assert!(matches!(dispatch(log_id, &[NativeArg::Float(1.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 0.0));
    }

    #[test]
    fn math_tangent_and_atan2_match_numeric_expectations() {
        let mut sink = TestSink;
        let (tan_id, _) = resolve("zun:math", "tanNumber").unwrap();
        let (atan2_id, _) = resolve("zun:math", "atan2Number").unwrap();
        assert!(matches!(dispatch(tan_id, &[NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 0.0));
        assert!(matches!(dispatch(atan2_id, &[NativeArg::Float(1.0), NativeArg::Float(0.0)], &mut sink).unwrap(), NativeValue::Float(value) if (value - std::f64::consts::FRAC_PI_2).abs() < 1e-12));
    }

    #[test]
    fn math_hypot_matches_numeric_expectations() {
        let (id, _) = resolve("zun:math", "hypotNumber").unwrap();
        let mut sink = TestSink;
        assert!(matches!(dispatch(id, &[NativeArg::Float(3.0), NativeArg::Float(4.0)], &mut sink).unwrap(), NativeValue::Float(value) if value == 5.0));
    }

    struct TestSink;

    impl Sink for TestSink {
        fn println(&mut self, _: &str) {}
    }
}
