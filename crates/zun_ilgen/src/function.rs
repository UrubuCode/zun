use super::*;

/// Lower one function body to ZIL bytes.
pub(super) fn lower_function<'a>(
    f: &'a hir::Function,
    layout: &'a Layout,
    module: &mut Module,
    closure_method_base: u32,
    regex_plan: &'a [RegexPlan],
) -> (Vec<u8>, Vec<ExceptionHandler>, Vec<(u32, Span)>) {
    let mut gen = FnLowerer {
        code: Vec::new(),
        layout,
        module,
        break_patches: Vec::new(),
        loop_labels: Vec::new(),
        pending_label: None,
        continue_targets: Vec::new(),
        continue_patches: Vec::new(),
        handlers: Vec::new(),
        finally_bodies: Vec::new(),
        break_finally_depths: Vec::new(),
        finally_holes: Vec::new(),
        closure_method_base,
        self_closure: None,
        points: Vec::new(),
        regex_plan,
        is_async: f.is_async,
        is_async_gen: f.is_async && f.is_generator,
    };
    if f.is_async && f.is_generator {
        gen.emit(Instruction::simple(Opcode::AsyncGenStart));
    } else if f.is_async {
        gen.emit(Instruction::simple(Opcode::AsyncStart));
    }
    let body_start = gen.emit_param_cells(&f.body, f.param_count);
    gen.emit_defaults(
        &f.defaults,
        f.param_count - f.defaults.len() as u16,
        &f.locals,
        &f.body[..body_start],
    );
    gen.stmts(&f.body[body_start..]);
    // Guarantee the body cannot run off its end: a void function gets an
    // implicit return. (Non-void paths are the verifier's job later.)
    if f.return_ty != Ty::Void && !matches!(f.body.last(), Some(Stmt::Return(_))) {
        // Running off the end of a value-returning body yields `undefined`.
        match f.return_ty {
            Ty::Any => gen.emit(Instruction::simple(Opcode::ConstUndef)),
            Ty::F64 | Ty::Int(_) => gen.emit_numeric_const(0.0, f.return_ty),
            Ty::Boolean => gen.emit(Instruction::simple(Opcode::ConstFalse)),
            _ => gen.emit(Instruction::simple(Opcode::ConstNull)),
        }
    }
    if f.return_ty == Ty::Void || !matches!(f.body.last(), Some(Stmt::Return(_))) {
        gen.emit(Instruction::simple(Opcode::Ret));
    }
    (gen.code, gen.handlers, gen.points)
}

/// Lower a synthetic closure function. Its hidden environment is local slot 0
/// and its user locals are shifted by one slot in the emitted method body.
pub(super) fn lower_closure_function<'a>(
    f: &'a hir::ClosureFunction,
    layout: &'a Layout,
    module: &mut Module,
    closure_method_base: u32,
    closure_index: u32,
    regex_plan: &'a [RegexPlan],
) -> (Vec<u8>, Vec<ExceptionHandler>, Vec<(u32, Span)>) {
    let mut gen = FnLowerer {
        code: Vec::new(),
        layout,
        module,
        break_patches: Vec::new(),
        loop_labels: Vec::new(),
        pending_label: None,
        continue_targets: Vec::new(),
        continue_patches: Vec::new(),
        handlers: Vec::new(),
        finally_bodies: Vec::new(),
        break_finally_depths: Vec::new(),
        finally_holes: Vec::new(),
        closure_method_base,
        self_closure: Some(closure_index),
        points: Vec::new(),
        regex_plan,
        is_async: f.is_async,
        is_async_gen: f.is_async && f.is_generator,
    };
    if f.is_async && f.is_generator {
        gen.emit(Instruction::simple(Opcode::AsyncGenStart));
    } else if f.is_async {
        gen.emit(Instruction::simple(Opcode::AsyncStart));
    }
    let body_start = gen.emit_param_cells(&f.body, f.param_count);
    gen.emit_defaults(&f.defaults, 1, &f.locals, &f.body[..body_start]);
    gen.stmts(&f.body[body_start..]);
    if f.return_ty != Ty::Void && !matches!(f.body.last(), Some(Stmt::Return(_))) {
        // Running off the end of a value-returning body yields `undefined`.
        match f.return_ty {
            Ty::Any => gen.emit(Instruction::simple(Opcode::ConstUndef)),
            Ty::F64 | Ty::Int(_) => gen.emit_numeric_const(0.0, f.return_ty),
            Ty::Boolean => gen.emit(Instruction::simple(Opcode::ConstFalse)),
            _ => gen.emit(Instruction::simple(Opcode::ConstNull)),
        }
    }
    if f.return_ty == Ty::Void || !matches!(f.body.last(), Some(Stmt::Return(_))) {
        gen.emit(Instruction::simple(Opcode::Ret));
    }
    (gen.code, gen.handlers, gen.points)
}

pub(super) fn capture_name(slot: u16) -> String {
    format!("$capture{slot}")
}
