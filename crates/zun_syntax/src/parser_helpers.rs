//! Small parser lookahead helpers extracted from the parser core.

use super::*;

impl Parser<'_> {
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
