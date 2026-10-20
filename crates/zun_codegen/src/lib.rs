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

mod eligible;
mod shim;
mod translate;

pub use eligible::eligible_methods;
pub use zun_il::{JitKind, NumKind};

#[inline]
pub extern "C" fn zun_jit_date_year(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_date()
        .unwrap()
        .0
}

/// Read a GC-owned array length without allocating or collecting.
/// Mutating and allocating array operations remain Tier 0 until JIT stack-map
/// and write-barrier support is available.
#[inline]
pub extern "C" fn zun_jit_array_length(address: i64) -> i64 {
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    zun_gc::with_current_heap(|heap| heap.array_len(object).map(|len| len as i64).unwrap_or(-1))
}
#[inline]
pub extern "C" fn zun_jit_date_is_valid(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis).millis().is_some() as i32
}
#[inline]
pub extern "C" fn zun_jit_date_month(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_date()
        .unwrap()
        .1 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_day(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_date()
        .unwrap()
        .2 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_hour(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_time()
        .unwrap()
        .0 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_minute(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_time()
        .unwrap()
        .1 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_second(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_time()
        .unwrap()
        .2 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_millisecond(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis)
        .utc_time()
        .unwrap()
        .3 as i32
}
#[inline]
pub extern "C" fn zun_jit_date_weekday(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis).weekday().unwrap() as i32
}
#[inline]
pub extern "C" fn zun_jit_date_day_of_year(millis: i64) -> i32 {
    zun_core::DateValue::from_millis(millis).ordinal().unwrap() as i32
}

#[inline]
pub extern "C" fn zun_jit_math_abs(value: f64) -> f64 {
    value.abs()
}
#[inline]
pub extern "C" fn zun_jit_math_sqrt(value: f64) -> f64 {
    value.sqrt()
}
#[inline]
pub extern "C" fn zun_jit_math_cbrt(value: f64) -> f64 {
    value.cbrt()
}
#[inline]
pub extern "C" fn zun_jit_math_sign(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        value
    } else if value.is_sign_positive() {
        1.0
    } else {
        -1.0
    }
}
#[inline]
pub extern "C" fn zun_jit_math_floor(value: f64) -> f64 {
    value.floor()
}
#[inline]
pub extern "C" fn zun_jit_math_ceil(value: f64) -> f64 {
    value.ceil()
}
#[inline]
pub extern "C" fn zun_jit_math_round(value: f64) -> f64 {
    value.round()
}
#[inline]
pub extern "C" fn zun_jit_math_trunc(value: f64) -> f64 {
    value.trunc()
}
#[inline]
pub extern "C" fn zun_jit_math_sin(value: f64) -> f64 {
    value.sin()
}
#[inline]
pub extern "C" fn zun_jit_math_cos(value: f64) -> f64 {
    value.cos()
}
#[inline]
pub extern "C" fn zun_jit_math_exp(value: f64) -> f64 {
    value.exp()
}
#[inline]
pub extern "C" fn zun_jit_math_log(value: f64) -> f64 {
    value.ln()
}
#[inline]
pub extern "C" fn zun_jit_math_log10(value: f64) -> f64 {
    value.log10()
}
#[inline]
pub extern "C" fn zun_jit_math_log2(value: f64) -> f64 {
    value.log2()
}
#[inline]
pub extern "C" fn zun_jit_math_expm1(value: f64) -> f64 {
    value.exp_m1()
}
#[inline]
pub extern "C" fn zun_jit_math_log1p(value: f64) -> f64 {
    value.ln_1p()
}
#[inline]
pub extern "C" fn zun_jit_math_sinh(value: f64) -> f64 {
    value.sinh()
}
#[inline]
pub extern "C" fn zun_jit_math_cosh(value: f64) -> f64 {
    value.cosh()
}
#[inline]
pub extern "C" fn zun_jit_math_tanh(value: f64) -> f64 {
    value.tanh()
}
#[inline]
pub extern "C" fn zun_jit_math_asinh(value: f64) -> f64 {
    value.asinh()
}
#[inline]
pub extern "C" fn zun_jit_math_acosh(value: f64) -> f64 {
    value.acosh()
}
#[inline]
pub extern "C" fn zun_jit_math_atanh(value: f64) -> f64 {
    value.atanh()
}
#[inline]
pub extern "C" fn zun_jit_math_tan(value: f64) -> f64 {
    value.tan()
}
#[inline]
pub extern "C" fn zun_jit_math_min(left: f64, right: f64) -> f64 {
    left.min(right)
}
#[inline]
pub extern "C" fn zun_jit_math_max(left: f64, right: f64) -> f64 {
    left.max(right)
}
#[inline]
pub extern "C" fn zun_jit_math_pow(left: f64, right: f64) -> f64 {
    left.powf(right)
}
#[inline]
pub extern "C" fn zun_jit_math_atan2(left: f64, right: f64) -> f64 {
    left.atan2(right)
}
#[inline]
pub extern "C" fn zun_jit_math_hypot(left: f64, right: f64) -> f64 {
    left.hypot(right)
}
#[inline]
pub extern "C" fn zun_jit_number_is_finite(value: f64) -> i32 {
    value.is_finite() as i32
}
#[inline]
pub extern "C" fn zun_jit_number_is_nan(value: f64) -> i32 {
    value.is_nan() as i32
}
#[inline]
pub extern "C" fn zun_jit_number_is_negative_zero(value: f64) -> i32 {
    (value == 0.0 && value.is_sign_negative()) as i32
}
#[inline]
pub extern "C" fn zun_jit_number_is_safe_integer(value: f64) -> i32 {
    (value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0) as i32
}

use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::AbiParam;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::FunctionBuilderContext;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module as ClifModule};
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
    stack_maps: HashMap<u32, Vec<StackMapInfo>>,
    stack_map_registry: zun_gc::StackMapRegistry,
}

impl core::fmt::Debug for Jit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Jit")
            .field("compiled", &self.func_ids.len())
            .finish()
    }
}

impl Jit {
    /// Create a JIT for `zm`: determine eligible methods and declare their
    /// Cranelift functions, without compiling any bodies yet. Bodies are
    /// defined on demand by [`Jit::ensure_compiled`] (lazy tiering) or all at
    /// once by [`Jit::compile`].
    pub fn new(zm: &Module) -> Result<Self, JitError> {
        let mut flags = settings::builder();
        // Optimize for speed; the e-graph optimizer is on by default in this
        // Cranelift version, so there is no flag to set for it.
        flags.set("opt_level", "speed").unwrap();
        let isa = cranelift_native::builder()
            .map_err(|e| JitError::UnsupportedHost(e.to_string()))?
            .finish(settings::Flags::new(flags))
            .map_err(|e| JitError::UnsupportedHost(e.to_string()))?;

        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        // Make the runtime allocator callable from native `newobj`.
        builder.symbol("zun_rt_alloc", zun_gc::zun_rt_alloc as *const u8);
        builder.symbol("zun_jit_array_length", zun_jit_array_length as *const u8);
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
        let module = JITModule::new(builder);

        let eligible = eligible_methods(zm);
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
            stack_maps: HashMap::new(),
            stack_map_registry: zun_gc::StackMapRegistry::new(),
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
        translate::translate_body(
            &mut self.module,
            zm,
            &mut self.ctx,
            &mut self.fb_ctx,
            &self.func_ids,
            &self.types,
            self.virtual_table_ptrs.as_ptr(),
            &body,
        )
        .map_err(|detail| JitError::Compile {
            method_rid: rid,
            detail,
        })?;
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
                        reference_slots: stack_map
                            .entries()
                            .map(|(_, offset)| offset)
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.stack_maps.insert(rid, maps);
        self.module.clear_context(&mut self.ctx);

        let sig = self.sigs[&rid].clone();
        let shim_id = shim::define_shim(
            &mut self.module,
            &mut self.ctx,
            &mut self.fb_ctx,
            &format!("m{rid}_shim"),
            self.func_ids[&rid],
            &sig.param_kinds,
            sig.returns,
        )
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
        let param_kinds = self
            .sigs
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?
            .param_kinds
            .clone();
        if param_kinds.len() != args.len() {
            return Err(JitError::Arity {
                expected: param_kinds.len(),
                found: args.len(),
            });
        }
        let native_roots: Vec<zun_gc::Obj> = args
            .iter()
            .zip(param_kinds)
            .filter_map(|(&arg, kind)| {
                (kind == JitKind::Object)
                    .then(|| unsafe { zun_gc::Obj::from_addr(arg as *mut u64) })
            })
            .collect();
        // Keep managed arguments visible for the complete native call. The VM
        // hook also registers its Value-level roots; this scope protects direct
        // callers of the public packed entry point as well.
        let _roots = unsafe { zun_gc::NativeRootScope::enter(&native_roots) };
        let _stack_maps = unsafe {
            zun_gc::ActiveStackMapScope::enter(&self.stack_map_registry)
        };
        let id = *self
            .shim_ids
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?;
        let ptr = self.module.get_finalized_function(id);
        let f: extern "C" fn(*const u64) -> u64 = unsafe { std::mem::transmute(ptr) };
        Ok(f(args.as_ptr()))
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
                Some(JitKind::Object) => {
                    let f: extern "C" fn() -> i64 = std::mem::transmute(ptr);
                    RetVal::Int(f())
                }
                Some(JitKind::Other) => return Err(JitError::Ineligible(method_rid)),
            })
        }
    }
}

/// Emit the JIT-eligible portion of a ZIL module as a relocatable native
/// object. The `.zm` module remains the fallback for methods that are not in
/// the typed Cranelift subset; linking and runtime entry-point selection are
/// deliberately left to the host build step.
pub fn compile_object(zm: &Module) -> Result<Vec<u8>, JitError> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    let isa = cranelift_native::builder()
        .map_err(|e| JitError::UnsupportedHost(e.to_string()))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| JitError::UnsupportedHost(e.to_string()))?;
    let builder = ObjectBuilder::new(isa, "zun-aot", cranelift_module::default_libcall_names())
        .map_err(|e| JitError::Module(e.to_string()))?;
    let mut module = ObjectModule::new(builder);
    let eligible = eligible_methods(zm);
    let types =
        zun_types::TypeSystem::from_module(zm).map_err(|e| JitError::Module(e.to_string()))?;
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
        let linkage = zm
            .entry_point
            .is_some_and(|token| token.rid() == rid)
            .then_some(Linkage::Export)
            .unwrap_or(Linkage::Local);
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
            &body,
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

/// Read a nullary integer-returning entry point, sign/zero-extending to i64.
///
/// # Safety
/// `ptr` must be a finalized nullary function returning an integer of the given
/// width and signedness.
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
        sig.params
            .push(AbiParam::new(clif_ty(JitKind::from_byte(k))));
    }
    if body.returns_value {
        sig.returns
            .push(AbiParam::new(clif_ty(JitKind::from_byte(body.return_kind))));
    }
    sig
}
