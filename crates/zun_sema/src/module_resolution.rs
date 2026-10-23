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
            if import.module == zun_natives::CORE_MODULE {
                self.resolve_core_import(import);
                continue;
            }
            let Some(module) = zun_natives::module(&import.module) else {
                if allow_external_modules
                    && matches!(import.module.as_str(), "zun:prelude" | "zun:std")
                {
                    for binding in &import.names {
                        let imported = if binding.is_default { "default" } else { self.text(&binding.imported) }.to_owned();
                        match self.external_functions.get(&imported) {
                            Some(external) if external.module == import.module => {}
                            Some(external) => self.error(
                                binding.imported.span,
                                format!(
                                    "`{imported}` is linked from `{}`, not `{}`",
                                    external.module, import.module
                                ),
                            ),
                            None => self.error(
                                binding.imported.span,
                                    format!("`{}` has no export named `{imported}`", import.module),
                            ),
                        }
                    }
                    continue;
                }
                if allow_external_modules && !import.module.starts_with("zun:") {
                    for binding in &import.names {
                        if binding.is_namespace {
                            self.namespace_imports.insert(
                                self.text(&binding.local).to_string(),
                                import.module.clone(),
                            );
                        } else if binding.is_default && import.module == "extended-string" {
                            let local = self.text(&binding.local).to_string();
                            self.type_aliases.insert(local, super::hir::Ty::String);
                        }
                    }
                    continue;
                }
                self.error(import.span, format!("unknown module `{}`", import.module));
                continue;
            };
            let _ = module;
            for binding in &import.names {
                if binding.is_namespace {
                    if zun_natives::module(&import.module).is_some() {
                        self.namespace_imports.insert(
                            self.text(&binding.local).to_string(),
                            import.module.clone(),
                        );
                    } else if allow_external_modules && !import.module.starts_with("zun:") {
                        self.namespace_imports.insert(
                            self.text(&binding.local).to_string(),
                            import.module.clone(),
                        );
                    } else {
                        self.error(binding.local.span, format!("unknown module `{}`", import.module));
                    }
                    continue;
                }
                let imported = if binding.is_default { "default" } else { self.text(&binding.imported) }.to_owned();
                let local = self.text(&binding.local).to_string();
                if binding.is_default && import.module == "extended-string" {
                    self.type_aliases.insert(local, super::hir::Ty::String);
                    continue;
                }
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

    /// Resolve an `import { ... } from "zun:core"` declaration. A named import
    /// binds either a per-type namespace (member access lowers to the native
    /// `call.builtin`) or, for `extern`-method and free-function bindings, a
    /// single intrinsic by its globally unique name.
    fn resolve_core_import(&mut self, import: &ast::Import) {
        for binding in &import.names {
            if binding.is_namespace {
                self.error(
                    binding.local.span,
                    "`import * as ns from \"zun:core\"` is not supported; import a namespace by name",
                );
                continue;
            }
            let imported = if binding.is_default {
                "default".to_owned()
            } else {
                self.text(&binding.imported).to_owned()
            };
            let local = self.text(&binding.local).to_string();
            if let Some(namespace) = zun_natives::namespace(&imported) {
                self.core_namespaces.insert(local, namespace);
            } else if let Some((id, f)) = zun_natives::resolve_name(&imported) {
                self.imports.insert(local, (id, f));
            } else {
                self.error(
                    binding.imported.span,
                    format!("`zun:core` has no export named `{imported}`"),
                );
            }
        }
    }
}
