use super::*;

mod call;
mod chain;
mod member;
mod update;

use chain::*;
use member::*;
use update::*;
pub(super) use call::convert_argument;
pub(super) use update::convert_effect_expression;

pub(super) fn convert_expression(source: &str, expression: &Expression<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    match expression {
        Expression::NumericLiteral(number) => Ok(Expr::Number {
            value: number.value,
            span: span(number.span),
        }),
        Expression::BigIntLiteral(bigint) => Ok(Expr::BigInt {
            value: bigint.value.as_str().to_owned(),
            span: span(bigint.span),
        }),
        Expression::StringLiteral(string) => {
            let value = js_str_to_string(string.value);
            Ok(Expr::String {
                value: value.to_owned(),
                span: span(string.span),
            })
        }
        Expression::BooleanLiteral(boolean) => Ok(Expr::Bool {
            value: boolean.value,
            span: span(boolean.span),
        }),
        Expression::NullLiteral(null) => Ok(Expr::Null {
            span: span(null.span),
        }),
        Expression::RegExpLiteral(regex) => Ok(Expr::RegexLiteral {
            pattern: regex.regex.pattern.text.to_string(),
            flags: regex.regex.flags.to_string(),
            span: span(regex.span),
        }),
        Expression::TemplateLiteral(template) => {
            let mut parts = Vec::with_capacity(template.quasis.len() + template.expressions.len());
            for (index, quasi) in template.quasis.iter().enumerate() {
                let text = quasi
                    .value
                    .cooked
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .or_else(|| Some(quasi.value.raw.as_str()))
                    .ok_or_else(|| {
                        "OXC adapter cannot represent this template segment".to_owned()
                    })?;
                parts.push(crate::ast::TemplatePart::Text(text.to_owned()));
                if let Some(expression) = template.expressions.get(index) {
                    parts.push(crate::ast::TemplatePart::Expr(Box::new(
                        convert_expression(source, expression)?,
                    )));
                }
            }
            Ok(Expr::Template {
                parts,
                span: span(template.span),
            })
        }
        Expression::TaggedTemplateExpression(tagged) => {
            let tag = Box::new(convert_expression(source, &tagged.tag)?);
            let quasi = &tagged.quasi;
            let mut cooked = Vec::with_capacity(quasi.quasis.len());
            let mut raw = Vec::with_capacity(quasi.quasis.len());
            let mut cooked_undefined = Vec::with_capacity(quasi.quasis.len());
            for element in &quasi.quasis {
                cooked_undefined.push(element.value.cooked.is_none());
                let cooked_text = element
                    .value
                    .cooked
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .unwrap_or_else(|| element.value.raw.as_str());
                cooked.push(cooked_text.to_owned());
                raw.push(element.value.raw.as_str().to_owned());
            }
            let mut exprs = Vec::with_capacity(quasi.expressions.len());
            for expression in &quasi.expressions {
                exprs.push(Box::new(convert_expression(source, expression)?));
            }
            Ok(Expr::TaggedTemplate {
                tag,
                cooked,
                cooked_undefined,
                raw,
                exprs,
                span: span(tagged.span),
            })
        }
        Expression::Identifier(identifier) => {
            // The global literal identifiers have no variable binding: map them
            // to their values so they resolve like JS (`undefined` ≡ `void 0`,
            // `NaN`/`Infinity` are float literals).
            match identifier.name.as_str() {
                "undefined" => Ok(Expr::Unary {
                    op: crate::ast::UnaryOp::Void,
                    operand: Box::new(Expr::Number { value: 0.0, span: span(identifier.span) }),
                    span: span(identifier.span),
                }),
                "NaN" => Ok(Expr::Number { value: f64::NAN, span: span(identifier.span) }),
                "Infinity" => Ok(Expr::Number { value: f64::INFINITY, span: span(identifier.span) }),
                _ => Ok(Expr::Ident(Name { span: span(identifier.span) })),
            }
        }
        // `super.method(...)`: the receiver name is the source text `super`.
        Expression::Super(super_expression) => Ok(Expr::Ident(Name {
            span: span(super_expression.span),
        })),
        Expression::ThisExpression(this_expression) => Ok(Expr::Ident(Name {
            span: span(this_expression.span),
        })),
        // `super` is modelled as an identifier whose text is `super`; sema
        // recognises it in call/method-call receiver position and lowers the
        // base-class dispatch. The span covers the `super` keyword in source.
        Expression::Super(super_keyword) => Ok(Expr::Ident(Name {
            span: span(super_keyword.span),
        })),
        Expression::ParenthesizedExpression(parenthesized) => {
            convert_expression(source, &parenthesized.expression)
        }
        Expression::SequenceExpression(sequence) => {
            // Desugar `(a, b, c)` into an immediately-invoked arrow
            // `(() => { a; b; return c; })()` so evaluation order and side
            // effects of the earlier expressions are preserved while the whole
            // sequence still yields the last expression's value.
            let whole = span(sequence.span);
            let (last, leading) = sequence
                .expressions
                .split_last()
                .ok_or("OXC adapter: empty sequence expression")?;
            // A sequence containing `yield` cannot live in a closure: hoist the
            // leading expressions to statements before the containing statement.
            if source[sequence.span.start as usize..sequence.span.end as usize].contains("yield") {
                let mut prelude = Vec::with_capacity(leading.len());
                for expression in leading {
                    prelude.push(Stmt::Expr(convert_expression(source, expression)?));
                }
                PENDING_PRELUDE.with(|pending| pending.borrow_mut().extend(prelude));
                return convert_expression(source, last);
            }
            let mut statements = Vec::with_capacity(sequence.expressions.len());
            for expression in leading {
                statements.push(Stmt::Expr(convert_expression(source, expression)?));
            }
            statements.push(Stmt::Return {
                value: Some(convert_expression(source, last)?),
                span: span(last.span()),
            });
            let iife = Expr::Function(FunctionExpr {
                params: Vec::new(),
                return_type: convert_type_annotation(source, None)?,
                body: Block { statements, span: whole },
                is_arrow: true,
                is_async: false,
                is_generator: false,
                span: whole,
            });
            Ok(Expr::ComputedCall {
                callee: Box::new(iife),
                args: Vec::new(),
                span: whole,
            })
        }
        Expression::TSAsExpression(assertion) => Ok(Expr::Cast {
            expr: Box::new(convert_expression(source, &assertion.expression)?),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(
                        assertion.type_annotation.span().start,
                        assertion.type_annotation.span().end,
                    ),
                },
                is_array: false,
            },
            span: span(assertion.span),
        }),
        Expression::TSTypeAssertion(assertion) => Ok(Expr::Cast {
            expr: Box::new(convert_expression(source, &assertion.expression)?),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(
                        assertion.type_annotation.span().start,
                        assertion.type_annotation.span().end,
                    ),
                },
                is_array: false,
            },
            span: span(assertion.span),
        }),
        Expression::TSSatisfiesExpression(assertion) => Ok(Expr::Cast {
            expr: Box::new(convert_expression(source, &assertion.expression)?),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(
                        assertion.type_annotation.span().start,
                        assertion.type_annotation.span().end,
                    ),
                },
                is_array: false,
            },
            span: span(assertion.span),
        }),
        Expression::TSNonNullExpression(assertion) => convert_expression(source, &assertion.expression),
        Expression::StaticMemberExpression(member) => {
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
        Expression::ComputedMemberExpression(member) => {
            let object = Box::new(convert_expression(source, &member.object)?);
            // `obj['name']` with a plain identifier-shaped literal is `obj.name`.
            if let Expression::StringLiteral(text) = &member.expression {
                if text.value.as_str().is_some_and(is_plain_identifier)
                    && (text.span.end - text.span.start) as usize
                        == text.value.as_str().map_or(0, str::len) + 2
                {
                    let field = Name { span: Span::new(text.span.start + 1, text.span.end - 1) };
                    return Ok(if member.optional {
                        Expr::OptionalField { object, field, span: span(member.span) }
                    } else {
                        Expr::Field { object, field, span: span(member.span) }
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
        Expression::PrivateFieldExpression(member) => {
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
        Expression::ChainExpression(chain) => convert_chain(source, chain),
        Expression::PrivateInExpression(private_in) => {
            // `#x in obj` is a brand check: resolve `#x` to the innermost
            // lexical class declaring it, then test `obj instanceof Class`
            // (instance members) or `obj === Class` (static members).
            let name = private_in.left.name.to_string();
            let found = PRIVATE_SCOPES.with(|scopes| {
                scopes.borrow().iter().rev().find_map(|(class_span, names)| {
                    names.iter().find(|(n, _)| *n == name).map(|(_, is_static)| (*class_span, *is_static))
                })
            });
            let Some((class_span, is_static)) = found else {
                return Err(format!("OXC adapter: unknown private name `#{name}` in brand check"));
            };
            Ok(Expr::Binary {
                op: if is_static { BinaryOp::Eq } else { BinaryOp::Instanceof },
                lhs: Box::new(convert_expression(source, &private_in.right)?),
                rhs: Box::new(Expr::Ident(Name { span: class_span })),
                span: span(private_in.span),
            })
        }
        Expression::BinaryExpression(binary) => {
            let op = match binary.operator {
                BinaryOperator::Equality => BinaryOp::LooseEq,
                BinaryOperator::StrictEquality => BinaryOp::Eq,
                BinaryOperator::Inequality => BinaryOp::LooseNe,
                BinaryOperator::StrictInequality => BinaryOp::Ne,
                BinaryOperator::LessThan => BinaryOp::Lt,
                BinaryOperator::LessEqualThan => BinaryOp::Le,
                BinaryOperator::GreaterThan => BinaryOp::Gt,
                BinaryOperator::GreaterEqualThan => BinaryOp::Ge,
                BinaryOperator::Addition => BinaryOp::Add,
                BinaryOperator::Subtraction => BinaryOp::Sub,
                BinaryOperator::Multiplication => BinaryOp::Mul,
                BinaryOperator::Division => BinaryOp::Div,
                BinaryOperator::Remainder => BinaryOp::Rem,
                BinaryOperator::Exponential => BinaryOp::Pow,
                BinaryOperator::BitwiseAnd => BinaryOp::BitAnd,
                BinaryOperator::BitwiseOR => BinaryOp::BitOr,
                BinaryOperator::BitwiseXOR => BinaryOp::BitXor,
                BinaryOperator::ShiftLeft => BinaryOp::Shl,
                BinaryOperator::ShiftRight => BinaryOp::Shr,
                BinaryOperator::ShiftRightZeroFill => BinaryOp::UShr,
                BinaryOperator::In => BinaryOp::In,
                BinaryOperator::Instanceof => BinaryOp::Instanceof,
                _ => return Err("OXC adapter does not support this binary operator yet".into()),
            };
            Ok(Expr::Binary {
                op,
                lhs: Box::new(convert_expression(source, &binary.left)?),
                rhs: Box::new(convert_expression(source, &binary.right)?),
                span: span(binary.span),
            })
        }
        Expression::LogicalExpression(logical) => {
            let op = match logical.operator {
                LogicalOperator::And => BinaryOp::And,
                LogicalOperator::Or => BinaryOp::Or,
                LogicalOperator::Coalesce => BinaryOp::Nullish,
            };
            Ok(Expr::Binary {
                op,
                lhs: Box::new(convert_expression(source, &logical.left)?),
                rhs: Box::new(convert_expression(source, &logical.right)?),
                span: span(logical.span),
            })
        }
        Expression::ConditionalExpression(conditional) => Ok(Expr::Conditional {
            condition: Box::new(convert_expression(source, &conditional.test)?),
            when_true: Box::new(convert_expression(source, &conditional.consequent)?),
            when_false: Box::new(convert_expression(source, &conditional.alternate)?),
            span: span(conditional.span),
        }),
        Expression::UnaryExpression(unary) => {
            // Unary plus (`+x`) is numeric coercion; the language has no such
            // operator, so desugar to `x - 0`, which applies ToNumber to the
            // operand with the same result for every primitive.
            if matches!(unary.operator, UnaryOperator::UnaryPlus) {
                // `x as any - 0` boxes the operand so the subtraction takes the
                // dynamic arithmetic path, which applies ToNumber at run time.
                let operand = convert_expression(source, &unary.argument)?;
                let boxed = Expr::Cast {
                    expr: Box::new(operand),
                    ty: crate::ast::TypeAnnotation {
                        name: Name { span: Span::new(unary.span.start, unary.span.start) },
                        is_array: false,
                    },
                    span: span(unary.span),
                };
                return Ok(Expr::Binary {
                    op: BinaryOp::Sub,
                    lhs: Box::new(boxed),
                    rhs: Box::new(Expr::Number { value: 0.0, span: span(unary.span) }),
                    span: span(unary.span),
                });
            }
            let op = match unary.operator {
                UnaryOperator::UnaryNegation => crate::ast::UnaryOp::Neg,
                UnaryOperator::LogicalNot => crate::ast::UnaryOp::Not,
                UnaryOperator::BitwiseNot => crate::ast::UnaryOp::BitNot,
                UnaryOperator::Void => crate::ast::UnaryOp::Void,
                UnaryOperator::Typeof => crate::ast::UnaryOp::Typeof,
                UnaryOperator::Delete => crate::ast::UnaryOp::Delete,
                _ => return Err("OXC adapter does not support this unary operator yet".into()),
            };
            Ok(Expr::Unary {
                op,
                operand: Box::new(convert_expression(source, &unary.argument)?),
                span: span(unary.span),
            })
        }
        Expression::ImportExpression(import) => {
            if !import.options.is_none() {
                return Err("OXC adapter does not support import options yet".into());
            }
            Ok(Expr::ImportCall {
                specifier: Box::new(convert_expression(source, &import.source)?),
                span: span(import.span),
            })
        }
        Expression::AwaitExpression(await_expression) => Ok(Expr::Await {
            operand: Box::new(convert_expression(source, &await_expression.argument)?),
            span: span(await_expression.span),
        }),
        Expression::YieldExpression(yield_expression) if yield_expression.delegate => {
            // `yield* source`: drive the delegate by hand. The iteration setup and
            // the forwarding loop become statements placed before the statement
            // that contains the expression (see `PENDING_PRELUDE`); the expression
            // itself reads the delegate's return value.
            let Some(argument) = yield_expression.argument.as_ref() else {
                return Err("OXC adapter: `yield*` requires an operand".into());
            };
            let sp = span(yield_expression.span);
            let operand = convert_expression(source, argument)?;
            let n = YIELD_STAR_COUNTER.with(|c| {
                c.set(c.get() + 1);
                c.get()
            });
            let it = crate::ast::synthetic_ident(&format!("__ys_it{n}"));
            let result = crate::ast::synthetic_ident(&format!("__ys_r{n}"));
            let ok = crate::ast::synthetic_ident(&format!("__ys_ok{n}"));
            let threw = crate::ast::synthetic_ident(&format!("__ys_th{n}"));
            let sent = crate::ast::synthetic_ident(&format!("__ys_s{n}"));
            let caught = crate::ast::synthetic_ident(&format!("__ys_c{n}"));
            let ret_answer = crate::ast::synthetic_ident(&format!("__ys_rr{n}"));
            let string = |text: &str| Expr::String { value: text.to_owned(), span: sp };
            let index = |name: Name, key: &str| Expr::Index {
                object: Box::new(Expr::Ident(name)),
                index: Box::new(string(key)),
                span: sp,
            };
            let step = |args: Vec<Expr>| Expr::ComputedCall { callee: Box::new(index(it, "next")), args, span: sp };
            // `typeof it.key !== "undefined"`: a delegate method that is present.
            let defined = |key: &str| Expr::Binary {
                op: BinaryOp::Ne,
                lhs: Box::new(Expr::Unary {
                    op: crate::ast::UnaryOp::Typeof,
                    operand: Box::new(index(it, key)),
                    span: sp,
                }),
                rhs: Box::new(string("undefined")),
                span: sp,
            };
            let get_iterator = Expr::ComputedCall {
                callee: Box::new(Expr::Index {
                    object: Box::new(operand),
                    index: Box::new(Expr::Field {
                        object: Box::new(Expr::Ident(crate::ast::synthetic_ident("Symbol"))),
                        field: crate::ast::synthetic_ident("iterator"),
                        span: sp,
                    }),
                    span: sp,
                }),
                args: Vec::new(),
                span: sp,
            };
            let prelude = vec![
                Stmt::Let { name: it, ty: None, init: get_iterator, span: sp },
                Stmt::Let { name: result, ty: None, init: step(Vec::new()), span: sp },
                Stmt::While {
                    cond: Expr::Unary {
                        op: crate::ast::UnaryOp::Not,
                        operand: Box::new(index(result, "done")),
                        span: sp,
                    },
                    body: Block {
                        statements: vec![
                            // The sent value, a thrown exception, or a return request on the
                            // outer generator is forwarded to the delegate.
                            Stmt::Let { name: ok, ty: None, init: Expr::Bool { value: false, span: sp }, span: sp },
                            Stmt::Let { name: threw, ty: None, init: Expr::Bool { value: false, span: sp }, span: sp },
                            Stmt::Let { name: sent, ty: None, init: Expr::Bool { value: false, span: sp }, span: sp },
                            Stmt::Try {
                                body: Block {
                                    statements: vec![
                                        Stmt::Expr(Expr::Assign {
                                            target: sent,
                                            value: Box::new(Expr::Yield {
                                                value: Box::new(index(result, "value")),
                                                span: sp,
                                            }),
                                            span: sp,
                                        }),
                                        Stmt::Expr(Expr::Assign {
                                            target: ok,
                                            value: Box::new(Expr::Bool { value: true, span: sp }),
                                            span: sp,
                                        }),
                                    ],
                                    span: sp,
                                },
                                catches: vec![crate::ast::CatchClause {
                                    binding: caught,
                                    ty: crate::ast::TypeAnnotation {
                                        name: Name { span: Span::new(0, 0) },
                                        is_array: false,
                                    },
                                    body: Block {
                                        statements: vec![
                                            Stmt::Expr(Expr::Assign {
                                                target: threw,
                                                value: Box::new(Expr::Bool { value: true, span: sp }),
                                                span: sp,
                                            }),
                                            Stmt::Expr(Expr::Assign {
                                                target: sent,
                                                value: Box::new(Expr::Ident(caught)),
                                                span: sp,
                                            }),
                                        ],
                                        span: sp,
                                    },
                                }],
                                finally: Some(Block {
                                    statements: vec![Stmt::If {
                                        cond: Expr::Unary {
                                            op: crate::ast::UnaryOp::Not,
                                            operand: Box::new(Expr::Ident(ok)),
                                            span: sp,
                                        },
                                        then_block: Block {
                                            statements: vec![Stmt::If {
                                                cond: Expr::Unary {
                                                    op: crate::ast::UnaryOp::Not,
                                                    operand: Box::new(Expr::Ident(threw)),
                                                    span: sp,
                                                },
                                                then_block: Block {
                                                    statements: vec![Stmt::If {
                                                        cond: defined("return"),
                                                        then_block: Block {
                                                            statements: vec![
                                                                // Forward the return request, validate the
                                                                // answer, and finish when the delegate did.
                                                                Stmt::Let {
                                                                    name: ret_answer,
                                                                    ty: None,
                                                                    init: Expr::ComputedCall {
                                                                        callee: Box::new(index(it, "return")),
                                                                        args: vec![Expr::Call {
                                                                            callee: crate::ast::synthetic_ident("__generatorReturnValue"),
                                                                            args: Vec::new(),
                                                                            span: sp,
                                                                        }],
                                                                        span: sp,
                                                                    },
                                                                    span: sp,
                                                                },
                                                                Stmt::If {
                                                                    cond: Expr::Binary {
                                                                        op: BinaryOp::Ne,
                                                                        lhs: Box::new(Expr::Unary {
                                                                            op: crate::ast::UnaryOp::Typeof,
                                                                            operand: Box::new(Expr::Ident(ret_answer)),
                                                                            span: sp,
                                                                        }),
                                                                        rhs: Box::new(string("object")),
                                                                        span: sp,
                                                                    },
                                                                    then_block: Block {
                                                                        statements: vec![Stmt::Throw {
                                                                            value: Expr::New {
                                                                                class: crate::ast::synthetic_ident("TypeError"),
                                                                                args: vec![string("iterator.return() did not return an object")],
                                                                                span: sp,
                                                                            },
                                                                            span: sp,
                                                                        }],
                                                                        span: sp,
                                                                    },
                                                                    else_block: None,
                                                                    span: sp,
                                                                },
                                                                Stmt::If {
                                                                    cond: index(ret_answer, "done"),
                                                                    then_block: Block {
                                                                        statements: vec![Stmt::Return {
                                                                            value: Some(index(ret_answer, "value")),
                                                                            span: sp,
                                                                        }],
                                                                        span: sp,
                                                                    },
                                                                    else_block: None,
                                                                    span: sp,
                                                                },
                                                            ],
                                                            span: sp,
                                                        },
                                                        else_block: None,
                                                        span: sp,
                                                    }],
                                                    span: sp,
                                                },
                                                else_block: None,
                                                span: sp,
                                            }],
                                            span: sp,
                                        },
                                        else_block: None,
                                        span: sp,
                                    }],
                                    span: sp,
                                }),
                                span: sp,
                            },
                            Stmt::If {
                                cond: Expr::Ident(ok),
                                then_block: Block {
                                    statements: vec![Stmt::Expr(Expr::Assign {
                                        target: result,
                                        value: Box::new(step(vec![Expr::Ident(sent)])),
                                        span: sp,
                                    })],
                                    span: sp,
                                },
                                else_block: Some(Block {
                                    statements: vec![
                                    // A delegate without `throw` is closed via `return`
                                    // (when present) before the call below raises a TypeError.
                                    Stmt::If {
                                        cond: Expr::Unary {
                                            op: crate::ast::UnaryOp::Not,
                                            operand: Box::new(defined("throw")),
                                            span: sp,
                                        },
                                        then_block: Block {
                                            statements: vec![Stmt::If {
                                                cond: defined("return"),
                                                then_block: Block {
                                                    statements: vec![Stmt::Expr(Expr::ComputedCall {
                                                        callee: Box::new(index(it, "return")),
                                                        args: Vec::new(),
                                                        span: sp,
                                                    })],
                                                    span: sp,
                                                },
                                                else_block: None,
                                                span: sp,
                                            }],
                                            span: sp,
                                        },
                                        else_block: None,
                                        span: sp,
                                    },
                                    Stmt::Expr(Expr::Assign {
                                        target: result,
                                        value: Box::new(Expr::ComputedCall {
                                            callee: Box::new(index(it, "throw")),
                                            args: vec![Expr::Ident(sent)],
                                            span: sp,
                                        }),
                                        span: sp,
                                    })],
                                    span: sp,
                                }),
                                span: sp,
                            },
                        ],
                        span: sp,
                    },
                    span: sp,
                },
            ];
            PENDING_PRELUDE.with(|pending| pending.borrow_mut().extend(prelude));
            Ok(index(result, "value"))
        }
        Expression::YieldExpression(yield_expression) => {
            if yield_expression.delegate {
                return Err("OXC adapter does not support delegated yield yet".into());
            }
            Ok(Expr::Yield {
                value: Box::new(match yield_expression.argument.as_ref() {
                    Some(argument) => convert_expression(source, argument)?,
                    // A bare `yield` yields `undefined`.
                    None => Expr::Unary {
                        op: crate::ast::UnaryOp::Void,
                        operand: Box::new(Expr::Number {
                            value: 0.0,
                            span: span(yield_expression.span),
                        }),
                        span: span(yield_expression.span),
                    },
                }),
                span: span(yield_expression.span),
            })
        }
        Expression::AssignmentExpression(assignment)
            if assignment.operator == AssignmentOperator::Assign
                && matches!(
                    &assignment.left,
                    AssignmentTarget::ArrayAssignmentTarget(_) | AssignmentTarget::ObjectAssignmentTarget(_)
                ) =>
        {
            convert_destructuring_assignment(
                source,
                &assignment.left,
                convert_expression(source, &assignment.right)?,
                span(assignment.span),
            )
        }
        Expression::AssignmentExpression(assignment) => {
            let compound = match assignment.operator {
                AssignmentOperator::Assign => None,
                AssignmentOperator::Addition => Some(BinaryOp::Add),
                AssignmentOperator::Subtraction => Some(BinaryOp::Sub),
                AssignmentOperator::Multiplication => Some(BinaryOp::Mul),
                AssignmentOperator::Division => Some(BinaryOp::Div),
                AssignmentOperator::Remainder => Some(BinaryOp::Rem),
                AssignmentOperator::Exponential => Some(BinaryOp::Pow),
                AssignmentOperator::BitwiseAnd => Some(BinaryOp::BitAnd),
                AssignmentOperator::BitwiseOR => Some(BinaryOp::BitOr),
                AssignmentOperator::BitwiseXOR => Some(BinaryOp::BitXor),
                AssignmentOperator::ShiftLeft => Some(BinaryOp::Shl),
                AssignmentOperator::ShiftRight => Some(BinaryOp::Shr),
                AssignmentOperator::ShiftRightZeroFill => Some(BinaryOp::UShr),
                // Logical assignments desugar to `a = a OP b`; the resulting
                // value matches the short-circuit form for every operand.
                AssignmentOperator::LogicalAnd => Some(BinaryOp::And),
                AssignmentOperator::LogicalOr => Some(BinaryOp::Or),
                AssignmentOperator::LogicalNullish => Some(BinaryOp::Nullish),
                _ => {
                    return Err("OXC adapter does not support compound assignment yet".into());
                }
            };
            if let Some(op) = compound {
                let whole = span(assignment.span);
                let logical = matches!(
                    assignment.operator,
                    AssignmentOperator::LogicalAnd
                        | AssignmentOperator::LogicalOr
                        | AssignmentOperator::LogicalNullish
                );
                let member = match &assignment.left {
                    AssignmentTarget::StaticMemberExpression(m) => Some(MemberRef::Static(m)),
                    AssignmentTarget::ComputedMemberExpression(m) => Some(MemberRef::Computed(m)),
                    AssignmentTarget::PrivateFieldExpression(m) => Some(MemberRef::Private(m)),
                    _ => None,
                };
                if let Some(member) = member {
                    // The target's object and key are evaluated exactly once;
                    // a logical assignment only stores when it does not
                    // short-circuit.
                    let (binds, read, store) = decompose_member(source, member)?;
                    let rhs = convert_expression(source, &assignment.right)?;
                    let body = if logical {
                        Expr::Binary {
                            op,
                            lhs: Box::new(read),
                            rhs: Box::new(store.build(rhs, whole)),
                            span: whole,
                        }
                    } else {
                        store.build(
                            Expr::Binary { op, lhs: Box::new(read), rhs: Box::new(rhs), span: whole },
                            whole,
                        )
                    };
                    return Ok(wrap_binds(binds, body, whole));
                }
                if logical {
                    if let AssignmentTarget::AssignmentTargetIdentifier(identifier) = &assignment.left {
                        let name = Name { span: span(identifier.span) };
                        let rhs = convert_expression(source, &assignment.right)?;
                        return Ok(Expr::Binary {
                            op,
                            lhs: Box::new(Expr::Ident(name)),
                            rhs: Box::new(Expr::Assign { target: name, value: Box::new(rhs), span: whole }),
                            span: whole,
                        });
                    }
                }
            }
            let mut value = convert_expression(source, &assignment.right)?;
            if let Some(op) = compound {
                // `a OP= b` desugars to `a = (a OP b)`. Known v1 limitation: a
                // target with side-effecting sub-expressions is evaluated twice.
                let read = match &assignment.left {
                    AssignmentTarget::AssignmentTargetIdentifier(identifier) => {
                        Expr::Ident(Name { span: span(identifier.span) })
                    }
                    AssignmentTarget::StaticMemberExpression(member) => Expr::Field {
                        object: Box::new(convert_expression(source, &member.object)?),
                        field: Name { span: span(member.property.span) },
                        span: span(member.span),
                    },
                    AssignmentTarget::ComputedMemberExpression(member) => Expr::Index {
                        object: Box::new(convert_expression(source, &member.object)?),
                        index: Box::new(convert_expression(source, &member.expression)?),
                        span: span(member.span),
                    },
                    AssignmentTarget::PrivateFieldExpression(member) => Expr::PrivateField {
                        object: Box::new(convert_expression(source, &member.object)?),
                        field: Name { span: span(member.field.span) },
                        span: span(member.span),
                    },
                    _ => {
                        return Err(
                            "OXC adapter supports identifier, field, and index assignments only"
                                .into(),
                        );
                    }
                };
                value = Expr::Binary {
                    op,
                    lhs: Box::new(read),
                    rhs: Box::new(value),
                    span: span(assignment.span),
                };
            }
            let value = Box::new(value);
            match &assignment.left {
                AssignmentTarget::AssignmentTargetIdentifier(identifier)
                    if is_immutable_self_name(identifier.name.as_str()) =>
                {
                    Ok(throw_const_assignment(*value, span(assignment.span)))
                }
                AssignmentTarget::AssignmentTargetIdentifier(identifier) => Ok(Expr::Assign {
                    target: Name {
                        span: span(identifier.span),
                    },
                    value,
                    span: span(assignment.span),
                }),
                AssignmentTarget::StaticMemberExpression(member) => Ok(Expr::FieldAssign {
                    object: Box::new(convert_expression(source, &member.object)?),
                    field: Name {
                        span: span(member.property.span),
                    },
                    value,
                    span: span(assignment.span),
                }),
                // `obj['name'] = v` with a plain identifier-shaped literal is `obj.name = v`.
                AssignmentTarget::ComputedMemberExpression(member)
                    if matches!(&member.expression, Expression::StringLiteral(text)
                        if text.value.as_str().is_some_and(is_plain_identifier)
                            && (text.span.end - text.span.start) as usize
                                == text.value.as_str().map_or(0, str::len) + 2) =>
                {
                    let Expression::StringLiteral(text) = &member.expression else { unreachable!() };
                    Ok(Expr::FieldAssign {
                        object: Box::new(convert_expression(source, &member.object)?),
                        field: Name { span: Span::new(text.span.start + 1, text.span.end - 1) },
                        value,
                        span: span(assignment.span),
                    })
                }
                AssignmentTarget::ComputedMemberExpression(member) => Ok(Expr::IndexAssign {
                    object: Box::new(convert_expression(source, &member.object)?),
                    index: Box::new(convert_expression(source, &member.expression)?),
                    value,
                    span: span(assignment.span),
                }),
                // Sema lowers a private store from `FieldAssign` over a
                // `PrivateField` target (the shape the hand parser emits).
                AssignmentTarget::PrivateFieldExpression(member) => Ok(Expr::FieldAssign {
                    object: Box::new(Expr::PrivateField {
                        object: Box::new(convert_expression(source, &member.object)?),
                        field: Name {
                            span: span(member.field.span),
                        },
                        span: span(member.span),
                    }),
                    field: Name {
                        span: span(member.field.span),
                    },
                    value,
                    span: span(assignment.span),
                }),
                // `(x as any) = v` assigns the identifier; the cast only
                // silences the type checker.
                AssignmentTarget::TSAsExpression(cast)
                    if matches!(peel_parens(&cast.expression), Expression::Identifier(_)) =>
                {
                    let Expression::Identifier(identifier) = peel_parens(&cast.expression) else { unreachable!() };
                    if is_immutable_self_name(identifier.name.as_str()) {
                        return Ok(throw_const_assignment(*value, span(assignment.span)));
                    }
                    Ok(Expr::Assign {
                        target: Name { span: span(identifier.span) },
                        value,
                        span: span(assignment.span),
                    })
                }
                _ => {
                    Err("OXC adapter supports identifier, field, and index assignments only".into())
                }
            }
        }
        Expression::CallExpression(call) => {
            // Direct `eval("<single expression>")` of an escape-free string literal:
            // the literal's text is parsed in place (a padded copy keeps every span
            // aligned with the real source) and converted like surrounding code.
            if let (Expression::Identifier(callee), 1, Some(oxc_ast::ast::Argument::StringLiteral(literal))) =
                (&call.callee, call.arguments.len(), call.arguments.first())
            {
                if callee.name == "eval" {
                    let (start, end) = (literal.span.start as usize, literal.span.end as usize);
                    if let Some(content) = source.get(start + 1..end.saturating_sub(1)) {
                        if !content.contains('\\') {
                            let padded = format!("{}{}", " ".repeat(start + 1), content);
                            let allocator = Allocator::default();
                            let parsed = Parser::new(&allocator, &padded, SourceType::mjs().with_typescript(true)).parse();
                            if parsed.diagnostics.is_empty() && parsed.program.body.len() == 1 {
                                if let Statement::ExpressionStatement(statement) = &parsed.program.body[0] {
                                    return convert_expression(&padded, &statement.expression);
                                }
                            }
                        }
                    }
                }
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
                    Expression::ParenthesizedExpression(inner) => {
                        callee_expression = &inner.expression
                    }
                    Expression::TSAsExpression(inner) => callee_expression = &inner.expression,
                    _ => break,
                }
            }
            match callee_expression {
                // `super(args)`: the callee name is the source text `super`.
                Expression::Super(super_expression) => Ok(Expr::Call {
                    callee: Name {
                        span: span(super_expression.span),
                    },
                    args,
                    span: span(call.span),
                }),
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
                Expression::PrivateFieldExpression(member) if !member.optional => {
                    Ok(Expr::MethodCall {
                        receiver: Box::new(convert_expression(source, &member.object)?),
                        method: Name {
                            span: span(member.field.span),
                        },
                        args,
                        span: span(call.span),
                    })
                }
                Expression::StaticMemberExpression(member) => {
                    // `String.prototype.m.call(thisArg, ...rest)` becomes
                    // `__thisString(thisArg).m(...rest)` (the checker maps the
                    // `String.prototype` callee text onto the prelude helper).
                    // `Promise.resolve.call(Promise, v)` is `Promise.resolve(v)`.
                    if !member.optional && member.property.name == "call" && !args.is_empty() {
                        if let (Expression::StaticMemberExpression(method_member), Expr::Ident(first)) = (&member.object, &args[0]) {
                            if !method_member.optional
                                && matches!(&method_member.object, Expression::Identifier(id) if id.name == "Promise")
                                && source.get(first.span.start as usize..first.span.end as usize) == Some("Promise")
                            {
                                let mut rest = args;
                                rest.remove(0);
                                return Ok(Expr::MethodCall {
                                    receiver: Box::new(convert_expression(source, &method_member.object)?),
                                    method: Name { span: span(method_member.property.span) },
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
                                    return Ok(Expr::MethodCall {
                                        receiver: Box::new(Expr::Call {
                                            callee: Name { span: span(proto.span) },
                                            args: vec![this_arg],
                                            span: span(proto.span),
                                        }),
                                        method: Name { span: span(method_member.property.span) },
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
                            Expression::TSAsExpression(inner)
                                if peels_to_global_namespace(&inner.expression) =>
                            {
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
                // `obj['name'](args)` with a plain identifier-shaped literal key is
                // the method call `obj.name(args)` (the name span skips the quotes).
                Expression::ComputedMemberExpression(member)
                    if !member.optional
                        && matches!(&member.expression, Expression::StringLiteral(text)
                            if text.value.as_str().is_some_and(is_plain_identifier)
                                && (text.span.end - text.span.start) as usize == text.value.as_str().map_or(0, str::len) + 2) =>
                {
                    let Expression::StringLiteral(text) = &member.expression else { unreachable!() };
                    Ok(Expr::MethodCall {
                        receiver: Box::new(convert_expression(source, &member.object)?),
                        method: Name { span: Span::new(text.span.start + 1, text.span.end - 1) },
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
                        if matches!(other, Expression::NumericLiteral(_) | Expression::StringLiteral(_) | Expression::BooleanLiteral(_) | Expression::NullLiteral(_)) { &call.callee } else { other },
                    )?),
                    args,
                    span: span(call.span),
                }),
            }
        }
        Expression::ObjectExpression(object) => {
            let mut properties = Vec::with_capacity(object.properties.len());
            for property in &object.properties {
                let property = match property {
                    ObjectPropertyKind::ObjectProperty(property) => property,
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        properties.push(crate::ast::ObjectMember::Spread {
                            expr: convert_expression(source, &spread.argument)?,
                            span: span(spread.span),
                        });
                        continue;
                    }
                };
                // Method shorthand and accessors carry a function value; a plain
                // data property is `PropertyKind::Init` and not a method.
                let member_kind = if property.method {
                    crate::ast::PropertyKind::Method
                } else {
                    match property.kind {
                        PropertyKind::Init => crate::ast::PropertyKind::Init,
                        PropertyKind::Get => crate::ast::PropertyKind::Get,
                        PropertyKind::Set => crate::ast::PropertyKind::Set,
                    }
                };
                let key = if property.computed {
                    let Some(key_expression) = property.key.as_expression() else {
                        return Err("OXC adapter requires an expression computed key".into());
                    };
                    crate::ast::PropertyKey::Computed(Box::new(convert_expression(source, key_expression)?))
                } else {
                    match &property.key {
                        PropertyKey::StaticIdentifier(identifier) => {
                            crate::ast::PropertyKey::Ident(Name {
                                span: span(identifier.span),
                            })
                        }
                        PropertyKey::StringLiteral(string) => {
                            let value = js_str_to_string(string.value);
                            crate::ast::PropertyKey::String {
                                value: value.to_owned(),
                                span: span(string.span),
                            }
                        }
                        PropertyKey::NumericLiteral(number) => crate::ast::PropertyKey::Number {
                            value: number.value,
                            span: span(number.span),
                        },
                        _ => {
                            return Err(
                                "OXC adapter supports identifier, string, and numeric property keys only"
                                    .into(),
                            )
                        }
                    }
                };
                let value = match member_kind {
                    crate::ast::PropertyKind::Init => convert_expression(source, &property.value)?,
                    // Method / getter / setter: the value is a function.
                    _ => {
                        let Expression::FunctionExpression(function) = &property.value else {
                            return Err(
                                "OXC adapter expected a function value for an object method or accessor"
                                    .into(),
                            );
                        };
                        convert_oxc_function(source, function)?
                    }
                };
                properties.push(crate::ast::ObjectMember::Property(crate::ast::ObjectProperty {
                    key,
                    value,
                    kind: member_kind,
                }));
            }
            Ok(Expr::ObjectLiteral {
                properties,
                span: span(object.span),
            })
        }
        Expression::ArrayExpression(array) => {
            let mut elements = Vec::with_capacity(array.elements.len());
            for element in &array.elements {
                match element {
                    ArrayExpressionElement::Elision(_) => {
                        elements.push(crate::ast::ArrayElement::Hole);
                    }
                    ArrayExpressionElement::SpreadElement(spread) => {
                        elements.push(crate::ast::ArrayElement::Spread(
                            convert_expression(source, &spread.argument)?,
                        ));
                    }
                    other => {
                        let Some(expression) = other.as_expression() else {
                            return Err(
                                "OXC adapter expected an array element expression".into()
                            );
                        };
                        elements.push(crate::ast::ArrayElement::Expr(
                            convert_expression(source, expression)?,
                        ));
                    }
                }
            }
            Ok(Expr::ArrayLiteral {
                elements,
                span: span(array.span),
            })
        }
        Expression::NewExpression(new_expression) => {
            let mut args = Vec::with_capacity(new_expression.arguments.len());
            for argument in &new_expression.arguments {
                args.push(convert_argument(source, argument)?);
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
                Expression::StaticMemberExpression(member)
                    if matches!(&member.object, Expression::Identifier(id) if id.name == "Intl") =>
                {
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
                            class: Name { span: span(identifier.span) },
                            args,
                            span: span(new_expression.span),
                        });
                    }
                    if matches!(peeled, Expression::ClassExpression(_)) {
                        if let Expr::Ident(name) = convert_expression(source, peeled)? {
                            return Ok(Expr::New {
                                class: name,
                                args,
                                span: span(new_expression.span),
                            });
                        }
                    }
                    Ok(Expr::NewDynamic {
                    callee: Box::new(convert_expression(source, callee)?),
                    args,
                    span: span(new_expression.span),
                    })
                }
            }
        }
        Expression::ArrowFunctionExpression(arrow) => convert_arrow(source, arrow),
        // A named function expression binds its own name inside its body:
        // `(() => { let name = function name() {...}; return name; })()` gives the
        // closure its self-reference (the `let` initializer already supports it).
        Expression::FunctionExpression(function)
            if function.id.as_ref().is_some_and(|id| {
                function.body.as_ref().is_some_and(|body| {
                    contains_word(&source[body.span.start as usize..body.span.end as usize], id.name.as_str())
                })
            }) =>
        {
            let id = function.id.as_ref().expect("checked above");
            let name = Name { span: Span::new(id.span.start, id.span.end) };
            let whole = Span::new(function.span.start, function.span.end);
            let (body_start, body_end) = function
                .body
                .as_ref()
                .map_or((0, 0), |body| (body.span.start as usize, body.span.end as usize));
            let body_text = &source[body_start..body_end];
            let shadowed = ["var", "let", "const", "function", "class"]
                .iter()
                .any(|keyword| body_text.contains(&format!("{keyword} {}", id.name)))
                || contains_word(&source[function.params.span.start as usize..function.params.span.end as usize], id.name.as_str());
            if !shadowed {
                IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().push(id.name.to_string()));
            }
            let inner = convert_oxc_function(source, function);
            if !shadowed {
                IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().pop());
            }
            let inner = inner?;
            let statements = vec![
                Stmt::Let { name, ty: None, init: inner, span: whole },
                Stmt::Return { value: Some(Expr::Ident(name)), span: whole },
            ];
            let iife = Expr::Function(FunctionExpr {
                params: Vec::new(),
                return_type: convert_type_annotation(source, None)?,
                body: Block { statements, span: whole },
                is_arrow: true,
                is_async: false,
                is_generator: false,
                span: whole,
            });
            Ok(Expr::ComputedCall { callee: Box::new(iife), args: Vec::new(), span: whole })
        }
        Expression::FunctionExpression(function) => convert_oxc_function(source, function),
        // A class expression is lifted to a hoisted class declaration (Zun
        // classes are program-global) and the expression becomes a reference
        // to it, which sema reads as the class constructor descriptor. An
        // anonymous class takes the name of the binding it initializes (set by
        // the declarator converter), else the `class` keyword span.
        Expression::ClassExpression(class_expression) => {
            let hint = CLASS_NAME_HINT.with(|cell| cell.borrow_mut().take());
            // A class bound by a declarator takes the binding's name so that
            // `new Binding()` resolves statically (limitation: an inner
            // reference to a distinct class-expression name is not bound).
            let name_span = match (hint, class_expression.id.as_ref()) {
                (Some(binding), _) => binding,
                (None, Some(id)) => Span::new(id.span.start, id.span.end),
                (None, None) => Span::new(
                    class_expression.span.start,
                    class_expression.span.start + 5,
                ),
            };
            if let Some(id) = class_expression.id.as_ref() {
                IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().push(id.name.to_string()));
            }
            let class = convert_class_declaration(source, class_expression);
            if class_expression.id.is_some() {
                IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().pop());
            }
            let mut class = class.map_err(|errors| errors.join("; "))?;
            let trailing = CLASS_TRAILING.with(|cells| std::mem::take(&mut *cells.borrow_mut()));
            if !trailing.is_empty() {
                PENDING_PRELUDE.with(|pending| pending.borrow_mut().extend(trailing));
            }
            class.name = Name { span: name_span };
            HOISTED_CLASSES.with(|cells| cells.borrow_mut().push(class));
            Ok(Expr::Ident(Name { span: name_span }))
        }
        Expression::UpdateExpression(update) => convert_update(source, update, true),
        // `new.target`: first-class class values are not modelled yet, so it
        // evaluates to `undefined` (the value it has in every plain call).
        Expression::NewTarget(meta) => {
            // The identifier text is `new.target`; the checker resolves it.
            Ok(Expr::Ident(Name { span: span(meta.span) }))
        }
        other => {
            let dbg = format!("{other:?}");
            let kind = dbg.split(|c: char| !c.is_alphanumeric()).find(|s| !s.is_empty()).unwrap_or("?");
            Err(format!("OXC adapter: unsupported expression `{kind}`"))
        }
    }
}
