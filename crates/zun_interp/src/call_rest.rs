//! Cold rest/spread call opcode arms extracted from the Tier 0 dispatch loop.
//!
//! Each method mirrors exactly one `match instr.opcode` arm from
//! [`Interp::call_inner`]; the live `stack`/`locals`/`pc` borrows and every
//! `RootLink` are passed through unchanged so GC safepoints stay precise.

use super::*;
use zun_il::Instruction;

impl Interp<'_> {
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(super) fn op_call_rest(
        &mut self,
        stack: &mut Vec<Value>,
        pc: &mut usize,
        locals: &mut Vec<Value>,
        body: &zun_module::MethodBody<'_>,
        instr: Instruction,
        next: usize,
        size: usize,
        method_rid: u32,
        parent: Option<&RootLink>,
    ) -> Result<ArmFlow, RunError> {
        let (token, argument_count) = operand_token_imm32(&instr.operand);
        if token.table() != Some(TableKind::MethodDef) || argument_count < 0 {
            return Err(RunError::BadToken);
        }
        let callee_rid = token.rid();
        let callee = self
            .module
            .tables
            .method_defs
            .get(callee_rid as usize - 1)
            .ok_or(RunError::BadToken)?;
        let callee_body = read_body(&self.module.code, callee.body_offset)
            .map_err(|e| RunError::Malformed(e.to_string()))?;
        let rest_index = callee_body
            .param_count
            .checked_sub(1)
            .ok_or(RunError::BadToken)?;
        let argument_count = argument_count as usize;
        if argument_count < rest_index as usize {
            return Err(RunError::BadToken);
        }
        let values = pop_n(stack, argument_count)?;
        let rest_values = &values[rest_index as usize..];
        let rest_array = zun_gc::with_current_heap_mut(|heap| -> Result<_, RunError> {
            let array = heap.alloc_array(rest_values.len());
            for (index, value) in rest_values.iter().enumerate() {
                let any = value.box_any(heap).ok_or(RunError::BadToken)?;
                let _ = heap.array_set(array, index, any);
            }
            Ok(array)
        })?;
        let mut args = values[..rest_index as usize].to_vec();
        args.push(Value::Object(rest_array));
        let link = RootLink {
            stack: std::ptr::from_mut(stack.as_mut_slice()),
            locals: std::ptr::from_mut(locals.as_mut_slice()),
            parent,
        };
        let result = match self.dispatch_outcome(callee_rid, args, Some(&link)) {
            ExecutionOutcome::Complete(value) => value,
            ExecutionOutcome::Thrown(exception) => {
                if self.route_exception(body, stack, pc, next - size, &exception) {
                    return Ok(ArmFlow::Continue);
                }
                return Err(RunError::Thrown(exception));
            }
            ExecutionOutcome::Fault(error) => return Err(error),
            ExecutionOutcome::Suspended(frame) => {
                return Err(self.propagate_suspended(
                    frame,
                    ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                ));
            }
        };
        stack.push(result);
        Ok(ArmFlow::Next)
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(super) fn op_call_rest_array(
        &mut self,
        stack: &mut Vec<Value>,
        pc: &mut usize,
        locals: &mut Vec<Value>,
        body: &zun_module::MethodBody<'_>,
        instr: Instruction,
        next: usize,
        size: usize,
        method_rid: u32,
        parent: Option<&RootLink>,
    ) -> Result<ArmFlow, RunError> {
        let token = operand_token(&instr.operand);
        if token.table() != Some(TableKind::MethodDef) {
            return Err(RunError::BadToken);
        }
        let callee_rid = token.rid();
        let callee = self
            .module
            .tables
            .method_defs
            .get(callee_rid as usize - 1)
            .ok_or(RunError::BadToken)?;
        let callee_body = read_body(&self.module.code, callee.body_offset)
            .map_err(|e| RunError::Malformed(e.to_string()))?;
        if callee_body.param_count != 1 {
            return Err(RunError::BadToken);
        }
        let array = pop(stack)?.as_object();
        let args = vec![Value::Object(array)];
        let link = RootLink {
            stack: std::ptr::from_mut(stack.as_mut_slice()),
            locals: std::ptr::from_mut(locals.as_mut_slice()),
            parent,
        };
        let result = match self.dispatch_outcome(callee_rid, args, Some(&link)) {
            ExecutionOutcome::Complete(value) => value,
            ExecutionOutcome::Thrown(exception) => {
                if self.route_exception(body, stack, pc, next - size, &exception) {
                    return Ok(ArmFlow::Continue);
                }
                return Err(RunError::Thrown(exception));
            }
            ExecutionOutcome::Fault(error) => return Err(error),
            ExecutionOutcome::Suspended(frame) => {
                return Err(self.propagate_suspended(
                    frame,
                    ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                ));
            }
        };
        stack.push(result);
        Ok(ArmFlow::Next)
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(super) fn op_call_rest_array_fixed(
        &mut self,
        stack: &mut Vec<Value>,
        pc: &mut usize,
        locals: &mut Vec<Value>,
        body: &zun_module::MethodBody<'_>,
        instr: Instruction,
        next: usize,
        size: usize,
        method_rid: u32,
        parent: Option<&RootLink>,
    ) -> Result<ArmFlow, RunError> {
        let (token, fixed_count) = operand_token_imm32(&instr.operand);
        if token.table() != Some(TableKind::MethodDef) || fixed_count < 0 {
            return Err(RunError::BadToken);
        }
        let callee_rid = token.rid();
        let callee = self
            .module
            .tables
            .method_defs
            .get(callee_rid as usize - 1)
            .ok_or(RunError::BadToken)?;
        let callee_body = read_body(&self.module.code, callee.body_offset)
            .map_err(|e| RunError::Malformed(e.to_string()))?;
        let fixed_count = fixed_count as usize;
        if callee_body.param_count != fixed_count as u16 + 1 {
            return Err(RunError::BadToken);
        }
        let mut args = pop_n(stack, fixed_count + 1)?;
        let array = args.pop().ok_or(RunError::BadToken)?.as_object();
        args.push(Value::Object(array));
        let link = RootLink {
            stack: std::ptr::from_mut(stack.as_mut_slice()),
            locals: std::ptr::from_mut(locals.as_mut_slice()),
            parent,
        };
        let result = match self.dispatch_outcome(callee_rid, args, Some(&link)) {
            ExecutionOutcome::Complete(value) => value,
            ExecutionOutcome::Thrown(exception) => {
                if self.route_exception(body, stack, pc, next - size, &exception) {
                    return Ok(ArmFlow::Continue);
                }
                return Err(RunError::Thrown(exception));
            }
            ExecutionOutcome::Fault(error) => return Err(error),
            ExecutionOutcome::Suspended(frame) => {
                return Err(self.propagate_suspended(
                    frame,
                    ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                ));
            }
        };
        stack.push(result);
        Ok(ArmFlow::Next)
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(super) fn op_call_indirect_rest(
        &mut self,
        stack: &mut Vec<Value>,
        pc: &mut usize,
        locals: &mut Vec<Value>,
        body: &zun_module::MethodBody<'_>,
        instr: Instruction,
        next: usize,
        size: usize,
        method_rid: u32,
        parent: Option<&RootLink>,
    ) -> Result<ArmFlow, RunError> {
        let (token, argument_count) = operand_token_imm32(&instr.operand);
        if token.table() != Some(TableKind::MethodDef) || argument_count < 0 {
            return Err(RunError::BadToken);
        }
        let argument_count = argument_count as usize;
        let values = pop_n(stack, argument_count)?;
        let function = pop(stack)?;
        let Value::Function(handle) = function else {
            return Err(RunError::BadToken);
        };
        if handle.method_rid() != token.rid() {
            return Err(RunError::BadToken);
        }
        let environment = handle
            .environment()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
        let target = self
            .module
            .tables
            .method_defs
            .get(handle.method_rid() as usize - 1)
            .ok_or(RunError::BadToken)?;
        let target_body = read_body(&self.module.code, target.body_offset)
            .map_err(|e| RunError::Malformed(e.to_string()))?;
        let hidden = usize::from(environment.is_some());
        let rest_index = target_body
            .param_count
            .checked_sub((hidden + 1) as u16)
            .ok_or(RunError::BadToken)? as usize;
        if argument_count < rest_index {
            return Err(RunError::BadToken);
        }
        let rest_values = &values[rest_index..];
        let rest_array = zun_gc::with_current_heap_mut(|heap| {
            let array = heap.alloc_array(rest_values.len());
            for (index, value) in rest_values.iter().enumerate() {
                let any = value.box_any(heap).ok_or(RunError::BadToken)?;
                let _ = heap.array_set(array, index, any);
            }
            Ok::<_, RunError>(array)
        })?;
        let mut call_args = Vec::with_capacity(hidden + rest_index + 1);
        if let Some(environment) = environment {
            call_args.push(Value::Object(environment));
        }
        call_args.extend(values[..rest_index].iter().cloned());
        call_args.push(Value::Object(rest_array));
        let link = RootLink {
            stack: std::ptr::from_mut(stack.as_mut_slice()),
            locals: std::ptr::from_mut(locals.as_mut_slice()),
            parent,
        };
        let result = match self.dispatch_outcome(handle.method_rid(), call_args, Some(&link)) {
            ExecutionOutcome::Complete(value) => value,
            ExecutionOutcome::Thrown(exception) => {
                if self.route_exception(body, stack, pc, next - size, &exception) {
                    return Ok(ArmFlow::Continue);
                }
                return Err(RunError::Thrown(exception));
            }
            ExecutionOutcome::Fault(error) => return Err(error),
            ExecutionOutcome::Suspended(frame) => {
                return Err(self.propagate_suspended(
                    frame,
                    ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                ));
            }
        };
        stack.push(result);
        Ok(ArmFlow::Next)
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(super) fn op_call_virt_rest(
        &mut self,
        stack: &mut Vec<Value>,
        pc: &mut usize,
        locals: &mut Vec<Value>,
        body: &zun_module::MethodBody<'_>,
        instr: Instruction,
        next: usize,
        size: usize,
        method_rid: u32,
        parent: Option<&RootLink>,
    ) -> Result<ArmFlow, RunError> {
        let (token, argument_count) = operand_token_imm32(&instr.operand);
        if token.table() != Some(TableKind::MethodDef) || argument_count < 0 {
            return Err(RunError::BadToken);
        }
        let static_method = self
            .module
            .tables
            .method_defs
            .get(token.rid() as usize - 1)
            .ok_or(RunError::BadToken)?;
        let static_body = read_body(&self.module.code, static_method.body_offset)
            .map_err(|e| RunError::Malformed(e.to_string()))?;
        let rest_index = static_body
            .param_count
            .checked_sub(1)
            .ok_or(RunError::BadToken)? as usize;
        let argument_count = argument_count as usize;
        if argument_count < rest_index {
            return Err(RunError::BadToken);
        }
        let values = pop_n(stack, argument_count)?;
        let primitive_receiver = matches!(values.first(), Some(Value::Str(_) | Value::Float(_, _)))
            || matches!(
                values.first(),
                Some(Value::Object(object))
                    if zun_gc::with_current_heap(|heap| heap.array_len(*object).is_some())
            );
        let target = if primitive_receiver {
            token.rid()
        } else {
            let type_rid = values
                .first()
                .ok_or(RunError::BadToken)?
                .as_object()
                .type_rid();
            self.types
                .resolve_virtual(type_rid, token.rid())
                .ok_or(RunError::BadToken)?
        };
        let rest_values = &values[rest_index..];
        let rest_array = zun_gc::with_current_heap_mut(|heap| -> Result<_, RunError> {
            let array = heap.alloc_array(rest_values.len());
            for (index, value) in rest_values.iter().enumerate() {
                let any = value.box_any(heap).ok_or(RunError::BadToken)?;
                let _ = heap.array_set(array, index, any);
            }
            Ok(array)
        })?;
        let mut args = values[..rest_index].to_vec();
        args.push(Value::Object(rest_array));
        let link = RootLink {
            stack: std::ptr::from_mut(stack.as_mut_slice()),
            locals: std::ptr::from_mut(locals.as_mut_slice()),
            parent,
        };
        let result = match self.dispatch_outcome(target, args, Some(&link)) {
            ExecutionOutcome::Complete(value) => value,
            ExecutionOutcome::Thrown(exception) => {
                if self.route_exception(body, stack, pc, next - size, &exception) {
                    return Ok(ArmFlow::Continue);
                }
                return Err(RunError::Thrown(exception));
            }
            ExecutionOutcome::Fault(error) => return Err(error),
            ExecutionOutcome::Suspended(frame) => {
                return Err(self.propagate_suspended(
                    frame,
                    ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                ));
            }
        };
        stack.push(result);
        Ok(ArmFlow::Next)
    }
}
