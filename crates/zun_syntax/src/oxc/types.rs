use super::*;

pub(super) fn convert_type_annotation(
    source: &str,
    annotation: Option<&oxc_ast::ast::TSTypeAnnotation<'_>>,
) -> Result<Option<crate::ast::TypeAnnotation>, String> {
    let Some(annotation) = annotation else {
        return Ok(None);
    };
    let start = annotation.span.start as usize;
    let end = annotation.span.end as usize;
    let raw = source
        .get(start..end)
        .ok_or_else(|| "OXC adapter received an invalid type span".to_owned())?;
    let type_start = raw
        .find(|character: char| !character.is_ascii_whitespace() && character != ':')
        .ok_or_else(|| "OXC adapter received an empty type annotation".to_owned())?;
    let mut type_text = raw[type_start..].trim();
    // `C | null` / `null | C`: a nullable reference. Zun keeps it a typed class
    // reference (null is the zero handle, as in the CLR); sema decides whether
    // the member really is a class and otherwise treats it as `any`.
    let mut nullable = false;
    if let Some(member) = nullable_union_member(type_text) {
        type_text = member;
        nullable = true;
    }
    // Strip trailing `[]` array suffixes; Zun arrays are untyped, so `T[]`
    // resolves to `Array` and the base name stays for diagnostics.
    let mut is_array = false;
    let mut array_depth = 0u8;
    while let Some(stripped) = type_text.strip_suffix("[]") {
        type_text = stripped.trim_end();
        is_array = true;
        array_depth = array_depth.saturating_add(1);
    }
    // Keep the base name of a generic (`Array<number>` -> `Array`); the type
    // arguments are erased like everything else.
    // A union / intersection at the top level (`Node<T> | null`) erases to `any`;
    // it must not be mistaken for its first member by the generic strip.
    {
        let mut depth = 0i32;
        let mut previous = ' ';
        for character in type_text.chars() {
            match character {
                '<' | '(' | '[' | '{' => depth += 1,
                '>' if previous == '=' => {}
                '>' | ')' | ']' | '}' => depth -= 1,
                '|' | '&' if depth == 0 => return Ok(None),
                _ => {}
            }
            previous = character;
        }
    }
    if let Some(generic) = type_text.find('<') {
        type_text = type_text[..generic].trim_end();
    }
    // Zun erases types (PLAN: typed-TS pragmatism). A plain named type keeps its
    // name (so `Array`/class receivers still resolve); anything the source
    // language cannot name here — a function type `(x) => y`, a union, a tuple,
    // an object or literal type — erases to an untyped (`any`) binding rather
    // than being rejected, matching JS/TS erasure semantics.
    if type_text.is_empty()
        || !type_text.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '$'
        })
    {
        return Ok(None);
    }
    // A name bound to a lowered class is a value, not a static type: it must
    // not resolve to a static class that shares the name elsewhere.
    if !is_array && super::runtime_class::names_runtime_class(type_text) {
        return Ok(None);
    }
    let name_start = start + type_start;
    Ok(Some(crate::ast::TypeAnnotation {
        name: Name {
            span: Span::new(name_start as u32, (name_start + type_text.len()) as u32),
        },
        is_array,
        array_depth,
        nullable,
    }))
}

/// The non-`null` member of a two-member top-level union `A | null` (either
/// order), or `None` for any other shape (those erase to `any`).
fn nullable_union_member(text: &str) -> Option<&str> {
    let mut depth = 0i32;
    let mut previous = ' ';
    let mut split = None;
    for (index, character) in text.char_indices() {
        match character {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' if previous == '=' => {}
            '>' | ')' | ']' | '}' => depth -= 1,
            '&' if depth == 0 => return None,
            '|' if depth == 0 => {
                if split.is_some() {
                    return None;
                }
                split = Some(index);
            }
            _ => {}
        }
        previous = character;
    }
    let split = split?;
    let (left, right) = (text[..split].trim(), text[split + 1..].trim());
    match (left, right) {
        (member, "null") | ("null", member) if !member.is_empty() && member != "null" => {
            Some(member)
        }
        _ => None,
    }
}

/// Convert an OXC `enum` declaration into the AST enum (numeric / string / auto
/// members; computed initializers are rejected).
pub(super) fn convert_enum_declaration(
    declaration: &oxc_ast::ast::TSEnumDeclaration<'_>,
) -> Result<crate::ast::EnumDecl, String> {
    use crate::ast::{EnumMember, EnumValue};
    let mut members = Vec::with_capacity(declaration.body.members.len());
    for member in &declaration.body.members {
        let name = match &member.id {
            oxc_ast::ast::TSEnumMemberName::Identifier(identifier) => Name {
                span: Span::new(identifier.span.start, identifier.span.end),
            },
            _ => return Err("OXC adapter supports identifier enum member names only".into()),
        };
        let value = match &member.initializer {
            None => EnumValue::Auto,
            Some(Expression::NumericLiteral(number)) => EnumValue::Num(number.value),
            Some(Expression::UnaryExpression(unary))
                if matches!(unary.operator, oxc_ast::ast::UnaryOperator::UnaryNegation) =>
            {
                match &unary.argument {
                    Expression::NumericLiteral(number) => EnumValue::Num(-number.value),
                    _ => return Err("OXC adapter supports literal enum initializers only".into()),
                }
            }
            Some(Expression::StringLiteral(text)) => EnumValue::Str(
                text.value
                    .as_str()
                    .ok_or("enum string initializer must be well-formed")?
                    .to_owned(),
            ),
            Some(_) => return Err("OXC adapter supports literal enum initializers only".into()),
        };
        members.push(EnumMember { name, value });
    }
    Ok(crate::ast::EnumDecl {
        name: Name {
            span: Span::new(declaration.id.span.start, declaration.id.span.end),
        },
        members,
        span: Span::new(declaration.span.start, declaration.span.end),
    })
}
