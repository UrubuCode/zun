//! Exception-region routing for the Tier 0 interpreter.

use crate::value::Value;
use super::Interp;

impl Interp<'_> {
    pub(super) fn route_exception(
        &self,
        body: &zun_module::MethodBody<'_>,
        stack: &mut Vec<Value>,
        pc: &mut usize,
        throw_pc: usize,
        exception: &Value,
    ) -> bool {
        route_exception(&self.types, body, stack, pc, throw_pc, exception)
    }
}

/// Route a thrown value to the innermost matching catch or finally clause.
pub(crate) fn route_exception(
    types: &zun_types::TypeSystem,
    body: &zun_module::MethodBody<'_>,
    stack: &mut Vec<Value>,
    pc: &mut usize,
    throw_pc: usize,
    exception: &Value,
) -> bool {
    let handler = body
        .handlers
        .iter()
        .filter(|handler| {
            handler.try_start as usize <= throw_pc && throw_pc < handler.try_end as usize
        })
        .filter(|handler| match handler.kind {
            zun_module::HandlerKind::Finally => true,
            zun_module::HandlerKind::Catch if is_generator_return(exception) => false,
            zun_module::HandlerKind::Catch => match (handler.catch_type, exception) {
                (Some(token), Value::Object(object)) => {
                    types.is_assignable(object.type_rid(), token.rid())
                }
                (None, _) => true,
                _ => false,
            },
        })
        .min_by_key(|handler| handler.try_end - handler.try_start);
    let Some(handler) = handler else {
        return false;
    };
    stack.clear();
    stack.push(exception.clone());
    *pc = handler.handler_start as usize;
    true
}

/// Marker thrown into a suspended generator to model a `return()` completion:
/// only `finally` handlers observe it; `catch` clauses never match it.
pub(crate) const GENERATOR_RETURN_SENTINEL: &str = "\u{0}zun.generatorReturn";

thread_local! {
    /// Set right before resuming a generator frame that must unwind as a
    /// `return()` completion (the iterator-close protocol).
    pub(crate) static GENERATOR_RETURN_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn is_generator_return(value: &Value) -> bool {
    matches!(value, Value::Str(text) if &**text == GENERATOR_RETURN_SENTINEL)
}

thread_local! {
    /// Real argument count announced by a statically compiled call site just
    /// before the call (consumed by the callee's frame entry).
    pub(crate) static CALL_ARGC_MARK: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Real argument count of the call that created the running frame.
    pub(crate) static CALL_ARGC_CURRENT: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
}
