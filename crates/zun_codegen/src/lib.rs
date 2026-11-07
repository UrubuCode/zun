//! Tier 1: a Cranelift JIT for the numeric ZIL subset.
//!
//! [`Jit`] compiles every eligible method (see [`eligible`]) to native code and
//! calls it directly. Each method becomes a Cranelift function whose parameters
//! and result are all `f64`; booleans travel as `0.0`/`1.0`. Control flow is
//! rebuilt into a Cranelift CFG: because the front-end leaves the evaluation
//! stack empty at every branch and branch target, basic blocks need no stack
//! parameters, and locals become Cranelift [`Variable`]s.
//!
//! This is the second implementation of ZIL execution; the interpreter is the
//! first. The equivalence tests run numeric programs through both and require
//! identical results — the contract the plan sets for Tier 0 vs Tier 1.

mod any_abi;
mod eligible;
mod fn_abi;
mod shim;
mod thunks;
mod translate;

pub use any_abi::JitAnyAbi;
pub use eligible::{eligible_methods, eligible_methods_bridged, eligible_methods_with_literals};
pub use zun_il::{JitKind, NumKind};

// Runtime thunks live in `thunks/`; re-export every public symbol at the
// crate root so the generated native code and the `builder.symbol` calls
// below keep referring to them by their original `zun_codegen::zun_jit_*` paths.
pub use thunks::*;

use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::{types, AbiParam};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::FunctionBuilderContext;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module as ClifModule};
use cranelift_object::{ObjectBuilder, ObjectModule};
use zun_module::{read_body, Module};

use translate::clif_ty;

/// Errors from building or invoking JIT code.
#[derive(Debug)]
pub enum JitError {
    /// The host architecture is not supported by Cranelift.
    UnsupportedHost(String),
    /// A Cranelift module operation failed.
    Module(String),
    /// Compilation of a specific method failed.
    Compile {
        /// The method RID.
        method_rid: u32,
        /// The failure detail.
        detail: String,
    },
    /// The requested method is not JIT-eligible.
    Ineligible(u32),
    /// The method's arity does not match the arguments provided.
    Arity {
        /// Parameters the method declares.
        expected: usize,
        /// Arguments supplied.
        found: usize,
    },
    /// An `Any` carrier contains an unknown runtime tag.
    InvalidAnyTag(u64),
}

impl core::fmt::Display for JitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            JitError::UnsupportedHost(e) => write!(f, "unsupported host: {e}"),
            JitError::Module(e) => write!(f, "module error: {e}"),
            JitError::Compile { method_rid, detail } => {
                write!(f, "compiling method #{method_rid}: {detail}")
            }
            JitError::Ineligible(rid) => write!(f, "method #{rid} is not JIT-eligible"),
            JitError::Arity { expected, found } => {
                write!(f, "arity mismatch: expected {expected}, found {found}")
            }
            JitError::InvalidAnyTag(tag) => write!(f, "invalid Any tag: {tag}"),
        }
    }
}

impl std::error::Error for JitError {}

/// A JIT that compiles a module's eligible numeric methods to native code.
/// The result of calling a compiled method, carrying its numeric kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RetVal {
    /// A float result (f32 widened to f64).
    Float(f64),
    /// An integer result (any width, in an i64; unsigned zero-extended).
    Int(i64),
    /// A boolean result.
    Bool(bool),
    /// No value (void).
    Void,
}

/// A machine-code safepoint carrying a user stack map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackMapInfo {
    /// Offset of the safepoint from the method's native entry point.
    pub code_offset: u32,
    /// Number of bytes covered by the map in the native frame.
    pub span: u32,
    /// Frame offsets, relative to the native stack pointer, containing managed
    /// references at this safepoint.
    pub reference_slots: Vec<u32>,
    /// Absolute address of the safepoint after JIT finalization.
    pub pc: usize,
}

/// A compiled method's call shape.
#[derive(Clone)]
struct Sig {
    param_kinds: Vec<JitKind>,
    returns: Option<JitKind>,
}

/// Host entry points and state for the JIT→interpreter bridge, supplied by the
/// VM (which owns the interpreter). All pointers must outlive any compiled code.
#[derive(Debug, Clone, Copy)]
pub struct BridgeConfig {
    /// `extern "C" zun_jit_call_interp(rid, args_ptr, argc) -> u64`.
    pub call_interp: *const u8,
    /// `extern "C" zun_jit_call_indirect(identity, environment, args_ptr, argc) -> u64`:
    /// dispatch a `CallIndirect` on a concrete function value, reconstructing a
    /// `FunctionHandle` from the two carrier words so a captured closure
    /// environment reaches the callee.
    pub call_indirect: *const u8,
    /// `extern "C" zun_jit_route_pending(catch_type_rid) -> u64` (0 = no/unmatched
    /// pending; else the caught object handle, slot cleared).
    pub route_pending: *const u8,
    /// Address of the per-thread pending-exception slot the JIT loads after calls.
    pub pending_slot_addr: u64,
}

pub struct Jit {
    module: JITModule,
    func_ids: HashMap<u32, FuncId>,
    shim_ids: HashMap<u32, FuncId>,
    eligible: HashSet<u32>,
    /// Eligible callees of each eligible method (for the compile closure).
    callees: HashMap<u32, Vec<u32>>,
    /// Methods whose bodies are defined and finalized.
    defined: HashSet<u32>,
    sigs: HashMap<u32, Sig>,
    ctx: cranelift_codegen::Context,
    fb_ctx: FunctionBuilderContext,
    types: zun_types::TypeSystem,
    virtual_tables: Vec<Box<[u64]>>,
    virtual_table_ptrs: Box<[u64]>,
    _literal_storage: Vec<Box<[u8]>>,
    literal_ptrs: Box<[u64]>,
    stack_maps: HashMap<u32, Vec<StackMapInfo>>,
    stack_map_registry: zun_gc::StackMapRegistry,
    /// Whether a JIT→interpreter call bridge is wired (its `zun_jit_call_interp`
    /// symbol registered and eligibility relaxed). When set, a call to an
    /// ineligible, non-throwing callee is lowered to a bridged call.
    bridge_enabled: bool,
    /// Address of the per-thread pending-exception slot the JIT loads after a
    /// call to detect a propagated throw (0 when no bridge).
    bridge_pending_addr: u64,
}

impl core::fmt::Debug for Jit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Jit")
            .field("compiled", &self.func_ids.len())
            .finish()
    }
}

impl Jit {
    fn register_heap_layouts(&self, heap: &mut zun_gc::Heap) {
        for type_rid in 1.. {
            if self.types.get(type_rid).is_none() {
                break;
            }
            heap.register_reference_slots(type_rid, self.types.reference_slots(type_rid));
        }
    }

    /// Create a JIT for `zm`: determine eligible methods and declare their
    /// Cranelift functions, without compiling any bodies yet. Bodies are
    /// defined on demand by [`Jit::ensure_compiled`] (lazy tiering) or all at
    /// once by [`Jit::compile`].
    pub fn new(zm: &Module) -> Result<Self, JitError> {
        Self::new_impl(zm, None)
    }

    /// Like [`Jit::new`], but wires a JIT→interpreter call bridge: `bridge_fn` is
    /// the address of the host `zun_jit_call_interp(rid, args_ptr, argc) -> u64`
    /// (provided by the VM, which owns the interpreter). Eligibility is relaxed
    /// so a method calling an ineligible, non-throwing method stays compiled and
    /// that call is bridged. The caller must install the VM-side execution
    /// context before running bridged code.
    ///
    /// # Safety
    /// `bridge_fn` must be a valid `extern "C"` function pointer with the bridge
    /// signature, valid for as long as compiled code can run.
    pub unsafe fn new_bridged(zm: &Module, bridge: BridgeConfig) -> Result<Self, JitError> {
        Self::new_impl(zm, Some(bridge))
    }

    /// Compile every eligible method eagerly with a bridge wired (see
    /// [`Jit::new_bridged`] and [`Jit::compile`]).
    ///
    /// # Safety
    /// See [`Jit::new_bridged`].
    pub unsafe fn compile_with_bridge(zm: &Module, bridge: BridgeConfig) -> Result<Self, JitError> {
        let mut jit = unsafe { Self::new_bridged(zm, bridge)? };
        let all: Vec<u32> = jit.eligible.iter().copied().collect();
        for rid in all {
            jit.ensure_compiled(zm, rid)?;
        }
        Ok(jit)
    }

    fn new_impl(zm: &Module, bridge: Option<BridgeConfig>) -> Result<Self, JitError> {
        let bridge_fn = bridge.as_ref().map(|b| b.call_interp);
        let bridge_pending_addr = bridge.as_ref().map(|b| b.pending_slot_addr).unwrap_or(0);
        let mut flags = settings::builder();
        // Optimize for speed; the e-graph optimizer is on by default in this
        // Cranelift version, so there is no flag to set for it.
        flags.set("opt_level", "speed").unwrap();
        // Keep a stable frame chain so a future GC safepoint can capture the
        // JIT frame pointer and return address alongside its stack map.
        flags.set("preserve_frame_pointers", "true").unwrap();
        let isa = cranelift_native::builder()
            .map_err(|e| JitError::UnsupportedHost(e.to_string()))?
            .finish(settings::Flags::new(flags))
            .map_err(|e| JitError::UnsupportedHost(e.to_string()))?;

        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        // Make the runtime allocator callable from native `newobj`.
        builder.symbol("zun_rt_alloc", zun_gc::zun_rt_alloc as *const u8);
        builder.symbol("zun_jit_array_length", zun_jit_array_length as *const u8);
        builder.symbol("zun_jit_string_length", zun_jit_string_length as *const u8);
        builder.symbol(
            "zun_jit_string_literal",
            zun_jit_string_literal as *const u8,
        );
        builder.symbol(
            "zun_jit_string_contains",
            zun_jit_string_contains as *const u8,
        );
        builder.symbol(
            "zun_jit_string_char_code_at",
            zun_jit_string_char_code_at as *const u8,
        );
        builder.symbol(
            "zun_jit_string_starts_with",
            zun_jit_string_starts_with as *const u8,
        );
        builder.symbol(
            "zun_jit_string_ends_with",
            zun_jit_string_ends_with as *const u8,
        );
        builder.symbol(
            "zun_jit_string_index_of",
            zun_jit_string_index_of as *const u8,
        );
        builder.symbol(
            "zun_jit_string_last_index_of",
            zun_jit_string_last_index_of as *const u8,
        );
        builder.symbol("zun_jit_string_count", zun_jit_string_count as *const u8);
        builder.symbol(
            "zun_jit_string_compare",
            zun_jit_string_compare as *const u8,
        );
        builder.symbol("zun_jit_string_concat", zun_jit_string_concat as *const u8);
        builder.symbol(
            "zun_jit_string_substring",
            zun_jit_string_substring as *const u8,
        );
        builder.symbol("zun_jit_string_trim", zun_jit_string_trim as *const u8);
        builder.symbol(
            "zun_jit_string_trim_start",
            zun_jit_string_trim_start as *const u8,
        );
        builder.symbol(
            "zun_jit_string_trim_end",
            zun_jit_string_trim_end as *const u8,
        );
        builder.symbol(
            "zun_jit_string_to_lower_case",
            zun_jit_string_to_lower_case as *const u8,
        );
        builder.symbol(
            "zun_jit_string_to_upper_case",
            zun_jit_string_to_upper_case as *const u8,
        );
        builder.symbol("zun_jit_string_repeat", zun_jit_string_repeat as *const u8);
        builder.symbol(
            "zun_jit_string_reverse",
            zun_jit_string_reverse as *const u8,
        );
        builder.symbol(
            "zun_jit_string_pad_start",
            zun_jit_string_pad_start as *const u8,
        );
        builder.symbol(
            "zun_jit_string_pad_end",
            zun_jit_string_pad_end as *const u8,
        );
        builder.symbol(
            "zun_jit_string_replace",
            zun_jit_string_replace as *const u8,
        );
        builder.symbol("zun_jit_string_to_int", zun_jit_string_to_int as *const u8);
        builder.symbol(
            "zun_jit_string_to_float",
            zun_jit_string_to_float as *const u8,
        );
        builder.symbol(
            "zun_jit_string_to_boolean",
            zun_jit_string_to_boolean as *const u8,
        );
        builder.symbol("zun_jit_buffer_length", zun_jit_buffer_length as *const u8);
        builder.symbol(
            "zun_jit_data_view_length",
            zun_jit_data_view_length as *const u8,
        );
        builder.symbol(
            "zun_jit_typed_array_length",
            zun_jit_typed_array_length as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_byte_length",
            zun_jit_data_view_byte_length as *const u8,
        );
        builder.symbol(
            "zun_jit_typed_array_byte_length",
            zun_jit_typed_array_byte_length as *const u8,
        );
        builder.symbol("zun_jit_buffer_get_u8", zun_jit_buffer_get_u8 as *const u8);
        builder.symbol(
            "zun_jit_buffer_read_u32",
            zun_jit_buffer_read_u32 as *const u8,
        );
        builder.symbol(
            "zun_jit_buffer_read_f64",
            zun_jit_buffer_read_f64 as *const u8,
        );
        builder.symbol("zun_jit_buffer_set_u8", zun_jit_buffer_set_u8 as *const u8);
        builder.symbol(
            "zun_jit_buffer_write_u32",
            zun_jit_buffer_write_u32 as *const u8,
        );
        builder.symbol(
            "zun_jit_buffer_write_f64",
            zun_jit_buffer_write_f64 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_get_u32",
            zun_jit_data_view_get_u32 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_get_u16",
            zun_jit_data_view_get_u16 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_get_f64",
            zun_jit_data_view_get_f64 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_set_u32",
            zun_jit_data_view_set_u32 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_set_u16",
            zun_jit_data_view_set_u16 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_set_f64",
            zun_jit_data_view_set_f64 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_get_i8",
            zun_jit_data_view_get_i8 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_get_u8",
            zun_jit_data_view_get_u8 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_get_i16",
            zun_jit_data_view_get_i16 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_get_i32",
            zun_jit_data_view_get_i32 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_get_f32",
            zun_jit_data_view_get_f32 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_set_i8",
            zun_jit_data_view_set_i8 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_set_u8",
            zun_jit_data_view_set_u8 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_set_i16",
            zun_jit_data_view_set_i16 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_set_i32",
            zun_jit_data_view_set_i32 as *const u8,
        );
        builder.symbol(
            "zun_jit_data_view_set_f32",
            zun_jit_data_view_set_f32 as *const u8,
        );
        builder.symbol("zun_jit_u8_array_get", zun_jit_u8_array_get as *const u8);
        builder.symbol("zun_jit_u8_array_set", zun_jit_u8_array_set as *const u8);
        builder.symbol("zun_jit_u16_array_get", zun_jit_u16_array_get as *const u8);
        builder.symbol("zun_jit_u16_array_set", zun_jit_u16_array_set as *const u8);
        builder.symbol("zun_jit_u32_array_get", zun_jit_u32_array_get as *const u8);
        builder.symbol("zun_jit_u32_array_set", zun_jit_u32_array_set as *const u8);
        builder.symbol("zun_jit_f64_array_get", zun_jit_f64_array_get as *const u8);
        builder.symbol("zun_jit_f64_array_set", zun_jit_f64_array_set as *const u8);
        builder.symbol("zun_jit_map_size", zun_jit_map_size as *const u8);
        builder.symbol("zun_jit_map_new", zun_jit_map_new as *const u8);
        builder.symbol("zun_jit_map_clear", zun_jit_map_clear as *const u8);
        builder.symbol("zun_jit_map_values", zun_jit_map_values as *const u8);
        builder.symbol("zun_jit_map_keys", zun_jit_map_keys as *const u8);
        builder.symbol("zun_jit_map_entries", zun_jit_map_entries as *const u8);
        builder.symbol("zun_jit_set_values", zun_jit_set_values as *const u8);
        builder.symbol("zun_jit_set_entries", zun_jit_set_entries as *const u8);
        builder.symbol("zun_jit_set_size", zun_jit_set_size as *const u8);
        builder.symbol("zun_jit_set_new", zun_jit_set_new as *const u8);
        builder.symbol("zun_jit_set_clear", zun_jit_set_clear as *const u8);
        builder.symbol("zun_jit_map_has_f64", zun_jit_map_has_f64 as *const u8);
        builder.symbol("zun_jit_set_has_f64", zun_jit_set_has_f64 as *const u8);
        builder.symbol("zun_jit_set_add_f64", zun_jit_set_add_f64 as *const u8);
        builder.symbol(
            "zun_jit_map_delete_f64",
            zun_jit_map_delete_f64 as *const u8,
        );
        builder.symbol(
            "zun_jit_set_delete_f64",
            zun_jit_set_delete_f64 as *const u8,
        );
        builder.symbol("zun_jit_map_set_f64", zun_jit_map_set_f64 as *const u8);
        builder.symbol(
            "zun_jit_map_get_f64_or",
            zun_jit_map_get_f64_or as *const u8,
        );
        builder.symbol("zun_jit_map_get_any", zun_jit_map_get_any as *const u8);
        builder.symbol("zun_jit_map_has_i64", zun_jit_map_has_i64 as *const u8);
        builder.symbol("zun_jit_map_has_i32", zun_jit_map_has_i32 as *const u8);
        builder.symbol("zun_jit_set_has_i64", zun_jit_set_has_i64 as *const u8);
        builder.symbol("zun_jit_set_has_i32", zun_jit_set_has_i32 as *const u8);
        builder.symbol("zun_jit_set_add_i64", zun_jit_set_add_i64 as *const u8);
        builder.symbol("zun_jit_set_add_i32", zun_jit_set_add_i32 as *const u8);
        builder.symbol(
            "zun_jit_map_delete_i32",
            zun_jit_map_delete_i32 as *const u8,
        );
        builder.symbol(
            "zun_jit_set_delete_i32",
            zun_jit_set_delete_i32 as *const u8,
        );
        builder.symbol(
            "zun_jit_map_delete_i64",
            zun_jit_map_delete_i64 as *const u8,
        );
        builder.symbol(
            "zun_jit_set_delete_i64",
            zun_jit_set_delete_i64 as *const u8,
        );
        builder.symbol("zun_jit_map_set_i64", zun_jit_map_set_i64 as *const u8);
        builder.symbol(
            "zun_jit_map_get_i64_or",
            zun_jit_map_get_i64_or as *const u8,
        );
        builder.symbol(
            "zun_jit_map_get_i32_or",
            zun_jit_map_get_i32_or as *const u8,
        );
        builder.symbol("zun_jit_map_set_i32", zun_jit_map_set_i32 as *const u8);
        builder.symbol("zun_jit_array_new", zun_jit_array_new as *const u8);
        builder.symbol("zun_jit_osr_poll", zun_jit_osr_poll as *const u8);
        builder.symbol(
            "zun_jit_array_load_f64",
            zun_jit_array_load_f64 as *const u8,
        );
        builder.symbol(
            "zun_jit_array_load_u64",
            zun_jit_array_load_u64 as *const u8,
        );
        builder.symbol(
            "zun_jit_array_load_i32",
            zun_jit_array_load_i32 as *const u8,
        );
        builder.symbol(
            "zun_jit_array_load_bool",
            zun_jit_array_load_bool as *const u8,
        );
        builder.symbol(
            "zun_jit_array_load_object",
            zun_jit_array_load_object as *const u8,
        );
        builder.symbol(
            "zun_jit_array_store_f64",
            zun_jit_array_store_f64 as *const u8,
        );
        builder.symbol(
            "zun_jit_array_store_u64",
            zun_jit_array_store_u64 as *const u8,
        );
        builder.symbol(
            "zun_jit_array_store_i32",
            zun_jit_array_store_i32 as *const u8,
        );
        builder.symbol(
            "zun_jit_array_store_bool",
            zun_jit_array_store_bool as *const u8,
        );
        builder.symbol(
            "zun_jit_array_store_object",
            zun_jit_array_store_object as *const u8,
        );
        builder.symbol("zun_jit_store_ref", zun_jit_store_ref as *const u8);
        builder.symbol("zun_jit_array_pop_f64", zun_jit_array_pop_f64 as *const u8);
        builder.symbol(
            "zun_jit_array_push_f64",
            zun_jit_array_push_f64 as *const u8,
        );
        builder.symbol("zun_jit_array_pop_u64", zun_jit_array_pop_u64 as *const u8);
        builder.symbol("zun_jit_array_pop_i32", zun_jit_array_pop_i32 as *const u8);
        builder.symbol(
            "zun_jit_array_push_u64",
            zun_jit_array_push_u64 as *const u8,
        );
        builder.symbol(
            "zun_jit_array_push_i32",
            zun_jit_array_push_i32 as *const u8,
        );
        builder.symbol(
            "zun_jit_array_pop_bool",
            zun_jit_array_pop_bool as *const u8,
        );
        builder.symbol(
            "zun_jit_array_push_bool",
            zun_jit_array_push_bool as *const u8,
        );
        builder.symbol(
            "zun_jit_array_pop_object",
            zun_jit_array_pop_object as *const u8,
        );
        builder.symbol(
            "zun_jit_array_push_object",
            zun_jit_array_push_object as *const u8,
        );
        builder.symbol(
            "zun_jit_array_load_any",
            zun_jit_array_load_any as *const u8,
        );
        builder.symbol(
            "zun_jit_array_store_any",
            zun_jit_array_store_any as *const u8,
        );
        builder.symbol("zun_jit_date_year", zun_jit_date_year as *const u8);
        builder.symbol("zun_jit_date_is_valid", zun_jit_date_is_valid as *const u8);
        builder.symbol("zun_jit_date_month", zun_jit_date_month as *const u8);
        builder.symbol("zun_jit_date_day", zun_jit_date_day as *const u8);
        builder.symbol("zun_jit_date_hour", zun_jit_date_hour as *const u8);
        builder.symbol("zun_jit_date_minute", zun_jit_date_minute as *const u8);
        builder.symbol("zun_jit_date_second", zun_jit_date_second as *const u8);
        builder.symbol(
            "zun_jit_date_millisecond",
            zun_jit_date_millisecond as *const u8,
        );
        builder.symbol("zun_jit_date_weekday", zun_jit_date_weekday as *const u8);
        builder.symbol(
            "zun_jit_date_day_of_year",
            zun_jit_date_day_of_year as *const u8,
        );
        builder.symbol("zun_jit_math_abs", zun_jit_math_abs as *const u8);
        builder.symbol("zun_jit_math_sqrt", zun_jit_math_sqrt as *const u8);
        builder.symbol("zun_jit_math_cbrt", zun_jit_math_cbrt as *const u8);
        builder.symbol("zun_jit_math_sign", zun_jit_math_sign as *const u8);
        builder.symbol("zun_jit_math_floor", zun_jit_math_floor as *const u8);
        builder.symbol("zun_jit_math_ceil", zun_jit_math_ceil as *const u8);
        builder.symbol("zun_jit_math_round", zun_jit_math_round as *const u8);
        builder.symbol("zun_jit_math_trunc", zun_jit_math_trunc as *const u8);
        builder.symbol("zun_jit_math_sin", zun_jit_math_sin as *const u8);
        builder.symbol("zun_jit_math_cos", zun_jit_math_cos as *const u8);
        builder.symbol("zun_jit_math_exp", zun_jit_math_exp as *const u8);
        builder.symbol("zun_jit_math_log", zun_jit_math_log as *const u8);
        builder.symbol("zun_jit_math_log10", zun_jit_math_log10 as *const u8);
        builder.symbol("zun_jit_math_log2", zun_jit_math_log2 as *const u8);
        builder.symbol("zun_jit_math_expm1", zun_jit_math_expm1 as *const u8);
        builder.symbol("zun_jit_math_log1p", zun_jit_math_log1p as *const u8);
        builder.symbol("zun_jit_math_sinh", zun_jit_math_sinh as *const u8);
        builder.symbol("zun_jit_math_cosh", zun_jit_math_cosh as *const u8);
        builder.symbol("zun_jit_math_tanh", zun_jit_math_tanh as *const u8);
        builder.symbol("zun_jit_math_asinh", zun_jit_math_asinh as *const u8);
        builder.symbol("zun_jit_math_acosh", zun_jit_math_acosh as *const u8);
        builder.symbol("zun_jit_math_atanh", zun_jit_math_atanh as *const u8);
        builder.symbol("zun_jit_math_tan", zun_jit_math_tan as *const u8);
        builder.symbol("zun_jit_math_min", zun_jit_math_min as *const u8);
        builder.symbol("zun_jit_math_max", zun_jit_math_max as *const u8);
        builder.symbol("zun_jit_math_pow", zun_jit_math_pow as *const u8);
        builder.symbol("zun_jit_math_atan2", zun_jit_math_atan2 as *const u8);
        builder.symbol("zun_jit_math_hypot", zun_jit_math_hypot as *const u8);
        builder.symbol("zun_jit_math_fmod", zun_jit_math_fmod as *const u8);
        builder.symbol("zun_jit_static_get", zun_jit_static_get as *const u8);
        builder.symbol("zun_jit_static_set", zun_jit_static_set as *const u8);
        builder.symbol(
            "zun_jit_number_is_finite",
            zun_jit_number_is_finite as *const u8,
        );
        builder.symbol("zun_jit_number_is_nan", zun_jit_number_is_nan as *const u8);
        builder.symbol(
            "zun_jit_number_is_negative_zero",
            zun_jit_number_is_negative_zero as *const u8,
        );
        builder.symbol(
            "zun_jit_number_is_safe_integer",
            zun_jit_number_is_safe_integer as *const u8,
        );
        if let Some(ref bridge) = bridge {
            builder.symbol("zun_jit_call_interp", bridge.call_interp);
            builder.symbol("zun_jit_call_indirect", bridge.call_indirect);
            builder.symbol("zun_jit_route_pending", bridge.route_pending);
        }
        let module = JITModule::new(builder);

        let mut literal_storage = Vec::new();
        let mut string_offset = 0usize;
        while string_offset < zm.strings.as_bytes().len() {
            let value = zm
                .strings
                .get(zun_metadata::StringIndex(string_offset as u32));
            literal_storage.push(value.as_bytes().to_vec().into_boxed_slice());
            string_offset = zm.strings.next_offset(string_offset);
        }
        let mut literal_ptrs = vec![0u64; zm.strings.as_bytes().len()];
        let mut string_offset = 0usize;
        let mut storage_index = 0usize;
        while string_offset < zm.strings.as_bytes().len() {
            literal_ptrs[string_offset] = literal_storage[storage_index].as_ptr() as u64;
            let _value = zm
                .strings
                .get(zun_metadata::StringIndex(string_offset as u32));
            string_offset = zm.strings.next_offset(string_offset);
            storage_index += 1;
        }
        let literal_ptrs = literal_ptrs.into_boxed_slice();
        let eligible = if bridge_fn.is_some() {
            eligible_methods_bridged(zm, true)
        } else {
            eligible_methods_with_literals(zm, true)
        };
        let types =
            zun_types::TypeSystem::from_module(zm).map_err(|e| JitError::Module(e.to_string()))?;
        let virtual_tables: Vec<Box<[u64]>> = (1..=types.len() as u32)
            .map(|type_rid| {
                types
                    .get(type_rid)
                    .map(|ty| vec![0; ty.vtable.len()].into_boxed_slice())
                    .unwrap_or_default()
            })
            .collect();
        let mut virtual_table_ptrs = vec![0u64; types.len() + 1];
        for (index, table) in virtual_tables.iter().enumerate() {
            virtual_table_ptrs[index + 1] = table.as_ptr() as usize as u64;
        }

        // Declare every eligible function and shim up front (signatures only) so
        // calls can reference any of them; bodies are defined later, eagerly by
        // `compile` or lazily by `ensure_compiled`.
        let mut jit = Self {
            module,
            func_ids: HashMap::new(),
            shim_ids: HashMap::new(),
            eligible: eligible.clone(),
            callees: HashMap::new(),
            defined: HashSet::new(),
            sigs: HashMap::new(),
            ctx: cranelift_codegen::Context::new(),
            fb_ctx: FunctionBuilderContext::new(),
            types,
            virtual_tables,
            virtual_table_ptrs: virtual_table_ptrs.into_boxed_slice(),
            _literal_storage: literal_storage,
            literal_ptrs,
            stack_maps: HashMap::new(),
            stack_map_registry: zun_gc::StackMapRegistry::new(),
            bridge_enabled: bridge_fn.is_some(),
            bridge_pending_addr,
        };

        let mut methods_by_name: HashMap<_, Vec<u32>> = HashMap::new();
        for (index, method) in zm.tables.method_defs.iter().enumerate() {
            methods_by_name
                .entry(method.name)
                .or_default()
                .push(index as u32 + 1);
        }

        for &rid in &eligible {
            let body = read_body(
                &zm.code,
                zm.tables.method_defs[rid as usize - 1].body_offset,
            )
            .map_err(|e| JitError::Compile {
                method_rid: rid,
                detail: e.to_string(),
            })?;
            let sig = body_signature(&jit.module, &body);
            let fid = jit
                .module
                .declare_function(&format!("m{rid}"), Linkage::Local, &sig)
                .map_err(|e| JitError::Module(e.to_string()))?;
            jit.func_ids.insert(rid, fid);

            let param_kinds: Vec<JitKind> = body.local_kinds[..body.param_count as usize]
                .iter()
                .map(|&b| JitKind::from_byte(b))
                .collect();
            jit.sigs.insert(
                rid,
                Sig {
                    param_kinds,
                    returns: body
                        .returns_value
                        .then(|| JitKind::from_byte(body.return_kind)),
                },
            );
            let instrs = zun_il::decode_all(body.code).map_err(|e| JitError::Compile {
                method_rid: rid,
                detail: e.to_string(),
            })?;
            let mut callees = Vec::new();
            for instr in instrs
                .iter()
                .filter(|i| matches!(i.opcode, zun_il::Opcode::Call | zun_il::Opcode::CallVirt))
            {
                let zun_il::Operand::Token(token) = instr.operand else {
                    continue;
                };
                if instr.opcode == zun_il::Opcode::CallVirt {
                    let name = zm.tables.method_defs[token.rid() as usize - 1].name;
                    callees.extend(
                        methods_by_name
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .copied()
                            .filter(|rid| eligible.contains(rid)),
                    );
                } else if eligible.contains(&token.rid()) {
                    callees.push(token.rid());
                }
            }
            jit.callees.insert(rid, callees);
        }
        Ok(jit)
    }

    /// Eagerly compile every eligible method. Used where all compiled code is
    /// wanted up front (e.g. equivalence tests).
    pub fn compile(zm: &Module) -> Result<Self, JitError> {
        let mut jit = Self::new(zm)?;
        let all: Vec<u32> = jit.eligible.iter().copied().collect();
        for rid in all {
            jit.ensure_compiled(zm, rid)?;
        }
        Ok(jit)
    }

    /// Ensure `rid` and every eligible method reachable from it are defined and
    /// finalized, so calling `rid` cannot jump into an undefined function.
    /// Returns whether `rid` is JIT-executable (i.e. eligible).
    pub fn ensure_compiled(&mut self, zm: &Module, rid: u32) -> Result<bool, JitError> {
        if !self.eligible.contains(&rid) {
            return Ok(false);
        }
        if self.defined.contains(&rid) {
            return Ok(true);
        }
        // Collect the transitive eligible-callee closure that still needs
        // defining, then define and finalize it as one batch.
        let mut worklist = vec![rid];
        let mut batch = Vec::new();
        let mut seen = HashSet::new();
        while let Some(m) = worklist.pop() {
            if !self.defined.contains(&m) && seen.insert(m) {
                batch.push(m);
                worklist.extend(self.callees[&m].iter().copied());
            }
        }
        for &m in &batch {
            self.define_method(zm, m)?;
        }
        self.module
            .finalize_definitions()
            .map_err(|e| JitError::Module(e.to_string()))?;
        for m in batch {
            self.defined.insert(m);
        }
        self.refresh_stack_map_pcs();
        self.refresh_virtual_table();
        Ok(true)
    }

    /// Translate and define one method body and its shim (no finalize).
    fn define_method(&mut self, zm: &Module, rid: u32) -> Result<(), JitError> {
        let body = read_body(
            &zm.code,
            zm.tables.method_defs[rid as usize - 1].body_offset,
        )
        .map_err(|e| JitError::Compile {
            method_rid: rid,
            detail: e.to_string(),
        })?;
        self.ctx.func.signature = body_signature(&self.module, &body);
        if let Err(detail) = translate::translate_body(
            &mut self.module,
            zm,
            &mut self.ctx,
            &mut self.fb_ctx,
            &self.func_ids,
            &self.types,
            self.virtual_table_ptrs.as_ptr(),
            self.literal_ptrs.as_ptr(),
            core::ptr::null(),
            &body,
            self.bridge_enabled,
            self.bridge_pending_addr,
        ) {
            // Translation failed partway, so `FunctionBuilder::finalize` never
            // ran: the builder context is left dirty and `ctx.func` holds a
            // half-built body. Reset both before returning, or the next
            // method's `FunctionBuilder::new` panics on `func_ctx.is_empty()`
            // and `define_function` would see stale IR. The method simply stays
            // on the interpreter (the tiering fallback).
            self.fb_ctx = FunctionBuilderContext::new();
            self.module.clear_context(&mut self.ctx);
            return Err(JitError::Compile {
                method_rid: rid,
                detail,
            });
        }
        self.module
            .define_function(self.func_ids[&rid], &mut self.ctx)
            .map_err(|e| JitError::Compile {
                method_rid: rid,
                detail: e.to_string(),
            })?;
        let maps = self
            .ctx
            .compiled_code()
            .map(|code| {
                code.buffer
                    .user_stack_maps()
                    .iter()
                    .map(|&(code_offset, span, ref stack_map)| StackMapInfo {
                        code_offset,
                        span,
                        pc: 0,
                        reference_slots: stack_map.entries().map(|(_, offset)| offset).collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.stack_maps.insert(rid, maps);
        self.module.clear_context(&mut self.ctx);

        let sig = self.sigs[&rid].clone();
        let shim_id = if sig.returns == Some(JitKind::Other) {
            shim::define_any_return_shim(
                &mut self.module,
                &mut self.ctx,
                &mut self.fb_ctx,
                &format!("m{rid}_any_shim"),
                self.func_ids[&rid],
                &sig.param_kinds,
            )
        } else {
            shim::define_shim(
                &mut self.module,
                &mut self.ctx,
                &mut self.fb_ctx,
                &format!("m{rid}_shim"),
                self.func_ids[&rid],
                &sig.param_kinds,
                sig.returns,
            )
        }
        .map_err(|detail| JitError::Compile {
            method_rid: rid,
            detail,
        })?;
        self.shim_ids.insert(rid, shim_id);
        Ok(())
    }

    fn refresh_virtual_table(&mut self) {
        for type_rid in 1..=self.types.len() as u32 {
            let Some(ty) = self.types.get(type_rid) else {
                continue;
            };
            for (slot, &method_rid) in ty.vtable.iter().enumerate() {
                if method_rid == 0 {
                    continue;
                }
                let Some(&func_id) = self.func_ids.get(&method_rid) else {
                    continue;
                };
                if !self.defined.contains(&method_rid) {
                    continue;
                }
                let ptr = self.module.get_finalized_function(func_id);
                self.virtual_tables[type_rid as usize - 1][slot] = ptr as usize as u64;
            }
        }
    }

    fn refresh_stack_map_pcs(&mut self) {
        for (&rid, maps) in &mut self.stack_maps {
            let Some(&func_id) = self.func_ids.get(&rid) else {
                continue;
            };
            let base = self.module.get_finalized_function(func_id) as usize;
            for map in maps {
                map.pc = base + map.code_offset as usize;
            }
        }
        self.stack_map_registry
            .replace(self.stack_maps.values().flat_map(|maps| {
                maps.iter().map(|map| zun_gc::NativeStackMap {
                    pc: map.pc,
                    span: map.span,
                    reference_slots: map.reference_slots.clone(),
                })
            }));
    }

    /// Whether `method_rid` is eligible for JIT compilation.
    pub fn is_compiled(&self, method_rid: u32) -> bool {
        self.eligible.contains(&method_rid)
    }

    /// Whether `method_rid`'s body has actually been compiled and finalized.
    pub fn is_defined(&self, method_rid: u32) -> bool {
        self.defined.contains(&method_rid)
    }

    /// Return the native safepoint maps emitted for a method, if it is JIT-defined.
    pub fn stack_maps(&self, method_rid: u32) -> Option<&[StackMapInfo]> {
        self.stack_maps.get(&method_rid).map(Vec::as_slice)
    }

    /// Access the GC-facing registry of absolute native safepoint maps.
    pub fn stack_map_registry(&self) -> &zun_gc::StackMapRegistry {
        &self.stack_map_registry
    }

    /// The parameter kinds of a compiled method (for marshalling arguments).
    pub fn param_kinds(&self, method_rid: u32) -> Option<&[JitKind]> {
        self.sigs.get(&method_rid).map(|s| s.param_kinds.as_slice())
    }

    /// The return kind of a compiled method, or `None` for void / unknown.
    pub fn return_kind(&self, method_rid: u32) -> Option<JitKind> {
        self.sigs.get(&method_rid).and_then(|s| s.returns)
    }

    /// Call a compiled method through its shim with arguments already packed
    /// into `u64` slots (ints as their i64 bits, floats as float bits), and
    /// return the packed result. zun_vm marshals `Value`s to and from this.
    ///
    /// # Safety contract
    /// The module was verified before compilation, and `args` must match the
    /// method's parameter count and kinds. Object arguments must be valid
    /// managed pointers for the current GC heap.
    pub fn call_packed(&self, method_rid: u32, args: &[u64]) -> Result<u64, JitError> {
        let signature = self
            .sigs
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?
            .clone();
        if signature.returns == Some(JitKind::Other) {
            return Err(JitError::Ineligible(method_rid));
        }
        if signature.param_kinds.contains(&JitKind::Other) {
            return Err(JitError::Ineligible(method_rid));
        }
        let param_kinds = signature.param_kinds;
        if param_kinds.len() != args.len() {
            return Err(JitError::Arity {
                expected: param_kinds.len(),
                found: args.len(),
            });
        }
        let native_roots: Vec<zun_gc::Obj> = args
            .iter()
            .zip(param_kinds)
            .filter(|&(&_arg, kind)| kind == JitKind::Object)
            .map(|(&arg, _kind)| unsafe { zun_gc::Obj::from_addr(arg as *mut u64) })
            .collect();
        // Keep managed arguments visible for the complete native call. The VM
        // hook also registers its Value-level roots; this scope protects direct
        // callers of the public packed entry point as well.
        let _roots = unsafe { zun_gc::NativeRootScope::enter(&native_roots) };
        let _stack_maps = unsafe { zun_gc::ActiveStackMapScope::enter(&self.stack_map_registry) };
        let id = *self
            .shim_ids
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?;
        let ptr = self.module.get_finalized_function(id);
        let f: extern "C" fn(*const u64) -> u64 = unsafe { std::mem::transmute(ptr) };
        Ok(f(args.as_ptr()))
    }

    /// Call a scalar-returning method whose parameters may include two-word
    /// `Any` carriers. This is the mixed counterpart to `call_packed_any`.
    pub fn call_packed_with_any_args(
        &self,
        method_rid: u32,
        args: &[u64],
    ) -> Result<u64, JitError> {
        let signature = self
            .sigs
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?
            .clone();
        if signature.returns == Some(JitKind::Other) {
            return Err(JitError::Ineligible(method_rid));
        }
        let expected_words: usize = signature
            .param_kinds
            .iter()
            .map(|&kind| any_abi::abi_word_count(kind))
            .sum();
        if expected_words != args.len() {
            return Err(JitError::Arity {
                expected: expected_words,
                found: args.len(),
            });
        }
        let mut native_roots = Vec::new();
        let mut cursor = 0;
        for &kind in &signature.param_kinds {
            if kind == JitKind::Object {
                native_roots.push(unsafe { zun_gc::Obj::from_addr(args[cursor] as *mut u64) });
            } else if kind == JitKind::Other {
                if let Some(value) =
                    zun_core::Any::from_raw_parts(args[cursor] as u8, args[cursor + 1])
                {
                    if let Some(object) = value.as_object() {
                        native_roots.push(unsafe { zun_gc::Obj::from_addr(object as *mut u64) });
                    }
                }
            }
            cursor += any_abi::abi_word_count(kind);
        }
        let _roots = unsafe { zun_gc::NativeRootScope::enter(&native_roots) };
        let _stack_maps = unsafe { zun_gc::ActiveStackMapScope::enter(&self.stack_map_registry) };
        let id = *self
            .shim_ids
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?;
        let ptr = self.module.get_finalized_function(id);
        let f: extern "C" fn(*const u64) -> u64 = unsafe { std::mem::transmute(ptr) };
        Ok(f(args.as_ptr()))
    }

    /// Call a compiled method using the explicit two-word `Any` return ABI.
    ///
    /// This is intentionally separate from [`Self::call_packed`], whose
    /// scalar return contract is exactly one `u64`. `Any` methods remain
    /// outside the eligibility set until their parameters and internal calls
    /// are lowered to the same carrier ABI.
    pub fn call_packed_any(&self, method_rid: u32, args: &[u64]) -> Result<JitAnyAbi, JitError> {
        let signature = self
            .sigs
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?
            .clone();
        if signature.returns != Some(JitKind::Other) {
            return Err(JitError::Ineligible(method_rid));
        }
        let expected_words: usize = signature
            .param_kinds
            .iter()
            .map(|&kind| any_abi::abi_word_count(kind))
            .sum();
        if expected_words != args.len() {
            return Err(JitError::Arity {
                expected: expected_words,
                found: args.len(),
            });
        }
        let mut native_roots = Vec::new();
        let mut word = 0;
        for kind in signature.param_kinds {
            if kind == JitKind::Object {
                native_roots.push(unsafe { zun_gc::Obj::from_addr(args[word] as *mut u64) });
            } else if kind == JitKind::Other {
                let tag = args[word];
                let payload = args[word + 1];
                let Some(value) = zun_core::Any::from_raw_parts(tag as u8, payload) else {
                    return Err(JitError::InvalidAnyTag(tag));
                };
                if value.tag() == zun_core::AnyTag::Object {
                    native_roots.push(unsafe { zun_gc::Obj::from_addr(payload as *mut u64) });
                }
            }
            word += any_abi::abi_word_count(kind);
        }
        let _roots = unsafe { zun_gc::NativeRootScope::enter(&native_roots) };
        let _stack_maps = unsafe { zun_gc::ActiveStackMapScope::enter(&self.stack_map_registry) };
        let id = *self
            .shim_ids
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?;
        let ptr = self.module.get_finalized_function(id);
        let f: extern "C" fn(*const u64, *mut JitAnyAbi) -> i32 =
            unsafe { std::mem::transmute(ptr) };
        let mut result = JitAnyAbi { tag: 0, payload: 0 };
        if f(args.as_ptr(), &mut result) == 0 {
            return Err(JitError::InvalidAnyTag(result.tag));
        }
        Ok(result)
    }

    pub fn call_packed_any_with_heap(
        &self,
        method_rid: u32,
        args: &[u64],
        heap: &mut zun_gc::Heap,
    ) -> Result<JitAnyAbi, JitError> {
        self.register_heap_layouts(heap);
        let _scope = unsafe { zun_gc::HeapScope::enter(heap as *mut _) };
        self.call_packed_any(method_rid, args)
    }

    /// Call a compiled method while registering `heap` as the current managed
    /// heap for runtime allocation and helper calls.
    ///
    /// This is the explicit host boundary for JIT code that can allocate or
    /// otherwise access managed storage. The scope is restored before return,
    /// so nested hosts keep ownership of their previous heap registration.
    pub fn call_packed_with_heap(
        &self,
        method_rid: u32,
        args: &[u64],
        heap: &mut zun_gc::Heap,
    ) -> Result<u64, JitError> {
        self.register_heap_layouts(heap);
        let _scope = unsafe { zun_gc::HeapScope::enter(heap as *mut _) };
        self.call_packed(method_rid, args)
    }

    /// Call a compiled method while publishing the caller-supplied native
    /// safepoint coordinates to the GC boundary.
    ///
    /// The host must provide a stopped frame stack pointer and a program
    /// counter covered by [`Self::stack_map_registry`]. This is the explicit
    /// coordinator path; automatic machine-context discovery remains a host
    /// responsibility.
    ///
    /// # Safety
    /// `pc` and `sp` must identify a live native frame whose stack memory stays
    /// valid for the complete call and remains stopped whenever the GC scans
    /// it. The packed arguments must satisfy the same contract as
    /// [`Self::call_packed`].
    pub unsafe fn call_packed_with_heap_and_native_frame(
        &self,
        method_rid: u32,
        args: &[u64],
        heap: &mut zun_gc::Heap,
        pc: usize,
        sp: *mut u8,
    ) -> Result<u64, JitError> {
        let _frame =
            unsafe { zun_gc::ActiveNativeFrameScope::enter(self.stack_map_registry(), pc, sp) };
        let _scope = unsafe { zun_gc::HeapScope::enter(heap as *mut _) };
        self.call_packed(method_rid, args)
    }

    /// Call a compiled method while publishing caller-supplied native
    /// safepoint coordinates, assuming the caller already registered the
    /// managed heap with [`zun_gc::HeapScope`].
    ///
    /// # Safety
    /// The frame coordinates and packed arguments must satisfy the same
    /// contract as [`Self::call_packed_with_heap_and_native_frame`], and a
    /// current heap must already be installed for any runtime helper calls.
    pub unsafe fn call_packed_with_native_frame(
        &self,
        method_rid: u32,
        args: &[u64],
        pc: usize,
        sp: *mut u8,
    ) -> Result<u64, JitError> {
        let _frame =
            unsafe { zun_gc::ActiveNativeFrameScope::enter(self.stack_map_registry(), pc, sp) };
        self.call_packed(method_rid, args)
    }

    /// Call a compiled zero-argument method (an entry point), returning its
    /// result tagged by kind.
    ///
    /// Only nullary methods are callable from Rust for now; methods that take
    /// arguments are still reached through Cranelift-to-Cranelift calls inside
    /// compiled code. The compiled code is trusted because the module was
    /// verified before compilation.
    pub fn call_entry(&self, method_rid: u32) -> Result<RetVal, JitError> {
        let sig = self
            .sigs
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?
            .clone();
        if !sig.param_kinds.is_empty() {
            return Err(JitError::Arity {
                expected: 0,
                found: sig.param_kinds.len(),
            });
        }
        let id = self.func_ids[&method_rid];
        let ptr = self.module.get_finalized_function(id);

        // Publish the finalized stack-map registry for the complete native
        // entry call. This keeps direct JIT entry points on the same precise
        // GC boundary as packed calls when a runtime safepoint is reached.
        let _stack_maps = unsafe { zun_gc::ActiveStackMapScope::enter(&self.stack_map_registry) };

        // Transmute the entry point to a nullary fn of its concrete return type,
        // then box the native result into a kind-tagged RetVal.
        use zun_il::NumKind::*;
        unsafe {
            Ok(match sig.returns {
                None => {
                    let f: extern "C" fn() = std::mem::transmute(ptr);
                    f();
                    RetVal::Void
                }
                Some(JitKind::Bool) => {
                    let f: extern "C" fn() -> i32 = std::mem::transmute(ptr);
                    RetVal::Bool(f() != 0)
                }
                Some(JitKind::Num(F64)) => {
                    let f: extern "C" fn() -> f64 = std::mem::transmute(ptr);
                    RetVal::Float(f())
                }
                Some(JitKind::Num(F32)) => {
                    let f: extern "C" fn() -> f32 = std::mem::transmute(ptr);
                    RetVal::Float(f() as f64)
                }
                Some(JitKind::Num(k)) => {
                    let (bits, signed) = k.int_parts().unwrap();
                    RetVal::Int(read_int(ptr, bits, signed))
                }
                Some(JitKind::Object) | Some(JitKind::String) => {
                    let f: extern "C" fn() -> i64 = std::mem::transmute(ptr);
                    RetVal::Int(f())
                }
                Some(JitKind::Other) => return Err(JitError::Ineligible(method_rid)),
                Some(JitKind::Array) => return Err(JitError::Ineligible(method_rid)),
                Some(JitKind::Function) => return Err(JitError::Ineligible(method_rid)),
            })
        }
    }

    /// Call a zero-argument entry point with an explicitly registered managed
    /// heap. This is the entry-point counterpart to [`Self::call_packed_with_heap`].
    pub fn call_entry_with_heap(
        &self,
        method_rid: u32,
        heap: &mut zun_gc::Heap,
    ) -> Result<RetVal, JitError> {
        self.register_heap_layouts(heap);
        let _scope = unsafe { zun_gc::HeapScope::enter(heap as *mut _) };
        self.call_entry(method_rid)
    }

    /// Call a zero-argument entry point while publishing caller-supplied
    /// native safepoint coordinates to the GC boundary.
    ///
    /// # Safety
    /// `pc` and `sp` must identify a live frame covered by this JIT's stack-map
    /// registry, and that frame must remain valid for the complete call.
    pub unsafe fn call_entry_with_heap_and_native_frame(
        &self,
        method_rid: u32,
        heap: &mut zun_gc::Heap,
        pc: usize,
        sp: *mut u8,
    ) -> Result<RetVal, JitError> {
        let _frame =
            unsafe { zun_gc::ActiveNativeFrameScope::enter(self.stack_map_registry(), pc, sp) };
        let _scope = unsafe { zun_gc::HeapScope::enter(heap as *mut _) };
        self.call_entry(method_rid)
    }
}

/// Emit the JIT-eligible portion of a ZIL module as a relocatable native
/// object. The `.zm` module remains the fallback for methods that are not in
/// the typed Cranelift subset; linking and runtime entry-point selection are
/// deliberately left to the host build step.
pub fn compile_object(zm: &Module) -> Result<Vec<u8>, JitError> {
    compile_object_for_target(zm, target_lexicon::Triple::host())
}

/// Emit a relocatable object for an explicitly selected target ISA.
///
/// The host wrapper above preserves the existing AOT behavior. Cross-target
/// callers must still provide a matching runtime library and linker inputs;
/// this API only selects the Cranelift object ISA.
pub fn compile_object_for_target(
    zm: &Module,
    target: target_lexicon::Triple,
) -> Result<Vec<u8>, JitError> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    flags.set("preserve_frame_pointers", "true").unwrap();
    let isa = cranelift_codegen::isa::lookup(target)
        .map_err(|e| JitError::UnsupportedHost(e.to_string()))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| JitError::UnsupportedHost(e.to_string()))?;
    let builder = ObjectBuilder::new(isa, "zun-aot", cranelift_module::default_libcall_names())
        .map_err(|e| JitError::Module(e.to_string()))?;
    let mut module = ObjectModule::new(builder);
    let eligible = eligible_methods_with_literals(zm, true);
    if let Some(entry) = zm.entry_point {
        let rid = entry.rid();
        if !eligible.contains(&rid) {
            return Err(JitError::Ineligible(rid));
        }
    }
    let types =
        zun_types::TypeSystem::from_module(zm).map_err(|e| JitError::Module(e.to_string()))?;
    let mut literal_data = Vec::<(DataId, DataDescription)>::new();
    let mut literal_data_ids = vec![0u32; zm.strings.as_bytes().len()];
    let mut string_offset = 0usize;
    while string_offset < zm.strings.as_bytes().len() {
        let value = zm
            .strings
            .get(zun_metadata::StringIndex(string_offset as u32));
        let data_id = module
            .declare_anonymous_data(false, false)
            .map_err(|e| JitError::Module(e.to_string()))?;
        let mut description = DataDescription::new();
        description.define(value.as_bytes().to_vec().into_boxed_slice());
        description.set_used(true);
        module
            .define_data(data_id, &description)
            .map_err(|e| JitError::Module(e.to_string()))?;
        literal_data_ids[string_offset] = data_id.as_u32();
        literal_data.push((data_id, description));
        string_offset = zm.strings.next_offset(string_offset);
    }
    let mut func_ids = HashMap::new();
    let mut ctx = cranelift_codegen::Context::new();
    let mut fb_ctx = FunctionBuilderContext::new();

    for &rid in &eligible {
        let body = read_body(
            &zm.code,
            zm.tables.method_defs[rid as usize - 1].body_offset,
        )
        .map_err(|e| JitError::Compile {
            method_rid: rid,
            detail: e.to_string(),
        })?;
        let sig = body_signature(&module, &body);
        let linkage = if zm.entry_point.is_some_and(|token| token.rid() == rid) {
            Linkage::Export
        } else {
            Linkage::Local
        };
        let fid = module
            .declare_function(&format!("m{rid}"), linkage, &sig)
            .map_err(|e| JitError::Module(e.to_string()))?;
        func_ids.insert(rid, fid);
    }

    for &rid in &eligible {
        let body = read_body(
            &zm.code,
            zm.tables.method_defs[rid as usize - 1].body_offset,
        )
        .map_err(|e| JitError::Compile {
            method_rid: rid,
            detail: e.to_string(),
        })?;
        ctx.func.signature = body_signature(&module, &body);
        translate::translate_body(
            &mut module,
            zm,
            &mut ctx,
            &mut fb_ctx,
            &func_ids,
            &types,
            core::ptr::null(),
            core::ptr::null(),
            literal_data_ids.as_ptr(),
            &body,
            false,
            0,
        )
        .map_err(|detail| JitError::Compile {
            method_rid: rid,
            detail,
        })?;
        module
            .define_function(func_ids[&rid], &mut ctx)
            .map_err(|e| JitError::Compile {
                method_rid: rid,
                detail: e.to_string(),
            })?;
    }
    module
        .finish()
        .emit()
        .map_err(|e| JitError::Module(e.to_string()))
}

/// Emit the serialized `.zm` payload as a read-only data object with a stable
/// exported symbol. The AOT linker can include this object alongside native
/// code so the fallback interpreter has the original IL available at runtime.
pub fn compile_metadata_object(module_bytes: &[u8]) -> Result<Vec<u8>, JitError> {
    compile_metadata_object_for_target(module_bytes, target_lexicon::Triple::host())
}

/// Emit the embedded module data object for an explicitly selected target.
pub fn compile_metadata_object_for_target(
    module_bytes: &[u8],
    target: target_lexicon::Triple,
) -> Result<Vec<u8>, JitError> {
    use object::write::{Object, Symbol, SymbolSection};
    use object::{
        Architecture, BinaryFormat, Endianness, SectionKind, SymbolFlags, SymbolKind, SymbolScope,
    };

    let format = match target.binary_format {
        target_lexicon::BinaryFormat::Coff => BinaryFormat::Coff,
        target_lexicon::BinaryFormat::Macho => BinaryFormat::MachO,
        target_lexicon::BinaryFormat::Elf => BinaryFormat::Elf,
        _ => {
            return Err(JitError::UnsupportedHost(format!(
                "unsupported metadata object format for target {target}"
            )))
        }
    };
    let architecture = match target.architecture {
        target_lexicon::Architecture::X86_64 => Architecture::X86_64,
        target_lexicon::Architecture::Aarch64(_) => Architecture::Aarch64,
        _ => {
            return Err(JitError::UnsupportedHost(format!(
                "unsupported metadata object architecture for target {target}"
            )))
        }
    };
    let mut object = Object::new(format, architecture, Endianness::Little);
    let section = object.add_section(Vec::new(), b".zun_meta".to_vec(), SectionKind::ReadOnlyData);
    let symbol = object.add_symbol(Symbol {
        name: b"zun_embedded_module".to_vec(),
        value: 0,
        size: module_bytes.len() as u64,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: SymbolSection::Section(section),
        flags: SymbolFlags::None,
    });
    object.add_symbol_data(symbol, section, module_bytes, 1);
    let length_symbol = object.add_symbol(Symbol {
        name: b"zun_embedded_module_len".to_vec(),
        value: 0,
        size: std::mem::size_of::<u64>() as u64,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: SymbolSection::Section(section),
        flags: SymbolFlags::None,
    });
    object.add_symbol_data(
        length_symbol,
        section,
        &(module_bytes.len() as u64).to_le_bytes(),
        8,
    );
    object.write().map_err(|e| JitError::Module(e.to_string()))
}

/// Read a nullary integer-returning entry point, sign/zero-extending to i64.
///
/// # Safety
/// `ptr` must be a finalized nullary function returning an integer of the given
/// width and signedness.
#[allow(clippy::missing_transmute_annotations)] // the ptr source type is `*const u8` in every arm
unsafe fn read_int(ptr: *const u8, bits: u8, signed: bool) -> i64 {
    unsafe {
        match bits {
            8 if signed => (std::mem::transmute::<_, extern "C" fn() -> i8>(ptr))() as i64,
            8 => (std::mem::transmute::<_, extern "C" fn() -> u8>(ptr))() as i64,
            16 if signed => (std::mem::transmute::<_, extern "C" fn() -> i16>(ptr))() as i64,
            16 => (std::mem::transmute::<_, extern "C" fn() -> u16>(ptr))() as i64,
            32 if signed => (std::mem::transmute::<_, extern "C" fn() -> i32>(ptr))() as i64,
            32 => (std::mem::transmute::<_, extern "C" fn() -> u32>(ptr))() as i64,
            _ if signed => (std::mem::transmute::<_, extern "C" fn() -> i64>(ptr))(),
            _ => (std::mem::transmute::<_, extern "C" fn() -> u64>(ptr))() as i64,
        }
    }
}

/// Build a method's Cranelift signature from its recorded value-type codes: one
/// parameter per leading local kind, and a result when it returns a value.
///
/// It uses the host's native (C) calling convention so entry points are callable
/// through `extern "C"` function pointers and calls between compiled methods
/// match. Switching managed calls to `CallConv::Tail` is a later refinement.
fn body_signature<M: ClifModule>(
    module: &M,
    body: &zun_module::MethodBody<'_>,
) -> cranelift_codegen::ir::Signature {
    let mut sig = cranelift_codegen::ir::Signature::new(module.isa().default_call_conv());
    for &k in &body.local_kinds[..body.param_count as usize] {
        let kind = JitKind::from_byte(k);
        if kind == JitKind::Other {
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
        } else {
            sig.params.push(AbiParam::new(clif_ty(kind)));
        }
    }
    if body.returns_value {
        let return_kind = JitKind::from_byte(body.return_kind);
        if return_kind == JitKind::Other {
            // `Any` is a stable two-word carrier: tag followed by payload.
            sig.returns.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
        } else {
            sig.returns.push(AbiParam::new(clif_ty(return_kind)));
        }
    }
    sig
}

#[cfg(test)]
mod tests {
    use super::{
        compile_metadata_object, zun_jit_array_load_any, zun_jit_array_load_object,
        zun_jit_array_store_any, zun_jit_array_store_object, zun_jit_map_size, zun_jit_set_size,
        zun_jit_store_ref, JitAnyAbi,
    };
    use object::{Object as _, ObjectSection as _, ObjectSymbol as _};

    #[test]
    fn metadata_object_exports_the_embedded_module_symbol() {
        let payload = b"ZUN-IL";
        let object = compile_metadata_object(payload).expect("metadata object");
        let file = object::File::parse(object.as_slice()).expect("parse metadata object");
        let _symbol = file
            .symbols()
            .find(|symbol| symbol.name().ok() == Some("zun_embedded_module"))
            .expect("embedded module symbol");
        assert!(file
            .symbols()
            .any(|symbol| symbol.name().ok() == Some("zun_embedded_module_len")));
        assert!(file.sections().any(|section| section
            .data()
            .ok()
            .is_some_and(|data| data.starts_with(payload))));
    }

    #[test]
    fn dynamic_array_load_helper_preserves_both_abi_words() {
        let mut heap = zun_gc::Heap::new();
        let array = heap.alloc_array(1);
        heap.array_set(array, 0, zun_core::Any::object(0x1234));
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap) };
        let mut carrier = JitAnyAbi { tag: 5, payload: 0 };
        let loaded = unsafe { zun_jit_array_load_any(array.addr() as i64, 0, &mut carrier) };
        assert_eq!(loaded, 1);
        assert_eq!(
            carrier,
            JitAnyAbi {
                tag: 3,
                payload: 0x1234
            }
        );
        assert_eq!(
            unsafe { zun_jit_array_load_any(array.addr() as i64, -1, &mut carrier) },
            0
        );
        assert_eq!(
            unsafe { zun_jit_array_load_any(array.addr() as i64, 0, std::ptr::null_mut()) },
            0
        );
    }

    #[test]
    fn dynamic_array_store_helper_validates_and_preserves_both_abi_words() {
        let mut heap = zun_gc::Heap::new();
        let array = heap.alloc_array(1);
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        assert_eq!(
            zun_jit_array_store_any(array.addr() as i64, 0, 3, 0x1234),
            1
        );
        assert_eq!(
            heap.array_get(array, 0),
            Some(zun_core::Any::object(0x1234))
        );
        assert_eq!(zun_jit_array_store_any(array.addr() as i64, 0, 255, 0), 0);
        assert_eq!(zun_jit_array_store_any(array.addr() as i64, -1, 0, 0), 0);
    }

    #[test]
    fn dynamic_array_store_helper_keeps_object_payload_traced() {
        let mut heap = zun_gc::Heap::new();
        let child = heap.alloc(1, 1);
        let array = heap.alloc_array(1);
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        assert_eq!(
            zun_jit_array_store_any(array.addr() as i64, 0, 3, child.addr() as u64),
            1
        );
        assert_eq!(unsafe { heap.collect(&[array], &|_| Vec::new()) }, 0);
        assert_eq!(
            heap.array_get(array, 0).and_then(|value| value.as_object()),
            Some(child.addr() as usize)
        );
        assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 2);
    }

    #[test]
    fn object_array_helpers_round_trip_managed_pointer() {
        let mut heap = zun_gc::Heap::new();
        let array = heap.alloc_array(1);
        let child = heap.alloc(1, 0);
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };

        assert_eq!(
            zun_jit_array_store_object(array.addr() as i64, 0, child.addr() as i64),
            1
        );
        assert_eq!(
            zun_jit_array_load_object(array.addr() as i64, 0),
            child.addr() as i64
        );
    }

    #[test]
    fn collection_size_helpers_read_managed_lengths() {
        let mut heap = zun_gc::Heap::new();
        let map = heap.alloc_map();
        let set = heap.alloc_set();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        assert_eq!(zun_jit_map_size(map.addr() as i64), 0);
        assert_eq!(zun_jit_set_size(set.addr() as i64), 0);
        heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::int(2));
        heap.set_add(set, zun_core::Any::int(3));
        assert_eq!(zun_jit_map_size(map.addr() as i64), 1);
        assert_eq!(zun_jit_set_size(set.addr() as i64), 1);
    }

    #[test]
    fn jit_reference_store_preserves_young_target_through_minor_collection() {
        let mut heap = zun_gc::Heap::new();
        let owner = heap.alloc(1, 1);
        unsafe { heap.collect(&[owner], &|_| vec![0]) };
        let child = heap.alloc(2, 0);
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };

        assert_eq!(
            zun_jit_store_ref(owner.addr() as i64, 0, child.addr() as i64),
            1
        );
        assert_eq!(unsafe { heap.collect_young(&[owner], &|_| vec![0]) }, 0);
        assert_eq!(owner.field(0), child.addr() as u64);
    }
}
