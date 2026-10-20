//! Small parser lookahead helpers extracted from the parser core.

use super::*;

impl Parser<'_> {
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
