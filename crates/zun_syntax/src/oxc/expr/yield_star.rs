use super::*;

/// The synthetic locals and span shared by the `yield*` forwarding loop.
#[derive(Clone, Copy)]
struct YieldStar {
    sp: Span,
    it: Name,
    result: Name,
    ok: Name,
    threw: Name,
    sent: Name,
    caught: Name,
    ret_answer: Name,
}

impl YieldStar {
    fn string(&self, text: &str) -> Expr {
        Expr::String {
            value: text.to_owned(),
            span: self.sp,
        }
    }

    fn index(&self, name: Name, key: &str) -> Expr {
        Expr::Index {
            object: Box::new(Expr::Ident(name)),
            index: Box::new(self.string(key)),
            span: self.sp,
        }
    }

    fn step(&self, args: Vec<Expr>) -> Expr {
        Expr::ComputedCall {
            callee: Box::new(self.index(self.it, "next")),
            args,
            span: self.sp,
        }
    }

    /// `typeof it.key !== "undefined"`: a delegate method that is present.
    fn defined(&self, key: &str) -> Expr {
        Expr::Binary {
            op: BinaryOp::Ne,
            lhs: Box::new(Expr::Unary {
                op: crate::ast::UnaryOp::Typeof,
                operand: Box::new(self.index(self.it, key)),
                span: self.sp,
            }),
            rhs: Box::new(self.string("undefined")),
            span: self.sp,
        }
    }

    /// The statements that set up the delegate and run the forwarding loop.
    fn prelude(&self, operand: Expr) -> Vec<Stmt> {
        let YieldStar { sp, it, result, .. } = *self;
        let index = |name: Name, key: &str| self.index(name, key);
        let step = |args: Vec<Expr>| self.step(args);
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
        vec![
            Stmt::Let {
                name: it,
                ty: None,
                init: get_iterator,
                span: sp,
            },
            Stmt::Let {
                name: result,
                ty: None,
                init: step(Vec::new()),
                span: sp,
            },
            Stmt::While {
                cond: Expr::Unary {
                    op: crate::ast::UnaryOp::Not,
                    operand: Box::new(index(result, "done")),
                    span: sp,
                },
                body: Block {
                    statements: self.loop_statements(),
                    span: sp,
                },
                span: sp,
            },
        ]
    }

    /// One round of the loop: forward the sent value (or thrown exception, or
    /// return request) to the delegate and take its next result.
    fn loop_statements(&self) -> Vec<Stmt> {
        let YieldStar {
            sp,
            ok,
            threw,
            sent,
            ..
        } = *self;
        vec![
            // The sent value, a thrown exception, or a return request on the
            // outer generator is forwarded to the delegate.
            Stmt::Let {
                name: ok,
                ty: None,
                init: Expr::Bool {
                    value: false,
                    span: sp,
                },
                span: sp,
            },
            Stmt::Let {
                name: threw,
                ty: None,
                init: Expr::Bool {
                    value: false,
                    span: sp,
                },
                span: sp,
            },
            Stmt::Let {
                name: sent,
                ty: None,
                init: Expr::Bool {
                    value: false,
                    span: sp,
                },
                span: sp,
            },
            self.forward_try(),
            self.advance(),
        ]
    }

    fn forward_try(&self) -> Stmt {
        let YieldStar {
            sp,
            result,
            ok,
            threw,
            sent,
            caught,
            ..
        } = *self;
        let index = |name: Name, key: &str| self.index(name, key);
        let defined = |key: &str| self.defined(key);
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
                        value: Box::new(Expr::Bool {
                            value: true,
                            span: sp,
                        }),
                        span: sp,
                    }),
                ],
                span: sp,
            },
            catches: vec![crate::ast::CatchClause {
                binding: caught,
                ty: crate::ast::TypeAnnotation {
                    name: Name {
                        span: Span::new(0, 0),
                    },
                    is_array: false,
                },
                body: Block {
                    statements: vec![
                        Stmt::Expr(Expr::Assign {
                            target: threw,
                            value: Box::new(Expr::Bool {
                                value: true,
                                span: sp,
                            }),
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
                                        statements: self.return_forwarding(),
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
        }
    }

    /// Forward the return request, validate the answer, and finish when the
    /// delegate did.
    fn return_forwarding(&self) -> Vec<Stmt> {
        let YieldStar {
            sp, it, ret_answer, ..
        } = *self;
        let index = |name: Name, key: &str| self.index(name, key);
        let string = |text: &str| self.string(text);
        vec![
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
        ]
    }

    fn advance(&self) -> Stmt {
        let YieldStar {
            sp,
            it,
            result,
            ok,
            sent,
            ..
        } = *self;
        let index = |name: Name, key: &str| self.index(name, key);
        let defined = |key: &str| self.defined(key);
        let step = |args: Vec<Expr>| self.step(args);
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
                    }),
                ],
                span: sp,
            }),
            span: sp,
        }
    }
}

/// `yield* source`: drive the delegate by hand. The iteration setup and
/// the forwarding loop become statements placed before the statement
/// that contains the expression (see `PENDING_PRELUDE`); the expression
/// itself reads the delegate's return value.
pub(super) fn convert_yield_star(
    source: &str,
    yield_expression: &oxc_ast::ast::YieldExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
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
    let state = YieldStar {
        sp,
        it,
        result,
        ok,
        threw,
        sent,
        caught,
        ret_answer,
    };
    let prelude = state.prelude(operand);
    PENDING_PRELUDE.with(|pending| pending.borrow_mut().extend(prelude));
    Ok(state.index(result, "value"))
}

pub(super) fn convert_yield(
    source: &str,
    yield_expression: &oxc_ast::ast::YieldExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
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
