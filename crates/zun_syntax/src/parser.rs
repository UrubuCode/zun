//! Recursive-descent parser.
//!
//! Consumes the token stream from [`lexer`](crate::lexer) and produces a
//! [`Program`]. Expressions use precedence climbing. On error the parser
//! diagnoses and synchronizes to the next statement boundary rather than
//! aborting, so one mistake yields one diagnostic, not a cascade.

use crate::ast::*;
use crate::lexer::{lex, Token, TokenKind};
use zun_common::{Diagnostic, Span};

#[path = "parser_helpers.rs"]
mod parser_helpers;

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

    fn peek_n(&self, offset: usize) -> Token {
        self.tokens
            .get(self.pos + offset)
            .copied()
            .unwrap_or_else(|| *self.tokens.last().expect("lexer always emits EOF"))
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
            let is_rest = self.eat(TokenKind::Ellipsis);
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
                is_rest,
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
            let is_rest = self.eat(TokenKind::Ellipsis);
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
                is_rest,
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

    fn let_statement(&mut self) -> Stmt {
        let start = self.bump().span; // `let`
        if self.eat(TokenKind::LBracket) {
            let mut names = Vec::new();
            while !self.at(TokenKind::RBracket) && !self.at(TokenKind::Eof) {
                names.push(self.name("an array destructuring binding"));
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RBracket, "`]` after array destructuring pattern");
            self.expect(TokenKind::Eq, "`=` after array destructuring pattern");
            let init = self.expression();
            let end = self.peek().span;
            self.expect(TokenKind::Semi, "`;`");
            return Stmt::ArrayDestructure {
                names,
                init,
                span: start.to(end),
            };
        }
        if self.eat(TokenKind::LBrace) {
            let mut names = Vec::new();
            while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
                names.push(self.name("an object destructuring binding"));
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RBrace, "`}` after object destructuring pattern");
            self.expect(TokenKind::Eq, "`=` after object destructuring pattern");
            let init = self.expression();
            let end = self.peek().span;
            self.expect(TokenKind::Semi, "`;`");
            return Stmt::ObjectDestructure {
                names,
                init,
                span: start.to(end),
            };
        }
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

    // --- expressions (precedence climbing) ---

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
            TokenKind::Template => {
                self.bump();
                let raw = self.text(t).to_string();
                Expr::Template {
                    parts: parse_template_parts(&raw, t.span, &mut self.diagnostics),
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
                            is_rest: false,
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

fn parse_template_parts(
    raw: &str,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<crate::ast::TemplatePart> {
    let inner = &raw[1..raw.len() - 1];
    let mut parts = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = find_template_marker(&inner[cursor..]) {
        let start = cursor + relative;
        if start > cursor {
            parts.push(crate::ast::TemplatePart::Text(decode_template_text(&inner[cursor..start])));
        }
        let expression_start = start + 2;
        let Some(end) = find_template_interpolation_end(inner, expression_start) else {
            diagnostics.push(Diagnostic::error("unterminated template interpolation").with_code("SYN0004").with_span(span));
            return parts;
        };
        let expression = inner[expression_start..end].trim();
        let mut expression_diagnostics = Vec::new();
        let Some(mut parsed) = parse_expression_fragment(expression, &mut expression_diagnostics) else {
            diagnostics.extend(expression_diagnostics);
            diagnostics.push(Diagnostic::error("invalid template interpolation").with_code("SYN0005").with_span(span));
            cursor = end + 1;
            continue;
        };
        if !expression_diagnostics.is_empty() {
            diagnostics.extend(expression_diagnostics);
        } else {
            let offset = expression.as_ptr() as usize - raw.as_ptr() as usize;
            rebase_expression_names(&mut parsed, span.start + offset as u32);
            parts.push(crate::ast::TemplatePart::Expr(Box::new(parsed)));
        }
        cursor = end + 1;
    }
    if cursor < inner.len() {
        parts.push(crate::ast::TemplatePart::Text(decode_template_text(&inner[cursor..])));
    }
    parts
}

fn find_template_marker(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == b'$' && bytes[index + 1] == b'{' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn find_template_interpolation_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' {
            index += 2;
            continue;
        }
        if let Some(current) = quote {
            if byte == current {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(byte, b'"' | b'\'') {
            quote = Some(byte);
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
        index += 1;
    }
    None
}

fn decode_template_text(text: &str) -> String {
    decode_string(&format!("\"{text}\""))
}

fn parse_expression_fragment(source: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<Expr> {
    let tokens = lex(source, diagnostics);
    let mut parser = Parser { source, tokens, pos: 0, diagnostics };
    let expression = parser.expression();
    if parser.at(TokenKind::Eof) && diagnostics.is_empty() {
        Some(expression)
    } else {
        None
    }
}

fn rebase_expression_names(expr: &mut Expr, base: u32) {
    let shift = |span: &mut Span| {
        *span = Span::new(span.start + base, span.end + base);
    };
    match expr {
        Expr::Ident(name) => shift(&mut name.span),
        Expr::Assign { target, value, .. } => { shift(&mut target.span); rebase_expression_names(value, base); }
        Expr::Binary { lhs, rhs, .. } => { rebase_expression_names(lhs, base); rebase_expression_names(rhs, base); }
        Expr::Conditional { condition, when_true, when_false, .. } => {
            rebase_expression_names(condition, base); rebase_expression_names(when_true, base); rebase_expression_names(when_false, base);
        }
        Expr::Unary { operand, .. } | Expr::Cast { expr: operand, .. } => rebase_expression_names(operand, base),
        Expr::Call { callee, args, .. } => { shift(&mut callee.span); for arg in args { rebase_expression_names(arg, base); } }
        Expr::ComputedCall { callee, args, .. } => { rebase_expression_names(callee, base); for arg in args { rebase_expression_names(arg, base); } }
        Expr::MethodCall { receiver, method, args, .. } | Expr::OptionalMethodCall { object: receiver, method, args, .. } => {
            rebase_expression_names(receiver, base); shift(&mut method.span); for arg in args { rebase_expression_names(arg, base); }
        }
        Expr::Spread { expr, .. } => rebase_expression_names(expr, base),
        Expr::OptionalField { object, field, .. } | Expr::Field { object, field, .. } => {
            rebase_expression_names(object, base); shift(&mut field.span);
        }
        Expr::New { class, args, .. } => { shift(&mut class.span); for arg in args { rebase_expression_names(arg, base); } }
        Expr::FieldAssign { object, field, value, .. } => {
            rebase_expression_names(object, base); shift(&mut field.span); rebase_expression_names(value, base);
        }
        Expr::Index { object, index, .. } => { rebase_expression_names(object, base); rebase_expression_names(index, base); }
        Expr::IndexAssign { object, index, value, .. } => {
            rebase_expression_names(object, base); rebase_expression_names(index, base); rebase_expression_names(value, base);
        }
        Expr::Function(_) | Expr::Number { .. } | Expr::String { .. } | Expr::Template { .. }
        | Expr::Bool { .. } | Expr::Null { .. } => {}
    }
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
    fn parses_template_literal_segments() {
        let src = "function main() { let name: string = \"Zun\"; return `hello ${name}!`; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(Expr::Template { parts, .. }), .. } = &p.functions[0].body.statements[1] else {
            panic!("expected template literal");
        };
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], crate::ast::TemplatePart::Text("hello ".into()));
        assert!(matches!(parts[1], crate::ast::TemplatePart::Expr(_)));
        assert_eq!(parts[2], crate::ast::TemplatePart::Text("!".into()));
    }

    #[test]
    fn parses_expression_inside_template_interpolation() {
        let src = "function main() { let value: number = 1.0; return `${value + 2.0}`; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(Expr::Template { parts, .. }), .. } = &p.functions[0].body.statements[1] else {
            panic!("expected template literal");
        };
        assert!(matches!(&parts[0], crate::ast::TemplatePart::Expr(expression) if matches!(expression.as_ref(), Expr::Binary { .. })));
    }

    #[test]
    fn keeps_braces_inside_quoted_template_interpolations() {
        let src = "function main() { return `${\"}\"}`; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(Expr::Template { parts, .. }), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected template literal");
        };
        assert!(matches!(&parts[0], crate::ast::TemplatePart::Expr(expression) if matches!(expression.as_ref(), Expr::String { value, .. } if value == "}")));
    }

    #[test]
    fn escaped_template_marker_stays_literal_text() {
        let src = r#"function main() { return `literal \${name}`; }"#;
        let p = parse_ok(src);
        let Stmt::Return { value: Some(Expr::Template { parts, .. }), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected template literal");
        };
        assert_eq!(parts.as_slice(), [crate::ast::TemplatePart::Text("literal ${name}".into())]);
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
    fn parses_array_destructuring_declaration() {
        let p = parse_ok("function main() { let [first, second] = values; }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::ArrayDestructure { ref names, .. } if names.len() == 2
        ));
    }

    #[test]
    fn parses_object_destructuring_declaration() {
        let p = parse_ok("function main() { let {answer, other} = object; }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::ObjectDestructure { ref names, .. } if names.len() == 2
        ));
    }

    #[test]
    fn parses_rest_parameter_without_enabling_lowering() {
        let p = parse_ok("function collect(...values: Array) { return values; }");
        assert!(p.functions[0].params[0].is_rest);
        assert!(p.functions[0].params[0].ty.is_some());
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
    fn parses_labeled_break_and_continue() {
        let mut diagnostics = Vec::new();
        let program = parse(
            "function main(): int { outer: while (true) { continue outer; break outer; } return 0; }",
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let statements = &program.functions[0].body.statements;
        assert!(matches!(
            &statements[0],
            Stmt::Labeled { statement, .. }
                if matches!(statement.as_ref(), Stmt::While { body, .. }
                    if matches!(body.statements[0], Stmt::Continue { label: Some(_), .. }))
        ));
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
