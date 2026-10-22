use super::{Checker, ClassInfo};
use crate::hir;
use zun_syntax::ast;

impl<'a> Checker<'a> {
    /// First pass over classes: register names (so annotations resolve) and
    /// remember each base's name for later resolution.
    pub(super) fn collect_classes(&mut self, program: &ast::Program) {
        for c in &program.classes {
            let name = self.text(&c.name).to_string();
            let index = self.classes.len() as u32;
            if self.classes_by_name.insert(name, index).is_some() {
                self.error(c.name.span, "duplicate class");
            }
            let base_name = c.base.as_ref().map(|b| self.text(b).to_string());
            self.classes.push(ClassInfo {
                base: None,
                base_name,
                own_fields: Vec::new(),
            });
        }
        // Resolve base names to indices now that all classes are known.
        for (i, c) in program.classes.iter().enumerate() {
            if let Some(base_name) = self.classes[i].base_name.clone() {
                match self.classes_by_name.get(&base_name) {
                    Some(&b) if b != i as u32 => self.classes[i].base = Some(b),
                    Some(_) => {
                        if let Some(b) = &c.base {
                            self.error(b.span, "a class cannot extend itself");
                        }
                    }
                    None => {
                        if let Some(b) = &c.base {
                            self.error(b.span, format!("unknown base class `{base_name}`"));
                        }
                    }
                }
            }
        }
    }

    /// Second pass: resolve each class's own field types, then build the HIR
    /// classes with the full (inherited-first) field layout.
    pub(super) fn check_classes(&mut self, program: &ast::Program) -> Vec<hir::Class> {
        for (index, c) in program.classes.iter().enumerate() {
            let mut own = Vec::new();
            for f in &c.fields {
                let fname = if f.is_private {
                    format!("#{}#{}", self.text(&c.name), self.text(&f.name))
                } else {
                    self.text(&f.name).to_string()
                };
                let ty = self.resolve_annotation(&f.ty);
                own.push((fname, ty, f.is_static, f.is_private));
            }
            self.classes[index].own_fields = own;
        }

        let mut out = Vec::with_capacity(program.classes.len());
        for (index, c) in program.classes.iter().enumerate() {
            let fields = self
                .full_fields(index as u32)
                .into_iter()
                .map(|(name, ty, is_static, is_private)| hir::ClassField { name, ty, is_static, is_private })
                .collect();
            out.push(hir::Class {
                name: self.text(&c.name).to_string(),
                base: self.classes[index].base,
                fields,
                methods: Vec::new(), // filled after signatures assign indices
            });
        }
        out
    }
}
