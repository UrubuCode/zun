use super::{Checker, ClassInfo};
use crate::hir;
use zun_common::Span;
use zun_syntax::ast;

impl<'a> Checker<'a> {
    /// Whether `name` denotes a run-time value (a builtin global constructor or
    /// a binding declared in the source) rather than a misspelled class.
    fn is_runtime_value_name(&self, name: &str) -> bool {
        const GLOBALS: &[&str] = &[
            "Iterator", "BigInt", "Symbol", "Object", "Function", "Array", "String", "Number",
            "Boolean", "Error", "Proxy", "Reflect", "Date", "RegExp", "Map", "Set", "WeakMap",
            "WeakSet", "Promise",
        ];
        if GLOBALS.contains(&name) {
            return true;
        }
        ["const", "let", "var", "function"].iter().any(|keyword| {
            let needle = format!("{keyword} {name}");
            self.source.match_indices(&needle).any(|(at, _)| {
                self.source[at + needle.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '$'))
            })
        })
    }

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
                span_start: c.span.start,
                base: None,
                base_name,
                dynamic_base: false,
                own_fields: Vec::new(),
                own_initializers: Vec::new(),
            });
        }
        // Resolve base names to indices now that all classes are known.
        for (i, c) in program.classes.iter().enumerate() {
            if let Some(base_name) = self.classes[i].base_name.clone() {
                // `extends Promise` (and the other built-in value types) derive
                // from the prelude's `__Builtin<Name>` wrapper class, which owns
                // the native value and forwards the built-in's methods to it.
                let wrapper = format!("__Builtin{base_name}");
                let base_name = if !self.classes_by_name.contains_key(&base_name)
                    && self.classes_by_name.contains_key(&wrapper)
                {
                    self.classes[i].base_name = Some(wrapper.clone());
                    wrapper
                } else {
                    base_name
                };
                match self.classes_by_name.get(&base_name) {
                    Some(&b) if b != i as u32 => self.classes[i].base = Some(b),
                    Some(_) => {
                        if let Some(b) = &c.base {
                            self.error(b.span, "a class cannot extend itself");
                        }
                    }
                    None => {
                        // A base that is not a known class is a runtime value
                        // (a builtin constructor, a mixin result): the class
                        // is compiled base-less.
                        if self.is_runtime_value_name(&base_name) {
                            self.classes[i].dynamic_base = true;
                            self.classes[i].base_name = None;
                        } else if let Some(b) = &c.base {
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
            self.classes[index].own_initializers = c
                .fields
                .iter()
                .map(|field| field.initializer.clone())
                .collect();
        }

        let mut out = Vec::with_capacity(program.classes.len());
        for (index, c) in program.classes.iter().enumerate() {
            let fields = self
                .full_fields(index as u32)
                .into_iter()
                .map(|(name, ty, is_static, is_private)| hir::ClassField {
                    name,
                    ty,
                    is_static,
                    is_private,
                })
                .collect();
            out.push(hir::Class {
                name: self.text(&c.name).to_string(),
                base: self.classes[index].base,
                decorators: c
                    .decorators
                    .iter()
                    .map(|d| self.lower_decorator(d))
                    .collect(),
                fields,
                methods: Vec::new(), // filled after signatures assign indices
            });
        }
        out
    }

    /// Lower an AST decorator to its inert HIR carry (source text only). The
    /// callee and argument spans are sliced from the original source so the
    /// HIR owns its strings and does not reference the AST.
    fn lower_decorator(&self, decorator: &ast::Decorator) -> hir::Decorator {
        let slice = |span: Span| self.source[span.start as usize..span.end as usize].to_string();
        hir::Decorator {
            callee: slice(decorator.callee.span()),
            args: decorator
                .args
                .as_ref()
                .map(|args| args.iter().map(|arg| slice(arg.span())).collect()),
        }
    }
}
