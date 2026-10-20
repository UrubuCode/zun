//! Recursive-descent parser.
//!
//! Consumes the token stream from [`lexer`](crate::lexer) and produces a
//! [`Program`]. Expressions use precedence climbing. On error the parser
//! diagnoses and synchronizes to the next statement boundary rather than
//! aborting, so one mistake yields one diagnostic, not a cascade.

use crate::ast::*;
use crate::lexer::{lex, Token, TokenKind};
use zun_common::{Diagnostic, Span};

/// Parse `source` into a [`Program`], appending any diagnostics.
pub fn parse(source: &str, diagnostics: &mut Vec<Diagnostic>) -> Program {
    let tokens = lex(source, diagnostics);
    Parser {
        source,
        tokens,
        pos: 0,
        diagnostics,
    }
    .program()
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl Parser<'_> {
    fn peek(&self) -> Token {
        self.tokens[self.pos]
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.peek().kind == kind
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos];
        if t.kind != TokenKind::Eof {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn text(&self, token: Token) -> &str {
        &self.source[token.span.start as usize..token.span.end as usize]
    }

    fn error_at(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code("SYN0003")
                .with_span(span),
        );
    }

    /// Expect `kind`; on mismatch diagnose and leave the token in place.
    fn expect(&mut self, kind: TokenKind, what: &str) -> Option<Token> {
        if self.at(kind) {
            Some(self.bump())
        } else {
            let found = self.peek();
            self.error_at(found.span, format!("expected {what}"));
            None
        }
    }

    // --- grammar ---

    fn program(&mut self) -> Program {
        let mut functions = Vec::new();
        let mut classes = Vec::new();
        let mut imports = Vec::new();
        while !self.at(TokenKind::Eof) {
            match self.peek().kind {
                TokenKind::KwImport => imports.push(self.import()),
                TokenKind::KwFunction => functions.push(self.function()),
                TokenKind::KwClass => classes.push(self.class()),
                _ => {
                    let t = self.peek();
                    self.error_at(t.span, "expected a function, class, or import declaration");
                    // Synchronize to the next top-level declaration or EOF.
                    while !self.at(TokenKind::Eof)
                        && !self.at(TokenKind::KwFunction)
                        && !self.at(TokenKind::KwClass)
                        && !self.at(TokenKind::KwImport)
                    {
                        self.bump();
                    }
                }
            }
        }
        Program {
            functions,
            classes,
            imports,
        }
    }

    fn import(&mut self) -> Import {
        let start = self.bump().span; // `import`
        self.expect(TokenKind::LBrace, "`{`");
        let mut names = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            names.push(self.name("an imported name"));
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RBrace, "`}`");
        self.expect(TokenKind::KwFrom, "`from`");
        let module = match self.expect(TokenKind::String, "a module string") {
            Some(t) => decode_string(self.text(t)),
            None => String::new(),
        };
        let end = self.peek().span;
        self.expect(TokenKind::Semi, "`;`");
        Import {
            names,
            module,
            span: start.to(end),
        }
    }

    fn class(&mut self) -> Class {
        let start = self.bump().span; // `class`
        let name = self.name("a class name");
        let base = if self.eat(TokenKind::KwExtends) {
            Some(self.name("a base class name"))
        } else {
            None
        };
        self.expect(TokenKind::LBrace, "`{`");
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let is_extern = self.eat(TokenKind::KwExtern);
            let member_name = self.name("a field or method name");
            // A `(` after the name starts a method; `:` starts a field.
            if self.at(TokenKind::LParen) {
                methods.push(self.method(member_name, is_extern));
            } else {
                self.expect(TokenKind::Colon, "`:` (fields require a type)");
                let ty = TypeAnnotation {
                    name: self.name("a type name"),
                };
                self.expect(TokenKind::Semi, "`;`");
                fields.push(ClassField {
                    name: member_name,
                    ty,
                });
            }
        }
        let close = self.peek().span;
        self.expect(TokenKind::RBrace, "`}`");
        Class {
            name,
            base,
            fields,
            methods,
            span: start.to(close),
        }
    }

    /// Parse a method, given its already-consumed name. Shares the parameter,
    /// return-type, and body grammar with top-level functions.
    fn method(&mut self, name: Name, is_extern: bool) -> Function {
        self.expect(TokenKind::LParen, "`(`");
        let mut params = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            let pname = self.name("a parameter name");
            let ty = self.optional_type_annotation();
            let default_value = if self.eat(TokenKind::Eq) {
                Some(Box::new(self.expression()))
            } else {
                None
            };
            params.push(Param {
                name: pname,
                ty,
                default_value,
            });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RParen, "`)`");
        let return_type = self.optional_type_annotation();
        let body = if is_extern {
            let end = self
                .expect(TokenKind::Semi, "`;` after extern method")
                .map(|t| t.span)
                .unwrap_or(name.span);
            Block {
                statements: Vec::new(),
                span: name.span.to(end),
            }
        } else {
            self.block()
        };
        let span = name.span.to(body.span);
        Function {
            name,
            params,
            return_type,
            is_extern,
            body,
            span,
        }
    }

    fn function(&mut self) -> Function {
        let start = self.bump().span; // `function`
        let name = self.name("a function name");

        self.expect(TokenKind::LParen, "`(`");
        let mut params = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            let pname = self.name("a parameter name");
            let ty = self.optional_type_annotation();
            let default_value = if self.eat(TokenKind::Eq) {
                Some(Box::new(self.expression()))
            } else {
                None
            };
            params.push(Param {
                name: pname,
                ty,
                default_value,
            });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RParen, "`)`");

        let return_type = self.optional_type_annotation();
        let body = self.block();
        let span = start.to(body.span);
        Function {
            name,
            params,
            return_type,
            is_extern: false,
            body,
            span,
        }
    }

    fn function_expression(&mut self) -> Expr {
        let start = self.bump().span; // `function`
                                      // A name is deliberately not accepted here: named declarations are
                                      // parsed at the program/class level, while this form is anonymous.
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

    fn parameters(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
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
            });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        params
    }

    fn name(&mut self, what: &str) -> Name {
        match self.expect(TokenKind::Ident, what) {
            Some(t) => Name { span: t.span },
            None => Name {
                span: Span::empty_at(self.peek().span.start),
            },
        }
    }

    fn optional_type_annotation(&mut self) -> Option<TypeAnnotation> {
        if self.eat(TokenKind::Colon) {
            Some(TypeAnnotation {
                name: self.name("a type name"),
            })
        } else {
            None
        }
    }

    fn block(&mut self) -> Block {
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

    fn statement(&mut self) -> Stmt {
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

    fn let_statement(&mut self) -> Stmt {
        let start = self.bump().span; // `let`
        let name = self.name("a variable name");
        let ty = self.optional_type_annotation();
        self.expect(TokenKind::Eq, "`=` (phase 1 requires an initializer)");
        let init = self.expression();
        let end = self.peek().span;
        self.expect(TokenKind::Semi, "`;`");
        Stmt::Let {
            name,
            ty,
            init,
            span: start.to(end),
        }
    }

    fn if_statement(&mut self) -> Stmt {
        let start = self.bump().span; // `if`
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

    /// Parse switch cases and reduce them to the existing if/else AST.
    /// Cases are deliberately non-fallthrough; `break` is consumed as the
    /// natural end of a case because the generated chain exits after a match.
    fn switch_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        self.expect(TokenKind::LParen, "`(` after switch");
        let value = self.expression();
        self.expect(TokenKind::RParen, "`)` after switch expression");
        self.expect(TokenKind::LBrace, "`{` after switch");
        let mut cases = Vec::new();
        let mut default_body = None;
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            if self.eat(TokenKind::KwCase) {
                let case_value = self.expression();
                self.expect(TokenKind::Colon, "`:` after case value");
                let mut statements = Vec::new();
                while !self.at(TokenKind::KwCase)
                    && !self.at(TokenKind::KwDefault)
                    && !self.at(TokenKind::RBrace)
                    && !self.at(TokenKind::Eof)
                {
                    let statement = self.statement();
                    if !matches!(statement, Stmt::Break { .. }) {
                        statements.push(statement);
                    }
                }
                let case_span = case_value.span();
                cases.push((
                    case_value,
                    Block {
                        statements,
                        span: case_span,
                    },
                ));
            } else if self.eat(TokenKind::KwDefault) {
                self.expect(TokenKind::Colon, "`:` after default");
                let mut statements = Vec::new();
                while !self.at(TokenKind::KwCase)
                    && !self.at(TokenKind::RBrace)
                    && !self.at(TokenKind::Eof)
                {
                    let statement = self.statement();
                    if !matches!(statement, Stmt::Break { .. }) {
                        statements.push(statement);
                    }
                }
                default_body = Some(Block {
                    statements,
                    span: self.peek().span,
                });
            } else {
                self.error_at(self.peek().span, "expected `case` or `default`");
                self.bump();
            }
        }
        let close = self.peek().span;
        self.expect(TokenKind::RBrace, "`}` after switch");
        let mut chain = default_body;
        for (case_value, body) in cases.into_iter().rev() {
            let cond = Expr::Binary {
                op: BinaryOp::Eq,
                lhs: Box::new(value.clone()),
                rhs: Box::new(case_value),
                span: value.span().to(body.span),
            };
            chain = Some(Block {
                span: start.to(close),
                statements: vec![Stmt::If {
                    cond,
                    then_block: body,
                    else_block: chain,
                    span: start.to(close),
                }],
            });
        }
        chain
            .map(|block| {
                block
                    .statements
                    .into_iter()
                    .next()
                    .unwrap_or(Stmt::Expr(Expr::Null { span: start }))
            })
            .unwrap_or(Stmt::Expr(Expr::Null { span: start }))
    }

    fn while_statement(&mut self) -> Stmt {
        let start = self.bump().span; // `while`
        self.expect(TokenKind::LParen, "`(`");
        let cond = self.expression();
        self.expect(TokenKind::RParen, "`)`");
        let body = self.block();
        let span = start.to(body.span);
        Stmt::While { cond, body, span }
    }

    fn do_while_statement(&mut self) -> Stmt {
        let start = self.bump().span; // `do`
        let body = self.block();
        self.expect(TokenKind::KwWhile, "`while` after do body");
        self.expect(TokenKind::LParen, "`(` after do-while");
        let cond = self.expression();
        let close = self.expect(TokenKind::RParen, "`)` after do-while condition");
        let end = close.map(|token| token.span).unwrap_or(cond.span());
        self.expect(TokenKind::Semi, "`;` after do-while");
        Stmt::DoWhile {
            body,
            cond,
            span: start.to(end),
        }
    }

    fn for_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        self.expect(TokenKind::LParen, "`(` after for");
        if self.eat(TokenKind::KwLet) {
            let name = self.name("a for binding name");
            let ty = self.optional_type_annotation();
            if self.eat(TokenKind::KwOf) {
                let iterable = self.expression();
                self.expect(TokenKind::RParen, "`)` after for-of iterable");
                let body = self.block();
                let span = start.to(body.span);
                return Stmt::ForOf {
                    name,
                    iterable,
                    body,
                    span,
                };
            }
            self.expect(TokenKind::Eq, "`=` in for initializer");
            let init = self.expression();
            let init_span = name.span.to(init.span());
            self.expect(TokenKind::Semi, "`;` after for initializer");
            let cond = (!self.at(TokenKind::Semi)).then(|| self.expression());
            self.expect(TokenKind::Semi, "`;` after for condition");
            let update = (!self.at(TokenKind::RParen)).then(|| self.expression());
            self.expect(TokenKind::RParen, "`)` after for update");
            let body = self.block();
            return Stmt::For {
                init: Some(Box::new(Stmt::Let {
                    name,
                    ty,
                    init,
                    span: init_span,
                })),
                cond,
                update,
                span: start.to(body.span),
                body,
            };
        }
        self.expect(TokenKind::Semi, "`;` after for initializer");
        let cond = (!self.at(TokenKind::Semi)).then(|| self.expression());
        self.expect(TokenKind::Semi, "`;` after for condition");
        let update = (!self.at(TokenKind::RParen)).then(|| self.expression());
        self.expect(TokenKind::RParen, "`)` after for update");
        let body = self.block();
        Stmt::For {
            init: None,
            cond,
            update,
            span: start.to(body.span),
            body,
        }
    }

    fn break_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        let end = self.peek().span;
        self.expect(TokenKind::Semi, "`;` after break");
        Stmt::Break {
            span: start.to(end),
        }
    }

    fn continue_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        let end = self.peek().span;
        self.expect(TokenKind::Semi, "`;` after continue");
        Stmt::Continue {
            span: start.to(end),
        }
    }

    fn return_statement(&mut self) -> Stmt {
        let start = self.bump().span; // `return`
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

    fn throw_statement(&mut self) -> Stmt {
        let start = self.bump().span;
        let value = self.expression();
        let end = self.peek().span;
        self.expect(TokenKind::Semi, "`;` after throw");
        Stmt::Throw {
            value,
            span: start.to(end),
        }
    }

    fn try_statement(&mut self) -> Stmt {
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

    // --- expressions (precedence climbing) ---

    fn expression(&mut self) -> Expr {
        self.assignment()
    }

    fn assignment(&mut self) -> Expr {
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

    fn conditional(&mut self) -> Expr {
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

    fn binary(&mut self, min_level: u8) -> Expr {
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

    fn unary(&mut self) -> Expr {
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

    /// `call ('as' type)*` — casts bind tighter than binary operators but looser
    /// than calls and member access.
    fn cast(&mut self) -> Expr {
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

    fn call(&mut self) -> Expr {
        let mut expr = self.primary();
        loop {
            match self.peek().kind {
                // `ident(args)` — only a bare name is callable in phase 2.
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
                // `object.field` or `object.method(args)`
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
                    let field = self.name("an optional field name");
                    let span = expr.span().to(field.span);
                    expr = Expr::OptionalField {
                        object: Box::new(expr),
                        field,
                        span,
                    };
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

    /// Parse `(a, b, c)` argument list; the opening `(` is already consumed and
    /// the closing `)` is consumed here.
    fn arguments(&mut self) -> Vec<Expr> {
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

    fn primary(&mut self) -> Expr {
        let t = self.peek();
        match t.kind {
            TokenKind::Number => {
                self.bump();
                let value = self.text(t).parse::<f64>().unwrap_or_else(|_| {
                    // The lexer only emits digit sequences, so this is unreachable
                    // in practice; diagnose rather than crash if it ever changes.
                    f64::NAN
                });
                Expr::Number {
                    value,
                    span: t.span,
                }
            }
            TokenKind::String => {
                self.bump();
                Expr::String {
                    value: decode_string(self.text(t)),
                    span: t.span,
                }
            }
            TokenKind::KwTrue => {
                self.bump();
                Expr::Bool {
                    value: true,
                    span: t.span,
                }
            }
            TokenKind::KwFalse => {
                self.bump();
                Expr::Bool {
                    value: false,
                    span: t.span,
                }
            }
            TokenKind::KwNull => {
                self.bump();
                Expr::Null { span: t.span }
            }
            TokenKind::KwNew => {
                let start = self.bump().span; // `new`
                let class = self.name("a class name");
                self.expect(TokenKind::LParen, "`(`");
                let args = self.arguments();
                let close = self.tokens[self.pos - 1].span;
                Expr::New {
                    class,
                    args,
                    span: start.to(close),
                }
            }
            TokenKind::KwFunction => self.function_expression(),
            TokenKind::Ident => {
                self.bump();
                let name = Name { span: t.span };
                if self.eat(TokenKind::Arrow) {
                    let body = self.arrow_body();
                    return Expr::Function(FunctionExpr {
                        params: vec![Param {
                            name,
                            ty: None,
                            default_value: None,
                        }],
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
                    return Expr::Function(FunctionExpr {
                        params,
                        return_type: None,
                        span: start.to(body.span),
                        body,
                        is_arrow: true,
                    });
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

    fn arrow_body(&mut self) -> Block {
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

    fn parenthesized_arrow_start(&self) -> bool {
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

/// Operator and binding level for a binary-operator token. Higher binds tighter.
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

/// Strip quotes and process escapes in a string literal's source text.
fn decode_string(raw: &str) -> String {
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('0') => out.push('\0'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Program {
        let mut diags = Vec::new();
        let p = parse(src, &mut diags);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        p
    }

    fn ident_text<'a>(src: &'a str, name: &Name) -> &'a str {
        &src[name.span.start as usize..name.span.end as usize]
    }

    #[test]
    fn parses_function_with_params_and_return_type() {
        let src = "function add(a: number, b: number): number { return a + b; }";
        let p = parse_ok(src);
        assert_eq!(p.functions.len(), 1);
        let f = &p.functions[0];
        assert_eq!(ident_text(src, &f.name), "add");
        assert_eq!(f.params.len(), 2);
        assert!(f.return_type.is_some());
        assert_eq!(f.body.statements.len(), 1);
    }

    #[test]
    fn parses_anonymous_function_and_arrow_forms() {
        let src = "function main() { let a = x => x + 1; let b = (x: number, y) => { return x + y; }; let c = function(x) { return x; }; }";
        let p = parse_ok(src);
        let body = &p.functions[0].body.statements;
        let Stmt::Let {
            init: Expr::Function(first),
            ..
        } = &body[0]
        else {
            panic!("expected single-parameter arrow");
        };
        assert!(first.is_arrow);
        assert_eq!(first.params.len(), 1);
        let Stmt::Let {
            init: Expr::Function(second),
            ..
        } = &body[1]
        else {
            panic!("expected parenthesized arrow");
        };
        assert!(second.is_arrow);
        assert_eq!(second.params.len(), 2);
        let Stmt::Let {
            init: Expr::Function(third),
            ..
        } = &body[2]
        else {
            panic!("expected anonymous function");
        };
        assert!(!third.is_arrow);
    }

    #[test]
    fn parses_default_parameter_expressions() {
        let p = parse_ok("function add(value: number = 2.0): number { return value; }");
        assert!(p.functions[0].params[0].default_value.is_some());
    }

    #[test]
    fn parses_spread_call_arguments() {
        let p = parse_ok("function main() { print(...values); }");
        let Stmt::Expr(Expr::Call { args, .. }) = &p.functions[0].body.statements[0] else {
            panic!("expected call expression");
        };
        assert!(matches!(args[0], Expr::Spread { .. }));
    }

    #[test]
    fn parses_for_of_statement() {
        let src = "function main() { for (let value of values) { print(value); } }";
        let p = parse_ok(src);
        let Stmt::ForOf {
            name,
            iterable,
            body,
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("expected for-of statement");
        };
        assert_eq!(ident_text(src, name), "value");
        assert!(matches!(iterable, Expr::Ident(_)));
        assert_eq!(body.statements.len(), 1);
    }

    #[test]
    fn precedence_mul_binds_tighter_than_add() {
        let src = "function f() { return 1 + 2 * 3; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(e), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected return");
        };
        let Expr::Binary {
            op: BinaryOp::Add,
            rhs,
            ..
        } = e
        else {
            panic!("expected add at root, got {e:?}");
        };
        assert!(matches!(
            **rhs,
            Expr::Binary {
                op: BinaryOp::Mul,
                ..
            }
        ));
    }

    #[test]
    fn comparison_binds_looser_than_arithmetic() {
        let src = "function f() { return 1 + 2 < 3 * 4; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(e), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected return");
        };
        assert!(matches!(
            e,
            Expr::Binary {
                op: BinaryOp::Lt,
                ..
            }
        ));
    }

    #[test]
    fn parses_let_if_while_and_calls() {
        let src = r#"
            function main() {
                let n: number = 10;
                let acc = 0;
                while (n > 0) {
                    acc = acc + n;
                    n = n - 1;
                }
                if (acc == 55) {
                    print("ok");
                } else {
                    print("bad");
                }
            }
        "#;
        let p = parse_ok(src);
        let body = &p.functions[0].body.statements;
        assert!(matches!(body[0], Stmt::Let { .. }));
        assert!(matches!(body[2], Stmt::While { .. }));
        assert!(matches!(
            body[3],
            Stmt::If {
                else_block: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn parses_do_while_statement() {
        let p = parse_ok("function main() { do { print(1); } while (false); }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::DoWhile { .. }
        ));
    }

    #[test]
    fn parses_c_style_for_statement() {
        let p =
            parse_ok("function main() { for (let i: int = 0; i < 3; i = i + 1) { print(i); } }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::For { .. }
        ));
    }

    #[test]
    fn parses_short_circuit_logical_operators() {
        let p = parse_ok("function main(): boolean { return true || false && false; }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::Return { .. }
        ));
    }

    #[test]
    fn parses_conditional_expression() {
        let p = parse_ok("function main(): int { return true ? 1 : 2; }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::Return { .. }
        ));
    }

    #[test]
    fn parses_nullish_and_optional_property_expressions() {
        let p = parse_ok(
            "function main(): any { let value: any = null; return value?.answer ?? null; }",
        );
        let Stmt::Return {
            value: Some(expr), ..
        } = &p.functions[0].body.statements[1]
        else {
            panic!("expected return expression");
        };
        assert!(matches!(
            expr,
            Expr::Binary {
                op: BinaryOp::Nullish,
                ..
            }
        ));
    }

    #[test]
    fn string_escapes_decode() {
        let src = r#"function f() { let s = "a\nb\"c"; }"#;
        let p = parse_ok(src);
        let Stmt::Let {
            init: Expr::String { value, .. },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("expected string let");
        };
        assert_eq!(value, "a\nb\"c");
    }

    #[test]
    fn unary_and_parens() {
        let src = "function f() { return -(1 + 2); }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(e), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected return");
        };
        let Expr::Unary {
            op: UnaryOp::Neg,
            operand,
            ..
        } = e
        else {
            panic!("expected negation, got {e:?}");
        };
        assert!(matches!(
            **operand,
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn missing_semicolon_diagnoses_but_still_parses() {
        let mut diags = Vec::new();
        let p = parse("function f() { let x = 1 }", &mut diags);
        assert_eq!(p.functions.len(), 1);
        assert!(!diags.is_empty());
    }

    #[test]
    fn invalid_assignment_target_diagnoses() {
        let mut diags = Vec::new();
        parse("function f() { 1 = 2; }", &mut diags);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("assignment target")));
    }

    #[test]
    fn stray_top_level_tokens_recover_to_next_function() {
        let mut diags = Vec::new();
        let p = parse("let x = 1; function ok() {}", &mut diags);
        assert_eq!(p.functions.len(), 1);
        assert!(!diags.is_empty());
    }

    #[test]
    fn parses_class_with_fields() {
        let src = "class Point { x: number; y: number; }";
        let p = parse_ok(src);
        assert_eq!(p.classes.len(), 1);
        let c = &p.classes[0];
        assert_eq!(ident_text(src, &c.name), "Point");
        assert_eq!(c.fields.len(), 2);
        assert_eq!(ident_text(src, &c.fields[0].name), "x");
    }

    #[test]
    fn parses_new_and_field_access() {
        let src = "function f() { let p = new Point(1, 2); print(p.x); }";
        let p = parse_ok(src);
        let body = &p.functions[0].body.statements;
        let Stmt::Let {
            init: Expr::New { args, .. },
            ..
        } = &body[0]
        else {
            panic!("expected new");
        };
        assert_eq!(args.len(), 2);
        let Stmt::Expr(Expr::Call { args, .. }) = &body[1] else {
            panic!("expected call");
        };
        assert!(matches!(args[0], Expr::Field { .. }));
    }

    #[test]
    fn parses_computed_symbol_iterator_call() {
        let program =
            parse_ok("function main(): any { let iterator = values[Symbol.iterator](); }");
        assert!(matches!(
            &program.functions[0].body.statements[0],
            Stmt::Let {
                init: Expr::ComputedCall { .. },
                ..
            }
        ));
    }

    #[test]
    fn parses_field_assignment() {
        let src = "function f() { p.x = 5; }";
        let p = parse_ok(src);
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::Expr(Expr::FieldAssign { .. })
        ));
    }

    #[test]
    fn parses_import() {
        let src = "import { readText, writeText } from \"zun:fs\";\nfunction main() {}";
        let p = parse_ok(src);
        assert_eq!(p.imports.len(), 1);
        assert_eq!(p.imports[0].module, "zun:fs");
        assert_eq!(p.imports[0].names.len(), 2);
        assert_eq!(p.functions.len(), 1);
    }

    #[test]
    fn parses_class_with_methods() {
        let src = "class Counter { n: number; inc() { n = n + 1; } get(): number { return n; } }";
        let p = parse_ok(src);
        let c = &p.classes[0];
        assert_eq!(c.fields.len(), 1);
        assert_eq!(c.methods.len(), 2);
        assert_eq!(ident_text(src, &c.methods[0].name), "inc");
    }

    #[test]
    fn parses_extends() {
        let src = "class Dog extends Animal { bark(): number { return 1; } }";
        let p = parse_ok(src);
        let c = &p.classes[0];
        assert!(c.base.is_some());
        assert_eq!(ident_text(src, c.base.as_ref().unwrap()), "Animal");
    }

    #[test]
    fn parses_method_call() {
        let src = "function f() { let r = c.add(1, 2); }";
        let p = parse_ok(src);
        let Stmt::Let {
            init: Expr::MethodCall { args, .. },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("expected method call");
        };
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn chained_field_access() {
        let src = "function f() { let z = a.b.c; }";
        let p = parse_ok(src);
        let Stmt::Let {
            init: Expr::Field { object, .. },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("expected field access");
        };
        assert!(matches!(**object, Expr::Field { .. }));
    }
}
