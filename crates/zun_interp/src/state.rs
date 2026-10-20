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
