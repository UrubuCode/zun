use super::*;

/// First index in `Class::methods` for each (name, accessor, static) member.
type MethodIndex =
    std::collections::HashMap<(String, Option<crate::ast::AccessorKind>, bool), usize>;

thread_local! {
    /// Per-compilation counter for synthetic computed-field key/slot names
    /// (`__ck{n}` / `__cf{n}`). Reset at the top of `parse_subset` so the
    /// generated names are deterministic across repeated compiles in the same
    /// process / thread.
    pub(super) static COMPUTED_FIELD_COUNTER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Map an OXC decorator node to our [`crate::ast::Decorator`].
///
/// Phase 1 is syntax only. The callee is the decorator expression (an
/// identifier or member-access chain); the `@foo(...)` call form carries its
/// arguments, a bare `@foo` carries `None`. Unsupported callee/argument forms
/// surface as an explicit error (no silent drop) because `convert_expression`
/// and `convert_argument` reject what they cannot represent.
pub(super) fn convert_decorator(
    source: &str,
    decorator: &OxcDecorator<'_>,
) -> Result<crate::ast::Decorator, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let (callee, args) = match &decorator.expression {
        Expression::CallExpression(call) => {
            let mut args = Vec::with_capacity(call.arguments.len());
            for argument in &call.arguments {
                args.push(convert_argument(source, argument)?);
            }
            (convert_expression(source, &call.callee)?, Some(args))
        }
        other => (convert_expression(source, other)?, None),
    };
    Ok(crate::ast::Decorator {
        callee,
        args,
        span: span(decorator.span),
    })
}

pub(super) fn convert_class_declaration(
    source: &str,
    class_declaration: &oxc_ast::ast::Class<'_>,
) -> Result<crate::ast::Class, Vec<String>> {
    let mut names: Vec<(String, bool)> = Vec::new();
    for element in &class_declaration.body.body {
        match element {
            ClassElement::PropertyDefinition(property) => {
                if let PropertyKey::PrivateIdentifier(identifier) = &property.key {
                    names.push((identifier.name.to_string(), property.r#static));
                }
            }
            ClassElement::MethodDefinition(method) => {
                if let PropertyKey::PrivateIdentifier(identifier) = &method.key {
                    names.push((identifier.name.to_string(), method.r#static));
                }
            }
            _ => {}
        }
    }
    let id_span = class_declaration
        .id
        .as_ref()
        .map(|id| Span::new(id.span.start, id.span.end))
        .unwrap_or(Span::new(
            class_declaration.span.start,
            class_declaration.span.end,
        ));
    PRIVATE_SCOPES.with(|scopes| scopes.borrow_mut().push((id_span, names)));
    let result = convert_class_declaration_inner(source, class_declaration);
    PRIVATE_SCOPES.with(|scopes| scopes.borrow_mut().pop());
    result
}

pub(super) fn convert_class_declaration_inner(
    source: &str,
    class_declaration: &oxc_ast::ast::Class<'_>,
) -> Result<crate::ast::Class, Vec<String>> {
    // Anonymous class expressions get their name patched by the caller.
    let id_span = class_declaration
        .id
        .as_ref()
        .map(|id| id.span)
        .unwrap_or(class_declaration.span);
    let base = convert_heritage(source, class_declaration)?;
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut method_index = MethodIndex::new();
    let mut symbol_methods = Vec::new();
    for element in &class_declaration.body.body {
        match element {
            ClassElement::PropertyDefinition(property)
                if property.computed
                    && property.key.as_expression().is_some()
                    && class_declaration.id.is_some() =>
            {
                convert_computed_field(source, class_declaration, property, &mut fields)?;
            }
            ClassElement::PropertyDefinition(property) => {
                convert_field(source, property, &mut fields)?;
            }
            ClassElement::MethodDefinition(method) => {
                convert_method(
                    source,
                    class_declaration,
                    method,
                    &mut methods,
                    &mut method_index,
                    &mut symbol_methods,
                )?;
            }
            ClassElement::StaticBlock(block) => {
                convert_static_block(source, block, &mut fields)?;
            }
            _ => {
                return Err(vec![
                    "OXC adapter does not support this class element yet".into()
                ])
            }
        }
    }
    // JS classes create fields by assignment: `this.name = …` in a
    // constructor of a base-less class declares an `any` field.
    if base.is_none() {
        infer_constructor_fields(source, class_declaration, &mut fields, &methods);
    }
    Ok(crate::ast::Class {
        name: Name {
            span: Span::new(id_span.start, id_span.end),
        },
        // OXC adapter erases generic type parameters (see above).
        type_params: Vec::new(),
        base,
        decorators: class_declaration
            .decorators
            .iter()
            .map(|decorator| convert_decorator(source, decorator))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| vec![error])?,
        fields,
        methods,
        symbol_methods,
        span: Span::new(class_declaration.span.start, class_declaration.span.end),
    })
}

/// Peel parentheses and TS casts off a heritage expression.
fn peel_heritage<'a, 'b>(mut expression: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        match expression {
            Expression::ParenthesizedExpression(inner) => expression = &inner.expression,
            Expression::TSAsExpression(inner) => expression = &inner.expression,
            Expression::TSNonNullExpression(inner) => expression = &inner.expression,
            _ => return expression,
        }
    }
}

/// Resolve the `extends` clause to a static base class name.
///
/// A bare identifier (possibly cast) is the base. Any other expression is
/// evaluated once, at the class definition point, into a hidden temp (so side
/// effects, ordering and thrown errors are preserved); the base is then taken
/// from the `typeof Name` return annotation of a called top-level function when
/// one is declared, and the class is base-less otherwise.
fn convert_heritage(
    source: &str,
    class_declaration: &oxc_ast::ast::Class<'_>,
) -> Result<Option<Name>, Vec<String>> {
    let Some(heritage) = class_declaration.heritage.as_ref() else {
        return Ok(None);
    };
    let expression = peel_heritage(&heritage.expression);
    if matches!(expression, Expression::NullLiteral(_)) {
        return Ok(None);
    }
    let self_name = class_declaration.id.as_ref().map(|id| id.name.as_str());
    if let Expression::Identifier(identifier) = expression {
        if Some(identifier.name.as_str()) != self_name {
            return Ok(Some(Name {
                span: Span::new(identifier.span.start, identifier.span.end),
            }));
        }
    }
    let value = convert_expression(source, expression).map_err(|error| vec![error])?;
    let span = Span::new(expression.span().start, expression.span().end);
    let n = COMPUTED_FIELD_COUNTER.with(|counter| {
        let n = counter.get();
        counter.set(n + 1);
        n
    });
    CLASS_TRAILING.with(|cells| {
        cells.borrow_mut().push(Stmt::Let {
            name: crate::ast::synthetic_ident(&format!("__hb{n}")),
            ty: None,
            init: value,
            span,
        })
    });
    // `f()` where `function f(...): typeof Name` names its base statically.
    if let Expression::CallExpression(call) = expression {
        if let Expression::Identifier(callee) = &call.callee {
            if let Some((start, end)) = declared_typeof_return(source, callee.name.as_str()) {
                return Ok(Some(Name {
                    span: Span::new(start as u32, end as u32),
                }));
            }
        }
    }
    Ok(None)
}

/// Locate `function <callee>(...): typeof <Name>` in the source and return the
/// byte range of `<Name>`.
fn declared_typeof_return(source: &str, callee: &str) -> Option<(usize, usize)> {
    let needle = format!("function {callee}(");
    let at = source.find(&needle)?;
    let rest = &source[at..];
    let close = rest.find(')')?;
    let after = rest[close + 1..].trim_start();
    let after = after.strip_prefix(':')?.trim_start();
    let after = after.strip_prefix("typeof")?;
    let trimmed = after.trim_start();
    let len = trimmed
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(trimmed.len());
    if len == 0 {
        return None;
    }
    let name_start = source.len() - trimmed.len();
    Some((name_start, name_start + len))
}

/// A field with a run-time computed key. The key is evaluated once, when the
/// class is defined (a module-level temp); every instance then defines the
/// property from a hidden initializer slot, which keeps the initialization
/// order of the fields.
fn convert_computed_field(
    source: &str,
    class_declaration: &oxc_ast::ast::Class<'_>,
    property: &oxc_ast::ast::PropertyDefinition<'_>,
    fields: &mut Vec<crate::ast::ClassField>,
) -> Result<(), Vec<String>> {
    let n = COMPUTED_FIELD_COUNTER.with(|counter| {
        let n = counter.get();
        counter.set(n + 1);
        n
    });
    let member_span = Span::new(property.span.start, property.span.end);
    let key_expression = property
        .key
        .as_expression()
        .ok_or_else(|| vec!["OXC adapter requires an expression computed field key".to_owned()])?;
    let key = convert_expression(source, key_expression).map_err(|error| vec![error])?;
    let value = match property.value.as_ref() {
        Some(expression) => convert_expression(source, expression).map_err(|error| vec![error])?,
        None => Expr::Unary {
            op: crate::ast::UnaryOp::Void,
            operand: Box::new(Expr::Number {
                value: 0.0,
                span: member_span,
            }),
            span: member_span,
        },
    };
    let slot = |name: &str, value: Expr| {
        crate::ast::ObjectMember::Property(crate::ast::ObjectProperty {
            key: crate::ast::PropertyKey::Ident(crate::ast::synthetic_ident(name)),
            value,
            kind: crate::ast::PropertyKind::Init,
        })
    };
    let flag = |name: &str| {
        slot(
            name,
            Expr::Bool {
                value: true,
                span: member_span,
            },
        )
    };
    let define = |target: Expr, key: Expr, value: Expr| Expr::MethodCall {
        receiver: Box::new(Expr::Ident(crate::ast::synthetic_ident("Object"))),
        method: crate::ast::synthetic_ident("defineProperty"),
        args: vec![
            target,
            key,
            Expr::ObjectLiteral {
                properties: vec![
                    slot("value", value),
                    flag("writable"),
                    flag("enumerable"),
                    flag("configurable"),
                ],
                span: member_span,
            },
        ],
        span: member_span,
    };
    let class_id = class_declaration
        .id
        .as_ref()
        .ok_or_else(|| vec!["OXC adapter requires a named class".to_owned()])?;
    if property.r#static {
        let class_ref = Expr::Ident(Name {
            span: Span::new(class_id.span.start, class_id.span.end),
        });
        CLASS_TRAILING.with(|cells| {
            cells
                .borrow_mut()
                .push(Stmt::Expr(define(class_ref, key, value)))
        });
    } else {
        let key_temp = crate::ast::synthetic_ident(&format!("__ck{n}"));
        CLASS_TRAILING.with(|cells| {
            cells.borrow_mut().push(Stmt::Let {
                name: key_temp,
                ty: None,
                init: key,
                span: member_span,
            })
        });
        fields.push(crate::ast::ClassField {
            name: crate::ast::synthetic_ident(&format!("__cf{n}")),
            decorators: Vec::new(),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(property.span.start, property.span.start),
                },
                is_array: false,
            },
            is_static: false,
            is_private: true,
            initializer: Some(Box::new(define(
                Expr::Ident(crate::ast::synthetic_ident("this")),
                Expr::Ident(key_temp),
                value,
            ))),
        });
    }
    Ok(())
}

/// A plain (identifier-keyed) class field.
fn convert_field(
    source: &str,
    property: &oxc_ast::ast::PropertyDefinition<'_>,
    fields: &mut Vec<crate::ast::ClassField>,
) -> Result<(), Vec<String>> {
    let (field_span, is_private) = match &property.key {
        PropertyKey::StaticIdentifier(identifier) => {
            (Span::new(identifier.span.start, identifier.span.end), false)
        }
        PropertyKey::PrivateIdentifier(identifier) => {
            (Span::new(identifier.span.start, identifier.span.end), true)
        }
        _ => {
            return Err(vec![
                "OXC adapter supports identifier class fields only".into()
            ])
        }
    };
    // A field without an explicit (or nameable) type erases
    // to `any`, encoded as a zero-width type span that sema
    // resolves to `Ty::Any` (JS/TS erasure semantics).
    let type_annotation = convert_type_annotation(source, property.type_annotation.as_deref())
        .map_err(|error| vec![error])?
        .unwrap_or(crate::ast::TypeAnnotation {
            name: Name {
                span: Span::new(field_span.start, field_span.start),
            },
            is_array: false,
        });
    fields.push(crate::ast::ClassField {
        name: Name { span: field_span },
        decorators: property
            .decorators
            .iter()
            .map(|decorator| convert_decorator(source, decorator))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| vec![error])?,
        ty: type_annotation,
        is_static: property.r#static,
        is_private,
        // A field without an initializer defaults to
        // `undefined` (`void 0`), so the implicit
        // constructor always has a value for every slot.
        initializer: Some(Box::new(match property.value.as_ref() {
            Some(expression) => {
                convert_expression(source, expression).map_err(|error| vec![error])?
            }
            None => Expr::Unary {
                op: crate::ast::UnaryOp::Void,
                operand: Box::new(Expr::Number {
                    value: 0.0,
                    span: field_span,
                }),
                span: field_span,
            },
        })),
    });
    Ok(())
}

/// A `static { ... }` block runs once with the other static initializers: a
/// synthetic static field (named by the block's own unique source text) whose
/// initializer is an immediately invoked arrow over the block body.
fn convert_static_block(
    source: &str,
    block: &oxc_ast::ast::StaticBlock<'_>,
    fields: &mut Vec<crate::ast::ClassField>,
) -> Result<(), Vec<String>> {
    let block_span = Span::new(block.span.start, block.span.end);
    let statements =
        convert_function_statements(source, &block.body, &[]).map_err(|error| vec![error])?;
    let arrow = Expr::Function(FunctionExpr {
        params: Vec::new(),
        return_type: convert_type_annotation(source, None).map_err(|error| vec![error])?,
        body: Block {
            statements,
            span: block_span,
        },
        is_arrow: true,
        is_async: false,
        is_generator: false,
        span: block_span,
    });
    fields.push(crate::ast::ClassField {
        name: Name { span: block_span },
        decorators: Vec::new(),
        ty: crate::ast::TypeAnnotation {
            name: Name {
                span: Span::new(block_span.start, block_span.start),
            },
            is_array: false,
        },
        is_static: true,
        is_private: false,
        initializer: Some(Box::new(Expr::ComputedCall {
            callee: Box::new(arrow),
            args: Vec::new(),
            span: block_span,
        })),
    });
    Ok(())
}

/// JS classes create fields by assignment: `this.name = …` in a constructor
/// of a base-less class declares an `any` field.
fn infer_constructor_fields(
    source: &str,
    class_declaration: &oxc_ast::ast::Class<'_>,
    fields: &mut Vec<crate::ast::ClassField>,
    methods: &[Function],
) {
    for element in &class_declaration.body.body {
        let ClassElement::MethodDefinition(method) = element else {
            continue;
        };
        if method.kind != oxc_ast::ast::MethodDefinitionKind::Constructor {
            continue;
        }
        let (start, end) = (method.span.start as usize, method.span.end as usize);
        let text = &source[start..end];
        let bytes = text.as_bytes();
        let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
        // Both `this.name =` and `this['name'] =` spellings.
        let mut candidates: Vec<(usize, usize)> = Vec::new();
        for (marker, skip) in [("this.", 5usize), ("this['", 6), ("this[\"", 6), ("(this as any).", 14)] {
            let mut from = 0;
            while let Some(found) = text[from..].find(marker) {
                let at = from + found;
                from = at + skip;
                if at > 0 && (is_word(bytes[at - 1]) || bytes[at - 1] == b'.') {
                    continue;
                }
                candidates.push((at, skip));
            }
        }
        for (at, skip) in candidates {
            let quoted = skip == 6;
            let name_start = at + skip;
            let mut name_end = name_start;
            while name_end < bytes.len() && is_word(bytes[name_end]) {
                name_end += 1;
            }
            if name_end == name_start || !is_plain_identifier(&text[name_start..name_end]) {
                continue;
            }
            let mut cursor = name_end;
            if quoted {
                // closing quote and bracket
                if bytes.get(cursor).is_none_or(|b| *b != 39u8 && *b != b'"')
                    || bytes.get(cursor + 1) != Some(&b']')
                {
                    continue;
                }
                cursor += 2;
            }
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let assigns = cursor < bytes.len()
                && bytes[cursor] == b'='
                && !matches!(bytes.get(cursor + 1), Some(b'=') | Some(b'>'));
            if !assigns {
                continue;
            }
            let field_name = &text[name_start..name_end];
            let known = fields.iter().any(|field: &crate::ast::ClassField| {
                source.get(field.name.span.start as usize..field.name.span.end as usize)
                    == Some(field_name)
            }) || methods.iter().any(|method: &crate::ast::Function| {
                source.get(method.name.span.start as usize..method.name.span.end as usize)
                    == Some(field_name)
            });
            if known {
                continue;
            }
            let field_span = Span::new((start + name_start) as u32, (start + name_end) as u32);
            fields.push(crate::ast::ClassField {
                name: Name { span: field_span },
                decorators: Vec::new(),
                ty: crate::ast::TypeAnnotation {
                    name: Name {
                        span: Span::new(field_span.start, field_span.start),
                    },
                    is_array: false,
                },
                is_static: false,
                is_private: false,
                initializer: Some(Box::new(Expr::Unary {
                    op: crate::ast::UnaryOp::Void,
                    operand: Box::new(Expr::Number {
                        value: 0.0,
                        span: field_span,
                    }),
                    span: field_span,
                })),
            });
        }
    }
}

/// Convert one class method, accessor or computed-key member and register it
/// on the class.
fn convert_method(
    source: &str,
    class_declaration: &oxc_ast::ast::Class<'_>,
    method: &oxc_ast::ast::MethodDefinition<'_>,
    methods: &mut Vec<Function>,
    method_index: &mut MethodIndex,
    symbol_methods: &mut Vec<crate::ast::ClassSymbolMethod>,
) -> Result<(), Vec<String>> {
    let (name_span, computed_key) = method_name_and_key(source, method)?;
    let Some(body) = method.value.body.as_ref() else {
        // `extern m(..): T;` arrives as a bodiless method
        // signature (see `preprocess_externs`).
        if let Some(module) = extern_mark(name_span.start) {
            push_extern_method(source, method, name_span, module, methods, method_index)?;
            return Ok(());
        }
        return Err(vec!["OXC adapter requires class method bodies".into()]);
    };
    let function = build_method_function(source, method, body, name_span)?;
    if install_runtime_computed_method(source, class_declaration, method, &computed_key)? {
        return Ok(());
    }
    register_method(
        source,
        method,
        computed_key,
        function,
        methods,
        method_index,
        symbol_methods,
    );
    Ok(())
}

/// A computed key (`[Symbol.iterator]() {}`) is lowered
/// to a Symbol-keyed side-table entry; a plain name is a
/// vtable method. `name_span` carries the key span in the
/// computed case (used only for diagnostics).
/// `['name']() {}` with a plain identifier-shaped literal is a named method.
fn method_name_and_key(
    source: &str,
    method: &oxc_ast::ast::MethodDefinition<'_>,
) -> Result<(Span, Option<Expr>), Vec<String>> {
    let literal_name = match (&method.key, method.computed) {
        (PropertyKey::StringLiteral(text), true)
            if text.value.as_str().is_some_and(is_plain_identifier)
                && (text.span.end - text.span.start) as usize
                    == text.value.as_str().map_or(0, str::len) + 2 =>
        {
            Some(Span::new(text.span.start + 1, text.span.end - 1))
        }
        _ => None,
    };
    let (name_span, computed_key) = if let Some(literal) = literal_name {
        (literal, None)
    } else if method.computed {
        let key_expression = method.key.as_expression().ok_or_else(|| {
            vec!["OXC adapter requires an expression computed method key".to_owned()]
        })?;
        let key = convert_expression(source, key_expression).map_err(|error| vec![error])?;
        (key.span(), Some(key))
    } else if let PropertyKey::StaticIdentifier(identifier) = &method.key {
        (Span::new(identifier.span.start, identifier.span.end), None)
    } else if let PropertyKey::PrivateIdentifier(identifier) = &method.key {
        (Span::new(identifier.span.start, identifier.span.end), None)
    } else {
        return Err(vec![
            "OXC adapter supports identifier class method names only".into(),
        ]);
    };
    Ok((name_span, computed_key))
}

/// A bodiless `extern` method signature becomes an extern `Function`.
fn push_extern_method(
    source: &str,
    method: &oxc_ast::ast::MethodDefinition<'_>,
    name_span: Span,
    module: Option<String>,
    methods: &mut Vec<Function>,
    method_index: &mut MethodIndex,
) -> Result<(), Vec<String>> {
    let mut prologue = Vec::new();
    let mut params = Vec::new();
    for parameter in &method.value.params.items {
        params.push(
            convert_formal_param(source, parameter, &mut prologue).map_err(|error| vec![error])?,
        );
    }
    let function = Function {
        name: Name { span: name_span },
        type_params: Vec::new(),
        decorators: Vec::new(),
        params,
        return_type: convert_type_annotation(source, method.value.return_type.as_deref())
            .map_err(|error| vec![error])?,
        is_extern: true,
        is_static: method.r#static,
        accessor: None,
        external_module: module,
        is_async: false,
        is_generator: false,
        body: Block {
            statements: Vec::new(),
            span: Span::new(method.span.start, method.span.end),
        },
        span: Span::new(method.span.start, method.span.end),
    };
    // Keep the first index for a given member, matching `find`'s semantics.
    let key = (
        source[function.name.span.start as usize..function.name.span.end as usize].to_owned(),
        function.accessor,
        function.is_static,
    );
    method_index.entry(key).or_insert(methods.len());
    methods.push(function);
    Ok(())
}

fn build_method_function(
    source: &str,
    method: &oxc_ast::ast::MethodDefinition<'_>,
    body: &oxc_ast::ast::FunctionBody<'_>,
    name_span: Span,
) -> Result<Function, Vec<String>> {
    // The method body uses the unified statement converter so
    // class methods support the same statement surface as any
    // other function body.
    let mut method_prologue = Vec::new();
    let mut params = Vec::with_capacity(method.value.params.items.len());
    for parameter in &method.value.params.items {
        params.push(
            convert_formal_param(source, parameter, &mut method_prologue)
                .map_err(|error| vec![error])?,
        );
    }
    let mut method_statements = method_prologue;
    method_statements.extend(
        convert_function_statements(source, &body.statements, &param_names(&method.value.params))
            .map_err(|error| vec![error])?,
    );
    if let Some(rest) = method.value.params.rest.as_ref() {
        let BindingPattern::BindingIdentifier(identifier) = &rest.rest.argument else {
            return Err(vec![
                "OXC adapter supports identifier rest parameters only".into()
            ]);
        };
        params.push(crate::ast::Param {
            name: Name {
                span: Span::new(identifier.span.start, identifier.span.end),
            },
            ty: convert_type_annotation(source, rest.type_annotation.as_deref())
                .map_err(|error| vec![error])?,
            default_value: None,
            is_rest: true,
        });
    }
    inject_arguments_object(
        source,
        Span::new(method.span.start, method.span.end),
        &mut params,
        &mut method_statements,
    );
    let decorators = method
        .decorators
        .iter()
        .map(|decorator| convert_decorator(source, decorator))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| vec![error])?;
    let function = Function {
        name: Name { span: name_span },
        // The OXC adapter is a partial secondary path; it
        // erases generic type parameters (the hand parser
        // carries them). Generics are erased either way.
        type_params: Vec::new(),
        decorators,
        params,
        return_type: convert_type_annotation(source, method.value.return_type.as_deref())
            .map_err(|error| vec![error])?,
        is_extern: false,
        // A computed-key member is modeled as a static
        // method (no implicit `this`).
        is_static: method.r#static,
        accessor: match method.kind {
            MethodDefinitionKind::Get => Some(crate::ast::AccessorKind::Get),
            MethodDefinitionKind::Set => Some(crate::ast::AccessorKind::Set),
            _ => None,
        },
        external_module: None,
        is_async: method.value.r#async,
        is_generator: method.value.generator,
        body: Block {
            statements: method_statements,
            span: Span::new(body.span.start, body.span.end),
        },
        span: Span::new(method.span.start, method.span.end),
    };
    Ok(function)
}

/// A computed key that is not a well-known `Symbol.x` is only known at run
/// time: the member is installed on the class (or its prototype) by a
/// statement emitted after the declaration. Returns whether it was installed.
fn install_runtime_computed_method(
    source: &str,
    class_declaration: &oxc_ast::ast::Class<'_>,
    method: &oxc_ast::ast::MethodDefinition<'_>,
    computed_key: &Option<Expr>,
) -> Result<bool, Vec<String>> {
    if let (Some(key), Some(class_id)) = (computed_key.as_ref(), class_declaration.id.as_ref()) {
        let well_known = matches!(key, Expr::Field { object, .. }
            if matches!(object.as_ref(), Expr::Ident(n) if source.get(n.span.start as usize..n.span.end as usize) == Some("Symbol")));
        if !well_known {
            let function_value =
                convert_oxc_function(source, &method.value).map_err(|error| vec![error])?;
            let class_ref = Expr::Ident(Name {
                span: Span::new(class_id.span.start, class_id.span.end),
            });
            let target = if method.r#static {
                class_ref
            } else {
                Expr::Field {
                    object: Box::new(class_ref),
                    field: crate::ast::synthetic_ident("prototype"),
                    span: Span::new(method.span.start, method.span.end),
                }
            };
            let flag = |name: &str, value: bool| {
                crate::ast::ObjectMember::Property(crate::ast::ObjectProperty {
                    key: crate::ast::PropertyKey::Ident(crate::ast::synthetic_ident(name)),
                    value: Expr::Bool {
                        value,
                        span: Span::new(method.span.start, method.span.end),
                    },
                    kind: crate::ast::PropertyKind::Init,
                })
            };
            let slot = |name: &str, value: Expr| {
                crate::ast::ObjectMember::Property(crate::ast::ObjectProperty {
                    key: crate::ast::PropertyKey::Ident(crate::ast::synthetic_ident(name)),
                    value,
                    kind: crate::ast::PropertyKind::Init,
                })
            };
            let members = match method.kind {
                MethodDefinitionKind::Get => {
                    vec![slot("get", function_value), flag("configurable", true)]
                }
                MethodDefinitionKind::Set => {
                    vec![slot("set", function_value), flag("configurable", true)]
                }
                _ => vec![
                    slot("value", function_value),
                    flag("writable", true),
                    flag("configurable", true),
                ],
            };
            let member_span = Span::new(method.span.start, method.span.end);
            CLASS_TRAILING.with(|cells| {
                cells.borrow_mut().push(Stmt::Expr(Expr::MethodCall {
                    receiver: Box::new(Expr::Ident(crate::ast::synthetic_ident("Object"))),
                    method: crate::ast::synthetic_ident("defineProperty"),
                    args: vec![
                        target,
                        key.clone(),
                        Expr::ObjectLiteral {
                            properties: members,
                            span: member_span,
                        },
                    ],
                    span: member_span,
                }))
            });
            return Ok(true);
        }
    }
    Ok(false)
}

fn register_method(
    source: &str,
    method: &oxc_ast::ast::MethodDefinition<'_>,
    computed_key: Option<Expr>,
    function: Function,
    methods: &mut Vec<Function>,
    method_index: &mut MethodIndex,
    symbol_methods: &mut Vec<crate::ast::ClassSymbolMethod>,
) {
    match computed_key {
        Some(key) => symbol_methods.push(crate::ast::ClassSymbolMethod {
            key,
            function,
            span: Span::new(method.span.start, method.span.end),
        }),
        None => {
            // A later declaration of the same member (same
            // name, accessor kind, staticness) replaces the
            // earlier one, as in JS.
            let key = (
                source[function.name.span.start as usize..function.name.span.end as usize]
                    .to_owned(),
                function.accessor,
                function.is_static,
            );
            if let Some(&index) = method_index.get(&key) {
                methods[index] = function;
            } else {
                method_index.insert(key, methods.len());
                methods.push(function);
            }
        }
    }
}
