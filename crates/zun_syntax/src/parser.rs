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
        self.diagnostics
            .push(Diagnostic::error(message).with_code("SYN0003").with_span(span));
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
        while !self.at(TokenKind::Eof) {
            match self.peek().kind {
                TokenKind::KwFunction => functions.push(self.function()),
                TokenKind::KwClass => classes.push(self.class()),
                _ => {
                    let t = self.peek();
                    self.error_at(t.span, "expected a function or class declaration");
                    // Synchronize to the next top-level declaration or EOF.
                    while !self.at(TokenKind::Eof)
                        && !self.at(TokenKind::KwFunction)
                        && !self.at(TokenKind::KwClass)
                    {
                        self.bump();
                    }
                }
            }
        }
        Program { functions, classes }
    }

    fn class(&mut self) -> Class {
        let start = self.bump().span; // `class`
        let name = self.name("a class name");
        self.expect(TokenKind::LBrace, "`{`");
        let mut fields = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let fname = self.name("a field name");
            self.expect(TokenKind::Colon, "`:` (fields require a type)");
            let ty = TypeAnnotation {
                name: self.name("a type name"),
            };
            self.expect(TokenKind::Semi, "`;`");
            fields.push(ClassField { name: fname, ty });
        }
        let close = self.peek().span;
        self.expect(TokenKind::RBrace, "`}`");
        Class {
            name,
            fields,
            span: start.to(close),
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
            params.push(Param { name: pname, ty });
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
            body,
            span,
        }
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
            TokenKind::KwWhile => self.while_statement(),
            TokenKind::KwReturn => self.return_statement(),
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

    fn while_statement(&mut self) -> Stmt {
        let start = self.bump().span; // `while`
        self.expect(TokenKind::LParen, "`(`");
        let cond = self.expression();
        self.expect(TokenKind::RParen, "`)`");
        let body = self.block();
        let span = start.to(body.span);
        Stmt::While { cond, body, span }
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

    // --- expressions (precedence climbing) ---

    fn expression(&mut self) -> Expr {
        self.assignment()
    }

    fn assignment(&mut self) -> Expr {
        let lhs = self.binary(0);
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
                _ => self.error_at(eq_span, "invalid assignment target"),
            }
        }
        lhs
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
        self.call()
    }

    fn call(&mut self) -> Expr {
        let mut expr = self.primary();
        loop {
            match self.peek().kind {
                // `ident(args)` — only a bare name is callable in phase 2.
                TokenKind::LParen => {
                    let Expr::Ident(callee) = expr else {
                        break;
                    };
                    self.bump();
                    let args = self.arguments();
                    let close = self.tokens[self.pos - 1].span;
                    expr = Expr::Call {
                        callee,
                        args,
                        span: callee.span.to(close),
                    };
                }
                // `object.field`
                TokenKind::Dot => {
                    self.bump();
                    let field = self.name("a field name");
                    let span = expr.span().to(field.span);
                    expr = Expr::Field {
                        object: Box::new(expr),
                        field,
                        span,
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
            args.push(self.expression());
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
                Expr::Number { value, span: t.span }
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
                Expr::Bool { value: true, span: t.span }
            }
            TokenKind::KwFalse => {
                self.bump();
                Expr::Bool { value: false, span: t.span }
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
            TokenKind::Ident => {
                self.bump();
                Expr::Ident(Name { span: t.span })
            }
            TokenKind::LParen => {
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
    fn precedence_mul_binds_tighter_than_add() {
        let src = "function f() { return 1 + 2 * 3; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(e), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected return");
        };
        let Expr::Binary { op: BinaryOp::Add, rhs, .. } = e else {
            panic!("expected add at root, got {e:?}");
        };
        assert!(matches!(**rhs, Expr::Binary { op: BinaryOp::Mul, .. }));
    }

    #[test]
    fn comparison_binds_looser_than_arithmetic() {
        let src = "function f() { return 1 + 2 < 3 * 4; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(e), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected return");
        };
        assert!(matches!(e, Expr::Binary { op: BinaryOp::Lt, .. }));
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
        assert!(matches!(body[3], Stmt::If { else_block: Some(_), .. }));
    }

    #[test]
    fn string_escapes_decode() {
        let src = r#"function f() { let s = "a\nb\"c"; }"#;
        let p = parse_ok(src);
        let Stmt::Let { init: Expr::String { value, .. }, .. } =
            &p.functions[0].body.statements[0]
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
        let Expr::Unary { op: UnaryOp::Neg, operand, .. } = e else {
            panic!("expected negation, got {e:?}");
        };
        assert!(matches!(**operand, Expr::Binary { op: BinaryOp::Add, .. }));
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
        assert!(diags.iter().any(|d| d.message.contains("assignment target")));
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
        let Stmt::Let { init: Expr::New { args, .. }, .. } = &body[0] else {
            panic!("expected new");
        };
        assert_eq!(args.len(), 2);
        let Stmt::Expr(Expr::Call { args, .. }) = &body[1] else {
            panic!("expected call");
        };
        assert!(matches!(args[0], Expr::Field { .. }));
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
    fn chained_field_access() {
        let src = "function f() { let z = a.b.c; }";
        let p = parse_ok(src);
        let Stmt::Let { init: Expr::Field { object, .. }, .. } =
            &p.functions[0].body.statements[0]
        else {
            panic!("expected field access");
        };
        assert!(matches!(**object, Expr::Field { .. }));
    }
}
