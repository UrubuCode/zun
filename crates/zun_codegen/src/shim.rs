//! Calling-convention adapters ("shims").
//!
//! Each compiled method gets a shim with a single uniform ABI,
//! `extern "C" fn(args: *const u64) -> u64`, so the interpreter can call any
//! compiled method through one Rust function-pointer type regardless of the
//! method's real signature. The shim loads each argument from the buffer,
//! converts it to the parameter's machine type, calls the real function, and
//! packs the result back into a `u64` (float bits for floats). zun_vm marshals
//! `Value`s to and from this buffer.

use cranelift_codegen::ir::{types, InstBuilder, MemFlags, Value};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module as ClifModule};
use zun_il::{JitKind, NumKind};

use crate::translate::clif_ty;

/// Build and define a shim for a real function, returning the shim's FuncId.
pub fn define_shim(
    module: &mut dyn ClifModule,
    ctx: &mut Context,
    fb_ctx: &mut FunctionBuilderContext,
    name: &str,
    real: FuncId,
    param_kinds: &[JitKind],
    ret: Option<JitKind>,
) -> Result<FuncId, String> {
    let conv = module.isa().default_call_conv();
    let ptr_ty = module.isa().pointer_type();

    // Shim signature: (args_ptr) -> u64.
    let mut shim_sig = cranelift_codegen::ir::Signature::new(conv);
    shim_sig
        .params
        .push(cranelift_codegen::ir::AbiParam::new(ptr_ty));
    shim_sig
        .returns
        .push(cranelift_codegen::ir::AbiParam::new(types::I64));
    let shim_id = module
        .declare_function(name, Linkage::Local, &shim_sig)
        .map_err(|e| e.to_string())?;

    ctx.func.signature = shim_sig;
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, fb_ctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let args_ptr = b.block_params(entry)[0];

        // Load and convert each argument from the buffer (8 bytes per slot).
        let mut args: Vec<Value> = Vec::with_capacity(param_kinds.len());
        for (i, &k) in param_kinds.iter().enumerate() {
            let raw = b
                .ins()
                .load(types::I64, MemFlags::trusted(), args_ptr, (i * 8) as i32);
            args.push(unpack_arg(&mut b, raw, k));
        }

        let fref = module.declare_func_in_func(real, b.func);
        let inst = b.ins().call(fref, &args);
        let packed = match ret {
            Some(k) => {
                let r = b.inst_results(inst)[0];
                pack_result(&mut b, r, k)
            }
            None => b.ins().iconst(types::I64, 0),
        };
        b.ins().return_(&[packed]);
        b.finalize();
    }

    module
        .define_function(shim_id, ctx)
        .map_err(|e| e.to_string())?;
    module.clear_context(ctx);
    Ok(shim_id)
}

/// Build a shim for the two-word `Any` return ABI.
///
/// `JitKind::Other` remains ineligible until its arguments and call sites are
/// lowered as carriers too; this function establishes the return-side ABI so
/// that future lowering cannot accidentally collapse `tag` and `payload`.
pub fn define_any_return_shim(
    module: &mut dyn ClifModule,
    ctx: &mut Context,
    fb_ctx: &mut FunctionBuilderContext,
    name: &str,
    real: FuncId,
    param_kinds: &[JitKind],
) -> Result<FuncId, String> {
    let conv = module.isa().default_call_conv();
    let ptr_ty = module.isa().pointer_type();
    let mut shim_sig = cranelift_codegen::ir::Signature::new(conv);
    shim_sig.params.push(cranelift_codegen::ir::AbiParam::new(ptr_ty));
    shim_sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
    shim_sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
    let shim_id = module
        .declare_function(name, Linkage::Local, &shim_sig)
        .map_err(|e| e.to_string())?;

    ctx.func.signature = shim_sig;
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, fb_ctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let args_ptr = b.block_params(entry)[0];
        let mut args = Vec::with_capacity(param_kinds.len());
        let mut offset = 0;
        for &kind in param_kinds {
            let raw = b.ins().load(types::I64, MemFlags::trusted(), args_ptr, offset);
            if kind == JitKind::Other {
                let payload = b.ins().load(types::I64, MemFlags::trusted(), args_ptr, offset + 8);
                args.push(raw);
                args.push(payload);
                offset += (crate::any_abi::abi_word_count(kind) * 8) as i32;
            } else {
                args.push(unpack_arg(&mut b, raw, kind));
                offset += (crate::any_abi::abi_word_count(kind) * 8) as i32;
            }
        }
        let fref = module.declare_func_in_func(real, b.func);
        let inst = b.ins().call(fref, &args);
        let results = b.inst_results(inst).to_vec();
        if results.len() != 2 {
            return Err("Any return function must produce tag and payload".into());
        }
        b.ins().return_(&[results[0], results[1]]);
        b.finalize();
    }
    module
        .define_function(shim_id, ctx)
        .map_err(|e| e.to_string())?;
    module.clear_context(ctx);
    Ok(shim_id)
}

/// Convert a raw i64 value into a value of kind `k` (shared with field loads).
pub(crate) fn unpack_arg(b: &mut FunctionBuilder, raw: Value, k: JitKind) -> Value {
    match k {
        JitKind::Num(NumKind::F64) => b.ins().bitcast(types::F64, MemFlags::new(), raw),
        JitKind::Num(NumKind::F32) => {
            let lo = b.ins().ireduce(types::I32, raw);
            b.ins().bitcast(types::F32, MemFlags::new(), lo)
        }
        _ => {
            // Integer or bool: reduce the i64 to the target width.
            let ty = clif_ty(k);
            if ty == types::I64 {
                raw
            } else {
                b.ins().ireduce(ty, raw)
            }
        }
    }
}

/// Pack a value of kind `k` into an i64 (shared with field stores).
pub(crate) fn pack_result(b: &mut FunctionBuilder, r: Value, k: JitKind) -> Value {
    match k {
        JitKind::Num(NumKind::F64) => b.ins().bitcast(types::I64, MemFlags::new(), r),
        JitKind::Num(NumKind::F32) => {
            let bits = b.ins().bitcast(types::I32, MemFlags::new(), r);
            b.ins().uextend(types::I64, bits)
        }
        _ => {
            let ty = clif_ty(k);
            if ty == types::I64 {
                r
            } else if is_signed(k) {
                b.ins().sextend(types::I64, r)
            } else {
                b.ins().uextend(types::I64, r)
            }
        }
    }
}

fn is_signed(k: JitKind) -> bool {
    matches!(k, JitKind::Num(n) if n.int_parts().map(|(_, s)| s).unwrap_or(false))
}
