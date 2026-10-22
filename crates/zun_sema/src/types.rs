use super::Checker;
use crate::hir;
use crate::hir::{IntKind, Ty};
use zun_syntax::ast;

impl<'a> Checker<'a> {
    pub(super) fn resolve_annotation(&mut self, annotation: &ast::TypeAnnotation) -> Ty {
        let text = self.text(&annotation.name).to_string();
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
            "RegExp" | "regexp" => Ty::RegExp,
            "BigInt" | "bigint" => Ty::BigInt,
            "FinalizationRegistry" | "finalizationregistry" => Ty::FinalizationRegistry,
            "Proxy" | "proxy" => Ty::Proxy,
            "Promise" | "promise" => Ty::Promise,
            "DataView" => Ty::BufferView(hir::BufferViewKind::DataView),
            "Uint8Array" => Ty::BufferView(hir::BufferViewKind::U8),
            "Uint16Array" => Ty::BufferView(hir::BufferViewKind::U16),
            "Uint32Array" => Ty::BufferView(hir::BufferViewKind::U32),
            "Float64Array" => Ty::BufferView(hir::BufferViewKind::F64),
            "dynamic" | "DynamicObject" => Ty::DynamicObject,
            "any" => Ty::Any,
            "Function" => Ty::Function(hir::DYNAMIC_FUNCTION),
            "Predicate" => Ty::Function(hir::DYNAMIC_PREDICATE),
            "Reducer" => Ty::Function(hir::DYNAMIC_REDUCER),
            "void" => Ty::Void,
            other => {
                if let Some(&func) = self.by_name.get(other) {
                    return Ty::Function(func);
                }
                match self.classes_by_name.get(other) {
                    Some(&index) => Ty::Instance(index),
                    None => {
                        self.error(annotation.name.span, format!("unknown type `{other}`"));
                        Ty::Error
                    }
                }
            }
        }
    }
}
