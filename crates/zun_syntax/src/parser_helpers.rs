//! Small parser lookahead helpers extracted from the parser core.

use super::*;

fn binary_op(kind: TokenKind) -> Option<(BinaryOp, u8)> {
    Some(match kind {
        TokenKind::EqEq => (BinaryOp::Eq, 1),
        TokenKind::BangEq => (BinaryOp::Ne, 1),
        TokenKind::Lt => (BinaryOp::Lt, 2),
        TokenKind::LtEq => (BinaryOp::Le, 2),
        TokenKind::Gt => (BinaryOp::Gt, 2),
        TokenKind::GtEq => (BinaryOp::Ge, 2),
        TokenKind::OrOr => (BinaryOp::Or, 0),
        TokenKind::Nullish => (BinaryOp::Nullish, 0),
        TokenKind::AndAnd => (BinaryOp::And, 1),
        TokenKind::Plus => (BinaryOp::Add, 3),
        TokenKind::Minus => (BinaryOp::Sub, 3),
        TokenKind::Star => (BinaryOp::Mul, 4),
        TokenKind::Slash => (BinaryOp::Div, 4),
        TokenKind::Percent => (BinaryOp::Rem, 4),
        _ => return None,
    })
}

impl Parser<'_> {
    pub(crate) fn primary(&mut self) -> Expr {
        let t = self.peek();
        match t.kind {
            TokenKind::Number => {
                self.bump();
                let value = self.text(t).parse::<f64>().unwrap_or_else(|_| f64::NAN);
                Expr::Number { value, span: t.span }
            }
            TokenKind::String => {
                self.bump();
                Expr::String { value: decode_string(self.text(t)), span: t.span }
            }
            TokenKind::Template => {
                self.bump();
                let raw = self.text(t).to_string();
                Expr::Template { parts: parse_template_parts(&raw, t.span, &mut self.diagnostics), span: t.span }
            }
            TokenKind::KwTrue => { self.bump(); Expr::Bool { value: true, span: t.span } }
            TokenKind::KwFalse => { self.bump(); Expr::Bool { value: false, span: t.span } }
            TokenKind::KwNull => { self.bump(); Expr::Null { span: t.span } }
            TokenKind::KwNew => {
                let start = self.bump().span;
                let class = self.name("a class name");
                self.expect(TokenKind::LParen, "`(`");
                let args = self.arguments();
                let close = self.tokens[self.pos - 1].span;
                Expr::New { class, args, span: start.to(close) }
            }
            TokenKind::KwFunction => self.function_expression(),
            TokenKind::Ident => {
                self.bump();
                let name = Name { span: t.span };
                if self.eat(TokenKind::Arrow) {
                    let body = self.arrow_body();
                    return Expr::Function(FunctionExpr {
                        params: vec![Param { name, ty: None, default_value: None, is_rest: false }],
                        return_type: None,
                        span: t.span.to(body.span),
                        body,
                        is_arrow: true,
                    });
                }
                Expr::Ident(name)
            }
            TokenKind::LParen => {
                if self.parenthesized_arrow_start() {
                    let start = self.bump().span;
                    let params = self.parameters();
                    self.expect(TokenKind::RParen, "`)` after arrow parameters");
                    self.expect(TokenKind::Arrow, "`=>` after arrow parameters");
                    let body = self.arrow_body();
                    return Expr::Function(FunctionExpr { params, return_type: None, span: start.to(body.span), body, is_arrow: true });
                }
                self.bump();
                let inner = self.expression();
                self.expect(TokenKind::RParen, "`)`");
                inner
            }
            _ => {
                self.error_at(t.span, "expected an expression");
                self.bump();
                Expr::Null { span: t.span }
            }
        }
    }

    pub(crate) fn function_expression(&mut self) -> Expr {
        let start = self.bump().span;
        self.expect(TokenKind::LParen, "`(` after anonymous function");
        let params = self.parameters();
        self.expect(TokenKind::RParen, "`)`");
        let return_type = self.optional_type_annotation();
        let body = self.block();
        Expr::Function(FunctionExpr {
            params,
            return_type,
            span: start.to(body.span),
            body,
            is_arrow: false,
        })
    }

    pub(crate) fn call(&mut self) -> Expr {
        let mut expr = self.primary();
        loop {
            match self.peek().kind {
                TokenKind::LParen => {
                    self.bump();
                    let args = self.arguments();
                    let close = self.tokens[self.pos - 1].span;
                    expr = match expr {
                        Expr::Ident(callee) => Expr::Call {
                            callee,
                            args,
                            span: callee.span.to(close),
                        },
                        callee => Expr::ComputedCall {
                            span: callee.span().to(close),
                            callee: Box::new(callee),
                            args,
                        },
                    };
                }
                TokenKind::Dot => {
                    self.bump();
                    let member = self.name("a field or method name");
                    if self.at(TokenKind::LParen) {
                        self.bump();
                        let args = self.arguments();
                        let close = self.tokens[self.pos - 1].span;
                        let span = expr.span().to(close);
                        expr = Expr::MethodCall {
                            receiver: Box::new(expr),
                            method: member,
                            args,
                            span,
                        };
                    } else {
                        let span = expr.span().to(member.span);
                        expr = Expr::Field {
                            object: Box::new(expr),
                            field: member,
                            span,
                        };
                    }
                }
                TokenKind::QuestionDot => {
                    self.bump();
                    let field = self.name("an optional field or method name");
                    if self.at(TokenKind::LParen) {
                        self.bump();
                        let args = self.arguments();
                        let close = self.tokens[self.pos - 1].span;
                        let span = expr.span().to(close);
                        expr = Expr::OptionalMethodCall {
                            object: Box::new(expr),
                            method: field,
                            args,
                            span,
                        };
                    } else {
                        let span = expr.span().to(field.span);
                        expr = Expr::OptionalField {
                            object: Box::new(expr),
                            field,
                            span,
                        };
                    }
                }
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.expression();
                    let close = self
                        .expect(TokenKind::RBracket, "`]`")
                        .map(|t| t.span)
                        .unwrap_or(index.span());
                    expr = Expr::Index {
                        span: expr.span().to(close),
                        object: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                _ => break,
            }
        }
        expr
    }

    pub(crate) fn arguments(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            if self.eat(TokenKind::Ellipsis) {
                let expr = self.expression();
                let span = expr.span();
                args.push(Expr::Spread {
                    expr: Box::new(expr),
                    span,
                });
            } else {
                args.push(self.expression());
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RParen, "`)`");
        args
    }

    pub(crate) fn cast(&mut self) -> Expr {
        let mut expr = self.call();
        while self.eat(TokenKind::KwAs) {
            let ty = TypeAnnotation {
                name: self.name("a type name"),
            };
            let span = expr.span().to(ty.name.span);
            expr = Expr::Cast {
                expr: Box::new(expr),
                ty,
                span,
            };
        }
        expr
    }

    pub(crate) fn unary(&mut self) -> Expr {
        let op = match self.peek().kind {
            TokenKind::Minus => Some(UnaryOp::Neg),
            TokenKind::Bang => Some(UnaryOp::Not),
            _ => None,
        };
        if let Some(op) = op {
            let start = self.bump().span;
            let operand = self.unary();
            let span = start.to(operand.span());
            return Expr::Unary {
                op,
                operand: Box::new(operand),
                span,
            };
        }
        self.cast()
    }

    pub(crate) fn binary(&mut self, min_level: u8) -> Expr {
        let mut lhs = self.unary();
        loop {
            let Some((op, level)) = binary_op(self.peek().kind) else {
                break;
            };
            if level < min_level {
                break;
            }
            self.bump();
            let rhs = self.binary(level + 1);
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        lhs
    }

    pub(crate) fn conditional(&mut self) -> Expr {
        let condition = self.binary(0);
        if !self.eat(TokenKind::Question) {
            return condition;
        }
        let when_true = self.expression();
        self.expect(TokenKind::Colon, "`:` in conditional expression");
        let when_false = self.expression();
        let span = condition.span().to(when_false.span());
        Expr::Conditional {
            condition: Box::new(condition),
            when_true: Box::new(when_true),
            when_false: Box::new(when_false),
            span,
        }
    }

    pub(crate) fn assignment(&mut self) -> Expr {
        let lhs = self.conditional();
        if self.at(TokenKind::Eq) {
            let eq_span = self.bump().span;
            let value = self.assignment();
            match lhs {
                Expr::Ident(target) => {
                    let span = target.span.to(value.span());
                    return Expr::Assign {
                        target,
                        value: Box::new(value),
                        span,
                    };
                }
                Expr::Field { object, field, .. } => {
                    let span = object.span().to(value.span());
                    return Expr::FieldAssign {
                        object,
                        field,
                        value: Box::new(value),
                        span,
                    };
                }
                Expr::Index { object, index, .. } => {
                    let span = object.span().to(value.span());
                    return Expr::IndexAssign {
                        object,
                        index,
                        value: Box::new(value),
                        span,
                    };
                }
                _ => self.error_at(eq_span, "invalid assignment target"),
            }
        }
        lhs
    }

    pub(crate) fn expression(&mut self) -> Expr {
        self.assignment()
    }

    pub(crate) fn try_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        let body = self.block();
        let mut catches = Vec::new();
        while self.eat(TokenKind::KwCatch) {
            self.expect(TokenKind::LParen, "`(` after catch");
            let binding = self.name("a catch binding");
            self.expect(TokenKind::Colon, "`:` after catch binding");
            let ty = TypeAnnotation {
                name: self.name("a catch type"),
            };
            self.expect(TokenKind::RParen, "`)` after catch binding");
            let catch_body = self.block();
            catches.push(CatchClause {
                binding,
                ty,
                body: catch_body,
            });
        }
        let finally = if self.eat(TokenKind::KwFinally) {
            Some(self.block())
        } else {
            None
        };
        if catches.is_empty() && finally.is_none() {
            self.error_at(self.peek().span, "try requires catch or finally");
        }
        let end = finally.as_ref().map_or_else(
            || catches.last().map_or(body.span, |c| c.body.span),
            |b| b.span,
        );
        Stmt::Try {
            body,
            catches,
            finally,
            span: start.to(end),
        }
    }

    pub(crate) fn return_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        let value = if self.at(TokenKind::Semi) {
            None
        } else {
            Some(self.expression())
        };
        let end = self.peek().span;
        self.expect(TokenKind::Semi, "`;`");
        Stmt::Return {
            value,
            span: start.to(end),
        }
    }

    pub(crate) fn throw_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        let value = self.expression();
        let end = self.peek().span;
        self.expect(TokenKind::Semi, "`;` after throw");
        Stmt::Throw {
            value,
            span: start.to(end),
        }
    }

    pub(crate) fn break_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        let label = if self.at(TokenKind::Ident) {
            Some(self.name("a break label"))
        } else {
            None
        };
        let end = self.peek().span;
        self.expect(TokenKind::Semi, "`;` after break");
        Stmt::Break {
            label,
            span: start.to(end),
        }
    }

    pub(crate) fn continue_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        let label = if self.at(TokenKind::Ident) {
            Some(self.name("a continue label"))
        } else {
            None
        };
        let end = self.peek().span;
        self.expect(TokenKind::Semi, "`;` after continue");
        Stmt::Continue {
            label,
            span: start.to(end),
        }
    }

    pub(crate) fn if_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        self.expect(TokenKind::LParen, "`(`");
        let cond = self.expression();
        self.expect(TokenKind::RParen, "`)`");
        let then_block = self.block();
        let mut span = start.to(then_block.span);
        let else_block = if self.eat(TokenKind::KwElse) {
            let b = self.block();
            span = span.to(b.span);
            Some(b)
        } else {
            None
        };
        Stmt::If {
            cond,
            then_block,
            else_block,
            span,
        }
    }

    pub(crate) fn statement(&mut self) -> Stmt {
        if self.at(TokenKind::Ident) && self.peek_n(1).kind == TokenKind::Colon {
            let label = self.name("a statement label");
            self.bump();
            let statement = self.statement();
            return Stmt::Labeled {
                label,
                statement: Box::new(statement),
                span: label.span,
            };
        }
        match self.peek().kind {
            TokenKind::KwLet => self.let_statement(),
            TokenKind::KwIf => self.if_statement(),
            TokenKind::KwSwitch => self.switch_statement(),
            TokenKind::KwWhile => self.while_statement(),
            TokenKind::KwDo => self.do_while_statement(),
            TokenKind::KwFor => self.for_statement(),
            TokenKind::KwBreak => self.break_statement(),
            TokenKind::KwContinue => self.continue_statement(),
            TokenKind::KwReturn => self.return_statement(),
            TokenKind::KwThrow => self.throw_statement(),
            TokenKind::KwTry => self.try_statement(),
            _ => {
                let expr = self.expression();
                self.expect(TokenKind::Semi, "`;`");
                Stmt::Expr(expr)
            }
        }
    }

    pub(crate) fn block(&mut self) -> Block {
        let open = self.peek().span;
        self.expect(TokenKind::LBrace, "`{`");
        let mut statements = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            statements.push(self.statement());
        }
        let close = self.peek().span;
        self.expect(TokenKind::RBrace, "`}`");
        Block {
            statements,
            span: open.to(close),
        }
    }

    pub(crate) fn parameters(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            let is_rest = self.eat(TokenKind::Ellipsis);
            let name = self.name("a parameter name");
            let ty = self.optional_type_annotation();
            let default_value = if self.eat(TokenKind::Eq) {
                Some(Box::new(self.expression()))
            } else {
                None
            };
            params.push(Param {
                name,
                ty,
                default_value,
                is_rest,
            });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        params
    }

    pub(crate) fn name(&mut self, what: &str) -> Name {
        match self.expect(TokenKind::Ident, what) {
            Some(t) => Name { span: t.span },
            None => Name {
                span: Span::empty_at(self.peek().span.start),
            },
        }
    }

    pub(crate) fn optional_type_annotation(&mut self) -> Option<TypeAnnotation> {
        if self.eat(TokenKind::Colon) {
            Some(TypeAnnotation {
                name: self.name("a type name"),
            })
        } else {
            None
        }
    }

    pub(crate) fn arrow_body(&mut self) -> Block {
        if self.at(TokenKind::LBrace) {
            return self.block();
        }
        let value = self.expression();
        let span = value.span();
        Block {
            statements: vec![Stmt::Return {
                value: Some(value),
                span,
            }],
            span,
        }
    }

    pub(crate) fn parenthesized_arrow_start(&self) -> bool {
        let mut index = self.pos + 1;
        let mut expect_name = true;
        while index < self.tokens.len() {
            match self.tokens[index].kind {
                TokenKind::RParen => {
                    return self.tokens.get(index + 1).map(|t| t.kind) == Some(TokenKind::Arrow);
                }
                TokenKind::Ident if expect_name => expect_name = false,
                TokenKind::Colon if !expect_name => expect_name = true,
                TokenKind::Comma if !expect_name => expect_name = true,
                _ => return false,
            }
            index += 1;
        }
        false
    }
}
