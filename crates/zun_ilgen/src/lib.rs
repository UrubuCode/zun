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

use zun_il::{encode, Instruction, NumKind, Opcode, Operand};
use zun_metadata::{
    BlobIndex, FieldDef, MemberRef, MethodDef, ModuleRef, TableKind, Token, TypeDef,
};
use zun_module::{write_body_with_handlers, ExceptionHandler, HandlerKind, Module};
use zun_sema::hir::{self, BinaryOp, ExprKind, FloatKind, IntKind, Stmt, Ty, UnaryOp};

/// Map a checked numeric type to the runtime [`NumKind`], or `None` if the type
/// is not numeric.
fn numkind_of(ty: Ty) -> Option<NumKind> {
    Some(match ty {
        Ty::Int(IntKind {
            bits: 8,
            signed: true,
        }) => NumKind::I8,
        Ty::Int(IntKind {
            bits: 16,
            signed: true,
        }) => NumKind::I16,
        Ty::Int(IntKind {
            bits: 32,
            signed: true,
        }) => NumKind::I32,
        Ty::Int(IntKind {
            bits: 64,
            signed: true,
        }) => NumKind::I64,
        Ty::Int(IntKind {
            bits: 8,
            signed: false,
        }) => NumKind::U8,
        Ty::Int(IntKind {
            bits: 16,
            signed: false,
        }) => NumKind::U16,
        Ty::Int(IntKind {
            bits: 32,
            signed: false,
        }) => NumKind::U32,
        Ty::Int(IntKind {
            bits: 64,
            signed: false,
        }) => NumKind::U64,
        Ty::Int(_) => return None,
        Ty::Float(FloatKind::F32) => NumKind::F32,
        Ty::Float(FloatKind::F64) => NumKind::F64,
        _ => return None,
    })
}

/// Tokens a lowered program needs to reference its own metadata.
struct Layout {
    /// TypeDef token per class index.
    type_tokens: Vec<Token>,
    /// FieldDef token per (class index, field slot).
    field_tokens: Vec<Vec<Token>>,
}

/// Lower a checked program into a `.zm` module.
pub fn lower(program: &hir::Program) -> Module {
    let mut module = Module::new();
    let layout = emit_classes(program, &mut module);

    // Reserve one MethodDef per function first so call sites can name any
    // function by token regardless of order. body_offset is patched after
    // each body is emitted.
    for f in &program.functions {
        let name = module.strings.intern(&f.name);
        module.tables.add_method_def(MethodDef {
            name,
            signature: BlobIndex(0), // signatures land with the verifier
            body_offset: u32::MAX,
        });
    }

    for (index, f) in program.functions.iter().enumerate() {
        let (code, handlers) = lower_function(f, &layout, &mut module);
        let local_kinds: Vec<u8> = f.locals.iter().map(|t| jitkind_byte(*t)).collect();
        let offset = write_body_with_handlers(
            &mut module.code,
            f.param_count,
            f.locals.len() as u16,
            f.return_ty != Ty::Void,
            jitkind_byte(f.return_ty),
            &local_kinds,
            &code,
            &handlers,
        );
        module.tables.method_defs[index].body_offset = offset;
    }

    module.entry_point = program
        .main
        .map(|index| Token::new(TableKind::MethodDef, index + 1));
    module
}

/// The method-body value-type code ([`zun_il::JitKind`]) for a checked type.
fn jitkind_byte(ty: Ty) -> u8 {
    use zun_il::JitKind;
    let kind = match numkind_of(ty) {
        Some(n) => JitKind::Num(n),
        None if ty == Ty::Boolean => JitKind::Bool,
        None if matches!(ty, Ty::Instance(_) | Ty::Array) => JitKind::Object,
        None => JitKind::Other,
    };
    kind.to_byte()
}

/// The field value-type code: like [`jitkind_byte`] but with a distinct string
/// code (12) so the runtime reconstructs the right `Value` from a raw slot
/// (see `zun_interp::value`). Objects share code 11 with `JitKind::Object`.
fn field_kind_byte(ty: Ty) -> u8 {
    match ty {
        Ty::String => 12, // FIELD_STRING
        _ => jitkind_byte(ty),
    }
}

/// Choose between the integer and float opcode for a numeric operation.
fn int_or_float(is_int: bool, int_op: Opcode, float_op: Opcode) -> Opcode {
    if is_int {
        int_op
    } else {
        float_op
    }
}

/// Total number of methods across all classes.
fn total_methods(program: &hir::Program) -> usize {
    program.classes.iter().map(|c| c.methods.len()).sum()
}

/// Emit TypeDef and FieldDef rows for every class and return their tokens.
fn emit_classes(program: &hir::Program, module: &mut Module) -> Layout {
    let mut type_tokens = Vec::with_capacity(program.classes.len());
    let mut field_tokens = Vec::with_capacity(program.classes.len());

    // The number of top-level functions: method MethodDef RIDs follow them.
    let top_level_fns = program.functions.len() - total_methods(program);

    for (class_idx, class) in program.classes.iter().enumerate() {
        let name = module.strings.intern(&class.name);

        // Field layout is the full inherited-first list; a FieldDef is emitted
        // per full-layout slot so ldfld/stfld tokens address the right slot.
        let mut this_class_fields = Vec::with_capacity(class.fields.len());
        let field_start = module.tables.field_defs.len() as u32 + 1; // RIDs are 1-based
        for field in &class.fields {
            let fname = module.strings.intern(&field.name);
            let tok = module.tables.add_field_def(FieldDef {
                name: fname,
                signature: BlobIndex(0),
                kind: field_kind_byte(field.ty),
            });
            this_class_fields.push(tok);
        }
        let field_end = module.tables.field_defs.len() as u32 + 1;

        // This class's own methods are a contiguous MethodDef range. Their RIDs
        // are func_index + 1, and func indices are top-level functions first,
        // then methods in class order, so the range is derived from them.
        let (method_start, method_end) = match (class.methods.first(), class.methods.last()) {
            (Some(&first), Some(&last)) => (first + 1, last + 2),
            _ => (0, 0),
        };
        let _ = top_level_fns;

        let base = match class.base {
            Some(b) => Token::new(TableKind::TypeDef, b + 1),
            None => Token::null(TableKind::TypeDef),
        };

        let tok = module.tables.add_type_def(TypeDef {
            name,
            namespace: zun_metadata::StringIndex(0),
            base,
            field_start,
            field_end,
            method_start,
            method_end,
        });
        debug_assert_eq!(tok.rid(), class_idx as u32 + 1);
        type_tokens.push(tok);
        field_tokens.push(this_class_fields);
    }

    Layout {
        type_tokens,
        field_tokens,
    }
}

/// Lower one function body to ZIL bytes.
fn lower_function(
    f: &hir::Function,
    layout: &Layout,
    module: &mut Module,
) -> (Vec<u8>, Vec<ExceptionHandler>) {
    let mut gen = FnLowerer {
        code: Vec::new(),
        layout,
        module,
        break_patches: Vec::new(),
        continue_targets: Vec::new(),
        handlers: Vec::new(),
    };
    gen.stmts(&f.body);
    // Guarantee the body cannot run off its end: a void function gets an
    // implicit return. (Non-void paths are the verifier's job later.)
    if f.return_ty == Ty::Void || !matches!(f.body.last(), Some(Stmt::Return(_))) {
        gen.emit(Instruction::simple(Opcode::Ret));
    }
    (gen.code, gen.handlers)
}

/// Handle to a forward branch whose displacement is patched once the target
/// offset is known.
struct BranchPatch {
    /// Byte offset of the 4-byte displacement inside the code buffer.
    operand_at: usize,
}

struct FnLowerer<'a> {
    code: Vec<u8>,
    layout: &'a Layout,
    module: &'a mut Module,
    break_patches: Vec<Vec<BranchPatch>>,
    continue_targets: Vec<Vec<usize>>,
    handlers: Vec<ExceptionHandler>,
}

impl FnLowerer<'_> {
    fn emit(&mut self, instr: Instruction) {
        encode(&instr, &mut self.code);
    }

    /// Push a numeric constant of the given type: a base `const.i32`/`const.i64`
    /// or `const.f64`, then a `conv` when the target kind is narrower/unsigned
    /// or single-precision.
    fn emit_numeric_const(&mut self, v: f64, ty: Ty) {
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
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(v as i32)));
                if kind != NumKind::I32 {
                    self.emit(Instruction::new(
                        Opcode::Conv,
                        Operand::Imm32(kind.to_imm()),
                    ));
                }
            }
        }
    }

    /// Emit a `conv` to `ty` (used by the `as` operator).
    fn emit_conv_to(&mut self, ty: Ty) {
        if let Some(kind) = numkind_of(ty) {
            self.emit(Instruction::new(
                Opcode::Conv,
                Operand::Imm32(kind.to_imm()),
            ));
        }
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
        self.code[patch.operand_at..patch.operand_at + 4].copy_from_slice(&disp.to_le_bytes());
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
                // Every expression leaves exactly one value on the stack in this
                // model — including void calls, which push a void value — so an
                // expression statement always discards one.
                self.expr(e);
                self.emit(Instruction::simple(Opcode::Pop));
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
                self.break_patches.push(Vec::new());
                self.continue_targets.push(vec![loop_start]);
                self.stmts(body);
                self.emit_branch_back(Opcode::Branch, loop_start);
                self.patch_to_here(to_exit);
                self.continue_targets.pop();
                for patch in self.break_patches.pop().unwrap_or_default() {
                    self.patch_to_here(patch);
                }
            }
            Stmt::Break => {
                let patch = self.emit_branch(Opcode::Branch);
                if let Some(patches) = self.break_patches.last_mut() {
                    patches.push(patch);
                }
            }
            Stmt::Continue => {
                if let Some(&target) = self
                    .continue_targets
                    .last()
                    .and_then(|targets| targets.last())
                {
                    self.emit_branch_back(Opcode::Branch, target);
                }
            }
            Stmt::Return(value) => {
                if let Some(v) = value {
                    self.expr(v);
                }
                self.emit(Instruction::simple(Opcode::Ret));
            }
            Stmt::Throw(value) => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::Throw));
            }
            Stmt::Try {
                body,
                catches,
                finally_body,
            } => {
                let try_start = self.code.len() as u32;
                self.stmts(body);
                let try_end = self.code.len() as u32;
                let mut exits = Vec::new();
                if let Some(finally_body) = finally_body {
                    self.stmts(finally_body);
                }
                exits.push(self.emit_branch(Opcode::Branch));

                for catch in catches {
                    let handler_start = self.code.len() as u32;
                    // A matched handler receives the exception object on the
                    // evaluation stack; bind it before entering user code.
                    self.emit(Instruction::new(
                        Opcode::StoreLocal,
                        Operand::Local(catch.slot),
                    ));
                    self.stmts(&catch.body);
                    if let Some(finally_body) = finally_body {
                        self.stmts(finally_body);
                    }
                    let handler_end = self.code.len() as u32;
                    exits.push(self.emit_branch(Opcode::Branch));
                    self.handlers.push(ExceptionHandler {
                        try_start,
                        try_end,
                        handler_start,
                        handler_end,
                        kind: HandlerKind::Catch,
                        catch_type: match catch.ty {
                            Ty::Instance(index) => Some(Token::new(TableKind::TypeDef, index + 1)),
                            _ => None,
                        },
                    });
                }

                if let Some(finally_body) = finally_body {
                    let handler_start = self.code.len() as u32;
                    self.stmts(finally_body);
                    self.emit(Instruction::simple(Opcode::Rethrow));
                    let handler_end = self.code.len() as u32;
                    self.handlers.push(ExceptionHandler {
                        try_start,
                        try_end,
                        handler_start,
                        handler_end,
                        kind: HandlerKind::Finally,
                        catch_type: None,
                    });
                }
                for exit in exits {
                    self.patch_to_here(exit);
                }
            }
        }
    }

    fn expr(&mut self, e: &hir::Expr) {
        match &e.kind {
            ExprKind::Number(v) => self.emit_numeric_const(*v, e.ty),
            ExprKind::String(s) => {
                let idx = self.module.strings.intern(s);
                self.emit(Instruction::new(
                    Opcode::ConstStr,
                    Operand::StringRef(idx.0),
                ));
            }
            ExprKind::Boolean(true) => self.emit(Instruction::simple(Opcode::ConstTrue)),
            ExprKind::Boolean(false) => self.emit(Instruction::simple(Opcode::ConstFalse)),
            ExprKind::Local(slot) => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
            }
            ExprKind::FunctionRef { func } => {
                self.emit(Instruction::new(
                    Opcode::LoadFunction,
                    Operand::Token(Token::new(TableKind::MethodDef, *func + 1)),
                ));
            }
            ExprKind::Assign { slot, value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
            }
            ExprKind::Binary { op, lhs, rhs } => {
                self.expr(lhs);
                self.expr(rhs);
                // Pick the opcode family from the operand type: int operands use
                // the i32 ops, floats the f64 ops, strings concatenate.
                let is_int = lhs.ty.is_int();
                let opcode = match op {
                    BinaryOp::Add if lhs.ty == Ty::String => Opcode::Concat,
                    BinaryOp::Add => int_or_float(is_int, Opcode::AddI32, Opcode::AddF64),
                    BinaryOp::Sub => int_or_float(is_int, Opcode::SubI32, Opcode::SubF64),
                    BinaryOp::Mul => int_or_float(is_int, Opcode::MulI32, Opcode::MulF64),
                    BinaryOp::Div => int_or_float(is_int, Opcode::DivI32, Opcode::DivF64),
                    BinaryOp::Rem => int_or_float(is_int, Opcode::RemI32, Opcode::RemF64),
                    // Comparisons share one opcode set; the runtime compares by
                    // the operands' actual kind.
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
                UnaryOp::Neg if operand.ty.is_int() => {
                    self.expr(operand);
                    self.emit(Instruction::simple(Opcode::NegI32));
                }
                UnaryOp::Neg => {
                    // 0 - x, with the zero in the operand's float kind so the
                    // subtraction keeps that kind.
                    self.emit_numeric_const(0.0, operand.ty);
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
            ExprKind::IndirectCall { callee, args, func } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
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
            ExprKind::CallBuiltin { id, args } => {
                for a in args {
                    self.expr(a);
                }
                self.emit(Instruction::new(
                    Opcode::CallBuiltin,
                    Operand::Imm32(*id as i32),
                ));
            }
            ExprKind::DynamicNew => {
                self.emit(Instruction::simple(Opcode::NewDynamic));
            }
            ExprKind::DynamicGet { object, name } => {
                self.expr(object);
                let property = self.module.strings.intern(name);
                self.emit(Instruction::new(
                    Opcode::LoadDynamic,
                    Operand::StringRef(property.0),
                ));
            }
            ExprKind::DynamicSet {
                object,
                name,
                value,
            } => {
                self.expr(object);
                self.expr(value);
                let property = self.module.strings.intern(name);
                self.emit(Instruction::new(
                    Opcode::StoreDynamic,
                    Operand::StringRef(property.0),
                ));
            }
            ExprKind::BoxAny { value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::BoxAny));
            }
            ExprKind::ExternalCall { module, name, args } => {
                for a in args {
                    self.expr(a);
                }
                let module_name = self.module.strings.intern(module);
                let member_name = self.module.strings.intern(name);
                let module_ref = self
                    .module
                    .tables
                    .add_module_ref(ModuleRef { name: module_name });
                let member_ref = self.module.tables.add_member_ref(MemberRef {
                    parent: module_ref,
                    name: member_name,
                    signature: BlobIndex(0),
                });
                self.emit(Instruction::new(Opcode::Call, Operand::Token(member_ref)));
            }
            ExprKind::MethodCall { func, args } => {
                // args[0] is the receiver; push it and the rest, then dispatch
                // virtually on the receiver's runtime class. The token names the
                // statically resolved method (for its name and arity).
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::CallVirt, Operand::Token(token)));
            }
            ExprKind::New { class, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = self.layout.type_tokens[*class as usize];
                self.emit(Instruction::new(Opcode::NewObj, Operand::Token(token)));
            }
            ExprKind::ArrayNew { length } => {
                self.expr(length);
                self.emit(Instruction::simple(Opcode::NewArray));
            }
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
            } => {
                // [object, value] -> stfld -> [value]. stfld leaves the stored
                // value so the assignment yields it; in statement position the
                // always-pop rule discards it.
                self.expr(object);
                self.expr(value);
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::StoreField, Operand::Token(token)));
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
            ExprKind::Convert { value } => {
                if value.ty == Ty::Any {
                    self.expr(value);
                    self.emit(Instruction::simple(Opcode::UnboxAny));
                } else {
                    self.expr(value);
                }
                self.emit_conv_to(e.ty);
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
             0014: pop\n\
             0015: ret\n"
        );
        assert_eq!(body.local_count, 1);
        assert!(module.entry_point.is_some());
    }

    #[test]
    fn while_loop_branches_point_at_the_right_places() {
        let module = lower_src("function main() { let n = 3; while (n > 0) { n = n - 1; } }");
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
        let module =
            lower_src("function main() { if (true) { print(\"a\"); } else { print(\"b\"); } }");
        let body = read_body(&module.code, module.tables.method_defs[0].body_offset).unwrap();
        let text = disassemble(body.code).unwrap();
        assert_eq!(
            text,
            "0000: const.true\n\
             0001: br.false +16\n\
             0006: const.str str:6\n\
             000B: call.builtin 0\n\
             0010: pop\n\
             0011: br +11\n\
             0016: const.str str:8\n\
             001B: call.builtin 0\n\
             0020: pop\n\
             0021: ret\n"
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

    #[test]
    fn try_catch_finally_lowers_handler_regions() {
        let module = lower_src(
            "class Error {} function main() { try { throw new Error(); } catch (error: Error) { print(1); } finally { print(2); } }",
        );
        let method = &module.tables.method_defs[0];
        let body = read_body(&module.code, method.body_offset).unwrap();
        assert_eq!(body.handlers.len(), 2);
        assert!(body.handlers.iter().any(|h| h.kind == HandlerKind::Catch));
        assert!(body.handlers.iter().any(|h| h.kind == HandlerKind::Finally));
    }
}
