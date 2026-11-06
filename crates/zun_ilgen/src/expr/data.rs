use super::*;

impl<'a, 'b> FnLowerer<'a, 'b> {
    /// A static regex test, via the compiled matcher or the runtime fallback.
    pub(super) fn lower_regex_match(&mut self, matcher: &u32, args: &[hir::Expr]) {
        // `self.regex_plan` is a borrow independent of `self`; bind it
        // locally so the arm can also borrow `self` mutably to emit.
        let plan = self.regex_plan;
        match &plan[*matcher as usize] {
            RegexPlan::Compiled { is_match_rid } => {
                let rid = *is_match_rid;
                self.expr(&args[0]); // subject string
                self.emit(Instruction::new(
                    Opcode::Call,
                    Operand::Token(Token::new(TableKind::MethodDef, rid)),
                ));
            }
            RegexPlan::Fallback { regexp_new_id, is_match_id, pattern, flags } => {
                // regexpIsMatch(regexpNew(pattern, flags), subject) —
                // identical to the ordinary (non-static) regex path.
                let (new_id, match_id) = (*regexp_new_id, *is_match_id);
                let p = self.module.strings.intern(pattern);
                self.emit(Instruction::new(Opcode::ConstStr, Operand::StringRef(p.0)));
                let f = self.module.strings.intern(flags);
                self.emit(Instruction::new(Opcode::ConstStr, Operand::StringRef(f.0)));
                self.emit(Instruction::new(
                    Opcode::CallBuiltin,
                    Operand::Imm32(new_id as i32),
                ));
                self.expr(&args[0]); // subject string
                self.emit(Instruction::new(
                    Opcode::CallBuiltin,
                    Operand::Imm32(match_id as i32),
                ));
            }
        }
    }

    /// Allocate an object, passing the arguments through to `newobj`.
    pub(super) fn lower_new(&mut self, class: &hir::ClassIndex, args: &[hir::Expr]) {
        for a in args {
            self.expr(a);
        }
        let token = self.layout.type_tokens[*class as usize];
        self.emit(Instruction::new(Opcode::NewObj, Operand::Token(token)));
    }

    /// Allocate with default field values, then run the constructor.
    pub(super) fn lower_new_with_constructor(&mut self, class: &hir::ClassIndex, defaults: &[hir::Expr], func: &hir::FuncIndex, args: &[hir::Expr]) {
        // Allocate with default field values, then run the constructor
        // on a duplicate of the instance handle. Every call leaves one
        // value (void included), so discard it; the instance remains.
        for d in defaults {
            self.expr(d);
        }
        let token = self.layout.type_tokens[*class as usize];
        self.emit(Instruction::new(Opcode::NewObj, Operand::Token(token)));
        self.emit(Instruction::simple(Opcode::Dup));
        for a in args {
            self.expr(a);
        }
        let ctor = Token::new(TableKind::MethodDef, *func + 1);
        self.emit(Instruction::new(Opcode::CallVirt, Operand::Token(ctor)));
        self.emit(Instruction::simple(Opcode::Pop));
    }

    /// Array literal of fixed length, with holes.
    pub(super) fn lower_array_literal(&mut self, elements: &[Option<hir::Expr>]) {
        // Allocate an array of fixed length, then store each element on
        // a duplicated handle at its positional index, discarding the
        // value `StoreArray` leaves behind. The array stays on the stack
        // as the literal's result. `StoreArray` boxes each value to Any.
        let length = i32::try_from(elements.len())
            .expect("array literal length exceeds i32 range");
        self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(length)));
        self.emit(Instruction::simple(Opcode::NewArray));
        for (index, element) in elements.iter().enumerate() {
            // A hole leaves the pre-allocated slot alone except that we
            // overwrite the default fill with `undefined`, so a hole
            // reads as `undefined` (not `0`) while still counting toward
            // `length`.
            let index = i32::try_from(index)
                .expect("array literal index exceeds i32 range");
            self.emit(Instruction::simple(Opcode::Dup));
            self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(index)));
            match element {
                Some(element) => {
                    self.expr(element);
                    self.emit(Instruction::simple(Opcode::StoreArray));
                    self.emit(Instruction::simple(Opcode::Pop));
                }
                None => {
                    // An elision: mark the slot as a hole (reads as
                    // `undefined`, but is absent from own-key enumeration).
                    let (id, _) = zun_natives::resolve("collections", "arrayMakeHole")
                        .expect("arrayMakeHole intrinsic");
                    self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(id as i32)));
                    self.emit(Instruction::simple(Opcode::Pop));
                }
            }
        }
    }

    /// Array literal containing holes and/or spreads.
    pub(super) fn lower_array_spread_literal(&mut self, parts: &[hir::ArrayPart]) {
        // Build a growable array (length 0) and append each part in
        // source order: a plain element and a hole each push one value
        // (`undefined` for a hole); a spread splices the iterable
        // source's elements in via `arraySpreadAppend`.
        self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(0)));
        self.emit(Instruction::simple(Opcode::NewArray));
        for part in parts {
            self.emit(Instruction::simple(Opcode::Dup));
            match part {
                hir::ArrayPart::Element(element) => {
                    self.expr(element);
                    self.emit(Instruction::simple(Opcode::ArrayPush));
                }
                hir::ArrayPart::Hole => {
                    let (id, _) = zun_natives::resolve("collections", "arrayPushHole")
                        .expect("arrayPushHole intrinsic");
                    self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(id as i32)));
                }
                hir::ArrayPart::Spread { builtin, source } => {
                    self.expr(source);
                    self.emit(Instruction::new(
                        Opcode::CallBuiltin,
                        Operand::Imm32(*builtin as i32),
                    ));
                }
            }
            // Discard the per-part result (push length / native return),
            // leaving the array as the literal's value.
            self.emit(Instruction::simple(Opcode::Pop));
        }
    }

    /// Instance field store; leaves the stored value.
    pub(super) fn lower_field_set(&mut self, object: &hir::Expr, class: &hir::ClassIndex, field_slot: &u16, value: &hir::Expr) {
        // [object, value] -> stfld -> [value]. stfld leaves the stored
        // value so the assignment yields it; in statement position the
        // always-pop rule discards it.
        self.expr(object);
        self.expr(value);
        let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
        self.emit(Instruction::new(Opcode::StoreField, Operand::Token(token)));
    }

    /// A string literal, interned into the module string heap.
    pub(super) fn lower_string_literal(&mut self, s: &str) {
        let idx = self.module.strings.intern(s);
        self.emit(Instruction::new(
            Opcode::ConstStr,
            Operand::StringRef(idx.0),
        ));
    }
}
