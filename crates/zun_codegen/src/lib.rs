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

use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::AbiParam;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::FunctionBuilderContext;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module as ClifModule};
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
        let module = JITModule::new(builder);

        let eligible = eligible_methods(zm);

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
        };

        for &rid in &eligible {
            let body = read_body(&zm.code, zm.tables.method_defs[rid as usize - 1].body_offset)
                .map_err(|e| JitError::Compile { method_rid: rid, detail: e.to_string() })?;
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
                    returns: body.returns_value.then(|| JitKind::from_byte(body.return_kind)),
                },
            );
            jit.callees.insert(
                rid,
                zun_il::decode_all(body.code)
                    .map_err(|e| JitError::Compile { method_rid: rid, detail: e.to_string() })?
                    .iter()
                    .filter(|i| matches!(i.opcode, zun_il::Opcode::Call | zun_il::Opcode::CallVirt))
                    .filter_map(|i| match i.operand {
                        zun_il::Operand::Token(t) => Some(t.rid()),
                        _ => None,
                    })
                    .filter(|c| eligible.contains(c))
                    .collect(),
            );
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
        Ok(true)
    }

    /// Translate and define one method body and its shim (no finalize).
    fn define_method(&mut self, zm: &Module, rid: u32) -> Result<(), JitError> {
        let body = read_body(&zm.code, zm.tables.method_defs[rid as usize - 1].body_offset)
            .map_err(|e| JitError::Compile { method_rid: rid, detail: e.to_string() })?;
        self.ctx.func.signature = body_signature(&self.module, &body);
        translate::translate_body(
            &mut self.module,
            zm,
            &mut self.ctx,
            &mut self.fb_ctx,
            &self.func_ids,
            &body,
        )
        .map_err(|detail| JitError::Compile { method_rid: rid, detail })?;
        self.module
            .define_function(self.func_ids[&rid], &mut self.ctx)
            .map_err(|e| JitError::Compile { method_rid: rid, detail: e.to_string() })?;
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
        .map_err(|detail| JitError::Compile { method_rid: rid, detail })?;
        self.shim_ids.insert(rid, shim_id);
        Ok(())
    }

    /// Whether `method_rid` is eligible for JIT compilation.
    pub fn is_compiled(&self, method_rid: u32) -> bool {
        self.eligible.contains(&method_rid)
    }

    /// Whether `method_rid`'s body has actually been compiled and finalized.
    pub fn is_defined(&self, method_rid: u32) -> bool {
        self.defined.contains(&method_rid)
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
    /// method's parameter count and kinds.
    pub fn call_packed(&self, method_rid: u32, args: &[u64]) -> Result<u64, JitError> {
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
fn body_signature(
    module: &JITModule,
    body: &zun_module::MethodBody<'_>,
) -> cranelift_codegen::ir::Signature {
    let mut sig = cranelift_codegen::ir::Signature::new(module.isa().default_call_conv());
    for &k in &body.local_kinds[..body.param_count as usize] {
        sig.params.push(AbiParam::new(clif_ty(JitKind::from_byte(k))));
    }
    if body.returns_value {
        sig.returns
            .push(AbiParam::new(clif_ty(JitKind::from_byte(body.return_kind))));
    }
    sig
}
