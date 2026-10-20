use super::gc_policy::GC_INTERVAL;
use super::{CallHook, Interp, Output, RunError, Value};
use super::state::ExecutionOutcome;
use zun_module::Module;

pub(super) fn run_method_with(
    module: &Module,
    method_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
    hook: Option<&dyn CallHook>,
    loader: Option<&zun_loader::ModuleLoader>,
) -> Result<Value, RunError> {
    let mut heap = zun_gc::Heap::new();
    let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
    let types = zun_types::TypeSystem::from_module(module).map_err(|_| RunError::BadToken)?;
    Interp {
        module,
        output,
        hook,
        loader,
        allocs: 0,
        gc_interval: GC_INTERVAL,
        types,
    }
    .call(method_rid, args, None)
}

impl Interp<'_> {
    pub(super) fn dispatch_outcome(
        &mut self,
        method_rid: u32,
        args: Vec<Value>,
        parent: Option<&super::RootLink>,
    ) -> ExecutionOutcome {
        ExecutionOutcome::from_result(self.dispatch(method_rid, args, parent))
    }

    /// Route a call to the JIT hook when it handles the method, otherwise
    /// continue with the Tier 0 interpreter.
    pub(super) fn dispatch(
        &mut self,
        method_rid: u32,
        args: Vec<Value>,
        parent: Option<&super::RootLink>,
    ) -> Result<Value, RunError> {
        if let Some(hook) = self.hook {
            if let Some(result) = hook.try_call(method_rid, &args) {
                return Ok(result);
            }
        }
        self.call(method_rid, args, parent)
    }
}
