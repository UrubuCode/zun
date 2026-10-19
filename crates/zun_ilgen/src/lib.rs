//! HIR → ZIL lowering.
//!
//! Walks the typed HIR from `zun_sema` and emits a complete `.zm`
//! [`Module`]: one `MethodDef` per function (RID = HIR index + 1, so call
//! tokens are known up front), framed bodies in the code section, and string
//! literals interned into the module string heap.
//!
//! Lowering invariants:
//! - `number` is f64 end to end: literals use `const.f64`, arithmetic the
//!   `.f64` opcodes; negation lowers as `0 - x`.
//! - Booleans lower to `const.true`/`const.false`; `!x` lowers as
//!   `x == false`.
//! - An assignment expression leaves its value on the stack (`dup` before
//!   `stloc`); expression statements `pop` any non-void result.
//! - Branches patch forward displacements after the target is known
//!   (displacements are relative to the end of the branch instruction).

use zun_il::{encode, Instruction, Opcode, Operand};
use zun_metadata::{MethodDef, TableKind, Token};
use zun_module::{write_body, Module};
use zun_sema::hir::{self, BinaryOp, Builtin, ExprKind, Stmt, Ty, UnaryOp};

/// Lower a checked program into a `.zm` module.
pub fn lower(program: &hir::Program) -> Module {
    let mut module = Module::new();

    // Reserve one MethodDef per function first so call sites can name any
    // function by token regardless of order. body_offset is patched after
    // each body is emitted.
    for f in &program.functions {
        let name = module.strings.intern(&f.name);
        module.tables.add_method_def(MethodDef {
            name,
            signature: zun_metadata::BlobIndex(0), // signatures land with the verifier
            body_offset: u32::MAX,
        });
    }

    for (index, f) in program.functions.iter().enumerate() {
        let code = lower_function(f, &mut module);
        let offset = write_body(
            &mut module.code,
            f.param_count,
            f.locals.len() as u16,
            &code,
        );
        module.tables.method_defs[index].body_offset = offset;
    }

    module.entry_point = program
        .main
        .map(|index| Token::new(TableKind::MethodDef, index + 1));
    module
}

/// Lower one function body to ZIL bytes.
fn lower_function(f: &hir::Function, module: &mut Module) -> Vec<u8> {
    let mut gen = FnLowerer {
        code: Vec::new(),
        module,
    };
    gen.stmts(&f.body);
    // Guarantee the body cannot run off its end: a void function gets an
    // implicit return. (Non-void paths are the verifier's job later.)
    if f.return_ty == Ty::Void || !matches!(f.body.last(), Some(Stmt::Return(_))) {
        gen.emit(Instruction::simple(Opcode::Ret));
    }
    gen.code
}

/// Handle to a forward branch whose displacement is patched once the target
/// offset is known.
struct BranchPatch {
    /// Byte offset of the 4-byte displacement inside the code buffer.
    operand_at: usize,
}

struct FnLowerer<'a> {
    code: Vec<u8>,
    module: &'a mut Module,
}

impl FnLowerer<'_> {
    fn emit(&mut self, instr: Instruction) {
        encode(&instr, &mut self.code);
    }

    /// Emit a branch with a placeholder displacement; returns a patch handle.
    fn emit_branch(&mut self, opcode: Opcode) -> BranchPatch {
        self.emit(Instruction::new(opcode, Operand::Branch(0)));
        BranchPatch {
            // The 4 displacement bytes sit at the end of what we just wrote.
            operand_at: self.code.len() - 4,
        }
    }

    /// Point `patch` at the current end of code.
    fn patch_to_here(&mut self, patch: BranchPatch) {
        let disp = (self.code.len() - (patch.operand_at + 4)) as i32;
        self.code[patch.operand_at..patch.operand_at + 4]
            .copy_from_slice(&disp.to_le_bytes());
    }

    /// Emit a backward branch to `target` (a previously recorded code offset).
    fn emit_branch_back(&mut self, opcode: Opcode, target: usize) {
        // Displacement is relative to the end of this instruction: 1 opcode
        // byte + 4 operand bytes past the current position.
        let disp = target as i64 - (self.code.len() + 5) as i64;
        self.emit(Instruction::new(opcode, Operand::Branch(disp as i32)));
    }

    fn stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { slot, init } => {
                self.expr(init);
                self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
            }
            Stmt::Expr(e) => {
                self.expr(e);
                if e.ty != Ty::Void {
                    self.emit(Instruction::simple(Opcode::Pop));
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                self.expr(cond);
                let to_else = self.emit_branch(Opcode::BranchFalse);
                self.stmts(then_body);
                if else_body.is_empty() {
                    self.patch_to_here(to_else);
                } else {
                    let to_end = self.emit_branch(Opcode::Branch);
                    self.patch_to_here(to_else);
                    self.stmts(else_body);
                    self.patch_to_here(to_end);
                }
            }
            Stmt::While { cond, body } => {
                let loop_start = self.code.len();
                self.expr(cond);
                let to_exit = self.emit_branch(Opcode::BranchFalse);
                self.stmts(body);
                self.emit_branch_back(Opcode::Branch, loop_start);
                self.patch_to_here(to_exit);
            }
            Stmt::Return(value) => {
                if let Some(v) = value {
                    self.expr(v);
                }
                self.emit(Instruction::simple(Opcode::Ret));
            }
        }
    }

    fn expr(&mut self, e: &hir::Expr) {
        match &e.kind {
            ExprKind::Number(v) => {
                self.emit(Instruction::new(Opcode::ConstF64, Operand::Float64(*v)));
            }
            ExprKind::String(s) => {
                let idx = self.module.strings.intern(s);
                self.emit(Instruction::new(Opcode::ConstStr, Operand::StringRef(idx.0)));
            }
            ExprKind::Boolean(true) => self.emit(Instruction::simple(Opcode::ConstTrue)),
            ExprKind::Boolean(false) => self.emit(Instruction::simple(Opcode::ConstFalse)),
            ExprKind::Local(slot) => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
            }
            ExprKind::Assign { slot, value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
            }
            ExprKind::Binary { op, lhs, rhs } => {
                self.expr(lhs);
                self.expr(rhs);
                let opcode = match op {
                    // Sema guarantees Add is number+number or string+string.
                    BinaryOp::Add if lhs.ty == Ty::String => Opcode::Concat,
                    BinaryOp::Add => Opcode::AddF64,
                    BinaryOp::Sub => Opcode::SubF64,
                    BinaryOp::Mul => Opcode::MulF64,
                    BinaryOp::Div => Opcode::DivF64,
                    BinaryOp::Rem => Opcode::RemF64,
                    BinaryOp::Eq => Opcode::CmpEq,
                    BinaryOp::Ne => Opcode::CmpNe,
                    BinaryOp::Lt => Opcode::CmpLt,
                    BinaryOp::Le => Opcode::CmpLe,
                    BinaryOp::Gt => Opcode::CmpGt,
                    BinaryOp::Ge => Opcode::CmpGe,
                };
                self.emit(Instruction::simple(opcode));
            }
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Neg => {
                    // 0 - x
                    self.emit(Instruction::new(Opcode::ConstF64, Operand::Float64(0.0)));
                    self.expr(operand);
                    self.emit(Instruction::simple(Opcode::SubF64));
                }
                UnaryOp::Not => {
                    // x == false
                    self.expr(operand);
                    self.emit(Instruction::simple(Opcode::ConstFalse));
                    self.emit(Instruction::simple(Opcode::CmpEq));
                }
            },
            ExprKind::Call { func, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::Call, Operand::Token(token)));
            }
            ExprKind::CallBuiltin { builtin, args } => {
                for a in args {
                    self.expr(a);
                }
                let id = match builtin {
                    Builtin::Print => 0,
                };
                self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(id)));
            }
            ExprKind::Error => unreachable!("ilgen runs only on error-free programs"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zun_il::disassemble;
    use zun_module::read_body;

    fn lower_src(src: &str) -> Module {
        let mut diags = Vec::new();
        let ast = zun_syntax::parse(src, &mut diags);
        assert!(diags.is_empty(), "parse: {diags:?}");
        let hir = zun_sema::check(src, &ast, &mut diags);
        assert!(diags.is_empty(), "sema: {diags:?}");
        lower(&hir)
    }

    #[test]
    fn simple_function_disassembles_as_expected() {
        let module = lower_src("function main() { let x = 1; print(x); }");
        let body = read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
        let text = disassemble(body.code).unwrap();
        assert_eq!(
            text,
            "0000: const.f64 1\n\
             0009: stloc 0\n\
             000C: ldloc 0\n\
             000F: call.builtin 0\n\
             0014: ret\n"
        );
        assert_eq!(body.local_count, 1);
        assert!(module.entry_point.is_some());
    }

    #[test]
    fn while_loop_branches_point_at_the_right_places() {
        let module = lower_src(
            "function main() { let n = 3; while (n > 0) { n = n - 1; } }",
        );
        let body = read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
        let text = disassemble(body.code).unwrap();
        assert_eq!(
            text,
            "0000: const.f64 3\n\
             0009: stloc 0\n\
             000C: ldloc 0\n\
             000F: const.f64 0\n\
             0018: cmp.gt\n\
             0019: br.false +23\n\
             001E: ldloc 0\n\
             0021: const.f64 1\n\
             002A: sub.f64\n\
             002B: dup\n\
             002C: stloc 0\n\
             002F: pop\n\
             0030: br -41\n\
             0035: ret\n"
        );
    }

    #[test]
    fn if_else_falls_through_correctly() {
        let module = lower_src(
            "function main() { if (true) { print(\"a\"); } else { print(\"b\"); } }",
        );
        let body = read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
        let text = disassemble(body.code).unwrap();
        assert_eq!(
            text,
            "0000: const.true\n\
             0001: br.false +15\n\
             0006: const.str str:6\n\
             000B: call.builtin 0\n\
             0010: br +10\n\
             0015: const.str str:8\n\
             001A: call.builtin 0\n\
             001F: ret\n"
        );
    }

    #[test]
    fn string_concat_and_user_calls_lower() {
        let module = lower_src(
            "function greet(who: string): string { return \"hi \" + who; }\n\
             function main() { print(greet(\"zun\")); }",
        );
        // main is function index 1 -> MethodDef RID 2.
        let body = read_body(&module.code, module.tables.method_defs[1].body_offset).unwrap();
        let text = disassemble(body.code).unwrap();
        assert!(text.contains("call MethodDef[1]"), "{text}");

        let greet = read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
        assert!(disassemble(greet.code).unwrap().contains("concat"));
        assert_eq!(
            module.entry_point,
            Some(Token::new(TableKind::MethodDef, 2))
        );
    }
}
