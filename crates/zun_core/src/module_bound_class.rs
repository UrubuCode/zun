/// A class (type) export of a loaded module, as seen through a dynamic namespace.
///
/// Mirrors [`ModuleBoundFunction`](crate::ModuleBoundFunction) for class exports:
/// a typed `TypeDef` is only meaningful inside the module that defined it, so this
/// value names its defining module by loader identity and the dynamic construction
/// path resolves it through the module loader. The identity stays on the loader
/// side; core only carries it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleBoundClass {
    module: String,
    type_rid: u32,
}

impl ModuleBoundClass {
    pub fn new(module: impl Into<String>, type_rid: u32) -> Self {
        Self { module: module.into(), type_rid }
    }

    /// Loader identity of the defining module.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// `TypeDef` row id inside the defining module.
    pub const fn type_rid(&self) -> u32 {
        self.type_rid
    }
}
