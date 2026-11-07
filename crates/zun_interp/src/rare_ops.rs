//! Rare opcode arms (throw/rethrow/cast/import) extracted from the Tier 0
//! dispatch loop. Each method mirrors exactly one `match instr.opcode` arm
//! from [`Interp::call_inner`].

use super::*;
use crate::throw_native_arm;
use zun_il::Instruction;

impl Interp<'_> {
    #[inline(always)]
    pub(super) fn op_throw(
        &mut self,
        mut stack: &mut Vec<Value>,
        pc: &mut usize,
        body: &zun_module::MethodBody<'_>,
        size: usize,
    ) -> Result<ArmFlow, RunError> {
        let exception = pop(&mut stack)?;
        let throw_pc = *pc - size;
        let handler = body
            .handlers
            .iter()
            .filter(|handler| {
                handler.try_start as usize <= throw_pc && throw_pc < handler.try_end as usize
            })
            .filter(|handler| match handler.kind {
                zun_module::HandlerKind::Finally => true,
                zun_module::HandlerKind::Catch => match (handler.catch_type, &exception) {
                    (Some(token), Value::Object(object)) => {
                        self.types.is_assignable(object.type_rid(), token.rid())
                    }
                    (None, _) => true,
                    _ => false,
                },
            })
            .min_by_key(|handler| handler.try_end - handler.try_start);
        if let Some(handler) = handler {
            stack.clear();
            stack.push(exception);
            *pc = handler.handler_start as usize;
        } else {
            return Err(RunError::Thrown(exception));
        }
        Ok(ArmFlow::Next)
    }

    #[inline(always)]
    pub(super) fn op_rethrow(
        &mut self,
        mut stack: &mut Vec<Value>,
        mut pc: &mut usize,
        body: &zun_module::MethodBody<'_>,
        next: usize,
        size: usize,
    ) -> Result<ArmFlow, RunError> {
        let exception = pop(&mut stack)?;
        if self.route_exception(body, &mut stack, &mut pc, next - size, &exception) {
            return Ok(ArmFlow::Continue);
        }
        Err(RunError::Thrown(exception))
    }

    #[inline(always)]
    pub(super) fn op_cast_class(
        &mut self,
        mut stack: &mut Vec<Value>,
        mut pc: &mut usize,
        body: &zun_module::MethodBody<'_>,
        instr: Instruction,
        next: usize,
        size: usize,
    ) -> Result<ArmFlow, RunError> {
        let token = match instr.operand {
            Operand::Token(token) if token.table() == Some(zun_metadata::TableKind::TypeDef) => {
                token
            }
            _ => return Err(RunError::BadToken),
        };
        let value = pop(&mut stack)?;
        let value = if let Value::Any(any) = value {
            zun_gc::with_current_heap(|heap| value::Value::unbox_any(any, heap))
                .ok_or(RunError::BadToken)?
        } else {
            value
        };
        match value {
            Value::Null => stack.push(Value::Null),
            Value::Object(object) if self.types.is_assignable(object.type_rid(), token.rid()) => {
                stack.push(Value::Object(object));
            }
            // A receiver of the wrong class (a private-brand
            // mismatch, or a bad downcast) is a catchable TypeError.
            _ => {
                throw_native_arm!(self, body, stack, pc, next - size, "TypeError: Cannot access a private member on an object whose class did not declare it".to_string());
            }
        }
        Ok(ArmFlow::Next)
    }

    #[inline(always)]
    pub(super) fn op_import_dynamic(
        &mut self,
        mut stack: &mut Vec<Value>,
    ) -> Result<ArmFlow, RunError> {
        let specifier = match pop(&mut stack)? {
            Value::Str(specifier) => specifier,
            _ => return Err(RunError::BadToken),
        };
        stack.push(namespace::zun_dynamic_import(self.loader, &specifier)?);
        Ok(ArmFlow::Next)
    }
}
