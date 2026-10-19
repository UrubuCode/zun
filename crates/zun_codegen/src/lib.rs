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
mod translate;

pub use eligible::eligible_methods;

use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::{AbiParam, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::FunctionBuilderContext;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module as ClifModule};
use zun_module::{read_body, Module};

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
pub struct Jit {
    module: JITModule,
    func_ids: HashMap<u32, FuncId>,
    eligible: HashSet<u32>,
}

impl core::fmt::Debug for Jit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Jit")
            .field("compiled", &self.func_ids.len())
            .finish()
    }
}

impl Jit {
    /// Compile every eligible method in `zm` to native code.
    pub fn compile(zm: &Module) -> Result<Self, JitError> {
        let mut flags = settings::builder();
        // Optimize for speed; the e-graph optimizer is on by default in this
        // Cranelift version, so there is no flag to set for it.
        flags.set("opt_level", "speed").unwrap();
        let isa = cranelift_native::builder()
            .map_err(|e| JitError::UnsupportedHost(e.to_string()))?
            .finish(settings::Flags::new(flags))
            .map_err(|e| JitError::UnsupportedHost(e.to_string()))?;

        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);

        let eligible = eligible_methods(zm);

        // Declare every eligible function first so calls can reference any of
        // them regardless of order.
        let mut func_ids = HashMap::new();
        for &rid in &eligible {
            let body = read_body(&zm.code, zm.tables.method_defs[rid as usize - 1].body_offset)
                .map_err(|e| JitError::Compile {
                    method_rid: rid,
                    detail: e.to_string(),
                })?;
            let sig = numeric_signature(&module, body.param_count as usize);
            let name = format!("m{rid}");
            let id = module
                .declare_function(&name, Linkage::Local, &sig)
                .map_err(|e| JitError::Module(e.to_string()))?;
            func_ids.insert(rid, id);
        }

        // Define each function body.
        let mut ctx = module.make_context();
        let mut fb_ctx = FunctionBuilderContext::new();
        for &rid in &eligible {
            let body = read_body(&zm.code, zm.tables.method_defs[rid as usize - 1].body_offset)
                .map_err(|e| JitError::Compile {
                    method_rid: rid,
                    detail: e.to_string(),
                })?;
            ctx.func.signature = numeric_signature(&module, body.param_count as usize);
            translate::translate_body(
                &mut module,
                &mut ctx,
                &mut fb_ctx,
                &func_ids,
                &body,
            )
            .map_err(|detail| JitError::Compile {
                method_rid: rid,
                detail,
            })?;
            let id = func_ids[&rid];
            module
                .define_function(id, &mut ctx)
                .map_err(|e| JitError::Compile {
                    method_rid: rid,
                    detail: e.to_string(),
                })?;
            module.clear_context(&mut ctx);
        }

        module
            .finalize_definitions()
            .map_err(|e| JitError::Module(e.to_string()))?;

        Ok(Self {
            module,
            func_ids,
            eligible,
        })
    }

    /// Whether `method_rid` was JIT-compiled.
    pub fn is_compiled(&self, method_rid: u32) -> bool {
        self.eligible.contains(&method_rid)
    }

    /// Call a compiled numeric method with `args`, returning its `f64` result.
    ///
    /// # Safety contract
    /// The compiled code is trusted because the module was verified before
    /// compilation; `args.len()` must equal the method's parameter count.
    pub fn call(&self, method_rid: u32, args: &[f64]) -> Result<f64, JitError> {
        let id = *self
            .func_ids
            .get(&method_rid)
            .ok_or(JitError::Ineligible(method_rid))?;
        let ptr = self.module.get_finalized_function(id);

        // Dispatch by arity through a small set of concrete fn signatures. The
        // numeric subset keeps arities small in practice.
        unsafe {
            Ok(match args.len() {
                0 => {
                    let f: extern "C" fn() -> f64 = std::mem::transmute(ptr);
                    f()
                }
                1 => {
                    let f: extern "C" fn(f64) -> f64 = std::mem::transmute(ptr);
                    f(args[0])
                }
                2 => {
                    let f: extern "C" fn(f64, f64) -> f64 = std::mem::transmute(ptr);
                    f(args[0], args[1])
                }
                3 => {
                    let f: extern "C" fn(f64, f64, f64) -> f64 = std::mem::transmute(ptr);
                    f(args[0], args[1], args[2])
                }
                n => return Err(JitError::Arity { expected: n.min(3), found: n }),
            })
        }
    }
}

/// Build an all-`f64` signature with `params` parameters and one `f64` result.
///
/// It uses the host's native (C) calling convention so the entry points can be
/// invoked directly through `extern "C"` function pointers from Rust, and so
/// calls between compiled methods match. Switching managed calls to
/// `CallConv::Tail` for tail-call optimization is a later refinement that needs
/// a C-convention trampoline at the Rust boundary.
fn numeric_signature(module: &JITModule, params: usize) -> cranelift_codegen::ir::Signature {
    let call_conv = module.isa().default_call_conv();
    let mut sig = cranelift_codegen::ir::Signature::new(call_conv);
    for _ in 0..params {
        sig.params.push(AbiParam::new(types::F64));
    }
    sig.returns.push(AbiParam::new(types::F64));
    sig
}
