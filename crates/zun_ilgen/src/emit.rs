use super::*;

impl<'a, 'b> FnLowerer<'a, 'b> {
    pub(super) fn emit(&mut self, instr: Instruction) {
        encode(&instr, &mut self.code);
    }

    /// Materialize omitted arguments before entering the function body. Calls
    /// use null sentinels so the framed ABI remains fixed-width.
    /// Emit the cells of closure-captured parameters (the leading `BoxedLet`s
    /// the checker put at the head of the body) ahead of the default-value
    /// prologue, so closures created by a default capture the live cell.
    /// Returns how many leading body statements were consumed.
    pub(super) fn emit_param_cells(&mut self, body: &'a [Stmt], param_count: u16) -> usize {
        let mut consumed = 0;
        for statement in body {
            match statement {
                Stmt::BoxedLet { slot, init }
                    if *slot < param_count
                        && matches!(&init.kind, ExprKind::Local(local) if local == slot) =>
                {
                    self.stmt(statement);
                    consumed += 1;
                }
                _ => break,
            }
        }
        consumed
    }

    pub(super) fn emit_defaults(
        &mut self,
        defaults: &[Option<hir::Expr>],
        slot_offset: u16,
        locals: &[zun_sema::hir::Ty],
        cells: &[Stmt],
    ) {
        let is_cell = |slot: u16| cells.iter().any(|s| matches!(s, Stmt::BoxedLet { slot: cell, .. } if *cell == slot));
        for (index, default) in defaults.iter().enumerate() {
            let Some(default) = default else { continue };
            let slot = slot_offset + index as u16;
            if is_cell(slot) {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(slot)));
                self.emit_dynamic_load("$value");
                if locals.get(slot as usize) == Some(&zun_sema::hir::Ty::Any) {
                    self.emit(Instruction::simple(Opcode::ConstUndef));
                } else {
                    self.emit(Instruction::simple(Opcode::ConstNull));
                }
                self.emit(Instruction::simple(Opcode::CmpEq));
                let skip = self.emit_branch(Opcode::BranchFalse);
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(slot)));
                self.expr(default);
                self.emit(Instruction::simple(Opcode::BoxAny));
                self.emit_dynamic_store("$value");
                self.emit(Instruction::simple(Opcode::Pop));
                self.patch_to_here(skip);
                continue;
            }
            self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(slot)));
            self.emit(Instruction::simple(Opcode::BoxAny));
            // Dynamic params use `undefined` as the omitted marker (an explicit
            // `null` keeps its value); typed params keep the null sentinel.
            if locals.get(slot as usize) == Some(&zun_sema::hir::Ty::Any) {
                self.emit(Instruction::simple(Opcode::ConstUndef));
            } else {
                self.emit(Instruction::simple(Opcode::ConstNull));
            }
            self.emit(Instruction::simple(Opcode::CmpEq));
            let skip = self.emit_branch(Opcode::BranchFalse);
            self.expr(default);
            self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(slot)));
            self.patch_to_here(skip);
        }
    }

    /// Inline the pending `finally` bodies that sit inside the target scope
    /// (those entered after `depth`), innermost first, for a `break` or
    /// `continue` that leaves them.
    pub(super) fn run_finally_to(&mut self, depth: usize, continuing: Option<usize>) {
        let saved = self.finally_bodies.clone();
        for (index, cleanup) in saved.iter().enumerate().rev() {
            if index < depth {
                break;
            }
            // `continue` on a for-of loop keeps its own iterator open.
            if let (Cleanup::IterClose { loop_index, .. }, Some(target)) = (cleanup, continuing) {
                if *loop_index == target {
                    continue;
                }
            }
            self.finally_bodies.truncate(index);
            let hole_start = self.code.len() as u32;
            self.emit_cleanup(*cleanup);
            self.finally_holes.push((hole_start, self.code.len() as u32, index));
        }
        self.finally_bodies = saved;
    }

    /// Emit one pending cleanup inline.
    pub(super) fn emit_cleanup(&mut self, cleanup: Cleanup<'a>) {
        match cleanup {
            Cleanup::Block(stmts) => self.stmts(stmts),
            Cleanup::IterClose { slot, .. } => {
                self.emit_iterator_close(slot, false);
                self.emit(Instruction::simple(Opcode::Pop));
            }
        }
    }

    /// Push the result of `generatorIteratorClose(iterator, throwing)`.
    pub(super) fn emit_iterator_close(&mut self, slot: u16, throwing: bool) {
        self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(slot)));
        self.emit(Instruction::simple(if throwing { Opcode::ConstTrue } else { Opcode::ConstFalse }));
        let (id, _) = zun_natives::resolve("buffer", "generatorIteratorClose")
            .expect("generatorIteratorClose intrinsic");
        self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(id as i32)));
    }

    /// Register a handler over `[start, end)`, minus the inlined `finally`
    /// copies (holes) owned by this try or by a try nested inside it: an
    /// exception thrown while a cleanup runs must not re-enter that cleanup.
    pub(super) fn push_protected(
        &mut self,
        entry_depth: usize,
        has_finally: bool,
        start: u32,
        end: u32,
        handler_start: u32,
        handler_end: u32,
        kind: HandlerKind,
        catch_type: Option<Token>,
    ) {
        let mut holes: Vec<(u32, u32)> = self
            .finally_holes
            .iter()
            .filter(|(hole_start, hole_end, owner)| {
                (*owner < entry_depth || (*owner == entry_depth && has_finally)) && *hole_start >= start && *hole_end <= end
            })
            .map(|(hole_start, hole_end, _)| (*hole_start, *hole_end))
            .collect();
        holes.sort_unstable();
        let mut cursor = start;
        for (hole_start, hole_end) in holes {
            if hole_start > cursor {
                self.handlers.push(ExceptionHandler {
                    try_start: cursor,
                    try_end: hole_start,
                    handler_start,
                    handler_end,
                    kind,
                    catch_type,
                });
            }
            cursor = cursor.max(hole_end);
        }
        if cursor < end {
            self.handlers.push(ExceptionHandler {
                try_start: cursor,
                try_end: end,
                handler_start,
                handler_end,
                kind,
                catch_type,
            });
        }
    }

    pub(super) fn emit_dynamic_load(&mut self, name: &str) {
        let property = self.module.strings.intern(name);
        self.emit(Instruction::new(
            Opcode::LoadDynamic,
            Operand::StringRef(property.0),
        ));
    }

    pub(super) fn emit_dynamic_store(&mut self, name: &str) {
        let property = self.module.strings.intern(name);
        self.emit(Instruction::new(
            Opcode::StoreDynamic,
            Operand::StringRef(property.0),
        ));
    }

    /// Push a numeric constant of the given type: a base `const.i32`/`const.i64`
    /// or `const.f64`, then a `conv` when the target kind is narrower/unsigned
    /// or single-precision.
    pub(super) fn emit_numeric_const(&mut self, v: f64, ty: Ty) {
        let kind = numkind_of(ty).unwrap_or(NumKind::F64);
        match kind {
            NumKind::F64 => {
                self.emit(Instruction::new(Opcode::ConstF64, Operand::Float64(v)));
            }
            NumKind::F32 => {
                self.emit(Instruction::new(Opcode::ConstF64, Operand::Float64(v)));
                self.emit(Instruction::new(
                    Opcode::Conv,
                    Operand::Imm32(NumKind::F32.to_imm()),
                ));
            }
            NumKind::I64 | NumKind::U64 => {
                self.emit(Instruction::new(Opcode::ConstI64, Operand::Imm64(v as i64)));
                if kind != NumKind::I64 {
                    self.emit(Instruction::new(
                        Opcode::Conv,
                        Operand::Imm32(kind.to_imm()),
                    ));
                }
            }
            _ => {
                // i8/i16/i32/u8/u16/u32: build as i32, then narrow if needed.
                //
                // Via i64, because `f64 as i32` saturates in Rust: a `u32`
                // literal above i32::MAX became i32::MAX, so
                // `let a: u32 = 4294967295` silently held 2147483647. Going
                // through i64 and truncating keeps the bit pattern, which the
                // following `conv` reinterprets as the target width. Sema has
                // already checked the literal fits that width.
                self.emit(Instruction::new(
                    Opcode::ConstI32,
                    Operand::Imm32(v as i64 as i32),
                ));
                if kind != NumKind::I32 {
                    self.emit(Instruction::new(
                        Opcode::Conv,
                        Operand::Imm32(kind.to_imm()),
                    ));
                }
            }
        }
    }

    /// Emit a `conv` to an explicit numeric kind.
    pub(super) fn emit_conv_kind(&mut self, kind: NumKind) {
        self.emit(Instruction::new(Opcode::Conv, Operand::Imm32(kind.to_imm())));
    }

    /// Emit the JS `ToInt32`/`ToUint32` coercion of the `number` on top of the
    /// stack. Routed through an i64 intermediate (`f64 -> i64 -> i32/u32`), not a
    /// direct `f64 -> u32`, because a direct unsigned saturating convert maps a
    /// negative operand to 0 while JS wraps it modulo 2^32 (`(-1) >>> 0` must be
    /// 4294967295). Both tiers share this IL, so they agree by construction.
    /// Values beyond the i64 range saturate rather than reduce modulo 2^32 — a
    /// documented edge, consistent with the saturating-convert policy.
    pub(super) fn emit_to_int32(&mut self, unsigned: bool) {
        // fmod by 2^32 first: the f64->i64 conversion saturates, so values at or
        // beyond 2^63 would otherwise lose their low 32 bits. The fmod is exact
        // and sends NaN/Infinity to NaN (which converts to 0).
        self.emit(Instruction::new(Opcode::ConstF64, Operand::Float64(4294967296.0)));
        self.emit(Instruction::simple(Opcode::RemF64));
        self.emit_conv_kind(NumKind::I64);
        self.emit_conv_kind(if unsigned { NumKind::U32 } else { NumKind::I32 });
    }

    /// Emit a `conv` to `ty` (used by the `as` operator).
    pub(super) fn emit_conv_to(&mut self, ty: Ty) {
        if let Some(kind) = numkind_of(ty) {
            self.emit(Instruction::new(
                Opcode::Conv,
                Operand::Imm32(kind.to_imm()),
            ));
        }
    }

    /// Emit a branch with a placeholder displacement; returns a patch handle.
    pub(super) fn emit_branch(&mut self, opcode: Opcode) -> BranchPatch {
        self.emit(Instruction::new(opcode, Operand::Branch(0)));
        BranchPatch {
            // The 4 displacement bytes sit at the end of what we just wrote.
            operand_at: self.code.len() - 4,
        }
    }

    /// Point `patch` at the current end of code.
    pub(super) fn patch_to_here(&mut self, patch: BranchPatch) {
        let disp = (self.code.len() - (patch.operand_at + 4)) as i32;
        self.code[patch.operand_at..patch.operand_at + 4].copy_from_slice(&disp.to_le_bytes());
    }

    /// Point `patch` at an arbitrary code offset.
    pub(super) fn patch_to_target(&mut self, patch: BranchPatch, target: usize) {
        let disp = (target as i64 - (patch.operand_at + 4) as i64) as i32;
        self.code[patch.operand_at..patch.operand_at + 4].copy_from_slice(&disp.to_le_bytes());
    }

    /// Emit a backward branch to `target` (a previously recorded code offset).
    pub(super) fn emit_branch_back(&mut self, opcode: Opcode, target: usize) {
        // Displacement is relative to the end of this instruction: 1 opcode
        // byte + 4 operand bytes past the current position.
        let disp = target as i64 - (self.code.len() + 5) as i64;
        self.emit(Instruction::new(opcode, Operand::Branch(disp as i32)));
    }
}
