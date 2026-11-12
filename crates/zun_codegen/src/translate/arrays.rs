//! Array opcodes (`ArrayLength`, `NewArray`, `ArrayPop`, `ArrayPush`) and the
//! imported array-helper `FuncRef` constructors (load/store/pop/push per kind,
//! plus `store_ref`). Extracted verbatim from `translate/mod.rs`; `LoadArray`
//! and `StoreArray` stay inline in the dispatcher and call these helpers across
//! the module boundary.

use super::*;

impl Translator<'_> {
    pub(crate) fn op_array_length(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (array, kind) = stack.pop().ok_or("stack underflow")?;
        if kind == JitKind::Object {
            self.builder.declare_value_needs_stack_map(array);
        }
        let length = self.array_length_fref();
        let inst = self.builder.ins().call(length, &[array]);
        let result = self.builder.inst_results(inst)[0];
        stack.push((result, JitKind::Num(NumKind::U64)));
        Ok(())
    }

    pub(crate) fn op_new_array(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (length, length_kind) = stack.pop().ok_or("stack underflow")?;
        if !matches!(length_kind, JitKind::Num(kind) if kind.int_parts().is_some()) {
            return Err("array length must be an integer".into());
        }
        let length = match clif_ty(length_kind) {
            types::I64 => length,
            ty if ty.is_int() => self.builder.ins().uextend(types::I64, length),
            _ => return Err("array length must be an integer".into()),
        };
        let alloc = self.array_new_fref();
        let inst = self.builder.ins().call(alloc, &[length]);
        let result = self.builder.inst_results(inst)[0];
        self.builder.declare_value_needs_stack_map(result);
        stack.push((result, JitKind::Array));
        Ok(())
    }

    pub(crate) fn op_array_pop(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (array, kind) = stack.pop().ok_or("stack underflow")?;
        if !matches!(kind, JitKind::Object | JitKind::Array) {
            return Err("array pop requires an object receiver".into());
        }
        self.builder.declare_value_needs_stack_map(array);
        // As for `LoadArray`: the pop kind comes from the method return kind,
        // which in an OSR loop region is the resume PC, not the element kind.
        if self.osr_exit.is_some() {
            return Err("array pop in an OSR loop region has no known element kind".into());
        }
        let pop_u64 = self.method_return_kind == JitKind::Num(NumKind::U64);
        let pop_i64 = self.method_return_kind == JitKind::Num(NumKind::I64);
        let pop_i32 = self.method_return_kind == JitKind::Num(NumKind::I32);
        let pop_narrow = matches!(
            self.method_return_kind,
            JitKind::Num(NumKind::I8 | NumKind::U8 | NumKind::I16 | NumKind::U16)
        );
        let pop_bool = self.method_return_kind == JitKind::Bool;
        let pop_object = self.method_return_kind == JitKind::Object;
        let pop = if pop_object {
            self.array_pop_object_fref()
        } else if pop_bool {
            self.array_pop_bool_fref()
        } else if pop_u64 || pop_i64 {
            self.array_pop_u64_fref()
        } else if pop_i32 || pop_narrow {
            self.array_pop_i32_fref()
        } else {
            self.array_pop_f64_fref()
        };
        let inst = self.builder.ins().call(pop, &[array]);
        if pop_object {
            self.builder
                .declare_value_needs_stack_map(self.builder.inst_results(inst)[0]);
        }
        let result = self.builder.inst_results(inst)[0];
        if pop_narrow {
            let width = match self.method_return_kind {
                JitKind::Num(NumKind::I8 | NumKind::U8) => types::I8,
                _ => types::I16,
            };
            let result = self.builder.ins().ireduce(width, result);
            stack.push((result, self.method_return_kind));
        } else {
            stack.push((
                result,
                if pop_object {
                    JitKind::Object
                } else if pop_bool {
                    JitKind::Bool
                } else if pop_u64 {
                    JitKind::Num(NumKind::U64)
                } else if pop_i64 {
                    JitKind::Num(NumKind::I64)
                } else if pop_i32 {
                    JitKind::Num(NumKind::I32)
                } else {
                    JitKind::Num(NumKind::F64)
                },
            ));
        }
        Ok(())
    }

    pub(crate) fn op_array_push(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (value, value_kind) = stack.pop().ok_or("stack underflow")?;
        let (array, array_kind) = stack.pop().ok_or("stack underflow")?;
        let push_object = value_kind == JitKind::Object;
        let push_u64 = value_kind == JitKind::Num(NumKind::U64);
        let push_i64 = value_kind == JitKind::Num(NumKind::I64);
        let push_i32 = value_kind == JitKind::Num(NumKind::I32);
        let push_narrow = matches!(
            value_kind,
            JitKind::Num(NumKind::I8 | NumKind::U8 | NumKind::I16 | NumKind::U16)
        );
        let push_bool = value_kind == JitKind::Bool;
        if !matches!(array_kind, JitKind::Object | JitKind::Array)
            || (!push_object
                && !push_u64
                && !push_i64
                && !push_i32
                && !push_narrow
                && !push_bool
                && value_kind != JitKind::Num(NumKind::F64))
        {
            return Err("numeric array push requires object and f64/u64 value".into());
        }
        self.builder.declare_value_needs_stack_map(array);
        if push_object {
            self.builder.declare_value_needs_stack_map(value);
        }
        let push_value = if push_narrow {
            match value_kind {
                JitKind::Num(NumKind::I8 | NumKind::U8) => {
                    self.builder.ins().uextend(types::I32, value)
                }
                _ => self.builder.ins().sextend(types::I32, value),
            }
        } else {
            value
        };
        let push = if push_object {
            self.array_push_object_fref()
        } else if push_bool {
            self.array_push_bool_fref()
        } else if push_u64 || push_i64 {
            self.array_push_u64_fref()
        } else if push_i32 || push_narrow {
            self.array_push_i32_fref()
        } else {
            self.array_push_f64_fref()
        };
        let inst = self.builder.ins().call(push, &[array, push_value]);
        stack.push((
            self.builder.inst_results(inst)[0],
            JitKind::Num(NumKind::I64),
        ));
        Ok(())
    }

    pub(crate) fn array_length_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let conv = self.module.isa().default_call_conv();
        let mut sig = Signature::new(conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_length", Linkage::Import, &sig)
            .expect("declare zun_jit_array_length");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_new_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_new", Linkage::Import, &sig)
            .expect("declare zun_jit_array_new");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_load_f64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));
        let id = self
            .module
            .declare_function("zun_jit_array_load_f64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_load_f64");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_load_u64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func = self
            .module
            .declare_function("zun_jit_array_load_u64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_load_u64");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_load_i32_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let func = self
            .module
            .declare_function("zun_jit_array_load_i32", Linkage::Import, &sig)
            .expect("declare zun_jit_array_load_i32");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_load_bool_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let func = self
            .module
            .declare_function("zun_jit_array_load_bool", Linkage::Import, &sig)
            .expect("declare zun_jit_array_load_bool");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_load_object_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_load_object", Linkage::Import, &sig)
            .expect("declare zun_jit_array_load_object");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_load_any_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_array_load_any", Linkage::Import, &sig)
            .expect("declare zun_jit_array_load_any");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_store_f64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_array_store_f64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_store_f64");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_store_u64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let func = self
            .module
            .declare_function("zun_jit_array_store_u64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_store_u64");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_store_i32_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I32));
        let func = self
            .module
            .declare_function("zun_jit_array_store_i32", Linkage::Import, &sig)
            .expect("declare zun_jit_array_store_i32");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_store_bool_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I32));
        let func = self
            .module
            .declare_function("zun_jit_array_store_bool", Linkage::Import, &sig)
            .expect("declare zun_jit_array_store_bool");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_store_object_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_array_store_object", Linkage::Import, &sig)
            .expect("declare zun_jit_array_store_object");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn store_ref_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_store_ref", Linkage::Import, &sig)
            .expect("declare zun_jit_store_ref");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_pop_f64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));
        let id = self
            .module
            .declare_function("zun_jit_array_pop_f64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_pop_f64");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_pop_u64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func = self
            .module
            .declare_function("zun_jit_array_pop_u64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_pop_u64");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_pop_i32_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let func = self
            .module
            .declare_function("zun_jit_array_pop_i32", Linkage::Import, &sig)
            .expect("declare zun_jit_array_pop_i32");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_pop_bool_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let func = self
            .module
            .declare_function("zun_jit_array_pop_bool", Linkage::Import, &sig)
            .expect("declare zun_jit_array_pop_bool");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_pop_object_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_pop_object", Linkage::Import, &sig)
            .expect("declare zun_jit_array_pop_object");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_push_f64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_push_f64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_push_f64");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn array_push_u64_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func = self
            .module
            .declare_function("zun_jit_array_push_u64", Linkage::Import, &sig)
            .expect("declare zun_jit_array_push_u64");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_push_i32_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        let func = self
            .module
            .declare_function("zun_jit_array_push_i32", Linkage::Import, &sig)
            .expect("declare zun_jit_array_push_i32");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_push_bool_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        let func = self
            .module
            .declare_function("zun_jit_array_push_bool", Linkage::Import, &sig)
            .expect("declare zun_jit_array_push_bool");
        self.module.declare_func_in_func(func, self.builder.func)
    }

    pub(crate) fn array_push_object_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_array_push_object", Linkage::Import, &sig)
            .expect("declare zun_jit_array_push_object");
        self.module.declare_func_in_func(id, self.builder.func)
    }
}
