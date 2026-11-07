use super::*;

/// Convert a JS string literal to a Rust `String`. Lone surrogates (which Rust
/// strings cannot hold) are mapped to the plane-15 private-use block
/// `U+F0000 + (unit - 0xD800)`; the runtime maps them back when it needs the
/// UTF-16 view of a string.
pub(super) fn js_str_to_string(value: oxc_str::JSStr<'_>) -> String {
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    value
        .chars()
        .map(|c| match c.to_char() {
            Some(ch) => ch,
            None => char::from_u32(0xF0000 + (c.to_u32() - 0xD800)).unwrap(),
        })
        .collect()
}

pub(super) fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Whether `text` mentions `word` as a whole identifier.
pub(super) fn contains_word(text: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(found) = text[from..].find(word) {
        let start = from + found;
        let end = start + word.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if !before.map(is_identifier_char).unwrap_or(false)
            && !after.map(is_identifier_char).unwrap_or(false)
        {
            return true;
        }
        from = end;
    }
    false
}

/// Whether `name` appears as the target of a plain `=` assignment anywhere in
/// `source` (a textual approximation; only used for function declarations).
pub(super) fn name_is_reassigned(source: &str, name: &str) -> bool {
    let bytes = source.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut from = 0;
    while let Some(found) = source[from..].find(name) {
        let start = from + found;
        let end = start + name.len();
        from = end;
        if start > 0 && (is_word(bytes[start - 1]) || bytes[start - 1] == b'.') {
            continue;
        }
        if end < bytes.len() && is_word(bytes[end]) {
            continue;
        }
        let mut cursor = end;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        // `(name as any) = v`: skip the cast and the closing parenthesis.
        if bytes[cursor..].starts_with(b"as ")
            && start > 0
            && bytes[..start].trim_ascii_end().ends_with(b"(")
        {
            if let Some(close) = source[cursor..].find(')') {
                cursor += close + 1;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
            }
        }
        if cursor < bytes.len()
            && bytes[cursor] == b'='
            && !matches!(bytes.get(cursor + 1), Some(b'=') | Some(b'>'))
        {
            let before = source[..start].trim_end();
            if before.ends_with("let") || before.ends_with("const") || before.ends_with("var") {
                continue;
            }
            return true;
        }
    }
    false
}

/// Whether `text` is a plain ASCII identifier (usable as a method name).
pub(super) fn is_plain_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// True when `expression` (through parentheses / `as` casts) is a bare
/// capitalized identifier such as `Map` or `Object`.
pub(super) fn peels_to_global_namespace(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::Identifier(id) => id
            .name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase()),
        Expression::ParenthesizedExpression(inner) => peels_to_global_namespace(&inner.expression),
        Expression::TSAsExpression(inner) => peels_to_global_namespace(&inner.expression),
        _ => false,
    }
}

pub(super) fn is_immutable_self_name(name: &str) -> bool {
    IMMUTABLE_SELF_NAMES.with(|names| names.borrow().iter().any(|n| n == name))
}

/// An assignment to an immutable binding: evaluates the value, then throws
/// `TypeError: Assignment to constant variable.`
pub(super) fn throw_const_assignment(value: Expr, whole: Span) -> Expr {
    let message = Expr::String {
        value: "Assignment to constant variable.".to_owned(),
        span: whole,
    };
    let statements = vec![
        Stmt::Expr(value),
        Stmt::Throw {
            value: Expr::New {
                class: crate::ast::synthetic_ident("TypeError"),
                args: vec![message],
                span: whole,
            },
            span: whole,
        },
    ];
    let iife = Expr::Function(FunctionExpr {
        params: Vec::new(),
        return_type: None,
        body: Block {
            statements,
            span: whole,
        },
        is_arrow: true,
        is_async: false,
        is_generator: false,
        span: whole,
    });
    Expr::ComputedCall {
        callee: Box::new(iife),
        args: Vec::new(),
        span: whole,
    }
}

pub(super) fn peel_parens<'a, 'b>(expression: &'a Expression<'b>) -> &'a Expression<'b> {
    let mut current = expression;
    while let Expression::ParenthesizedExpression(inner) = current {
        current = &inner.expression;
    }
    current
}
