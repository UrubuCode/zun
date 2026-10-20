use super::Value;

pub trait CallHook {
    fn try_call(&self, method_rid: u32, args: &[Value]) -> Option<Value>;

    fn on_backedge(&self, _method_rid: u32, _target_pc: usize) {}
}
