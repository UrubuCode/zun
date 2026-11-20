use super::*;

pub(in crate::oxc) fn convert_argument(
    source: &str,
    argument: &Argument<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // `...spread` is the only argument form that is not itself an expression;
    // every other argument is an ordinary expression, so route it through the
    // full expression converter rather than re-listing a subset here.
    if let Argument::SpreadElement(spread) = argument {
        return Ok(Expr::Spread {
            expr: Box::new(convert_expression(source, &spread.argument)?),
            span: span(spread.span),
        });
    }
    let expression = argument
        .as_expression()
        .ok_or("OXC adapter cannot represent this argument")?;
    // A static `Number.f` / `Math.f` passed as a callback (`xs.filter(Number.isNaN)`)
    // becomes `(f) => Number.f(f)`; the method-name span doubles as the
    // parameter name, which is harmless because the body only reads it once.
    if let Expression::StaticMemberExpression(member) = expression {
        if let Expression::Identifier(object) = &member.object {
            let object_name = source
                .get(object.span.start as usize..object.span.end as usize)
                .unwrap_or("");
            let property = source
                .get(member.property.span.start as usize..member.property.span.end as usize)
                .unwrap_or("");
            // `prototype` is the one lowercase static that is a value, never a
            // function to wrap (`Object.keys(Number.prototype)`).
            let callable = property != "prototype"
                && property
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase());
            if matches!(object_name, "Number" | "Math") && callable && !member.optional {
                let property_name = Name {
                    span: span(member.property.span),
                };
                // `Math.max` / `Math.min` / `Math.hypot` take any number of
                // arguments, so their function value is `(...f) => Math.f(...f)`:
                // `Reflect.apply(Math.max, null, [2, 7])` reaches every element.
                let variadic = object_name == "Math" && matches!(property, "max" | "min" | "hypot");
                let argument = Expr::Ident(Name {
                    span: span(member.property.span),
                });
                let call = Expr::MethodCall {
                    receiver: Box::new(Expr::Ident(Name {
                        span: span(object.span),
                    })),
                    method: Name {
                        span: span(member.property.span),
                    },
                    args: vec![if variadic {
                        Expr::Spread {
                            expr: Box::new(argument),
                            span: span(member.span),
                        }
                    } else {
                        argument
                    }],
                    span: span(member.span),
                };
                return Ok(Expr::Function(FunctionExpr {
                    params: vec![crate::ast::Param {
                        name: property_name,
                        ty: None,
                        default_value: None,
                        is_rest: variadic,
                    }],
                    return_type: None,
                    body: Block {
                        statements: vec![Stmt::Return {
                            value: Some(call),
                            span: span(member.span),
                        }],
                        span: span(member.span),
                    },
                    is_arrow: true,
                    is_async: false,
                    is_generator: false,
                    this_class: None,
                    span: span(member.span),
                }));
            }
        }
    }
    convert_expression(source, expression)
}

/// Direct `eval("<single expression>")` of an escape-free string literal:
/// the literal's text is parsed in place (a padded copy keeps every span
/// aligned with the real source) and converted like surrounding code.
fn try_convert_direct_eval(
    source: &str,
    call: &oxc_ast::ast::CallExpression<'_>,
) -> Option<Result<Expr, String>> {
    if let (
        Expression::Identifier(callee),
        1,
        Some(oxc_ast::ast::Argument::StringLiteral(literal)),
    ) = (&call.callee, call.arguments.len(), call.arguments.first())
    {
        if callee.name == "eval" {
            let (start, end) = (literal.span.start as usize, literal.span.end as usize);
            if let Some(content) = source.get(start + 1..end.saturating_sub(1)) {
                if !content.contains('\\') {
                    let padded = format!("{}{}", " ".repeat(start + 1), content);
                    let allocator = Allocator::default();
                    let parsed =
                        Parser::new(&allocator, &padded, SourceType::mjs().with_typescript(true))
                            .parse();
                    if parsed.diagnostics.is_empty() && parsed.program.body.len() == 1 {
                        if let Statement::ExpressionStatement(statement) = &parsed.program.body[0] {
                            return Some(convert_expression(&padded, &statement.expression));
                        }
                    }
                }
            }
        }
    }
    None
}

pub(super) fn convert_call(
    source: &str,
    call: &oxc_ast::ast::CallExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    if let Some(result) = try_convert_direct_eval(source, call) {
        return result;
    }
    let mut args = Vec::with_capacity(call.arguments.len());
    for argument in &call.arguments {
        args.push(convert_argument(source, argument)?);
    }
    // Strip parentheses and `as` casts around the callee so
    // `(Math.sqrt as any)(x)` is the plain call `Math.sqrt(x)`.
    let mut callee_expression = &call.callee;
    loop {
        match callee_expression {
            Expression::ParenthesizedExpression(inner) => callee_expression = &inner.expression,
            Expression::TSAsExpression(inner) => callee_expression = &inner.expression,
            _ => break,
        }
    }
    match callee_expression {
        // `super(args)`: the callee name is the source text `super`. In a
        // run-time class the base is a value, so the call is lowered here.
        Expression::Super(super_expression) => {
            if super::super::runtime_class::in_runtime_class() {
                if let Some(lowered) =
                    super::super::runtime_class::runtime_super_call(args, span(call.span))
                {
                    return lowered;
                }
                return Err("`super(...)` could not be lowered in a run-time class".into());
            }
            Ok(Expr::Call {
                callee: Name {
                    span: span(super_expression.span),
                },
                args,
                span: span(call.span),
            })
        }
        // `super.name(args)` in a run-time class: the base's member, called
        // on the current `this`.
        Expression::StaticMemberExpression(member)
            if !member.optional
                && matches!(member.object, Expression::Super(_))
                && super::super::runtime_class::in_runtime_class() =>
        {
            super::super::runtime_class::runtime_super_member_call(
                Ok(Name {
                    span: span(member.property.span),
                }),
                args,
                span(call.span),
            )
            .ok_or_else(|| "`super` could not be lowered in a run-time class".to_string())
        }
        Expression::ComputedMemberExpression(member)
            if !member.optional
                && matches!(member.object, Expression::Super(_))
                && super::super::runtime_class::in_runtime_class() =>
        {
            let key = convert_expression(source, &member.expression)?;
            super::super::runtime_class::runtime_super_member_call(Err(key), args, span(call.span))
                .ok_or_else(|| "`super` could not be lowered in a run-time class".to_string())
        }
        Expression::Identifier(identifier) => Ok(Expr::Call {
            callee: Name {
                span: span(identifier.span),
            },
            args,
            span: span(call.span),
        }),
        // `Array.fromAsync(src, mapFn?)` calls the prelude coroutine
        // `__zunArrayFromAsync`.
        Expression::StaticMemberExpression(member)
            if !member.optional
                && member.property.name == "fromAsync"
                && matches!(&member.object, Expression::Identifier(id) if id.name == "Array")
                && !args.is_empty()
                && args.len() <= 2
                && !args.iter().any(|a| matches!(a, Expr::Spread { .. })) =>
        {
            Ok(Expr::Call {
                callee: crate::ast::synthetic_ident("__zunArrayFromAsync"),
                args,
                span: span(call.span),
            })
        }
        // `Intl.fn(args)`: a dotted callee name the checker maps onto a
        // prelude function (see `Checker::text`).
        Expression::StaticMemberExpression(member)
            if !member.optional
                && matches!(&member.object, Expression::Identifier(id) if id.name == "Intl") =>
        {
            Ok(Expr::Call {
                callee: Name {
                    span: span(member.span),
                },
                args,
                span: span(call.span),
            })
        }
        // `this.#m(args)`: a private method is an ordinary vtable
        // method whose name keeps the `#` (so it cannot collide with a
        // public member).
        // A private member of a run-time class is read from its store (with
        // the brand check) and called on the receiver.
        Expression::PrivateFieldExpression(member)
            if !member.optional
                && super::super::runtime_private::runtime_private(member.field.name.as_str())
                    .is_some() =>
        {
            let slot = super::super::runtime_private::runtime_private(member.field.name.as_str())
                .expect("checked by the guard");
            Ok(slot.call(
                convert_expression(source, &member.object)?,
                args,
                span(call.span),
            ))
        }
        Expression::PrivateFieldExpression(member) if !member.optional => Ok(Expr::MethodCall {
            receiver: Box::new(convert_expression(source, &member.object)?),
            method: Name {
                span: span(member.field.span),
            },
            args,
            span: span(call.span),
        }),
        Expression::StaticMemberExpression(member) => {
            convert_static_member_call(source, call, member, args)
        }
        // `super[key](args)` calls the base class's member with the current
        // `this`: `Object.getPrototypeOf(C.prototype)[key].call(this, args)`.
        Expression::ComputedMemberExpression(member)
            if !member.optional && matches!(member.object, Expression::Super(_)) =>
        {
            match super::super::class::super_home_object(span(member.span)) {
                Some(home) => {
                    let mut call_args = vec![Expr::Ident(crate::ast::synthetic_ident("this"))];
                    call_args.extend(args);
                    Ok(Expr::MethodCall {
                        receiver: Box::new(Expr::Index {
                            object: Box::new(home),
                            index: Box::new(convert_expression(source, &member.expression)?),
                            span: span(member.span),
                        }),
                        method: crate::ast::synthetic_ident("call"),
                        args: call_args,
                        span: span(call.span),
                    })
                }
                None => Err("`super[key]` is only valid inside a method of a named class".into()),
            }
        }
        // `obj['name'](args)` with a plain identifier-shaped literal key is
        // the method call `obj.name(args)` (the name span skips the quotes).
        Expression::ComputedMemberExpression(member)
            if !member.optional
                && matches!(&member.expression, Expression::StringLiteral(text)
                    if text.value.as_str().is_some_and(is_plain_identifier)
                        && (text.span.end - text.span.start) as usize == text.value.as_str().map_or(0, str::len) + 2) =>
        {
            let Expression::StringLiteral(text) = &member.expression else {
                unreachable!()
            };
            Ok(Expr::MethodCall {
                receiver: Box::new(convert_expression(source, &member.object)?),
                method: Name {
                    span: Span::new(text.span.start + 1, text.span.end - 1),
                },
                args,
                span: span(call.span),
            })
        }
        // Any other callee is an arbitrary function-valued expression
        // (`a[b]()`, `f()()`, `(cond ? f : g)(…)`): call it indirectly.
        other => Ok(Expr::ComputedCall {
            callee: Box::new(convert_expression(
                source,
                // `(1 as any)()`: keep the `any` cast so a literal callee is dynamic.
                if matches!(
                    other,
                    Expression::NumericLiteral(_)
                        | Expression::StringLiteral(_)
                        | Expression::BooleanLiteral(_)
                        | Expression::NullLiteral(_)
                ) {
                    &call.callee
                } else {
                    other
                },
            )?),
            args,
            span: span(call.span),
        }),
    }
}

fn convert_static_member_call(
    source: &str,
    call: &oxc_ast::ast::CallExpression<'_>,
    member: &oxc_ast::ast::StaticMemberExpression<'_>,
    args: Vec<Expr>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // `String.prototype.m.call(thisArg, ...rest)` becomes
    // `__thisString(thisArg).m(...rest)` (the checker maps the
    // `String.prototype` callee text onto the prelude helper).
    // `Promise.resolve.call(Promise, v)` is `Promise.resolve(v)`.
    if !member.optional && member.property.name == "call" && !args.is_empty() {
        if let (Expression::StaticMemberExpression(method_member), Expr::Ident(first)) =
            (&member.object, &args[0])
        {
            if !method_member.optional
                && matches!(&method_member.object, Expression::Identifier(id) if id.name == "Promise")
                && source.get(first.span.start as usize..first.span.end as usize) == Some("Promise")
            {
                let mut rest = args;
                rest.remove(0);
                return Ok(Expr::MethodCall {
                    receiver: Box::new(convert_expression(source, &method_member.object)?),
                    method: Name {
                        span: span(method_member.property.span),
                    },
                    args: rest,
                    span: span(call.span),
                });
            }
        }
    }
    if !member.optional && member.property.name == "call" {
        if let Expression::StaticMemberExpression(method_member) = &member.object {
            if let Expression::StaticMemberExpression(proto) = &method_member.object {
                if !method_member.optional
                    && !proto.optional
                    && proto.property.name == "prototype"
                    && matches!(&proto.object, Expression::Identifier(id) if id.name == "String")
                    && !args.is_empty()
                    && !matches!(args[0], Expr::Spread { .. })
                {
                    let mut rest = args;
                    let this_arg = rest.remove(0);
                    // `valueOf` / `toString` take thisStringValue, not ToString:
                    // a non-string receiver is a TypeError, and for a string
                    // both methods are the identity.
                    if rest.is_empty()
                        && matches!(method_member.property.name.as_str(), "valueOf" | "toString")
                    {
                        return Ok(Expr::Call {
                            callee: crate::ast::synthetic_ident("__thisStringValue"),
                            args: vec![this_arg],
                            span: span(call.span),
                        });
                    }
                    return Ok(Expr::MethodCall {
                        receiver: Box::new(Expr::Call {
                            callee: Name {
                                span: span(proto.span),
                            },
                            args: vec![this_arg],
                            span: span(proto.span),
                        }),
                        method: Name {
                            span: span(method_member.property.span),
                        },
                        args: rest,
                        span: span(call.span),
                    });
                }
            }
        }
    }
    // `(Map as any).groupBy(...)`: a cast or parentheses around a
    // capitalized global namespace identifier is that namespace.
    let mut receiver_expression = &member.object;
    loop {
        match receiver_expression {
            Expression::ParenthesizedExpression(inner)
                if peels_to_global_namespace(&inner.expression) =>
            {
                receiver_expression = &inner.expression
            }
            Expression::TSAsExpression(inner) if peels_to_global_namespace(&inner.expression) => {
                receiver_expression = &inner.expression
            }
            _ => break,
        }
    }
    let receiver = Box::new(convert_expression(source, receiver_expression)?);
    let method = Name {
        span: span(member.property.span),
    };
    if member.optional {
        Ok(Expr::OptionalMethodCall {
            object: receiver,
            method,
            args,
            span: span(call.span),
        })
    } else {
        Ok(Expr::MethodCall {
            receiver,
            method,
            args,
            span: span(call.span),
        })
    }
}

pub(super) fn convert_new_expression(
    source: &str,
    new_expression: &oxc_ast::ast::NewExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut args = Vec::with_capacity(new_expression.arguments.len());
    for argument in &new_expression.arguments {
        args.push(convert_argument(source, argument)?);
    }
    // A name bound to a lowered class constructs the value that binding
    // holds, never a static class that happens to share the name.
    {
        let mut peeled = &new_expression.callee;
        loop {
            match peeled {
                Expression::ParenthesizedExpression(inner) => peeled = &inner.expression,
                Expression::TSAsExpression(inner) => peeled = &inner.expression,
                _ => break,
            }
        }
        if let Expression::Identifier(identifier) = peeled {
            if super::super::runtime_class::names_runtime_class(identifier.name.as_str()) {
                return Ok(Expr::NewDynamic {
                    callee: Box::new(Expr::Ident(Name {
                        span: span(identifier.span),
                    })),
                    args,
                    span: span(new_expression.span),
                });
            }
        }
    }
    // A bare identifier constructor resolves a named class; any other
    // callee (`new ns.Ctor()`, `new (expr)()`) becomes a dynamic new on
    // the evaluated callee expression.
    match &new_expression.callee {
        Expression::Identifier(identifier) => Ok(Expr::New {
            class: Name {
                span: span(identifier.span),
            },
            args,
            span: span(new_expression.span),
        }),
        // `new Intl.X(...)`: a dotted class name the checker maps onto
        // the prelude `IntlX` classes (see `Checker::text`).
        Expression::StaticMemberExpression(member) if matches!(&member.object, Expression::Identifier(id) if id.name == "Intl") => {
            Ok(Expr::New {
                class: Name {
                    span: span(member.span),
                },
                args,
                span: span(new_expression.span),
            })
        }
        callee => {
            // `new (class X {})(...)`: the class expression is hoisted to a
            // named class, so construct it statically.
            let mut peeled = callee;
            loop {
                match peeled {
                    Expression::ParenthesizedExpression(inner) => peeled = &inner.expression,
                    Expression::TSAsExpression(inner) => peeled = &inner.expression,
                    _ => break,
                }
            }
            // `new (Name as any)(...)`: the cast only silences the type
            // checker, so this constructs the named class.
            if let Expression::Identifier(identifier) = peeled {
                return Ok(Expr::New {
                    class: Name {
                        span: span(identifier.span),
                    },
                    args,
                    span: span(new_expression.span),
                });
            }
            if matches!(peeled, Expression::ClassExpression(_)) {
                // Constructed where it is written, so it need not be a
                // run-time value (unless its base is one).
                super::super::runtime_class::set_new_callee_class(true);
                let converted = convert_expression(source, peeled);
                super::super::runtime_class::set_new_callee_class(false);
                let converted = converted?;
                let Expr::Ident(name) = converted else {
                    return Ok(Expr::NewDynamic {
                        callee: Box::new(converted),
                        args,
                        span: span(new_expression.span),
                    });
                };
                return Ok(Expr::New {
                    class: name,
                    args,
                    span: span(new_expression.span),
                });
            }
            Ok(Expr::NewDynamic {
                callee: Box::new(convert_expression(source, callee)?),
                args,
                span: span(new_expression.span),
            })
        }
    }
}

pub(super) fn convert_import_expression(
    source: &str,
    import: &oxc_ast::ast::ImportExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    if !import.options.is_none() {
        return Err("OXC adapter does not support import options yet".into());
    }
    Ok(Expr::ImportCall {
        specifier: Box::new(convert_expression(source, &import.source)?),
        span: span(import.span),
    })
}
