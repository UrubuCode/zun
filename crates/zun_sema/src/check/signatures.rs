use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn collect_signatures(&mut self, program: &ast::Program) {
        // Register every top-level name before resolving annotations. This
        // permits a function-typed parameter to refer to a function declared
        // later in the source file.
        for (index, f) in program.functions.iter().enumerate() {
            let name = self.text(&f.name).to_string();
            if self.by_name.insert(name, index as u32).is_some() {
                self.error(f.name.span, "duplicate function declaration");
            }
        }
        for f in &program.functions {
            let name = self.text(&f.name).to_string();
            if matches!(f.body.statements.first(), Some(ast::Stmt::Let { name, .. }) if self.text(name) == "arguments") {
                self.arguments_functions.insert(self.signatures.len() as u32);
            }
            let params: Vec<Ty> = f
                .params
                .iter()
                .map(|p| match &p.ty {
                    // An unannotated parameter is untyped (`any`), as in JS/TS
                    // without `--noImplicitAny`; types are erased anyway.
                    Some(t) => self.resolve_annotation(t),
                    None => Ty::Any,
                })
                .collect();
            let return_ty = match &f.return_type {
                Some(t) => self.resolve_annotation(t),
                // An unannotated return type is `any` when the body returns a
                // value, else `void` — so a value-returning JS function is not
                // forced to `void` (which would reject every `return x`).
                None => {
                    if f.is_async {
                        Ty::Promise
                    } else if block_returns_value(&f.body) {
                        Ty::Any
                    } else {
                        Ty::Void
                    }
                }
            };
            let optional_count = {
                let mut count = 0;
                for (index, p) in f.params.iter().enumerate().rev() {
                    if p.is_rest {
                        continue;
                    }
                    if p.default_value.is_some() || params.get(index) == Some(&Ty::Any) {
                        count += 1;
                    } else {
                        break;
                    }
                }
                count
            };
            self.signatures.push(Signature {
                name,
                params,
                return_ty,
                // Trailing defaulted parameters, and trailing dynamic (`any`) ones —
                // JS lets any argument be omitted, which reads as `undefined`.
                optional_count,
                rest_param: f.params.iter().position(|parameter| parameter.is_rest),
                is_generator: f.is_generator,
                is_static: false,
            });
        }

        // Method signatures follow the top-level functions, in (class, method)
        // order. Instance methods have an implicit leading `this`; static
        // methods intentionally do not.
        self.methods_by_class = vec![HashMap::new(); program.classes.len()];
        for (class_idx, class) in program.classes.iter().enumerate() {
            for m in &class.methods {
                let base_name = self.text(&m.name).to_string();
                let name = match m.accessor {
                    Some(ast::AccessorKind::Get) => format!("get_{base_name}"),
                    Some(ast::AccessorKind::Set) => format!("set_{base_name}"),
                    None => base_name,
                };
                let receiver_ty = match self.text(&class.name) {
                    "String" => Ty::String,
                    "Number" => Ty::F64,
                    "Array" => Ty::Array,
                    "Map" => Ty::Map,
                    "Set" => Ty::Set,
                    "WeakMap" => Ty::WeakMap,
                    "WeakSet" => Ty::WeakSet,
                    "WeakRef" => Ty::WeakRef,
                    "RegExp" => Ty::RegExp,
                    "BigInt" => Ty::BigInt,
                    "FinalizationRegistry" => Ty::FinalizationRegistry,
                    _ => Ty::Instance(class_idx as u32),
                };
                // A static method that mentions `this`/`super` receives the
                // receiving class value as a hidden first parameter.
                let hidden_class_param = m.is_static
                    && !m.is_extern
                    && self
                        .source
                        .get(m.body.span.start as usize..m.body.span.end as usize)
                        .is_some_and(|text| {
                            text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                                .any(|word| word == "this" || word == "super")
                        });
                let mut params = if m.is_static {
                    if hidden_class_param { vec![Ty::Any] } else { Vec::new() }
                } else {
                    vec![receiver_ty]
                };
                for p in &m.params {
                    params.push(match &p.ty {
                        Some(t) => self.resolve_annotation(t),
                        // An untyped parameter is dynamic (`any`), matching JS.
                        None => Ty::Any,
                    });
                }
                let fluent = m.return_type.is_none()
                    && m.accessor.is_none()
                    && !m.is_static
                    && !m.is_async
                    && !m.is_generator
                    && !program.classes.iter().any(|other| {
                        other.base.is_some_and(|base| self.text(&base) == self.text(&class.name))
                    })
                    && self.block_returns_only_this(&m.body);
                let return_ty = match &m.return_type {
                    Some(t) => self.resolve_annotation(t),
                    None => {
                        if fluent {
                            Ty::Instance(class_idx as u32)
                        } else if block_returns_value(&m.body) {
                            Ty::Any
                        } else {
                            Ty::Void
                        }
                    }
                };
                let func_index = self.signatures.len() as u32;
                if self.methods_by_class[class_idx]
                    .insert(name.clone(), func_index)
                    .is_some()
                {
                    self.error(m.name.span, format!("duplicate method `{name}`"));
                }
                let name = if hidden_class_param {
                    self.static_this.insert(func_index, class_idx as u32);
                    format!("{name}@this")
                } else {
                    name
                };
                self.signatures.push(Signature {
                    name,
                    params,
                    return_ty,
                    optional_count: m
                        .params
                        .iter()
                        .rev()
                        .take_while(|p| p.default_value.is_some())
                        .count(),
                    rest_param: m
                        .params
                        .iter()
                        .position(|parameter| parameter.is_rest)
                        .map(|index| index + 1),
                    is_generator: false,
                    is_static: m.is_static,
                });
            }
        }

        // Computed-symbol-key methods follow all vtable methods, in (class,
        // method) order, so every existing func index is unchanged. Each is
        // modeled as a static function (no implicit `this`) and recorded for
        // the module-initializer side-table registration.
        for (class_idx, class) in program.classes.iter().enumerate() {
            for sm in &class.symbol_methods {
                // A non-static symbol method receives the instance as an implicit
                // leading `this` (flagged by the `@this` name suffix, which the
                // interpreter's `with_this_arg` keys on); a static one has none.
                let takes_this = !sm.function.is_static;
                let mut params = if takes_this {
                    vec![Ty::Instance(class_idx as u32)]
                } else {
                    Vec::new()
                };
                for p in &sm.function.params {
                    params.push(match &p.ty {
                        Some(t) => self.resolve_annotation(t),
                        // An untyped parameter is dynamic (`any`), matching JS.
                        None => Ty::Any,
                    });
                }
                let return_ty = match &sm.function.return_type {
                    Some(t) => self.resolve_annotation(t),
                    None => {
                        if block_returns_value(&sm.function.body) {
                            Ty::Any
                        } else {
                            Ty::Void
                        }
                    }
                };
                let func_index = self.signatures.len() as u32;
                self.signatures.push(Signature {
                    name: if takes_this {
                        format!("[symbol]@{class_idx}@this")
                    } else {
                        format!("[symbol]@{class_idx}")
                    },
                    params,
                    return_ty,
                    optional_count: sm
                        .function
                        .params
                        .iter()
                        .rev()
                        .take_while(|p| p.default_value.is_some())
                        .count(),
                    rest_param: sm
                        .function
                        .params
                        .iter()
                        .position(|parameter| parameter.is_rest)
                        .map(|index| index + usize::from(takes_this)),
                    is_generator: false,
                    is_static: !takes_this,
                });
                match self.well_known_symbol_of_key(&sm.key) {
                    Some(symbol) => {
                        self.class_symbol_methods.push((
                            class_idx as u32,
                            symbol.id().get(),
                            func_index,
                        ));
                        if symbol == zun_core::WellKnownSymbol::HasInstance {
                            self.classes_with_has_instance.insert(class_idx as u32);
                        }
                        if symbol == zun_core::WellKnownSymbol::Iterator {
                            self.classes_with_iterator.insert(class_idx as u32);
                        }
                    }
                    None => self.error(
                        sm.key.span(),
                        "a class computed member key must be a well-known Symbol \
                         (e.g. `Symbol.hasInstance`, `Symbol.iterator`)",
                    ),
                }
            }
        }
    }

    /// Recognize a computed class-member key that is a well-known Symbol
    /// (`Symbol.hasInstance`, `Symbol.iterator`, …).
    /// `re[Symbol.replace|match|matchAll|split|search](str, ...)` on a RegExp
    /// value (a literal, or any expression statically typed `RegExp`).
    pub(in crate::check) fn is_regex_symbol_call(&mut self, cx: &mut FunctionCx, callee: &ast::Expr, args: &[ast::Expr]) -> bool {
        let (ast::Expr::Index { object, index, .. }, Some(first)) = (callee, args.first()) else { return false };
        if matches!(first, ast::Expr::Spread { .. }) {
            return false;
        }
        if !matches!(
            self.well_known_symbol_of_key(index),
            Some(zun_core::WellKnownSymbol::Split)
                | Some(zun_core::WellKnownSymbol::Replace)
                | Some(zun_core::WellKnownSymbol::Match)
                | Some(zun_core::WellKnownSymbol::Search)
                | Some(zun_core::WellKnownSymbol::MatchAll)
        ) {
            return false;
        }
        matches!(object.as_ref(), ast::Expr::RegexLiteral { .. }) || self.check_expr(cx, object).ty == Ty::RegExp
    }

    pub(in crate::check) fn well_known_symbol_of_key(&self, key: &ast::Expr) -> Option<zun_core::WellKnownSymbol> {
        let (object, field) = match key {
            ast::Expr::Field { object, field, .. } => {
                let ast::Expr::Ident(name) = object.as_ref() else { return None };
                (self.text(name), self.text(field))
            }
            ast::Expr::StaticField { class, field, .. } => (self.text(class), self.text(field)),
            _ => return None,
        };
        if object != "Symbol" || self.by_name.contains_key("Symbol") {
            return None;
        }
        zun_core::WellKnownSymbol::from_member(field)
    }
}
