//! String opcodes and the `CallVirt` string-method sub-dispatch, plus the
//! imported string-helper `FuncRef` constructors. Extracted verbatim from
//! `translate/mod.rs`. `op_call_string` is the string-method branch of the
//! `Call`/`CallVirt` arm: it reads the resolved method name (NOT an opcode) and
//! returns `true` when it handled the call. The frefs here are shared by these
//! methods and by `op_concat`/`op_const_str`.

use super::*;

impl Translator<'_> {
    pub(crate) fn op_const_str(
        &mut self,
        instr: &zun_il::Instruction,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let Operand::StringRef(index) = instr.operand else {
            return Err("const.str operand".into());
        };
        if self.literal_data_ids.is_null() && self.literal_ptrs.is_null() {
            return Err("const.str requires runtime literal storage".into());
        }
        let len = self.zm.strings.get(zun_metadata::StringIndex(index)).len();
        let ptr = if self.literal_data_ids.is_null() {
            let address = unsafe { *self.literal_ptrs.add(index as usize) };
            self.builder.ins().iconst(types::I64, address as i64)
        } else {
            let data_id = unsafe { *self.literal_data_ids.add(index as usize) };
            let global = self
                .module
                .declare_data_in_func(DataId::from_u32(data_id), self.builder.func);
            self.builder.ins().global_value(types::I64, global)
        };
        let len = self.builder.ins().iconst(types::I64, len as i64);
        let literal = self.string_literal_fref();
        let call = self.builder.ins().call(literal, &[ptr, len]);
        let result = self.builder.inst_results(call)[0];
        // Interning may allocate and trigger a collection. Keep
        // the managed handle visible to the precise stack map
        // for subsequent safepoints in this method.
        self.builder.declare_value_needs_stack_map(result);
        stack.push((result, JitKind::String));
        Ok(())
    }

    pub(crate) fn op_concat(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        // String `+`: pop two managed string handles, call the shared
        // runtime concat helper (the same one the `.concat` method
        // lowers to), and push the new string. Both operands and the
        // result are GC references, so each is kept in a stack map
        // across the allocating call.
        let (right, right_kind) = stack.pop().ok_or("concat right")?;
        let (left, left_kind) = stack.pop().ok_or("concat left")?;
        if left_kind != JitKind::String || right_kind != JitKind::String {
            return Err("concat expects string operands".into());
        }
        self.builder.declare_value_needs_stack_map(left);
        self.builder.declare_value_needs_stack_map(right);
        let helper = self.string_concat_fref();
        let call = self.builder.ins().call(helper, &[left, right]);
        let result = self.builder.inst_results(call)[0];
        self.builder.declare_value_needs_stack_map(result);
        stack.push((result, JitKind::String));
        Ok(())
    }

    /// The `CallVirt` string-method sub-dispatch. Returns `Ok(true)` when the
    /// receiver's method was lowered inline here, `Ok(false)` to fall through to
    /// the generic call path. Reads the already-resolved method name; it is not
    /// an opcode table.
    pub(crate) fn op_call_string(
        &mut self,
        method_name: Option<&str>,
        stack: &mut Vec<Slot>,
    ) -> Result<bool, String> {
        match method_name {
            Some("length") => {
                let (receiver, _) = stack.pop().ok_or("string length receiver")?;
                self.builder.declare_value_needs_stack_map(receiver);
                let length = self.string_length_fref();
                let call = self.builder.ins().call(length, &[receiver]);
                let result = self.builder.inst_results(call)[0];
                stack.push((result, JitKind::Num(NumKind::U64)));
                return Ok(true);
            }
            Some("contains") | Some("includes") => {
                let (needle, needle_kind) = stack.pop().ok_or("string contains needle")?;
                let (receiver, receiver_kind) = stack.pop().ok_or("string contains receiver")?;
                if receiver_kind != JitKind::String || needle_kind != JitKind::String {
                    return Err("string contains expects string arguments".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                self.builder.declare_value_needs_stack_map(needle);
                let contains = self.string_contains_fref();
                let call = self.builder.ins().call(contains, &[receiver, needle]);
                let result = self.builder.inst_results(call)[0];
                stack.push((result, JitKind::Bool));
                return Ok(true);
            }
            Some("charCodeAt") => {
                let (index, index_kind) = stack.pop().ok_or("string charCodeAt index")?;
                let (receiver, receiver_kind) = stack.pop().ok_or("string charCodeAt receiver")?;
                if receiver_kind != JitKind::String
                    || !matches!(index_kind, JitKind::Num(kind) if kind.int_parts().is_some())
                {
                    return Err("string charCodeAt expects string and integer arguments".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                let index = match clif_ty(index_kind) {
                    types::I64 => index,
                    ty if ty.is_int() => self.builder.ins().uextend(types::I64, index),
                    _ => return Err("string charCodeAt index must be an integer".into()),
                };
                let char_code = self.string_char_code_at_fref();
                let call = self.builder.ins().call(char_code, &[receiver, index]);
                let result = self.builder.inst_results(call)[0];
                stack.push((result, JitKind::Num(NumKind::U32)));
                return Ok(true);
            }
            Some("startsWith") | Some("endsWith") => {
                let (needle, needle_kind) = stack.pop().ok_or("string prefix/suffix")?;
                let (receiver, receiver_kind) =
                    stack.pop().ok_or("string prefix/suffix receiver")?;
                if receiver_kind != JitKind::String || needle_kind != JitKind::String {
                    return Err("string prefix/suffix expects string arguments".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                self.builder.declare_value_needs_stack_map(needle);
                let helper = if method_name == Some("startsWith") {
                    self.string_starts_with_fref()
                } else {
                    self.string_ends_with_fref()
                };
                let call = self.builder.ins().call(helper, &[receiver, needle]);
                let result = self.builder.inst_results(call)[0];
                stack.push((result, JitKind::Bool));
                return Ok(true);
            }
            Some("indexOf") => {
                let (needle, needle_kind) = stack.pop().ok_or("string indexOf needle")?;
                let (receiver, receiver_kind) = stack.pop().ok_or("string indexOf receiver")?;
                if receiver_kind != JitKind::String || needle_kind != JitKind::String {
                    return Err("string indexOf expects string arguments".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                self.builder.declare_value_needs_stack_map(needle);
                let helper = self.string_index_of_fref();
                let call = self.builder.ins().call(helper, &[receiver, needle]);
                let result = self.builder.inst_results(call)[0];
                stack.push((result, JitKind::Num(NumKind::I32)));
                return Ok(true);
            }
            Some("lastIndexOf") => {
                let (needle, needle_kind) = stack.pop().ok_or("string lastIndexOf needle")?;
                let (receiver, receiver_kind) = stack.pop().ok_or("string lastIndexOf receiver")?;
                if receiver_kind != JitKind::String || needle_kind != JitKind::String {
                    return Err("string lastIndexOf expects string arguments".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                self.builder.declare_value_needs_stack_map(needle);
                let helper = self.string_last_index_of_fref();
                let call = self.builder.ins().call(helper, &[receiver, needle]);
                let result = self.builder.inst_results(call)[0];
                stack.push((result, JitKind::Num(NumKind::I32)));
                return Ok(true);
            }
            Some("count") => {
                let (needle, needle_kind) = stack.pop().ok_or("string count needle")?;
                let (receiver, receiver_kind) = stack.pop().ok_or("string count receiver")?;
                if receiver_kind != JitKind::String || needle_kind != JitKind::String {
                    return Err("string count expects string arguments".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                self.builder.declare_value_needs_stack_map(needle);
                let helper = self.string_count_fref();
                let call = self.builder.ins().call(helper, &[receiver, needle]);
                let result = self.builder.inst_results(call)[0];
                stack.push((result, JitKind::Num(NumKind::I32)));
                return Ok(true);
            }
            Some("compare") => {
                let (right, right_kind) = stack.pop().ok_or("string compare right")?;
                let (left, left_kind) = stack.pop().ok_or("string compare left")?;
                if left_kind != JitKind::String || right_kind != JitKind::String {
                    return Err("string compare expects string arguments".into());
                }
                self.builder.declare_value_needs_stack_map(left);
                self.builder.declare_value_needs_stack_map(right);
                let helper = self.string_compare_fref();
                let call = self.builder.ins().call(helper, &[left, right]);
                let result = self.builder.inst_results(call)[0];
                stack.push((result, JitKind::Num(NumKind::I32)));
                return Ok(true);
            }
            Some("concat") => {
                let (right, right_kind) = stack.pop().ok_or("string concat right")?;
                let (left, left_kind) = stack.pop().ok_or("string concat left")?;
                if left_kind != JitKind::String || right_kind != JitKind::String {
                    return Err("string concat expects string arguments".into());
                }
                self.builder.declare_value_needs_stack_map(left);
                self.builder.declare_value_needs_stack_map(right);
                let helper = self.string_concat_fref();
                let call = self.builder.ins().call(helper, &[left, right]);
                let result = self.builder.inst_results(call)[0];
                self.builder.declare_value_needs_stack_map(result);
                stack.push((result, JitKind::String));
                return Ok(true);
            }
            Some("substring") => {
                let (end, end_kind) = stack.pop().ok_or("substring end")?;
                let (start, start_kind) = stack.pop().ok_or("substring start")?;
                let (receiver, receiver_kind) = stack.pop().ok_or("substring receiver")?;
                if receiver_kind != JitKind::String
                    || !matches!(start_kind, JitKind::Num(kind) if kind.int_parts().is_some())
                    || !matches!(end_kind, JitKind::Num(kind) if kind.int_parts().is_some())
                {
                    return Err("substring expects string and integer arguments".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                let start = match clif_ty(start_kind) {
                    types::I64 => start,
                    ty if ty.is_int() => self.builder.ins().uextend(types::I64, start),
                    _ => return Err("substring start must be an integer".into()),
                };
                let end = match clif_ty(end_kind) {
                    types::I64 => end,
                    ty if ty.is_int() => self.builder.ins().uextend(types::I64, end),
                    _ => return Err("substring end must be an integer".into()),
                };
                let helper = self.string_substring_fref();
                let call = self.builder.ins().call(helper, &[receiver, start, end]);
                let result = self.builder.inst_results(call)[0];
                self.builder.declare_value_needs_stack_map(result);
                stack.push((result, JitKind::String));
                return Ok(true);
            }
            Some("trim") => {
                let (receiver, receiver_kind) = stack.pop().ok_or("trim receiver")?;
                if receiver_kind != JitKind::String {
                    return Err("trim expects a string receiver".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                let helper = self.string_trim_fref("trim");
                let call = self.builder.ins().call(helper, &[receiver]);
                let result = self.builder.inst_results(call)[0];
                self.builder.declare_value_needs_stack_map(result);
                stack.push((result, JitKind::String));
                return Ok(true);
            }
            Some("trimStart" | "trimEnd") => {
                let (receiver, receiver_kind) = stack.pop().ok_or("trim receiver")?;
                if receiver_kind != JitKind::String {
                    return Err("trim expects a string receiver".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                let helper = self.string_trim_fref(method_name.unwrap_or("trim"));
                let call = self.builder.ins().call(helper, &[receiver]);
                let result = self.builder.inst_results(call)[0];
                self.builder.declare_value_needs_stack_map(result);
                stack.push((result, JitKind::String));
                return Ok(true);
            }
            Some("toLowerCase" | "toUpperCase") => {
                let (receiver, receiver_kind) = stack.pop().ok_or("case receiver")?;
                if receiver_kind != JitKind::String {
                    return Err("case conversion expects a string receiver".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                let helper = self.string_transform_fref(method_name.unwrap_or("toLowerCase"));
                let call = self.builder.ins().call(helper, &[receiver]);
                let result = self.builder.inst_results(call)[0];
                self.builder.declare_value_needs_stack_map(result);
                stack.push((result, JitKind::String));
                return Ok(true);
            }
            Some("reverse") => {
                let (receiver, receiver_kind) = stack.pop().ok_or("reverse receiver")?;
                if receiver_kind != JitKind::String {
                    return Err("reverse expects a string receiver".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                let helper = self.string_reverse_fref();
                let call = self.builder.ins().call(helper, &[receiver]);
                let result = self.builder.inst_results(call)[0];
                self.builder.declare_value_needs_stack_map(result);
                stack.push((result, JitKind::String));
                return Ok(true);
            }
            Some("padStart" | "padEnd") => {
                let (fill, fill_kind) = stack.pop().ok_or("pad fill")?;
                let (target, target_kind) = stack.pop().ok_or("pad target")?;
                let (receiver, receiver_kind) = stack.pop().ok_or("pad receiver")?;
                if receiver_kind != JitKind::String
                    || fill_kind != JitKind::String
                    || !matches!(target_kind, JitKind::Num(kind) if kind.int_parts().is_some())
                {
                    return Err("pad expects string, integer, and string arguments".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                self.builder.declare_value_needs_stack_map(fill);
                let target = match clif_ty(target_kind) {
                    types::I64 => target,
                    ty if ty.is_int() => self.builder.ins().uextend(types::I64, target),
                    _ => return Err("pad target must be an integer".into()),
                };
                let helper = self.string_pad_fref(method_name.unwrap_or("padStart"));
                let call = self.builder.ins().call(helper, &[receiver, target, fill]);
                let result = self.builder.inst_results(call)[0];
                self.builder.declare_value_needs_stack_map(result);
                stack.push((result, JitKind::String));
                return Ok(true);
            }
            Some("replace") => {
                let (replacement, replacement_kind) = stack.pop().ok_or("replace replacement")?;
                let (needle, needle_kind) = stack.pop().ok_or("replace needle")?;
                let (receiver, receiver_kind) = stack.pop().ok_or("replace receiver")?;
                if receiver_kind != JitKind::String
                    || needle_kind != JitKind::String
                    || replacement_kind != JitKind::String
                {
                    return Err("replace expects three strings".into());
                }
                self.builder.declare_value_needs_stack_map(receiver);
                self.builder.declare_value_needs_stack_map(needle);
                self.builder.declare_value_needs_stack_map(replacement);
                let helper = self.string_replace_fref();
                let call = self
                    .builder
                    .ins()
                    .call(helper, &[receiver, needle, replacement]);
                let result = self.builder.inst_results(call)[0];
                self.builder.declare_value_needs_stack_map(result);
                stack.push((result, JitKind::String));
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    pub(crate) fn string_length_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_length", Linkage::Import, &sig)
            .expect("declare zun_jit_string_length");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_literal_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_literal", Linkage::Import, &sig)
            .expect("declare zun_jit_string_literal");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_contains_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_contains", Linkage::Import, &sig)
            .expect("declare zun_jit_string_contains");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_char_code_at_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_char_code_at", Linkage::Import, &sig)
            .expect("declare zun_jit_string_char_code_at");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_starts_with_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        self.string_predicate_fref("zun_jit_string_starts_with")
    }

    pub(crate) fn string_ends_with_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        self.string_predicate_fref("zun_jit_string_ends_with")
    }

    pub(crate) fn string_index_of_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_index_of", Linkage::Import, &sig)
            .expect("declare zun_jit_string_index_of");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_last_index_of_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_last_index_of", Linkage::Import, &sig)
            .expect("declare zun_jit_string_last_index_of");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_count_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_count", Linkage::Import, &sig)
            .expect("declare zun_jit_string_count");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_compare_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("zun_jit_string_compare", Linkage::Import, &sig)
            .expect("declare zun_jit_string_compare");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_concat_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_concat", Linkage::Import, &sig)
            .expect("declare zun_jit_string_concat");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_substring_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_substring", Linkage::Import, &sig)
            .expect("declare zun_jit_string_substring");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_trim_fref(&mut self, name: &str) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function(
                match name {
                    "trimStart" => "zun_jit_string_trim_start",
                    "trimEnd" => "zun_jit_string_trim_end",
                    _ => "zun_jit_string_trim",
                },
                Linkage::Import,
                &sig,
            )
            .expect("declare string trim helper");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_transform_fref(&mut self, name: &str) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let symbol = match name {
            "toUpperCase" => "zun_jit_string_to_upper_case",
            _ => "zun_jit_string_to_lower_case",
        };
        let id = self
            .module
            .declare_function(symbol, Linkage::Import, &sig)
            .expect("declare string case helper");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_reverse_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_reverse", Linkage::Import, &sig)
            .expect("declare zun_jit_string_reverse");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_pad_fref(&mut self, name: &str) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        for _ in 0..3 {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        let symbol = if name == "padEnd" {
            "zun_jit_string_pad_end"
        } else {
            "zun_jit_string_pad_start"
        };
        let id = self
            .module
            .declare_function(symbol, Linkage::Import, &sig)
            .expect("declare string pad helper");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_replace_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        for _ in 0..3 {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_string_replace", Linkage::Import, &sig)
            .expect("declare zun_jit_string_replace");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn string_predicate_fref(&mut self, name: &str) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.builder.func.signature.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function(name, Linkage::Import, &sig)
            .expect("declare string predicate");
        self.module.declare_func_in_func(id, self.builder.func)
    }
}
