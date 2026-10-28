use super::Checker;
use crate::hir;
use crate::hir::{IntKind, Ty};
use zun_syntax::ast;

/// A resolved `enum` member value, folded in at `Enum.Member` field accesses.
#[derive(Debug, Clone)]
pub(super) enum EnumConst {
    /// A numeric member: folds to a `number` literal.
    Num(f64),
    /// A string member: folds to a `string` literal.
    Str(String),
}

impl<'a> Checker<'a> {
    /// Resolve every `enum` declaration: compute member values (numeric members
    /// auto-increment from the previous numeric value, starting at 0; string
    /// members require an explicit initializer) and record them for constant
    /// folding. The enum name resolves as a type to the members' shared
    /// representation — `number` when all-numeric, `string` when all-string,
    /// else `any` — registered through `type_aliases`.
    pub(super) fn collect_enums(&mut self, program: &ast::Program) {
        for decl in &program.enums {
            let enum_name = self.text(&decl.name).to_string();
            let mut members: std::collections::HashMap<String, EnumConst> =
                std::collections::HashMap::new();
            let mut next_auto = 0.0_f64;
            let mut any_num = false;
            let mut any_str = false;
            for member in &decl.members {
                let mname = self.text(&member.name).to_string();
                let value = match &member.value {
                    ast::EnumValue::Auto => {
                        let v = next_auto;
                        next_auto += 1.0;
                        any_num = true;
                        EnumConst::Num(v)
                    }
                    ast::EnumValue::Num(n) => {
                        next_auto = n + 1.0;
                        any_num = true;
                        EnumConst::Num(*n)
                    }
                    ast::EnumValue::Str(s) => {
                        any_str = true;
                        EnumConst::Str(s.clone())
                    }
                };
                members.insert(mname, value);
            }
            let ty = match (any_num, any_str) {
                (true, false) => Ty::F64,
                (false, true) => Ty::String,
                _ => Ty::Any,
            };
            // Make the enum name usable as a type and known to the fallback.
            self.type_aliases.insert(enum_name.clone(), ty);
            self.enums.insert(enum_name, members);
        }
    }

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

    /// Register `type X = ...` aliases and (erased) `interface X { ... }`
    /// declarations. Each name is first seeded into the erased-name fallback so
    /// forward and self references degrade to `any` rather than erroring, then
    /// its right-hand side is resolved and recorded in `type_aliases`, which
    /// `resolve_annotation` consults first (so a simple `type Id = number`
    /// keeps its frozen type while a structural/interface alias stays `any`).
    pub(super) fn collect_type_aliases(&mut self, program: &ast::Program) {
        for alias in &program.type_aliases {
            let name = self.text(&alias.name).to_string();
            self.type_param_names.insert(name);
        }
        for alias in &program.type_aliases {
            let name = self.text(&alias.name).to_string();
            let ty = self.resolve_annotation(&alias.value);
            self.type_aliases.insert(name, ty);
        }
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
