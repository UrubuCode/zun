use super::*;

/// A member-expression assignment target (`o.f`, `o[k]`, `o.#p`).
pub(super) enum MemberRef<'a, 'b> {
    Static(&'a oxc_ast::ast::StaticMemberExpression<'b>),
    Computed(&'a oxc_ast::ast::ComputedMemberExpression<'b>),
    Private(&'a oxc_ast::ast::PrivateFieldExpression<'b>),
}

/// How a decomposed member target stores a value.
pub(super) enum MemberStore {
    Field { object: Expr, field: Name },
    Index { object: Expr, index: Expr },
}

impl MemberStore {
    pub(super) fn build(&self, value: Expr, span: Span) -> Expr {
        match self {
            MemberStore::Field { object, field } => Expr::FieldAssign {
                object: Box::new(object.clone()),
                field: *field,
                value: Box::new(value),
                span,
            },
            MemberStore::Index { object, index } => Expr::IndexAssign {
                object: Box::new(object.clone()),
                index: Box::new(index.clone()),
                value: Box::new(value),
                span,
            },
        }
    }
}

pub(super) fn is_simple_operand(expression: &Expression<'_>) -> bool {
    matches!(
        expression,
        Expression::Identifier(_)
            | Expression::ThisExpression(_)
            | Expression::Super(_)
            | Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
    )
}

/// Split a member target into the sub-expressions that must be evaluated once
/// (`binds`, outermost first), a read of the target, and a store builder; the
/// read/store reference the bound values through [`Expr::ChainRoot`].
pub(super) fn decompose_member(
    source: &str,
    member: MemberRef<'_, '_>,
) -> Result<(Vec<Expr>, Expr, MemberStore), String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let (object_ast, key_ast) = match &member {
        MemberRef::Static(m) => (&m.object, None),
        MemberRef::Computed(m) => (&m.object, Some(&m.expression)),
        MemberRef::Private(m) => (&m.object, None),
    };
    let bind_object = !is_simple_operand(object_ast);
    let bind_key = key_ast.map(|key| !is_simple_operand(key)).unwrap_or(false);
    let count = bind_object as u32 + bind_key as u32;
    let mut binds = Vec::new();
    let object_span = span(object_ast.span());
    let object_expr = convert_expression(source, object_ast)?;
    let object = if bind_object {
        binds.push(object_expr);
        Expr::ChainRoot {
            depth: count - 1,
            span: object_span,
        }
    } else {
        object_expr
    };
    let key = match key_ast {
        Some(key) => {
            let key_span = span(key.span());
            let key_expr = convert_expression(source, key)?;
            Some(if bind_key {
                binds.push(key_expr);
                Expr::ChainRoot {
                    depth: 0,
                    span: key_span,
                }
            } else {
                key_expr
            })
        }
        None => None,
    };
    Ok(match member {
        MemberRef::Static(m) => {
            let field = Name {
                span: span(m.property.span),
            };
            (
                binds,
                Expr::Field {
                    object: Box::new(object.clone()),
                    field,
                    span: span(m.span),
                },
                MemberStore::Field { object, field },
            )
        }
        MemberRef::Computed(m) => {
            let index = key.expect("computed member has a key");
            (
                binds,
                Expr::Index {
                    object: Box::new(object.clone()),
                    index: Box::new(index.clone()),
                    span: span(m.span),
                },
                MemberStore::Index { object, index },
            )
        }
        MemberRef::Private(m) => {
            let field = Name {
                span: span(m.field.span),
            };
            let private = Expr::PrivateField {
                object: Box::new(object),
                field,
                span: span(m.span),
            };
            (
                binds,
                private.clone(),
                MemberStore::Field {
                    object: private,
                    field,
                },
            )
        }
    })
}

/// Wrap `body` in `TempBind`s for `binds` (outermost first).
pub(super) fn wrap_binds(binds: Vec<Expr>, body: Expr, span: Span) -> Expr {
    let mut result = body;
    for value in binds.into_iter().rev() {
        result = Expr::TempBind {
            value: Box::new(value),
            body: Box::new(result),
            span,
        };
    }
    result
}

pub(super) fn convert_computed_member(
    source: &str,
    member: &oxc_ast::ast::ComputedMemberExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let object = Box::new(convert_expression(source, &member.object)?);
    // `obj['name']` with a plain identifier-shaped literal is `obj.name`.
    if let Expression::StringLiteral(text) = &member.expression {
        if text.value.as_str().is_some_and(is_plain_identifier)
            && (text.span.end - text.span.start) as usize
                == text.value.as_str().map_or(0, str::len) + 2
        {
            let field = Name {
                span: Span::new(text.span.start + 1, text.span.end - 1),
            };
            return Ok(if member.optional {
                Expr::OptionalField {
                    object,
                    field,
                    span: span(member.span),
                }
            } else {
                Expr::Field {
                    object,
                    field,
                    span: span(member.span),
                }
            });
        }
    }
    let index = Box::new(convert_expression(source, &member.expression)?);
    if member.optional {
        return Ok(Expr::OptionalIndex {
            object,
            index,
            span: span(member.span),
        });
    }
    Ok(Expr::Index {
        object,
        index,
        span: span(member.span),
    })
}

pub(super) fn convert_private_in(
    source: &str,
    private_in: &oxc_ast::ast::PrivateInExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // `#x in obj` is a brand check: resolve `#x` to the innermost
    // lexical class declaring it, then test `obj instanceof Class`
    // (instance members) or `obj === Class` (static members).
    let name = private_in.left.name.to_string();
    let found = PRIVATE_SCOPES.with(|scopes| {
        scopes
            .borrow()
            .iter()
            .rev()
            .find_map(|(class_span, names)| {
                names
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, is_static)| (*class_span, *is_static))
            })
    });
    let Some((class_span, is_static)) = found else {
        return Err(format!(
            "OXC adapter: unknown private name `#{name}` in brand check"
        ));
    };
    Ok(Expr::Binary {
        op: if is_static {
            BinaryOp::Eq
        } else {
            BinaryOp::Instanceof
        },
        lhs: Box::new(convert_expression(source, &private_in.right)?),
        rhs: Box::new(Expr::Ident(Name { span: class_span })),
        span: span(private_in.span),
    })
}

pub(super) fn convert_static_member(
    source: &str,
    member: &oxc_ast::ast::StaticMemberExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let object = Box::new(convert_expression(source, &member.object)?);
    let field = Name {
        span: span(member.property.span),
    };
    if member.optional {
        Ok(Expr::OptionalField {
            object,
            field,
            span: span(member.span),
        })
    } else {
        Ok(Expr::Field {
            object,
            field,
            span: span(member.span),
        })
    }
}

pub(super) fn convert_private_field(
    source: &str,
    member: &oxc_ast::ast::PrivateFieldExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    if member.optional {
        return Err("OXC adapter does not support optional private members yet".into());
    }
    Ok(Expr::PrivateField {
        object: Box::new(convert_expression(source, &member.object)?),
        field: Name {
            span: span(member.field.span),
        },
        span: span(member.span),
    })
}
