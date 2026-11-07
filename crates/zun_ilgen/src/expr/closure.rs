use super::*;
use crate::function::capture_name;

impl<'a, 'b> FnLowerer<'a, 'b> {
    /// Push the enclosing closure itself as a closure value.
    pub(super) fn lower_self_closure(&mut self) {
        let function = self.self_closure.expect("SelfClosure outside a closure");
        self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
        let token = Token::new(
            TableKind::MethodDef,
            self.closure_method_base + function + 1,
        );
        self.emit(Instruction::new(Opcode::MakeClosure, Operand::Token(token)));
    }

    /// Load the captured cell object from the environment.
    pub(super) fn lower_capture_ref(&mut self, environment_slot: &u16) {
        self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
        self.emit_dynamic_load(&capture_name(*environment_slot));
        self.emit(Instruction::simple(Opcode::UnboxAny));
    }

    /// Store a value into a closure cell, leaving it on the stack.
    pub(super) fn lower_cell_store(&mut self, slot: &hir::LocalSlot, value: &hir::Expr) {
        self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
        self.expr(value);
        self.emit(Instruction::simple(Opcode::BoxAny));
        self.emit_dynamic_store("$value");
        self.emit(Instruction::simple(Opcode::UnboxAny));
    }

    /// Build the capture environment and create the closure value.
    pub(super) fn lower_closure(&mut self, function: &u32, captures: &[hir::ClosureCapture]) {
        self.emit(Instruction::simple(Opcode::NewDynamic));
        for capture in captures {
            self.emit(Instruction::simple(Opcode::Dup));
            if capture.capture.mode == CaptureMode::ByCell {
                match capture.capture.source {
                    CaptureSource::Local(slot) => {
                        self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(slot)));
                        self.emit(Instruction::simple(Opcode::BoxAny));
                    }
                    CaptureSource::Environment {
                        environment_slot, ..
                    } => {
                        self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                        self.emit_dynamic_load(&capture_name(environment_slot));
                    }
                }
            } else {
                self.expr(&capture.value);
                // A function value cannot box to `Any`; it stays raw and
                // `StoreDynamic` routes it to the environment's function
                // slot. (ByCell/mutated function captures stay deferred:
                // the `$value` cell slot is an `Any` and cannot hold one.)
                if !matches!(capture.capture.ty, Ty::Function(_)) {
                    self.emit(Instruction::simple(Opcode::BoxAny));
                }
            }
            self.emit_dynamic_store(&capture_name(capture.capture.environment_slot));
            self.emit(Instruction::simple(Opcode::Pop));
        }
        let token = Token::new(
            TableKind::MethodDef,
            self.closure_method_base + *function + 1,
        );
        self.emit(Instruction::new(Opcode::MakeClosure, Operand::Token(token)));
    }

    /// Read a captured variable from the closure environment.
    pub(super) fn lower_capture_load(
        &mut self,
        environment_slot: &u16,
        mode: &CaptureMode,
        ty: &Ty,
    ) {
        self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
        self.emit_dynamic_load(&capture_name(*environment_slot));
        if *mode == CaptureMode::ByCell {
            self.emit(Instruction::simple(Opcode::UnboxAny));
            self.emit_dynamic_load("$value");
        }
        // A by-value function capture lives in the function slot and
        // `LoadDynamic` already yields `Value::Function`: no unbox.
        if *mode == CaptureMode::ByCell || !matches!(ty, Ty::Function(_)) {
            self.emit(Instruction::simple(Opcode::UnboxAny));
        }
    }

    /// Write a captured variable in the closure environment.
    pub(super) fn lower_capture_store(
        &mut self,
        environment_slot: &u16,
        mode: &CaptureMode,
        value: &hir::Expr,
    ) {
        self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
        if *mode == CaptureMode::ByCell {
            self.emit_dynamic_load(&capture_name(*environment_slot));
            self.emit(Instruction::simple(Opcode::UnboxAny));
        }
        self.expr(value);
        self.emit(Instruction::simple(Opcode::BoxAny));
        if *mode == CaptureMode::ByCell {
            self.emit_dynamic_store("$value");
        } else {
            self.emit_dynamic_store(&capture_name(*environment_slot));
        }
        self.emit(Instruction::simple(Opcode::UnboxAny));
    }

    /// Read the value out of a closure cell.
    pub(super) fn lower_cell_load(&mut self, slot: &hir::LocalSlot) {
        self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
        self.emit_dynamic_load("$value");
        self.emit(Instruction::simple(Opcode::UnboxAny));
    }
}
