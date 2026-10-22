use super::Checker;
use zun_syntax::ast;

impl<'a> Checker<'a> {
    /// Resolve `import { ... } from "zun:..."` declarations to intrinsics,
    /// binding each imported name to its signature and dispatch id.
    pub(super) fn resolve_imports(
        &mut self,
        program: &ast::Program,
        allow_external_modules: bool,
    ) {
        for import in &program.imports {
            let Some(module) = zun_natives::module(&import.module) else {
                if allow_external_modules && !import.module.starts_with("zun:") {
                    continue;
                }
                self.error(import.span, format!("unknown module `{}`", import.module));
                continue;
            };
            let _ = module;
            for binding in &import.names {
                let imported = self.text(&binding.imported).to_string();
                let local = self.text(&binding.local).to_string();
                match zun_natives::resolve(&import.module, &imported) {
                    Some((id, f)) => {
                        self.imports.insert(local, (id, f));
                    }
                    None => self.error(
                        binding.imported.span,
                        format!("`{}` has no export named `{imported}`", import.module),
                    ),
                }
            }
        }
    }
}
