use super::*;

mod call;
mod closure;
mod data;
mod dynamic;
mod ops;

impl<'a, 'b> FnLowerer<'a, 'b> {
    pub(super) fn expr(&mut self, e: &hir::Expr) {
        if !e.span.is_empty() {
            self.points.push((self.code.len() as u32, e.span));
        }
        match &e.kind {
            ExprKind::Number(v) => self.emit_numeric_const(*v, e.ty),
            ExprKind::String(s) => self.lower_string_literal(s),
            ExprKind::Boolean(true) => self.emit(Instruction::simple(Opcode::ConstTrue)),
            ExprKind::Boolean(false) => self.emit(Instruction::simple(Opcode::ConstFalse)),
            ExprKind::Null => self.emit(Instruction::simple(Opcode::ConstNull)),
            ExprKind::Local(slot) => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
            }
            ExprKind::CellLoad { slot } => self.lower_cell_load(slot),
            ExprKind::CellRef { slot } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
            }
            ExprKind::SelfClosure => self.lower_self_closure(),
            ExprKind::CaptureRef { environment_slot } => self.lower_capture_ref(environment_slot),
            ExprKind::CellStore { slot, value } => self.lower_cell_store(slot, value),
            ExprKind::FunctionRef { func } => self.lower_function_ref(func),
            ExprKind::Closure { function, captures } => self.lower_closure(function, captures),
            ExprKind::CaptureLoad {
                environment_slot,
                mode,
                ty,
            } => self.lower_capture_load(environment_slot, mode, ty),
            ExprKind::CaptureStore {
                environment_slot,
                mode,
                value,
                ..
            } => self.lower_capture_store(environment_slot, mode, value),
            ExprKind::Sequence { effect, value } => self.lower_sequence(effect, value),
            ExprKind::Assign { slot, value } => self.lower_assign(slot, value),
            ExprKind::Await { operand } => self.lower_await(operand),
            ExprKind::ImportDynamic { specifier } => {
                self.expr(specifier);
                self.emit(Instruction::simple(Opcode::ImportDynamic));
            }
            ExprKind::Yield { value } => self.lower_yield(value),
            ExprKind::Binary { op, lhs, rhs } => self.lower_binary(op, lhs, rhs),
            ExprKind::Conditional {
                condition,
                when_true,
                when_false,
            } => self.lower_conditional(condition, when_true, when_false),
            ExprKind::Unary { op, operand } => self.lower_unary(op, operand),
            ExprKind::Call { func, args } => self.lower_call(func, args),
            ExprKind::CallRest { func, args } => self.lower_call_rest(func, args),
            ExprKind::CallRestArray { func, args } => self.lower_call_rest_array(func, args),
            ExprKind::CallRestArrayFixed {
                func,
                fixed_args,
                args,
            } => self.lower_call_rest_array_fixed(func, fixed_args, args),
            ExprKind::GeneratorCall {
                func,
                args,
                native_id,
            } => self.lower_generator_call(func, args, native_id),
            ExprKind::FunctionBind {
                callee,
                args,
                native_id,
            } => self.lower_function_bind(callee, args, native_id),
            ExprKind::IndirectCall { callee, args, func } => {
                self.lower_indirect_call(callee, args, func)
            }
            ExprKind::IndirectCallRest { callee, args, func } => {
                self.lower_indirect_call_rest(callee, args, func)
            }
            ExprKind::OptionalIndirectCall { callee, args, func } => {
                self.lower_optional_indirect_call(callee, args, func)
            }
            ExprKind::CallBuiltin { id, args } => self.lower_call_builtin(id, args),
            ExprKind::RegexMatch { matcher, args } => self.lower_regex_match(matcher, args),
            ExprKind::DynamicNew => {
                self.emit(Instruction::simple(Opcode::NewDynamic));
            }
            ExprKind::RealmGlobal => {
                self.emit(Instruction::simple(Opcode::RealmGlobal));
            }
            ExprKind::DynamicGet { object, name } => self.lower_dynamic_get(object, name),
            ExprKind::DynamicGetValue { object, key } => {
                self.expr(object);
                self.expr(key);
                self.emit(Instruction::simple(Opcode::LoadDynamicValue));
            }
            ExprKind::OptionalDynamicGet { object, name } => {
                self.lower_optional_dynamic_get(object, name)
            }
            ExprKind::OptionalDynamicGetValue { object, key } => {
                self.lower_optional_dynamic_get_value(object, key)
            }
            ExprKind::OptionalCallBuiltin { id, args } => {
                self.lower_optional_call_builtin(id, args)
            }
            ExprKind::DynamicSet {
                object,
                name,
                value,
            } => self.lower_dynamic_set(object, name, value),
            ExprKind::DynamicSetValue { object, key, value } => {
                self.expr(object);
                self.expr(key);
                self.expr(value);
                self.emit(Instruction::simple(Opcode::StoreDynamicValue));
            }
            ExprKind::DynamicObjectLiteral { entries } => {
                self.lower_dynamic_object_literal(entries)
            }
            ExprKind::BoxAny { value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::BoxAny));
            }
            ExprKind::ExternalCall { module, name, args } => {
                self.lower_external_call(module, name, args, e)
            }
            ExprKind::MethodCall { func, args } => self.lower_method_call(func, args),
            ExprKind::StaticCall { func, args } => self.lower_static_call(func, args),
            ExprKind::MethodCallRest { func, args } => self.lower_method_call_rest(func, args),
            ExprKind::MethodCallRestArray {
                func,
                receiver,
                args,
            } => self.lower_method_call_rest_array(func, receiver, args),
            ExprKind::MethodCallRestArrayFixed {
                func,
                receiver,
                fixed_args,
                args,
            } => self.lower_method_call_rest_array_fixed(func, receiver, fixed_args, args),
            ExprKind::New { class, args } => self.lower_new(class, args),
            ExprKind::NewWithConstructor {
                class,
                defaults,
                func,
                args,
            } => self.lower_new_with_constructor(class, defaults, func, args),
            ExprKind::ArrayNew { length } => {
                self.expr(length);
                self.emit(Instruction::simple(Opcode::NewArray));
            }
            ExprKind::ArrayLiteral { elements } => self.lower_array_literal(elements),
            ExprKind::ArraySpreadLiteral { parts } => self.lower_array_spread_literal(parts),
            ExprKind::FieldGet {
                object,
                class,
                field_slot,
            } => {
                self.expr(object);
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::LoadField, Operand::Token(token)));
            }
            ExprKind::FieldSet {
                object,
                class,
                field_slot,
                value,
            } => self.lower_field_set(object, class, field_slot, value),
            ExprKind::StaticFieldGet { class, field_slot } => {
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::LoadStatic, Operand::Token(token)));
            }
            ExprKind::StaticFieldSet {
                class,
                field_slot,
                value,
            } => {
                self.expr(value);
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::StoreStatic, Operand::Token(token)));
            }
            ExprKind::ArrayGet { object, index } => {
                self.expr(object);
                self.expr(index);
                self.emit(Instruction::simple(Opcode::LoadArray));
            }
            ExprKind::ArraySet {
                object,
                index,
                value,
            } => {
                self.expr(object);
                self.expr(index);
                self.expr(value);
                self.emit(Instruction::simple(Opcode::StoreArray));
            }
            ExprKind::ArrayLength { object } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::ArrayLength));
            }
            ExprKind::ArrayPop { object } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::ArrayPop));
            }
            ExprKind::ArrayPush { object, value } => {
                self.expr(object);
                self.expr(value);
                self.emit(Instruction::simple(Opcode::ArrayPush));
            }
            ExprKind::Convert { value } => self.lower_convert(value, e),
            ExprKind::Error => unreachable!("ilgen runs only on error-free programs"),
        }
    }
}
