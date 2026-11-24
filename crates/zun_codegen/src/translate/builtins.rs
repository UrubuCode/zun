//! The `CallBuiltin` opcode: lowering of native builtins to imported JIT
//! helpers. Extracted from `translate/mod.rs`; the descriptor data lives in
//! `builtins_table.rs` and is read here via `builtin_desc`. The name matches
//! here are NOT an opcode table — they read the `zun_natives::by_id` name the
//! dispatcher already resolved, special-casing the few builtins whose lowering
//! is not a straight descriptor-driven helper call.

use super::builtins_table::builtin_desc;
use super::*;

impl Translator<'_> {
    #[allow(clippy::if_same_then_else)] // branches are kept distinct per builtin for clarity
    pub(crate) fn op_call_builtin(
        &mut self,
        instr: &zun_il::Instruction,
        stack: &mut Vec<Slot>,
        builtin_ids: &mut HashMap<&'static str, FuncId>,
    ) -> Result<(), String> {
        let id = match instr.operand {
            Operand::Imm32(id) => id as u32,
            _ => return Err("builtin operand".into()),
        };
        if let Some(helper_name) = match zun_natives::by_id(id).map(|f| f.name) {
            Some("mapClear") => Some("zun_jit_map_clear"),
            Some("setClear") => Some("zun_jit_set_clear"),
            _ => None,
        } {
            let (receiver, kind) = stack.pop().ok_or("clear receiver")?;
            if kind != JitKind::Object {
                return Err("clear requires a managed object".into());
            }
            self.builder.declare_value_needs_stack_map(receiver);
            let fid = if let Some(&fid) = builtin_ids.get(helper_name) {
                fid
            } else {
                let mut sig = Signature::new(self.builder.func.signature.call_conv);
                sig.params.push(AbiParam::new(types::I64));
                sig.returns.push(AbiParam::new(types::I32));
                let fid = self
                    .module
                    .declare_function(helper_name, Linkage::Import, &sig)
                    .map_err(|e| e.to_string())?;
                builtin_ids.insert(helper_name, fid);
                fid
            };
            let fref = self.module.declare_func_in_func(fid, self.builder.func);
            self.builder.ins().call(fref, &[receiver]);
            // The interpreter pushes `undefined` for a `clear()` call.
            self.push_any_const(stack, AnyConst::Undef);
            return Ok(());
        }
        if zun_natives::by_id(id).map(|f| f.name) == Some("mapGet") {
            if stack.len() < 2 {
                return Err("mapGet requires two arguments".into());
            }
            let key = stack.pop().ok_or("mapGet key")?;
            let (map, map_kind) = stack.pop().ok_or("mapGet receiver")?;
            if map_kind != JitKind::Object {
                return Err("mapGet requires a Map object".into());
            }
            self.builder.declare_value_needs_stack_map(map);
            let key = self.coerce_slot(key, JitKind::Other, "mapGet key")?;
            let key = self.any_of(key)?;
            self.emit_map_get_any(stack, map, key);
            return Ok(());
        }
        let (name, arg_count, arg_type, result_kind, result_type) =
            builtin_desc(zun_natives::by_id(id).map(|f| f.name)).ok_or("unsupported builtin")?;
        if stack.len() < arg_count {
            return Err("builtin stack underflow".into());
        }
        let args: Vec<Slot> = stack.split_off(stack.len() - arg_count);
        if args.iter().any(|&(_, kind)| kind == JitKind::Other) {
            return Err(format!("builtin {name} takes an any operand: no lowered form"));
        }
        // Every imported helper is a potential GC boundary. Keep
        // managed receivers/arguments visible before the call,
        // regardless of whether the helper currently allocates.
        for &(value, kind) in &args {
            if matches!(kind, JitKind::Object | JitKind::String) {
                self.builder.declare_value_needs_stack_map(value);
            }
        }
        let arg_types = match name {
            "zun_jit_string_char_code_at" => vec![types::I64, types::I64],
            "zun_jit_string_char_code_at_num" => vec![types::I64, types::F64],
            "zun_jit_buffer_set_u8" => vec![types::I64, types::I64, types::I8],
            "zun_jit_buffer_write_u32" => vec![types::I64, types::I64, types::I32],
            "zun_jit_buffer_write_f64" => vec![types::I64, types::I64, types::F64],
            "zun_jit_data_view_get_u32" => vec![types::I64, types::I64, types::I32],
            "zun_jit_data_view_get_u16" => vec![types::I64, types::I64, types::I32],
            "zun_jit_data_view_get_f64" => vec![types::I64, types::I64, types::I32],
            "zun_jit_data_view_set_u32" => vec![types::I64, types::I64, types::I32, types::I32],
            "zun_jit_data_view_set_u16" => vec![types::I64, types::I64, types::I16, types::I32],
            "zun_jit_data_view_set_f64" => vec![types::I64, types::I64, types::F64, types::I32],
            // 8-bit accessors take no endianness argument.
            "zun_jit_data_view_set_i8" | "zun_jit_data_view_set_u8" => {
                vec![types::I64, types::I64, types::I8]
            }
            "zun_jit_data_view_get_i16"
            | "zun_jit_data_view_get_i32"
            | "zun_jit_data_view_get_f32" => {
                vec![types::I64, types::I64, types::I32]
            }
            "zun_jit_data_view_set_i16" => {
                vec![types::I64, types::I64, types::I16, types::I32]
            }
            "zun_jit_data_view_set_i32" => {
                vec![types::I64, types::I64, types::I32, types::I32]
            }
            "zun_jit_data_view_set_f32" => {
                vec![types::I64, types::I64, types::F32, types::I32]
            }
            "zun_jit_u8_array_get" => vec![types::I64, types::I64],
            "zun_jit_u8_array_set" => vec![types::I64, types::I64, types::I8],
            "zun_jit_u16_array_get" => vec![types::I64, types::I64],
            "zun_jit_u16_array_set" => vec![types::I64, types::I64, types::I16],
            "zun_jit_u32_array_get" => vec![types::I64, types::I64],
            "zun_jit_u32_array_set" => vec![types::I64, types::I64, types::I32],
            "zun_jit_f64_array_get" => vec![types::I64, types::I64],
            "zun_jit_f64_array_set" => vec![types::I64, types::I64, types::F64],
            "zun_jit_map_has_f64"
            | "zun_jit_set_has_f64"
            | "zun_jit_set_add_f64"
            | "zun_jit_map_delete_f64"
            | "zun_jit_set_delete_f64"
            | "zun_jit_map_set_f64"
            | "zun_jit_map_get_f64_or" => {
                if name == "zun_jit_map_set_f64" {
                    vec![types::I64, types::F64, types::F64]
                } else if name == "zun_jit_map_get_f64_or" {
                    vec![types::I64, types::F64, types::F64]
                } else {
                    vec![types::I64, types::F64]
                }
            }
            "zun_jit_map_has_i64"
            | "zun_jit_set_has_i64"
            | "zun_jit_set_add_i64"
            | "zun_jit_map_delete_i64"
            | "zun_jit_set_delete_i64" => vec![types::I64, types::I64],
            "zun_jit_map_has_i32" | "zun_jit_set_has_i32" => vec![types::I64, types::I32],
            "zun_jit_set_add_i32" => vec![types::I64, types::I32],
            "zun_jit_map_delete_i32" | "zun_jit_set_delete_i32" => vec![types::I64, types::I32],
            "zun_jit_map_set_i64" | "zun_jit_map_get_i64_or" => {
                vec![types::I64, types::I64, types::I64]
            }
            "zun_jit_map_get_i32_or" => vec![types::I64, types::I32, types::I32],
            "zun_jit_map_set_i32" => vec![types::I64, types::I32, types::I32],
            _ => vec![arg_type; arg_count],
        };
        let fid = if let Some(&fid) = builtin_ids.get(name) {
            fid
        } else {
            let mut sig = Signature::new(self.builder.func.signature.call_conv);
            for arg_type in arg_types {
                sig.params.push(AbiParam::new(arg_type));
            }
            sig.returns.push(AbiParam::new(result_type));
            let fid = self
                .module
                .declare_function(name, Linkage::Import, &sig)
                .map_err(|e| e.to_string())?;
            builtin_ids.insert(name, fid);
            fid
        };
        let fref = self.module.declare_func_in_func(fid, self.builder.func);
        let values: Vec<Value> = args.iter().map(|(value, _)| *value).collect();
        let inst = self.builder.ins().call(fref, &values);
        let result = self.builder.inst_results(inst)[0];
        if matches!(result_kind, JitKind::Object | JitKind::String) {
            self.builder.declare_value_needs_stack_map(result);
        }
        stack.push((result, result_kind));
        Ok(())
    }
}
