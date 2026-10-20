use super::{CallHook, Output, Value};
use zun_module::Module;

pub(super) struct Interp<'a> {
    pub(super) module: &'a Module,
    pub(super) output: &'a mut dyn Output,
    pub(super) hook: Option<&'a dyn CallHook>,
    pub(super) loader: Option<&'a zun_loader::ModuleLoader>,
    pub(super) allocs: u32,
    pub(super) gc_interval: u32,
    pub(super) types: zun_types::TypeSystem,
}

/// A link in the chain of suspended caller frames, used to find precise GC
/// roots.
pub(super) struct RootLink<'a> {
    pub(super) stack: &'a [Value],
    pub(super) locals: &'a [Value],
    pub(super) parent: Option<&'a RootLink<'a>>,
}

/// Explicit interpreter activation record. Keeping the resumable pieces
/// together is the first step toward async suspension; the synchronous loop
/// still owns and advances one frame at a time for now.
pub(super) struct ExecutionFrame {
    pub(super) method_rid: u32,
    pub(super) pc: usize,
    pub(super) stack: Vec<Value>,
    pub(super) locals: Vec<Value>,
}

impl ExecutionFrame {
    pub(super) fn new(method_rid: u32, locals: Vec<Value>) -> Self {
        Self {
            method_rid,
            pc: 0,
            stack: Vec::new(),
            locals,
        }
    }
}
