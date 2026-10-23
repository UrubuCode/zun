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
use parser_helpers::decode_string;

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
                TokenKind::KwFunction => functions.push(self.function(false, false)),
                TokenKind::KwExtern if self.peek_n(1).kind == TokenKind::KwFunction => {
                    self.bump();
                    functions.push(self.function(false, true));
                }
                TokenKind::KwAsync if self.peek_n(1).kind == TokenKind::KwFunction => {
                    self.bump();
                    functions.push(self.function(true, false));
                }
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
        let mut names = Vec::new();
        if self.eat(TokenKind::LBrace) {
            while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
                let imported = self.name("an imported name");
                let local = if self.eat(TokenKind::KwAs) {
                    self.name("an import alias")
                } else {
                    imported.clone()
                };
                names.push(ImportName { imported, local, is_default: false, is_namespace: false });
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::RBrace, "`}`");
        } else {
            // A default import is represented as an export named `default`;
            // sema/linking decides whether that export is a type or value.
            let local = self.name("a default import name");
            let imported = local.clone();
            names.push(ImportName { imported, local, is_default: true, is_namespace: false });
        }
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
            // Neither `expect` nor `name` consumes the token it rejects, so a
            // member the grammar cannot start leaves the cursor where it was
            // and the loop spins on it, reporting the same error and growing
            // the member lists until the process dies — `class P { y: i64 = 5; }`
            // used to abort on an 8 GiB allocation rather than report a
            // syntax error.
            //
            // Skip what cannot begin a member instead of parsing it into a
            // field made entirely of error nodes, so the well-formed members
            // around it still come through.
            if !matches!(
                self.peek().kind,
                TokenKind::Ident | TokenKind::Hash | TokenKind::KwStatic | TokenKind::KwExtern
            ) {
                let span = self.peek().span;
                self.error_at(span, "expected a field or method name".to_string());
                self.bump();
                continue;
            }
            let entry = self.pos;
            let is_private = self.eat(TokenKind::Hash);
            let is_static = self.eat(TokenKind::KwStatic);
            let is_extern = self.eat(TokenKind::KwExtern);
            let member_name = self.name("a field or method name");
            let accessor = if !is_static && !is_extern && !self.at(TokenKind::LParen)
                && (self.text(self.tokens[self.pos - 1]) == "get" || self.text(self.tokens[self.pos - 1]) == "set")
            {
                let kind = if self.text(self.tokens[self.pos - 1]) == "get" { AccessorKind::Get } else { AccessorKind::Set };
                Some((kind, self.name("a property name")))
            } else { None };
            // A `(` after the name starts a method; `:` starts a field.
            if self.at(TokenKind::LParen) {
                let (accessor, name) = accessor.map_or((None, member_name), |(kind, name)| (Some(kind), name));
                methods.push(self.method(name, is_extern, is_static, accessor));
            } else {
                self.expect(TokenKind::Colon, "`:` (fields require a type)");
                let ty = TypeAnnotation {
                    name: self.name("a type name"),
                };
                // `field: T = expr;` gives the field a default, which makes
                // its constructor argument optional.
                let initializer = if self.eat(TokenKind::Eq) {
                    Some(Box::new(self.expression()))
                } else {
                    None
                };
                self.expect(TokenKind::Semi, "`;`");
                fields.push(ClassField {
                    name: member_name,
                    ty,
                    is_static,
                    is_private,
                    initializer,
                });
            }

            if self.pos == entry {
                // No rule consumed anything: the errors are already reported,
                // so skip the offending token and carry on with the next
                // member rather than hanging.
                self.bump();
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
    fn method(&mut self, name: Name, is_extern: bool, is_static: bool, accessor: Option<AccessorKind>) -> Function {
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
            is_static,
            accessor,
            external_module: None,
            is_async: false,
            is_generator: false,
            body,
            span,
        }
    }

    fn function(&mut self, is_async: bool, is_extern: bool) -> Function {
        let start = self.bump().span; // `function`
        let is_generator = self.eat(TokenKind::Star);
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
        let external_module = if is_extern {
            self.expect(TokenKind::KwFrom, "`from` after extern function")
                .and_then(|_| self.expect(TokenKind::String, "a native library string"))
                .map(|token| decode_string(self.text(token)))
        } else {
            None
        };
        let body = if is_extern {
            let end = self
                .expect(TokenKind::Semi, "`;` after extern function")
                .map(|token| token.span)
                .unwrap_or(name.span);
            Block { statements: Vec::new(), span: name.span.to(end) }
        } else {
            self.block()
        };
        let span = start.to(body.span);
        Function {
            name,
            params,
            return_type,
            is_extern,
            is_static: false,
            accessor: None,
            external_module,
            is_async,
            is_generator,
            body,
            span,
        }
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

}

#[cfg(test)]
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
