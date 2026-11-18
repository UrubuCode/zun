//! Capture detection for classes written inside a function.
//!
//! A class compiled to a static TypeDef has no environment: its members
//! cannot reach the locals of the function it is written in. Such a class is
//! correct only on the run-time (closure) path of [`super::runtime_class`],
//! where every member is a function value closed over the enclosing
//! environment through the ordinary capture analysis -- by reference, shared
//! with any arrow of the same scope.
//!
//! This module answers the one question that routing needs: *does the class
//! body mention a binding of an enclosing function?* Both halves are
//! deliberately conservative towards the run-time path, which is plain
//! JavaScript semantics, and never towards the static one, which would turn a
//! capture into a `ReferenceError`:
//!
//! * the bindings of a function are collected from its AST (parameters,
//!   `var` / `let` / `const`, nested function declarations, catch
//!   parameters), not descending into nested functions;
//! * the class body is scanned as tokens, discarding the positions where a
//!   name is certainly not a variable reference (a property after `.`, a
//!   `key:`, a member name at class-body level).

use super::*;

/// The names a function declares, split by what they hold.
#[derive(Clone, Default)]
pub(super) struct ScopeNames {
    /// Run-time bindings: parameters, variables, nested functions.
    pub(super) bindings: Vec<String>,
    /// Names of classes written in the function (declarations, and bindings
    /// initialized by a class expression). Such a name is a static class
    /// unless the class is lowered, which the lowering records separately.
    pub(super) classes: Vec<String>,
}

/// Append the names bound by `pattern`.
fn pattern_names(pattern: &BindingPattern<'_>, names: &mut ScopeNames) {
    for identifier in pattern.get_binding_identifiers() {
        names.bindings.push(identifier.name.to_string());
    }
}

fn declaration_names(declaration: &oxc_ast::ast::VariableDeclaration<'_>, names: &mut ScopeNames) {
    for declarator in &declaration.declarations {
        // A binding initialized by a class expression names a static class
        // unless that class is itself lowered, in which case the lowering
        // registers the name (see `register_runtime_binding`).
        if declarator
            .init
            .as_ref()
            .is_some_and(|init| matches!(init, Expression::ClassExpression(_)))
        {
            if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
                names.classes.push(identifier.name.to_string());
            }
            continue;
        }
        pattern_names(&declarator.id, names);
    }
}

/// Append the names `statement` binds in the function that contains it.
fn statement_names(statement: &Statement<'_>, names: &mut ScopeNames) {
    match statement {
        Statement::VariableDeclaration(declaration) => declaration_names(declaration, names),
        Statement::FunctionDeclaration(function) => {
            if let Some(id) = function.id.as_ref() {
                names.bindings.push(id.name.to_string());
            }
        }
        Statement::ClassDeclaration(class) => {
            if let Some(id) = class.id.as_ref() {
                names.classes.push(id.name.to_string());
            }
        }
        Statement::BlockStatement(block) => statements_names(&block.body, names),
        Statement::IfStatement(branch) => {
            statement_names(&branch.consequent, names);
            if let Some(alternate) = branch.alternate.as_ref() {
                statement_names(alternate, names);
            }
        }
        Statement::ForStatement(looped) => {
            if let Some(ForStatementInit::VariableDeclaration(declaration)) = looped.init.as_ref() {
                declaration_names(declaration, names);
            }
            statement_names(&looped.body, names);
        }
        Statement::ForInStatement(looped) => {
            if let ForStatementLeft::VariableDeclaration(declaration) = &looped.left {
                declaration_names(declaration, names);
            }
            statement_names(&looped.body, names);
        }
        Statement::ForOfStatement(looped) => {
            if let ForStatementLeft::VariableDeclaration(declaration) = &looped.left {
                declaration_names(declaration, names);
            }
            statement_names(&looped.body, names);
        }
        Statement::WhileStatement(looped) => statement_names(&looped.body, names),
        Statement::DoWhileStatement(looped) => statement_names(&looped.body, names),
        Statement::LabeledStatement(labeled) => statement_names(&labeled.body, names),
        Statement::SwitchStatement(switch) => {
            for case in &switch.cases {
                statements_names(&case.consequent, names);
            }
        }
        Statement::TryStatement(attempt) => {
            statements_names(&attempt.block.body, names);
            if let Some(handler) = attempt.handler.as_ref() {
                if let Some(parameter) = handler.param.as_ref() {
                    pattern_names(&parameter.pattern, names);
                }
                statements_names(&handler.body.body, names);
            }
            if let Some(finalizer) = attempt.finalizer.as_ref() {
                statements_names(&finalizer.body, names);
            }
        }
        _ => {}
    }
}

fn statements_names(statements: &[Statement<'_>], names: &mut ScopeNames) {
    for statement in statements {
        statement_names(statement, names);
    }
}

/// The names a function binds: its parameters and the declarations of its
/// body, at any block depth, without entering nested functions or classes.
pub(super) fn function_bindings(
    params: &oxc_ast::ast::FormalParameters<'_>,
    body: Option<&oxc_ast::ast::FunctionBody<'_>>,
) -> ScopeNames {
    let mut names = ScopeNames::default();
    for parameter in &params.items {
        pattern_names(&parameter.pattern, &mut names);
    }
    if let Some(rest) = params.rest.as_ref() {
        pattern_names(&rest.rest.argument, &mut names);
    }
    if let Some(body) = body {
        statements_names(&body.statements, &mut names);
    }
    names
}

/// One lexical unit of a class body, as far as capture detection cares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token<'t> {
    Word(&'t str),
    Punct(u8),
    /// A string, number or template chunk: a value, never a name.
    Value,
    Newline,
}

/// Split `text` into tokens. Comments and string contents are dropped; a
/// template's `${ ... }` holes are tokenized as parenthesized code.
fn tokenize(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    // Per open template hole: the number of braces opened inside it.
    let mut holes: Vec<u32> = Vec::new();
    let mut index = 0;
    let mut in_template = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_template {
            match byte {
                b'\\' => index += 2,
                b'`' => {
                    in_template = false;
                    tokens.push(Token::Value);
                    index += 1;
                }
                b'$' if bytes.get(index + 1) == Some(&b'{') => {
                    in_template = false;
                    holes.push(0);
                    tokens.push(Token::Value);
                    tokens.push(Token::Punct(b'('));
                    index += 2;
                }
                _ => index += 1,
            }
            continue;
        }
        match byte {
            b'\n' => {
                tokens.push(Token::Newline);
                index += 1;
            }
            _ if byte.is_ascii_whitespace() => index += 1,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index < bytes.len()
                    && !(bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/'))
                {
                    index += 1;
                }
                index += 2;
            }
            b'\'' | b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != byte && bytes[index] != b'\n' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index += 1;
                tokens.push(Token::Value);
            }
            b'`' => {
                in_template = true;
                index += 1;
            }
            b'{' => {
                if let Some(depth) = holes.last_mut() {
                    *depth += 1;
                }
                tokens.push(Token::Punct(b'{'));
                index += 1;
            }
            b'}' if holes.last() == Some(&0) => {
                holes.pop();
                in_template = true;
                tokens.push(Token::Punct(b')'));
                index += 1;
            }
            b'}' => {
                if let Some(depth) = holes.last_mut() {
                    *depth -= 1;
                }
                tokens.push(Token::Punct(b'}'));
                index += 1;
            }
            _ if byte.is_ascii_digit() => {
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric()
                        || bytes[index] == b'_'
                        || bytes[index] == b'.')
                {
                    index += 1;
                }
                tokens.push(Token::Value);
            }
            _ if byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' || byte >= 0x80 => {
                let start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric()
                        || bytes[index] == b'_'
                        || bytes[index] == b'$'
                        || bytes[index] >= 0x80)
                {
                    index += 1;
                }
                tokens.push(Token::Word(&text[start..index]));
            }
            _ => {
                tokens.push(Token::Punct(byte));
                index += 1;
            }
        }
    }
    tokens
}

/// Words that may precede a member name at class-body level.
const MEMBER_MODIFIERS: &[&str] = &[
    "public", "private", "protected", "readonly", "static", "declare", "override", "abstract",
    "get", "set", "async",
];

/// Whether `body` -- the text of a class body, braces included -- mentions
/// one of `names` in a position that can be a variable reference.
pub(super) fn body_mentions_binding(body: &str, names: &[String]) -> bool {
    if names.is_empty() {
        return false;
    }
    let tokens = tokenize(body);
    let (mut braces, mut parens, mut brackets) = (0_i32, 0_i32, 0_i32);
    // Inside a field initializer at class-body level (`name = <here>`).
    let mut in_initializer = false;
    // The previous token that is not a line break, and the one before it.
    let mut previous: Option<Token<'_>> = None;
    let mut before_previous: Option<Token<'_>> = None;
    for (index, token) in tokens.iter().copied().enumerate() {
        let member_level = braces == 1 && parens == 0 && brackets == 0;
        let mut following = tokens[index + 1..]
            .iter()
            .copied()
            .filter(|token| *token != Token::Newline);
        let next = following.next();
        match token {
            Token::Newline => {
                // A line break ends an initializer that looks complete.
                if member_level
                    && matches!(
                        previous,
                        Some(Token::Word(_) | Token::Value | Token::Punct(b')' | b']' | b'}'))
                    )
                {
                    in_initializer = false;
                }
                continue;
            }
            Token::Punct(punct) => match punct {
                b'{' => braces += 1,
                b'}' => braces -= 1,
                b'(' => parens += 1,
                b')' => parens -= 1,
                b'[' => brackets += 1,
                b']' => brackets -= 1,
                b';' if member_level => in_initializer = false,
                b'=' if member_level
                    && !matches!(next, Some(Token::Punct(b'=' | b'>')))
                    && !matches!(previous, Some(Token::Punct(b'=' | b'!' | b'<' | b'>'))) =>
                {
                    in_initializer = true;
                }
                _ => {}
            },
            Token::Value => {}
            Token::Word(word) => {
                let property = previous == Some(Token::Punct(b'.'))
                    && before_previous != Some(Token::Punct(b'.'));
                let keyed = match next {
                    Some(Token::Punct(b':')) => true,
                    Some(Token::Punct(b'?' | b'!')) => following.next() == Some(Token::Punct(b':')),
                    _ => false,
                };
                let key_position = match previous {
                    None | Some(Token::Punct(b'{' | b'}' | b',' | b';' | b'(')) => true,
                    Some(Token::Word(modifier)) => MEMBER_MODIFIERS.contains(&modifier),
                    _ => false,
                };
                let is_key = keyed && key_position;
                let is_member_name = member_level && !in_initializer;
                if !property
                    && !is_key
                    && !is_member_name
                    && names.iter().any(|name| name == word)
                {
                    return true;
                }
            }
        }
        before_previous = previous;
        previous = Some(token);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::body_mentions_binding;

    fn mentions(body: &str, names: &[&str]) -> bool {
        let names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        body_mentions_binding(body, &names)
    }

    #[test]
    fn a_method_body_reference_is_a_capture() {
        assert!(mentions("{ get(): number { return k } }", &["k"]));
        assert!(mentions("{ inc() { return ++c } }", &["c"]));
        assert!(mentions("{ m() { return [...k] } }", &["k"]));
        assert!(mentions("{ m() { return { k } } }", &["k"]));
        assert!(mentions("{ m() { return c ? k : 1 } }", &["k"]));
        assert!(mentions("{ m() { return `a${k}b` } }", &["k"]));
        assert!(mentions("{ m() { return `it's ${ {x: k}.x }` + 'a' } }", &["k"]));
    }

    #[test]
    fn initializers_computed_keys_and_static_blocks_capture() {
        assert!(mentions("{ x = k }", &["k"]));
        assert!(mentions("{ x: number = k; }", &["k"]));
        assert!(mentions("{ [k]() {} }", &["k"]));
        assert!(mentions("{ static { k() } }", &["k"]));
        assert!(mentions("{ m(a = k) {} }", &["k"]));
    }

    #[test]
    fn names_that_are_not_references_do_not_capture() {
        assert!(!mentions("{ name: string; m() { return this.name } }", &["name"]));
        assert!(!mentions("{ name = 1\n value() { return 2 } }", &["name", "value"]));
        assert!(!mentions(
            "{ static readonly name?: string; get value() { return 1 } }",
            &["name", "value"]
        ));
        assert!(!mentions("{ m() { return { k: 1 }.k + o?.k } }", &["k"]));
        assert!(!mentions("{ m() { return 'k' + \"k\" + `k` } // k\n /* k */ }", &["k"]));
        assert!(!mentions("{ m(o: { k: number }) { return 1 } }", &["k"]));
        assert!(!mentions("{ m() { return other } }", &["k"]));
        assert!(!mentions("{ m() { return k } }", &[]));
    }
}
