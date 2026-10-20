//! Lexer.
//!
//! Turns source text into a flat vector of spanned tokens. The lexer never
//! aborts: unknown characters become `TokenKind::Error` tokens plus a
//! diagnostic, so the parser always receives a complete stream.

use zun_common::{Diagnostic, Span};

/// The kind of a lexical token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    // Literals and names
    /// A numeric literal (integer or decimal).
    Number,
    /// A double-quoted string literal.
    String,
    /// An identifier.
    Ident,

    // Keywords
    /// `function`
    KwFunction,
    /// `let`
    KwLet,
    /// `if`
    KwIf,
    /// `else`
    KwElse,
    /// `switch`
    KwSwitch,
    /// `case`
    KwCase,
    /// `default`
    KwDefault,
    /// `while`
    KwWhile,
    /// `do`
    KwDo,
    /// `for`
    KwFor,
    /// `of`
    KwOf,
    /// `break`
    KwBreak,
    /// `continue`
    KwContinue,
    /// `return`
    KwReturn,
    /// `throw`
    KwThrow,
    /// `try`
    KwTry,
    /// `catch`
    KwCatch,
    /// `finally`
    KwFinally,
    /// `true`
    KwTrue,
    /// `false`
    KwFalse,
    /// `null`
    KwNull,
    /// `class`
    KwClass,
    /// `new`
    KwNew,
    /// `extends`
    KwExtends,
    /// `as`
    KwAs,
    /// `import`
    KwImport,
    /// `from`
    KwFrom,
    /// `extern`
    KwExtern,

    // Punctuation
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `,`
    Comma,
    /// `;`
    Semi,
    /// `:`
    Colon,
    /// `?`
    Question,
    /// `??`
    Nullish,
    /// `?.`
    QuestionDot,
    /// `.`
    Dot,

    // Operators
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `=`
    Eq,
    /// `=>`
    Arrow,
    /// `==` (lexed from `===` too; strictness is a later concern)
    EqEq,
    /// `!=`
    BangEq,
    /// `<`
    Lt,
    /// `<=`
    LtEq,
    /// `>`
    Gt,
    /// `>=`
    GtEq,
    /// `&&`
    AndAnd,
    /// `||`
    OrOr,
    /// `!`
    Bang,

    /// An unrecognized character (already diagnosed).
    Error,
    /// End of input. Always the last token.
    Eof,
}

/// A token: a kind plus where it came from. Text is recovered by slicing the
/// source with the span, so tokens stay `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// What was lexed.
    pub kind: TokenKind,
    /// Where in the source it sits.
    pub span: Span,
}

/// Lex `source` into tokens, appending diagnostics for invalid input.
pub fn lex(source: &str, diagnostics: &mut Vec<Diagnostic>) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    // Skip a UTF-8 byte-order mark if the file starts with one, so editors that
    // add a BOM do not produce spurious errors on the first token.
    let mut pos = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };

    while pos < bytes.len() {
        let start = pos;
        let b = bytes[pos];
        let kind = match b {
            b' ' | b'\t' | b'\r' | b'\n' => {
                pos += 1;
                continue;
            }
            b'/' if bytes.get(pos + 1) == Some(&b'/') => {
                while pos < bytes.len() && bytes[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }
            b'0'..=b'9' => {
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
                if bytes.get(pos) == Some(&b'.')
                    && bytes.get(pos + 1).is_some_and(u8::is_ascii_digit)
                {
                    pos += 1;
                    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                        pos += 1;
                    }
                }
                TokenKind::Number
            }
            b'"' => {
                pos += 1;
                while pos < bytes.len() && bytes[pos] != b'"' && bytes[pos] != b'\n' {
                    // Skip the escaped character so an escaped quote does not
                    // terminate the literal.
                    pos += if bytes[pos] == b'\\' { 2 } else { 1 };
                }
                if bytes.get(pos) == Some(&b'"') {
                    pos += 1;
                    TokenKind::String
                } else {
                    diagnostics.push(
                        Diagnostic::error("unterminated string literal")
                            .with_code("SYN0001")
                            .with_span(Span::new(start as u32, pos as u32)),
                    );
                    TokenKind::Error
                }
            }
            b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'$' => {
                while pos < bytes.len()
                    && (bytes[pos].is_ascii_alphanumeric()
                        || bytes[pos] == b'_'
                        || bytes[pos] == b'$')
                {
                    pos += 1;
                }
                keyword_or_ident(&source[start..pos])
            }
            _ => {
                pos += 1;
                match b {
                    b'(' => TokenKind::LParen,
                    b')' => TokenKind::RParen,
                    b'{' => TokenKind::LBrace,
                    b'}' => TokenKind::RBrace,
                    b'[' => TokenKind::LBracket,
                    b']' => TokenKind::RBracket,
                    b',' => TokenKind::Comma,
                    b';' => TokenKind::Semi,
                    b':' => TokenKind::Colon,
                    b'?' => {
                        if bytes.get(pos) == Some(&b'?') {
                            pos += 1;
                            TokenKind::Nullish
                        } else if bytes.get(pos) == Some(&b'.') {
                            pos += 1;
                            TokenKind::QuestionDot
                        } else {
                            TokenKind::Question
                        }
                    }
                    b'.' => TokenKind::Dot,
                    b'+' => TokenKind::Plus,
                    b'-' => TokenKind::Minus,
                    b'*' => TokenKind::Star,
                    b'/' => TokenKind::Slash,
                    b'%' => TokenKind::Percent,
                    b'=' => {
                        if bytes.get(pos) == Some(&b'=') {
                            pos += 1;
                            // Accept `===` as equality for now.
                            if bytes.get(pos) == Some(&b'=') {
                                pos += 1;
                            }
                            TokenKind::EqEq
                        } else if bytes.get(pos) == Some(&b'>') {
                            pos += 1;
                            TokenKind::Arrow
                        } else {
                            TokenKind::Eq
                        }
                    }
                    b'!' => {
                        if bytes.get(pos) == Some(&b'=') {
                            pos += 1;
                            if bytes.get(pos) == Some(&b'=') {
                                pos += 1;
                            }
                            TokenKind::BangEq
                        } else {
                            TokenKind::Bang
                        }
                    }
                    b'&' if bytes.get(pos) == Some(&b'&') => {
                        pos += 1;
                        TokenKind::AndAnd
                    }
                    b'|' if bytes.get(pos) == Some(&b'|') => {
                        pos += 1;
                        TokenKind::OrOr
                    }
                    b'<' => {
                        if bytes.get(pos) == Some(&b'=') {
                            pos += 1;
                            TokenKind::LtEq
                        } else {
                            TokenKind::Lt
                        }
                    }
                    b'>' => {
                        if bytes.get(pos) == Some(&b'=') {
                            pos += 1;
                            TokenKind::GtEq
                        } else {
                            TokenKind::Gt
                        }
                    }
                    _ => {
                        diagnostics.push(
                            Diagnostic::error(format!("unexpected character {:?}", b as char))
                                .with_code("SYN0002")
                                .with_span(Span::new(start as u32, pos as u32)),
                        );
                        TokenKind::Error
                    }
                }
            }
        };
        tokens.push(Token {
            kind,
            span: Span::new(start as u32, pos as u32),
        });
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::empty_at(bytes.len() as u32),
    });
    tokens
}

fn keyword_or_ident(text: &str) -> TokenKind {
    match text {
        "function" => TokenKind::KwFunction,
        "let" => TokenKind::KwLet,
        "if" => TokenKind::KwIf,
        "else" => TokenKind::KwElse,
        "switch" => TokenKind::KwSwitch,
        "case" => TokenKind::KwCase,
        "default" => TokenKind::KwDefault,
        "while" => TokenKind::KwWhile,
        "do" => TokenKind::KwDo,
        "for" => TokenKind::KwFor,
        "of" => TokenKind::KwOf,
        "break" => TokenKind::KwBreak,
        "continue" => TokenKind::KwContinue,
        "return" => TokenKind::KwReturn,
        "throw" => TokenKind::KwThrow,
        "try" => TokenKind::KwTry,
        "catch" => TokenKind::KwCatch,
        "finally" => TokenKind::KwFinally,
        "true" => TokenKind::KwTrue,
        "false" => TokenKind::KwFalse,
        "null" => TokenKind::KwNull,
        "class" => TokenKind::KwClass,
        "new" => TokenKind::KwNew,
        "extends" => TokenKind::KwExtends,
        "as" => TokenKind::KwAs,
        "import" => TokenKind::KwImport,
        "from" => TokenKind::KwFrom,
        "extern" => TokenKind::KwExtern,
        _ => TokenKind::Ident,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let mut diags = Vec::new();
        let toks = lex(src, &mut diags);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        toks.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_function_header() {
        use TokenKind::*;
        assert_eq!(
            kinds("function add(a: number) {}"),
            vec![KwFunction, Ident, LParen, Ident, Colon, Ident, RParen, LBrace, RBrace, Eof]
        );
    }

    #[test]
    fn numbers_and_decimals() {
        use TokenKind::*;
        assert_eq!(kinds("42 3.14"), vec![Number, Number, Eof]);
    }

    #[test]
    fn strings_and_escapes() {
        use TokenKind::*;
        assert_eq!(kinds(r#""hi" "a\"b""#), vec![String, String, Eof]);
    }

    #[test]
    fn comments_are_skipped() {
        use TokenKind::*;
        assert_eq!(kinds("1 // two\n3"), vec![Number, Number, Eof]);
    }

    #[test]
    fn multi_char_operators() {
        use TokenKind::*;
        assert_eq!(
            kinds("== === != <= >= = => < >"),
            vec![EqEq, EqEq, BangEq, LtEq, GtEq, Eq, Arrow, Lt, Gt, Eof]
        );
    }

    #[test]
    fn nullish_and_optional_chaining_are_distinct_tokens() {
        use TokenKind::*;
        assert_eq!(
            kinds("value ?? fallback; value?.field"),
            vec![Ident, Nullish, Ident, Semi, Ident, QuestionDot, Ident, Eof]
        );
    }

    #[test]
    fn unterminated_string_diagnoses() {
        let mut diags = Vec::new();
        let toks = lex("\"oops", &mut diags);
        assert_eq!(toks[0].kind, TokenKind::Error);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "SYN0001");
    }

    #[test]
    fn leading_bom_is_skipped() {
        use TokenKind::*;
        assert_eq!(
            kinds("\u{FEFF}let x = 1;"),
            vec![KwLet, Ident, Eq, Number, Semi, Eof]
        );
    }

    #[test]
    fn spans_slice_source() {
        let mut diags = Vec::new();
        let src = "let x = 10;";
        let toks = lex(src, &mut diags);
        let x = toks[1];
        assert_eq!(&src[x.span.start as usize..x.span.end as usize], "x");
    }
}
