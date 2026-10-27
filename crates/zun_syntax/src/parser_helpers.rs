//! Small parser lookahead helpers extracted from the parser core.

use super::*;

pub(crate) fn decode_string(raw: &str) -> String {
    let inner: Vec<char> = raw[1..raw.len() - 1].chars().collect();
    let mut out = String::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        let c = inner[i];
        i += 1;
        if c != '\\' {
            out.push(c);
            continue;
        }
        if i >= inner.len() {
            break;
        }
        let escape = inner[i];
        i += 1;
        match escape {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'b' => out.push('\u{08}'),
            'f' => out.push('\u{0C}'),
            'v' => out.push('\u{0B}'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            '`' => out.push('`'),
            // `\xHH` — exactly two hex digits.
            'x' => match hex_escape(&inner[i..], 2) {
                Some((ch, n)) => { out.push(ch); i += n; }
                None => out.push('x'),
            },
            // `\uHHHH`, or `\u{H...}` with a braced code point.
            'u' => {
                if inner.get(i) == Some(&'{') {
                    match inner[i + 1..].iter().position(|&h| h == '}') {
                        Some(end) => {
                            let hex: String = inner[i + 1..i + 1 + end].iter().collect();
                            if let Some(ch) =
                                u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                            {
                                out.push(ch);
                            }
                            i += end + 2; // consume `{ … }`
                        }
                        None => out.push('u'),
                    }
                } else {
                    match hex_escape(&inner[i..], 4) {
                        Some((ch, n)) => { out.push(ch); i += n; }
                        None => out.push('u'),
                    }
                }
            }
            // Any other escape is the character itself (JS semantics), e.g. `\y`
            // is `y`.
            other => out.push(other),
        }
    }
    out
}

/// Decode exactly `n` hex digits at the start of `rest` into a `char`, returning
/// it with the count consumed. `None` if there are not `n` hex digits or the
/// value is not a scalar value.
fn hex_escape(rest: &[char], n: usize) -> Option<(char, usize)> {
    if rest.len() < n || !rest[..n].iter().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let hex: String = rest[..n].iter().collect();
    let code = u32::from_str_radix(&hex, 16).ok()?;
    Some((char::from_u32(code)?, n))
}

pub(crate) fn rebase_expression_names(expr: &mut Expr, base: u32) {
    let shift = |span: &mut Span| *span = Span::new(span.start + base, span.end + base);
    match expr {
        Expr::Ident(name) => shift(&mut name.span),
        Expr::Assign { target, value, .. } => { shift(&mut target.span); rebase_expression_names(value, base); }
        Expr::Binary { lhs, rhs, .. } => { rebase_expression_names(lhs, base); rebase_expression_names(rhs, base); }
        Expr::Conditional { condition, when_true, when_false, .. } => { rebase_expression_names(condition, base); rebase_expression_names(when_true, base); rebase_expression_names(when_false, base); }
        Expr::Unary { operand, .. } | Expr::Cast { expr: operand, .. } => rebase_expression_names(operand, base),
        Expr::Await { operand, .. } => rebase_expression_names(operand, base),
        Expr::ImportCall { specifier, .. } => rebase_expression_names(specifier, base),
        Expr::Yield { value, .. } => rebase_expression_names(value, base),
        Expr::Call { callee, args, .. } => { shift(&mut callee.span); for arg in args { rebase_expression_names(arg, base); } }
        Expr::ComputedCall { callee, args, .. } => { rebase_expression_names(callee, base); for arg in args { rebase_expression_names(arg, base); } }
        Expr::MethodCall { receiver, method, args, .. } | Expr::OptionalMethodCall { object: receiver, method, args, .. } => { rebase_expression_names(receiver, base); shift(&mut method.span); for arg in args { rebase_expression_names(arg, base); } }
        Expr::StaticMethodCall { class, method, args, .. } => { shift(&mut class.span); shift(&mut method.span); for arg in args { rebase_expression_names(arg, base); } }
        Expr::StaticField { class, field, .. } => { shift(&mut class.span); shift(&mut field.span); }
        Expr::Spread { expr, .. } => rebase_expression_names(expr, base),
        Expr::OptionalField { object, field, .. } | Expr::Field { object, field, .. } | Expr::PrivateField { object, field, .. } => { rebase_expression_names(object, base); shift(&mut field.span); }
        Expr::OptionalIndex { object, index, .. } => { rebase_expression_names(object, base); rebase_expression_names(index, base); }
        Expr::OptionalCall { callee, args, .. } => { rebase_expression_names(callee, base); for arg in args { rebase_expression_names(arg, base); } }
        Expr::New { class, args, .. } => { shift(&mut class.span); for arg in args { rebase_expression_names(arg, base); } }
        Expr::NewDynamic { callee, args, .. } => { rebase_expression_names(callee, base); for arg in args { rebase_expression_names(arg, base); } }
        Expr::FieldAssign { object, field, value, .. } | Expr::PrivateFieldAssign { object, field, value, .. } => { rebase_expression_names(object, base); shift(&mut field.span); rebase_expression_names(value, base); }
        Expr::Index { object, index, .. } => { rebase_expression_names(object, base); rebase_expression_names(index, base); }
        Expr::IndexAssign { object, index, value, .. } => { rebase_expression_names(object, base); rebase_expression_names(index, base); rebase_expression_names(value, base); }
        Expr::ObjectLiteral { properties, .. } => {
            for member in properties {
                match member {
                    ObjectMember::Property(property) => {
                        match &mut property.key {
                            PropertyKey::Ident(name) => shift(&mut name.span),
                            PropertyKey::String { span, .. } | PropertyKey::Number { span, .. } => shift(span),
                            PropertyKey::Computed(key) => rebase_expression_names(key, base),
                        }
                        rebase_expression_names(&mut property.value, base);
                    }
                    ObjectMember::Spread { expr, .. } => rebase_expression_names(expr, base),
                }
            }
        }
        Expr::ArrayLiteral { elements, .. } => {
            for element in elements {
                match element {
                    ArrayElement::Expr(expr) | ArrayElement::Spread(expr) => rebase_expression_names(expr, base),
                    ArrayElement::Hole => {}
                }
            }
        }
        Expr::TaggedTemplate { tag, exprs, .. } => { rebase_expression_names(tag, base); for expr in exprs { rebase_expression_names(expr, base); } }
        Expr::Function(_) | Expr::Number { .. } | Expr::BigInt { .. } | Expr::String { .. } | Expr::Template { .. } | Expr::RegexLiteral { .. } | Expr::Bool { .. } | Expr::Null { .. } => {}
    }
}

pub(crate) fn parse_template_parts(raw: &str, span: Span, diagnostics: &mut Vec<Diagnostic>) -> Vec<crate::ast::TemplatePart> {
    let inner = &raw[1..raw.len() - 1];
    let mut parts = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = find_template_marker(&inner[cursor..]) {
        let start = cursor + relative;
        if start > cursor { parts.push(crate::ast::TemplatePart::Text(decode_template_text(&inner[cursor..start]))); }
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
        if !expression_diagnostics.is_empty() { diagnostics.extend(expression_diagnostics); }
        else {
            let offset = expression.as_ptr() as usize - raw.as_ptr() as usize;
            rebase_expression_names(&mut parsed, span.start + offset as u32);
            parts.push(crate::ast::TemplatePart::Expr(Box::new(parsed)));
        }
        cursor = end + 1;
    }
    if cursor < inner.len() { parts.push(crate::ast::TemplatePart::Text(decode_template_text(&inner[cursor..]))); }
    parts
}

/// Parse a tagged-template literal body into aligned `(cooked, raw)` literal
/// chunks and the interpolated expressions. `cooked.len() == raw.len() ==
/// exprs.len() + 1`: there is always a (possibly empty) chunk before the first
/// substitution, between each pair, and after the last. `raw` chunks keep the
/// source text as written (no escape decoding); `cooked` chunks are decoded.
#[allow(clippy::type_complexity)]
pub(crate) fn parse_tagged_template_parts(
    raw: &str,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<String>, Vec<String>, Vec<Box<Expr>>) {
    let inner = &raw[1..raw.len() - 1];
    let mut cooked = Vec::new();
    let mut raws = Vec::new();
    let mut exprs = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = find_template_marker(&inner[cursor..]) {
        let start = cursor + relative;
        let chunk = &inner[cursor..start];
        cooked.push(decode_template_text(chunk));
        raws.push(chunk.to_string());
        let expression_start = start + 2;
        let Some(end) = find_template_interpolation_end(inner, expression_start) else {
            diagnostics.push(
                Diagnostic::error("unterminated template interpolation")
                    .with_code("SYN0004")
                    .with_span(span),
            );
            return (cooked, raws, exprs);
        };
        let expression = inner[expression_start..end].trim();
        let mut expression_diagnostics = Vec::new();
        match parse_expression_fragment(expression, &mut expression_diagnostics) {
            Some(mut parsed) if expression_diagnostics.is_empty() => {
                let offset = expression.as_ptr() as usize - raw.as_ptr() as usize;
                rebase_expression_names(&mut parsed, span.start + offset as u32);
                exprs.push(Box::new(parsed));
            }
            _ => {
                diagnostics.extend(expression_diagnostics);
                diagnostics.push(
                    Diagnostic::error("invalid template interpolation")
                        .with_code("SYN0005")
                        .with_span(span),
                );
                // Keep chunk/expr alignment: emit a null placeholder so the
                // chunk just pushed still has a trailing substitution.
                exprs.push(Box::new(Expr::Null { span }));
            }
        }
        cursor = end + 1;
    }
    let tail = &inner[cursor..];
    cooked.push(decode_template_text(tail));
    raws.push(tail.to_string());
    (cooked, raws, exprs)
}

pub(crate) fn parse_expression_fragment(source: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<Expr> {
    let tokens = lex(source, diagnostics);
    let mut parser = Parser { source, tokens, pos: 0, diagnostics };
    let expression = parser.expression();
    if parser.at(TokenKind::Eof) && diagnostics.is_empty() { Some(expression) } else { None }
}

pub(crate) fn find_template_marker(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\\' { index += 2; continue; }
        if bytes[index] == b'$' && bytes[index + 1] == b'{' { return Some(index); }
        index += 1;
    }
    None
}

pub(crate) fn find_template_interpolation_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' { index += 2; continue; }
        if let Some(current) = quote {
            if byte == current { quote = None; }
            index += 1;
            continue;
        }
        if matches!(byte, b'"' | b'\'') { quote = Some(byte); }
        else if byte == b'{' { depth += 1; }
        else if byte == b'}' {
            if depth == 0 { return Some(index); }
            depth -= 1;
        }
        index += 1;
    }
    None
}

pub(crate) fn decode_template_text(text: &str) -> String { decode_string(&format!("\"{text}\"")) }

/// Map a token to its binary operator and precedence level (higher binds
/// tighter). The order mirrors JS: `|| ?? < && < | < ^ < & < == < relational <
/// shift < additive < multiplicative < **`.
fn binary_op(kind: TokenKind) -> Option<(BinaryOp, u8)> {
    Some(match kind {
        TokenKind::OrOr => (BinaryOp::Or, 1),
        TokenKind::Nullish => (BinaryOp::Nullish, 1),
        TokenKind::AndAnd => (BinaryOp::And, 2),
        TokenKind::Pipe => (BinaryOp::BitOr, 3),
        TokenKind::Caret => (BinaryOp::BitXor, 4),
        TokenKind::Amp => (BinaryOp::BitAnd, 5),
        TokenKind::EqEq => (BinaryOp::Eq, 6),
        TokenKind::BangEq => (BinaryOp::Ne, 6),
        TokenKind::Lt => (BinaryOp::Lt, 7),
        TokenKind::LtEq => (BinaryOp::Le, 7),
        TokenKind::Gt => (BinaryOp::Gt, 7),
        TokenKind::GtEq => (BinaryOp::Ge, 7),
        TokenKind::KwIn => (BinaryOp::In, 7),
        TokenKind::KwInstanceof => (BinaryOp::Instanceof, 7),
        TokenKind::Shl => (BinaryOp::Shl, 8),
        TokenKind::Shr => (BinaryOp::Shr, 8),
        TokenKind::UShr => (BinaryOp::UShr, 8),
        TokenKind::Plus => (BinaryOp::Add, 9),
        TokenKind::Minus => (BinaryOp::Sub, 9),
        TokenKind::Star => (BinaryOp::Mul, 10),
        TokenKind::Slash => (BinaryOp::Div, 10),
        TokenKind::Percent => (BinaryOp::Rem, 10),
        TokenKind::StarStar => (BinaryOp::Pow, 11),
        _ => return None,
    })
}

/// Map a compound-assignment token to the binary operator it applies.
fn compound_assign_op(kind: TokenKind) -> Option<BinaryOp> {
    Some(match kind {
        TokenKind::PlusEq => BinaryOp::Add,
        TokenKind::MinusEq => BinaryOp::Sub,
        TokenKind::StarEq => BinaryOp::Mul,
        TokenKind::SlashEq => BinaryOp::Div,
        TokenKind::PercentEq => BinaryOp::Rem,
        TokenKind::StarStarEq => BinaryOp::Pow,
        TokenKind::AmpEq => BinaryOp::BitAnd,
        TokenKind::PipeEq => BinaryOp::BitOr,
        TokenKind::CaretEq => BinaryOp::BitXor,
        TokenKind::ShlEq => BinaryOp::Shl,
        TokenKind::ShrEq => BinaryOp::Shr,
        TokenKind::UShrEq => BinaryOp::UShr,
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
            TokenKind::BigInt => {
                self.bump();
                let text = self.text(t);
                Expr::BigInt { value: text[..text.len() - 1].to_owned(), span: t.span }
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
            TokenKind::Regex => {
                self.bump();
                let raw = self.text(t);
                // The lexer guarantees `/body/flags` with a closing slash.
                let close = raw.rfind('/').unwrap_or(0);
                Expr::RegexLiteral {
                    pattern: raw[1..close].to_owned(),
                    flags: raw[close + 1..].to_owned(),
                    span: t.span,
                }
            }
            TokenKind::KwTrue => { self.bump(); Expr::Bool { value: true, span: t.span } }
            TokenKind::KwFalse => { self.bump(); Expr::Bool { value: false, span: t.span } }
            TokenKind::KwNull => { self.bump(); Expr::Null { span: t.span } }
            TokenKind::KwNew => {
                let start = self.bump().span;
                if self.at(TokenKind::LParen) {
                    // `new (expr)(args)`: a parenthesized callee expression.
                    self.bump();
                    let callee = self.expression();
                    self.expect(TokenKind::RParen, "`)`");
                    self.expect(TokenKind::LParen, "`(`");
                    let args = self.arguments();
                    let close = self.tokens[self.pos - 1].span;
                    return Expr::NewDynamic { callee: Box::new(callee), args, span: start.to(close) };
                }
                let class = self.dotted_name("a class name");
                self.expect(TokenKind::LParen, "`(`");
                let args = self.arguments();
                let close = self.tokens[self.pos - 1].span;
                Expr::New { class, args, span: start.to(close) }
            }
            TokenKind::KwFunction => self.function_expression(false),
            TokenKind::KwAsync => {
                let start = self.bump().span;
                if self.at(TokenKind::KwFunction) {
                    return self.function_expression(true);
                }
                let params = if self.at(TokenKind::Ident) {
                    let name = self.name("an async arrow parameter");
                    vec![Param { name, ty: None, default_value: None, is_rest: false }]
                } else {
                    self.expect(TokenKind::LParen, "`(` after async");
                    let params = self.parameters();
                    self.expect(TokenKind::RParen, "`)` after async arrow parameters");
                    params
                };
                self.expect(TokenKind::Arrow, "`=>` after async arrow parameters");
                let body = self.arrow_body();
                Expr::Function(FunctionExpr {
                    params,
                    return_type: None,
                    span: start.to(body.span),
                    body,
                    is_arrow: true,
                    is_async: true,
                    is_generator: false,
                })
            }
            TokenKind::KwImport => {
                let start = self.bump().span;
                self.expect(TokenKind::LParen, "`(` after import");
                let specifier = self.expression();
                let end = self.expect(TokenKind::RParen, "`)` after import specifier").map_or(specifier.span(), |token| token.span);
                Expr::ImportCall { span: start.to(end), specifier: Box::new(specifier) }
            }
            TokenKind::KwAwait => {
                let start = self.bump().span;
                let operand = self.expression();
                Expr::Await { span: start.to(operand.span()), operand: Box::new(operand) }
            }
            TokenKind::KwYield => {
                let start = self.bump().span;
                let value = self.expression();
                Expr::Yield { span: start.to(value.span()), value: Box::new(value) }
            }
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
                        is_async: false,
                        is_generator: false,
                    });
                }
                Expr::Ident(name)
            }
            TokenKind::LBrace => self.object_literal(),
            TokenKind::LBracket => self.array_literal(),
            TokenKind::LParen => {
                if self.parenthesized_arrow_start() {
                    let start = self.bump().span;
                    let params = self.parameters();
                    self.expect(TokenKind::RParen, "`)` after arrow parameters");
                    self.expect(TokenKind::Arrow, "`=>` after arrow parameters");
                    let body = self.arrow_body();
                    return Expr::Function(FunctionExpr { params, return_type: None, span: start.to(body.span), body, is_arrow: true, is_async: false, is_generator: false });
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

    /// Parse an object literal `{ key: value, ... }` in expression position.
    ///
    /// The leading `{` has not been consumed. Statement-position `{` never
    /// reaches here: statements with no leading keyword fall through to
    /// expression parsing, and this grammar has no bare block statement, so the
    /// block-vs-object ambiguity does not arise in the hand parser.
    pub(crate) fn object_literal(&mut self) -> Expr {
        let start = self.bump().span; // `{`
        let mut properties = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            // Spread member `...expr`.
            if self.at(TokenKind::Ellipsis) {
                let spread_start = self.bump().span;
                let expr = self.assignment();
                let span = spread_start.to(expr.span());
                properties.push(ObjectMember::Spread { expr, span });
                if !self.eat(TokenKind::Comma) {
                    break;
                }
                continue;
            }
            // A leading `get`/`set` identifier introduces an accessor only when
            // it is followed by a property name (not `:` / `(` / `,` / `}`,
            // which would make `get`/`set` itself the property name).
            let mut accessor: Option<PropertyKind> = None;
            if self.at(TokenKind::Ident) {
                let text = self.text(self.peek());
                if (text == "get" || text == "set") && self.accessor_follows() {
                    accessor = Some(if text == "get" { PropertyKind::Get } else { PropertyKind::Set });
                    self.bump(); // consume `get`/`set`
                }
            }
            let key = if self.eat(TokenKind::LBracket) {
                let key = self.assignment();
                self.expect(TokenKind::RBracket, "`]` after computed property key");
                PropertyKey::Computed(Box::new(key))
            } else {
                match self.peek().kind {
                    TokenKind::String => {
                        let token = self.bump();
                        PropertyKey::String {
                            value: decode_string(self.text(token)),
                            span: token.span,
                        }
                    }
                    TokenKind::Number => {
                        let token = self.bump();
                        let value = self.text(token).parse::<f64>().unwrap_or(f64::NAN);
                        PropertyKey::Number { value, span: token.span }
                    }
                    kind if kind == TokenKind::Ident || kind.is_keyword() => {
                        let token = self.bump();
                        PropertyKey::Ident(Name { span: token.span })
                    }
                    _ => {
                        self.error_at(self.peek().span, "expected a property name");
                        self.bump();
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                        continue;
                    }
                }
            };
            // A getter/setter, or a method shorthand `name(...) {...}`, parse
            // their function form from the `(`.
            if let Some(kind) = accessor {
                let value = self.object_method_body();
                properties.push(ObjectMember::Property(ObjectProperty { key, value, kind }));
            } else if self.at(TokenKind::LParen) {
                let value = self.object_method_body();
                properties.push(ObjectMember::Property(ObjectProperty {
                    key,
                    value,
                    kind: PropertyKind::Method,
                }));
            } else {
                let value = if self.eat(TokenKind::Colon) {
                    self.assignment()
                } else if let PropertyKey::Ident(name) = &key {
                    // Shorthand `{ x }` desugars to `{ x: x }`.
                    Expr::Ident(*name)
                } else {
                    self.error_at(self.peek().span, "`:` after property name");
                    Expr::Null { span: self.peek().span }
                };
                properties.push(ObjectMember::Property(ObjectProperty {
                    key,
                    value,
                    kind: PropertyKind::Init,
                }));
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.peek().span;
        self.expect(TokenKind::RBrace, "`}` after object literal");
        Expr::ObjectLiteral {
            properties,
            span: start.to(close),
        }
    }

    /// Whether the token after a leading `get`/`set` begins a property name
    /// (so `get`/`set` is an accessor keyword rather than the property itself).
    fn accessor_follows(&self) -> bool {
        match self.peek_n(1).kind {
            TokenKind::Ident | TokenKind::String | TokenKind::Number | TokenKind::LBracket => true,
            kind => kind.is_keyword(),
        }
    }

    /// Parse the `(params) [: ret] { body }` of an object method, getter, or
    /// setter (the key has already been consumed), yielding a function value.
    fn object_method_body(&mut self) -> Expr {
        let start = self.peek().span;
        self.expect(TokenKind::LParen, "`(` after method name");
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
            is_async: false,
            is_generator: false,
        })
    }

    /// Parse an array literal `[ e1, e2, ... ]` in expression position.
    ///
    /// The leading `[` has not been consumed. A single trailing comma is
    /// accepted and `[]` is a valid empty literal. Elision/holes (`[1, , 3]`)
    /// and spread (`[...xs]`) are deferred: a bare `,` or a `...` element is
    /// reported as unsupported rather than silently miscompiled.
    pub(crate) fn array_literal(&mut self) -> Expr {
        let start = self.bump().span; // `[`
        let mut elements = Vec::new();
        while !self.at(TokenKind::RBracket) && !self.at(TokenKind::Eof) {
            if self.at(TokenKind::Comma) {
                // A bare `,` is an elision/hole: count the index, consume the
                // comma, and continue (a trailing hole before `]` is dropped by
                // ECMAScript, matching how a trailing comma adds no element).
                elements.push(ArrayElement::Hole);
                self.bump();
                continue;
            }
            if self.at(TokenKind::Ellipsis) {
                self.bump();
                elements.push(ArrayElement::Spread(self.assignment()));
            } else {
                elements.push(ArrayElement::Expr(self.assignment()));
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        let close = self.peek().span;
        self.expect(TokenKind::RBracket, "`]` after array literal");
        Expr::ArrayLiteral {
            elements,
            span: start.to(close),
        }
    }

    pub(crate) fn function_expression(&mut self, is_async: bool) -> Expr {
        let start = self.bump().span;
        let is_generator = self.eat(TokenKind::Star);
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
            is_async,
            is_generator,
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
                TokenKind::Hash => {
                    self.bump();
                    let field = self.name("a private field name");
                    if self.at(TokenKind::LParen) {
                        self.error_at(field.span, "private methods are not supported");
                    }
                    let span = expr.span().to(field.span);
                    expr = Expr::PrivateField { object: Box::new(expr), field, span };
                }
                TokenKind::Dot => {
                    self.bump();
                    if self.eat(TokenKind::Hash) {
                        let field = self.name("a private field name");
                        let span = expr.span().to(field.span);
                        expr = Expr::PrivateField { object: Box::new(expr), field, span };
                        continue;
                    }
                    let member = self.member_name("a field or method name");
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
                    let field = self.member_name("an optional field or method name");
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
                TokenKind::Template => {
                    // Tagged template: `tag`...``. The template token directly
                    // follows the tag expression with no intervening operator.
                    let t = self.bump();
                    let raw = self.text(t).to_string();
                    let (cooked, raw_chunks, exprs) =
                        parse_tagged_template_parts(&raw, t.span, &mut self.diagnostics);
                    expr = Expr::TaggedTemplate {
                        span: expr.span().to(t.span),
                        tag: Box::new(expr),
                        cooked,
                        raw: raw_chunks,
                        exprs,
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
            let ty = self.type_annotation("a type name");
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
            TokenKind::Tilde => Some(UnaryOp::BitNot),
            TokenKind::KwVoid => Some(UnaryOp::Void),
            TokenKind::KwTypeof => Some(UnaryOp::Typeof),
            TokenKind::KwDelete => Some(UnaryOp::Delete),
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
            // `**` is right-associative: recurse at the same level so
            // `2 ** 3 ** 2` parses as `2 ** (3 ** 2)`.
            let rhs = if op == BinaryOp::Pow {
                self.binary(level)
            } else {
                self.binary(level + 1)
            };
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
        let compound = compound_assign_op(self.peek().kind);
        if self.at(TokenKind::Eq) || compound.is_some() {
            let eq_span = self.bump().span;
            let mut value = self.assignment();
            if let Some(op) = compound {
                // `a OP= b` desugars to `a = (a OP b)`.
                // Known v1 limitation: the target is cloned as the left operand,
                // so a field/index target whose object or index sub-expression
                // has side effects (`f().x += 1`, `arr[g()] += 1`) evaluates
                // that sub-expression twice. Ident, `this.field` and `obj[i]`
                // (ident object and index) forms are exact.
                let span = lhs.span().to(value.span());
                value = Expr::Binary {
                    op,
                    lhs: Box::new(lhs.clone()),
                    rhs: Box::new(value),
                    span,
                };
            }
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
                    return Expr::FieldAssign { object, field, value: Box::new(value), span };
                }
                Expr::PrivateField { object, field, .. } => {
                    let span = object.span().to(value.span());
                    let private_span = object.span().to(field.span);
                    let private = Expr::PrivateField { object, field, span: private_span };
                    return Expr::FieldAssign { object: Box::new(private), field, value: Box::new(value), span };
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
            let ty = self.type_annotation("a catch type");
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
            // `x?: T` marks the parameter optional. With no explicit default it
            // is sugar for `x: T = null` (the omitted-argument sentinel), reusing
            // the default-parameter machinery.
            let optional = self.eat(TokenKind::Question);
            let mut ty = self.optional_type_annotation();
            let default_value = if self.eat(TokenKind::Eq) {
                Some(Box::new(self.expression()))
            } else if optional {
                // `x?: T` with no explicit default. Zun has no union types, so
                // the parameter is `any` (it accepts the value or `null`),
                // defaulting to `null` when the argument is omitted. The empty
                // span resolves to `Ty::Any` in sema.
                ty = Some(TypeAnnotation { name: Name { span: Span::empty_at(name.span.start) }, is_array: false });
                Some(Box::new(Expr::Null { span: name.span }))
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

    /// Parse a possibly dotted constructor/type name (`A` or `A.B`). A dotted
    /// name becomes a single `Name` whose span covers the whole `A.B` text; only
    /// the `Intl` namespace shim in sema gives such names a meaning. Whitespace
    /// around the dot is not supported (the span must be contiguous source).
    pub(crate) fn dotted_name(&mut self, what: &str) -> Name {
        let mut name = self.name(what);
        while self.at(TokenKind::Dot)
            && self.peek_n(1).kind == TokenKind::Ident
            && self.peek_n(1).span.start == self.peek().span.end
            && self.peek().span.start == name.span.end
        {
            self.bump();
            let member = self.bump();
            name.span = name.span.to(member.span);
        }
        name
    }

    /// Parse a type name. Type positions accept `void` (a keyword since it is
    /// also the unary `void` operator) as well as ordinary identifiers.
    pub(crate) fn type_name(&mut self, what: &str) -> Name {
        if self.at(TokenKind::KwVoid) {
            let t = self.bump();
            Name { span: t.span }
        } else {
            self.dotted_name(what)
        }
    }

    pub(crate) fn name(&mut self, what: &str) -> Name {
        match self.expect(TokenKind::Ident, what) {
            Some(t) => Name { span: t.span },
            None => Name {
                span: Span::empty_at(self.peek().span.start),
            },
        }
    }

    /// Parse a member name after `.`/`?.`. Unlike [`name`](Self::name), this
    /// accepts reserved keywords (`Symbol.for`, `obj.class`, ...), matching the
    /// ECMAScript rule that keywords are valid property names.
    pub(crate) fn member_name(&mut self, what: &str) -> Name {
        let token = self.peek();
        if token.kind == TokenKind::Ident || token.kind.is_keyword() {
            self.bump();
            Name { span: token.span }
        } else {
            self.name(what)
        }
    }

    /// Parse a type name plus any trailing `[]` array suffixes (`T`, `T[]`,
    /// `T[][]`). Zun arrays are untyped, so any depth erases to `Array`.
    pub(crate) fn type_annotation(&mut self, what: &str) -> TypeAnnotation {
        let name = self.type_name(what);
        let mut is_array = false;
        while self.at(TokenKind::LBracket) {
            self.bump();
            self.expect(TokenKind::RBracket, "`]` to close an array type");
            is_array = true;
        }
        TypeAnnotation { name, is_array }
    }

    pub(crate) fn optional_type_annotation(&mut self) -> Option<TypeAnnotation> {
        if self.eat(TokenKind::Colon) {
            Some(self.type_annotation("a type name"))
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
