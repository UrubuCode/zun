/// A function export of a loaded module, as seen through a dynamic namespace.
///
/// Unlike the typed [`FunctionHandle`](crate::FunctionHandle), which is only
/// meaningful inside the module that produced it, this value names its defining
/// module by loader identity so the dynamic call path can resolve it through the
/// module loader. The identity stays on the loader side; core only carries it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleBoundFunction {
    module: String,
    method_rid: u32,
}

impl ModuleBoundFunction {
    pub fn new(module: impl Into<String>, method_rid: u32) -> Self {
        Self { module: module.into(), method_rid }
    }

    /// Loader identity of the defining module.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// `MethodDef` row id inside the defining module.
    pub const fn method_rid(&self) -> u32 {
        self.method_rid
    }
}
