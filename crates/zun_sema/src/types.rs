use super::Checker;
use crate::hir;
use crate::hir::{IntKind, Ty};
use zun_syntax::ast;

impl<'a> Checker<'a> {
    /// Gather every generic type-parameter name declared in the program into
    /// `type_param_names` (see the field doc). Runs once before any annotation
    /// is resolved. Erasure means names, not bounds, are all that matter.
    pub(super) fn collect_type_param_names(&mut self, program: &ast::Program) {
        let mut names: Vec<String> = Vec::new();
        let mut record = |source: &str, params: &[ast::Name]| {
            for p in params {
                names.push(source[p.span.start as usize..p.span.end as usize].to_string());
            }
        };
        for f in &program.functions {
            record(self.source, &f.type_params);
        }
        for c in &program.classes {
            record(self.source, &c.type_params);
            for m in &c.methods {
                record(self.source, &m.type_params);
            }
            for sm in &c.symbol_methods {
                record(self.source, &sm.function.type_params);
            }
        }
        self.type_param_names.extend(names);
    }

    pub(super) fn resolve_annotation(&mut self, annotation: &ast::TypeAnnotation) -> Ty {
        // `T[]` (any element type, any depth) is an untyped `Array`.
        if annotation.is_array {
            return Ty::Array;
        }
        if annotation.name.span.start == annotation.name.span.end {
            return Ty::Any;
        }
        let text = self.text(&annotation.name).to_string();
        if let Some(&ty) = self.type_aliases.get(&text) {
            return ty;
        }
        // Declarative realm globals (PLAN §3.1) take precedence; on a miss we
        // fall through to the hardcoded base-type matches unchanged. A
        // class-backed binding (`Error` + subclasses) has no static annotation
        // type — the class index is program-assigned — so it is resolved
        // through `classes_by_name` like any other class name below.
        if let Some(binding) = self.realm_globals.get(&text) {
            if !matches!(binding.ctor, crate::realm::CtorLowering::ClassFallThrough) {
                return binding.ty;
            }
        }
        // The typed-array constructor names come from the master table, so an
        // annotation cannot name a kind the rest of the wiring lacks.
        if let Some(kind) = crate::typed_arrays::view_kind(text.as_str()) {
            return Ty::BufferView(kind);
        }
        match text.as_str() {
            "number" | "float" | "f64" => Ty::F64,
            "f32" => Ty::F32,
            "int" | "i32" => Ty::Int(IntKind::I32),
            "i8" => Ty::Int(IntKind::I8),
            "i16" => Ty::Int(IntKind::I16),
            "i64" => Ty::Int(IntKind::I64),
            "u8" => Ty::Int(IntKind::U8),
            "u16" => Ty::Int(IntKind::U16),
            "u32" => Ty::Int(IntKind::U32),
            "u64" => Ty::Int(IntKind::U64),
            "string" => Ty::String,
            "boolean" => Ty::Boolean,
            "Array" | "array" => Ty::Array,
            "ArrayBuffer" | "arraybuffer" => Ty::Buffer,
            "Map" | "map" => Ty::Map,
            "Set" | "set" => Ty::Set,
            "WeakMap" | "weakmap" => Ty::WeakMap,
            "WeakSet" | "weakset" => Ty::WeakSet,
            "WeakRef" | "weakref" => Ty::WeakRef,
            "RegExp" | "regexp" => Ty::RegExp,
            "BigInt" | "bigint" => Ty::BigInt,
            "FinalizationRegistry" | "finalizationregistry" => Ty::FinalizationRegistry,
            "Proxy" | "proxy" => Ty::Proxy,
            "Promise" | "promise" => Ty::Promise,
            "DataView" => Ty::BufferView(hir::BufferViewKind::DataView),
            "dynamic" | "DynamicObject" => Ty::DynamicObject,
            "Symbol" | "symbol" => Ty::Symbol,
            "any" => Ty::Any,
            "Function" => Ty::Function(hir::DYNAMIC_FUNCTION),
            "Predicate" => Ty::Function(hir::DYNAMIC_PREDICATE),
            "Reducer" => Ty::Function(hir::DYNAMIC_REDUCER),
            "void" => Ty::Void,
            other => {
                if let Some(&func) = self.by_name.get(other) {
                    return Ty::Function(func);
                }
                if let Some(&index) = self.classes_by_name.get(other) {
                    return Ty::Instance(index);
                }
                // Final fallback: an erased generic type parameter (`T`) has no
                // concrete type, so it resolves to `any`. Checked last so it
                // never shadows a real class/builtin/alias of the same name.
                if self.type_param_names.contains(other) {
                    return Ty::Any;
                }
                self.error(annotation.name.span, format!("unknown type `{other}`"));
                Ty::Error
            }
        }
    }
}
