use super::*;

impl<'a, 'b> FnLowerer<'a, 'b> {
    /// Read a named property of a dynamic object.
    pub(super) fn lower_dynamic_get(&mut self, object: &hir::Expr, name: &str) {
        self.expr(object);
        let property = self.module.strings.intern(name);
        self.emit(Instruction::new(
            Opcode::LoadDynamic,
            Operand::StringRef(property.0),
        ));
    }

    /// Optional named property read (`o?.name`).
    pub(super) fn lower_optional_dynamic_get(&mut self, object: &hir::Expr, name: &str) {
        self.expr(object);
        self.emit(Instruction::simple(Opcode::Dup));
        self.emit(Instruction::simple(Opcode::ConstNull));
        self.emit(Instruction::simple(Opcode::CmpEq));
        let use_null = self.emit_branch(Opcode::BranchTrue);
        self.emit(Instruction::simple(Opcode::Dup));
        self.emit(Instruction::simple(Opcode::ConstUndef));
        self.emit(Instruction::simple(Opcode::CmpEq));
        let use_undef = self.emit_branch(Opcode::BranchTrue);
        let property = self.module.strings.intern(name);
        self.emit(Instruction::new(
            Opcode::LoadDynamic,
            Operand::StringRef(property.0),
        ));
        let end = self.emit_branch(Opcode::Branch);
        self.patch_to_here(use_null);
        self.patch_to_here(use_undef);
        self.emit(Instruction::simple(Opcode::Pop));
        self.emit(Instruction::simple(Opcode::ConstUndef));
        self.patch_to_here(end);
    }

    /// Optional computed property read (`o?.[key]`).
    pub(super) fn lower_optional_dynamic_get_value(&mut self, object: &hir::Expr, key: &hir::Expr) {
        self.expr(object);
        self.emit(Instruction::simple(Opcode::Dup));
        self.emit(Instruction::simple(Opcode::ConstNull));
        self.emit(Instruction::simple(Opcode::CmpEq));
        let use_null = self.emit_branch(Opcode::BranchTrue);
        self.emit(Instruction::simple(Opcode::Dup));
        self.emit(Instruction::simple(Opcode::ConstUndef));
        self.emit(Instruction::simple(Opcode::CmpEq));
        let use_undef = self.emit_branch(Opcode::BranchTrue);
        self.expr(key);
        self.emit(Instruction::simple(Opcode::LoadDynamicValue));
        let end = self.emit_branch(Opcode::Branch);
        self.patch_to_here(use_null);
        self.patch_to_here(use_undef);
        self.emit(Instruction::simple(Opcode::Pop));
        self.emit(Instruction::simple(Opcode::ConstUndef));
        self.patch_to_here(end);
    }

    /// Optional native call, skipped when the receiver is null/undefined.
    pub(super) fn lower_optional_call_builtin(&mut self, id: &u32, args: &[hir::Expr]) {
        self.expr(&args[0]);
        self.emit(Instruction::simple(Opcode::Dup));
        self.emit(Instruction::simple(Opcode::ConstNull));
        self.emit(Instruction::simple(Opcode::CmpEq));
        let use_null = self.emit_branch(Opcode::BranchTrue);
        self.emit(Instruction::simple(Opcode::Dup));
        self.emit(Instruction::simple(Opcode::ConstUndef));
        self.emit(Instruction::simple(Opcode::CmpEq));
        let use_undef = self.emit_branch(Opcode::BranchTrue);
        for arg in &args[1..] {
            self.expr(arg);
        }
        self.emit(Instruction::new(
            Opcode::CallBuiltin,
            Operand::Imm32(*id as i32),
        ));
        let end = self.emit_branch(Opcode::Branch);
        self.patch_to_here(use_null);
        self.patch_to_here(use_undef);
        self.emit(Instruction::simple(Opcode::Pop));
        self.emit(Instruction::simple(Opcode::ConstUndef));
        self.patch_to_here(end);
    }

    /// Store a named property on a dynamic object.
    pub(super) fn lower_dynamic_set(&mut self, object: &hir::Expr, name: &str, value: &hir::Expr) {
        self.expr(object);
        self.expr(value);
        let property = self.module.strings.intern(name);
        self.emit(Instruction::new(
            Opcode::StoreDynamic,
            Operand::StringRef(property.0),
        ));
    }

    /// Object literal: allocate once, then set each entry on a duplicated handle.
    pub(super) fn lower_dynamic_object_literal(&mut self, entries: &[hir::ObjectEntry]) {
        // Allocate once, then set each property on a duplicated handle,
        // discarding the value the store leaves behind. The object stays
        // on the stack as the literal's result.
        self.emit(Instruction::simple(Opcode::NewDynamic));
        for entry in entries {
            self.emit(Instruction::simple(Opcode::Dup));
            match entry {
                hir::ObjectEntry::Data { key, value } => match key {
                    hir::ObjectKey::Named(name) => {
                        self.expr(value);
                        let property = self.module.strings.intern(name);
                        self.emit(Instruction::new(
                            Opcode::StoreDynamic,
                            Operand::StringRef(property.0),
                        ));
                    }
                    hir::ObjectKey::Computed(key) => {
                        self.expr(key);
                        self.expr(value);
                        self.emit(Instruction::simple(Opcode::StoreDynamicValue));
                    }
                },
                hir::ObjectEntry::Install { builtin, args } => {
                    // The object is the dup'd handle (arg0); push the
                    // remaining args, call the native, and discard its
                    // result so the object stays as the literal's value.
                    for arg in args {
                        self.expr(arg);
                    }
                    self.emit(Instruction::new(
                        Opcode::CallBuiltin,
                        Operand::Imm32(*builtin as i32),
                    ));
                }
            }
            self.emit(Instruction::simple(Opcode::Pop));
        }
    }
}
