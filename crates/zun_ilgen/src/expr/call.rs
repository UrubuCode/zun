use super::*;

impl<'a, 'b> FnLowerer<'a, 'b> {
    /// Direct call.
    pub(super) fn lower_call(&mut self, func: &hir::FuncIndex, args: &[hir::Expr]) {
        for a in args {
            self.expr(a);
        }
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(Opcode::Call, Operand::Token(token)));
    }

    /// Direct call of a rest-parameter function.
    pub(super) fn lower_call_rest(&mut self, func: &hir::FuncIndex, args: &[hir::Expr]) {
        for arg in args {
            self.expr(arg);
        }
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(
            Opcode::CallRest,
            Operand::TokenImm32(token, args.len() as i32),
        ));
    }

    /// Rest call whose rest arguments are a prebuilt array.
    pub(super) fn lower_call_rest_array(&mut self, func: &hir::FuncIndex, args: &hir::Expr) {
        self.expr(args);
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(
            Opcode::CallRestArray,
            Operand::Token(token),
        ));
    }

    /// Rest call with fixed arguments plus a prebuilt rest array.
    pub(super) fn lower_call_rest_array_fixed(&mut self, func: &hir::FuncIndex, fixed_args: &[hir::Expr], args: &hir::Expr) {
        for arg in fixed_args {
            self.expr(arg);
        }
        self.expr(args);
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(
            Opcode::CallRestArrayFixed,
            Operand::TokenImm32(token, fixed_args.len() as i32),
        ));
    }

    /// Create a generator by packing arguments into an array.
    pub(super) fn lower_generator_call(&mut self, func: &hir::FuncIndex, args: &[hir::Expr], native_id: &u32) {
        self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(*func as i32 + 1)));
        self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(args.len() as i32)));
        self.emit(Instruction::simple(Opcode::NewArray));
        for (index, arg) in args.iter().enumerate() {
            self.emit(Instruction::simple(Opcode::Dup));
            self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(index as i32)));
            self.expr(arg);
            self.emit(Instruction::simple(Opcode::StoreArray));
            self.emit(Instruction::simple(Opcode::Pop));
        }
        self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(*native_id as i32)));
    }

    /// `fn.bind(...)` through the `functionBind` builtin.
    pub(super) fn lower_function_bind(&mut self, callee: &hir::Expr, args: &[hir::Expr], native_id: &u32) {
        // Push the target function, then build the managed array of
        // prepended arguments, then invoke the `functionBind` builtin
        // (params `[Function, Array]`).
        self.expr(callee);
        self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(args.len() as i32)));
        self.emit(Instruction::simple(Opcode::NewArray));
        for (index, arg) in args.iter().enumerate() {
            self.emit(Instruction::simple(Opcode::Dup));
            self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(index as i32)));
            self.expr(arg);
            self.emit(Instruction::simple(Opcode::StoreArray));
            self.emit(Instruction::simple(Opcode::Pop));
        }
        self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(*native_id as i32)));
    }

    /// Call through a function value.
    pub(super) fn lower_indirect_call(&mut self, callee: &hir::Expr, args: &[hir::Expr], func: &hir::FuncIndex) {
        self.expr(callee);
        for a in args {
            self.expr(a);
        }
        // A dynamic function value called with other than one argument
        // (e.g. a multi-argument bound call) carries its count.
        if *func == hir::DYNAMIC_FUNCTION && args.len() != 1 {
            self.emit(Instruction::new(
                Opcode::CallIndirectN,
                Operand::Imm32(args.len() as i32),
            ));
        } else {
            let token = if *func == hir::DYNAMIC_FUNCTION || *func == hir::DYNAMIC_PREDICATE {
                Token::null(TableKind::MethodDef)
            } else if *func == hir::DYNAMIC_REDUCER {
                Token::null(TableKind::MemberRef)
            } else {
                Token::new(TableKind::MethodDef, *func + 1)
            };
            self.emit(Instruction::new(
                Opcode::CallIndirect,
                Operand::Token(token),
            ));
        }
    }

    /// Rest call through a function value.
    pub(super) fn lower_indirect_call_rest(&mut self, callee: &hir::Expr, args: &[hir::Expr], func: &hir::FuncIndex) {
        self.expr(callee);
        for arg in args {
            self.expr(arg);
        }
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(
            Opcode::CallIndirectRest,
            Operand::TokenImm32(token, args.len() as i32),
        ));
    }

    /// Optional call through a function value (`f?.()`).
    pub(super) fn lower_optional_indirect_call(&mut self, callee: &hir::Expr, args: &[hir::Expr], func: &hir::FuncIndex) {
        self.expr(callee);
        self.emit(Instruction::simple(Opcode::Dup));
        self.emit(Instruction::simple(Opcode::ConstNull));
        self.emit(Instruction::simple(Opcode::CmpEq));
        let use_null = self.emit_branch(Opcode::BranchTrue);
        self.emit(Instruction::simple(Opcode::Dup));
        self.emit(Instruction::simple(Opcode::ConstUndef));
        self.emit(Instruction::simple(Opcode::CmpEq));
        let use_undef = self.emit_branch(Opcode::BranchTrue);
        for arg in args { self.expr(arg); }
        if *func == hir::DYNAMIC_FUNCTION && args.len() != 1 {
            self.emit(Instruction::new(Opcode::CallIndirectN, Operand::Imm32(args.len() as i32)));
        } else {
            let token = if *func == hir::DYNAMIC_FUNCTION || *func == hir::DYNAMIC_PREDICATE {
                Token::null(TableKind::MethodDef)
            } else if *func == hir::DYNAMIC_REDUCER {
                Token::null(TableKind::MemberRef)
            } else { Token::new(TableKind::MethodDef, *func + 1) };
            self.emit(Instruction::new(Opcode::CallIndirect, Operand::Token(token)));
        }
        let end = self.emit_branch(Opcode::Branch);
        self.patch_to_here(use_null);
        self.patch_to_here(use_undef);
        self.emit(Instruction::simple(Opcode::Pop));
        self.emit(Instruction::simple(Opcode::ConstUndef));
        self.patch_to_here(end);
    }

    /// Native call, or a single machine instruction for `zun:math` intrinsics.
    pub(super) fn lower_call_builtin(&mut self, id: &u32, args: &[hir::Expr]) {
        for a in args {
            self.expr(a);
        }
        // A `zun:math` intrinsic that the machine can do in one
        // instruction is emitted as that instruction; everything else
        // keeps calling the native.
        match math_machine_op(*id, args.len()) {
            Some(op) => self.emit(Instruction::simple(op)),
            None => self.emit(Instruction::new(
                Opcode::CallBuiltin,
                Operand::Imm32(*id as i32),
            )),
        }
    }

    /// Call through a module/member reference with typed FFI signature bytes.
    pub(super) fn lower_external_call(&mut self, module: &str, name: &str, args: &[hir::Expr], e: &hir::Expr) {
        for a in args {
            self.expr(a);
        }
        let module_name = self.module.strings.intern(module);
        let member_name = self.module.strings.intern(name);
        let module_ref = self
            .module
            .tables
            .add_module_ref(ModuleRef { name: module_name });
        let arity = u8::try_from(args.len()).expect("external call arity exceeds metadata limit");
        // Keep arity first for the verifier, followed by scalar type
        // tags for the typed native-call boundary. Zero denotes a
        // managed/non-blittable type at the explicit stub boundary.
        let mut signature_bytes = Vec::with_capacity(args.len() + 2);
        signature_bytes.push(arity);
        signature_bytes.extend(args.iter().map(|arg| ffi_type_tag(arg.ty)));
        signature_bytes.push(ffi_type_tag(e.ty));
        let signature = self.module.blobs.intern(&signature_bytes);
        let member_ref = self.module.tables.add_member_ref(MemberRef {
            parent: module_ref,
            name: member_name,
            signature,
        });
        self.emit(Instruction::new(Opcode::Call, Operand::Token(member_ref)));
    }

    /// Virtual method call; `args[0]` is the receiver.
    pub(super) fn lower_method_call(&mut self, func: &hir::FuncIndex, args: &[hir::Expr]) {
        // args[0] is the receiver; push it and the rest, then dispatch
        // virtually on the receiver's runtime class. The token names the
        // statically resolved method (for its name and arity).
        for a in args {
            self.expr(a);
        }
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(Opcode::CallVirt, Operand::Token(token)));
    }

    /// Static method call.
    pub(super) fn lower_static_call(&mut self, func: &hir::FuncIndex, args: &[hir::Expr]) {
        for a in args {
            self.expr(a);
        }
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(Opcode::Call, Operand::Token(token)));
    }

    /// Virtual call of a rest-parameter method.
    pub(super) fn lower_method_call_rest(&mut self, func: &hir::FuncIndex, args: &[hir::Expr]) {
        for a in args {
            self.expr(a);
        }
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(
            Opcode::CallVirtRest,
            Operand::TokenImm32(token, args.len() as i32),
        ));
    }

    /// Virtual rest call with a prebuilt rest array.
    pub(super) fn lower_method_call_rest_array(&mut self, func: &hir::FuncIndex, receiver: &hir::Expr, args: &hir::Expr) {
        self.expr(receiver);
        self.expr(args);
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(Opcode::CallVirtRestArray, Operand::Token(token)));
    }

    /// Virtual rest call with fixed arguments plus a prebuilt rest array.
    pub(super) fn lower_method_call_rest_array_fixed(&mut self, func: &hir::FuncIndex, receiver: &hir::Expr, fixed_args: &[hir::Expr], args: &hir::Expr) {
        self.expr(receiver);
        for fixed in fixed_args {
            self.expr(fixed);
        }
        self.expr(args);
        let token = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(
            Opcode::CallVirtRestArrayFixed,
            Operand::TokenImm32(token, fixed_args.len() as i32),
        ));
    }

    /// A reference to a top-level function as a first-class value.
    pub(super) fn lower_function_ref(&mut self, func: &hir::FuncIndex) {
        self.emit(Instruction::new(
            Opcode::LoadFunction,
            Operand::Token(Token::new(TableKind::MethodDef, *func + 1)),
        ));
    }
}
