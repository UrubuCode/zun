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
            zun_module::HandlerKind::Catch => match (handler.catch_type, exception) {
                (Some(token), Value::Object(object)) => {
                    types.is_assignable(object.type_rid(), token.rid())
                }
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
