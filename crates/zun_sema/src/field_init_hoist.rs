//! Instance field initializers that observe construction order.
//!
//! `new C()` evaluates plain field defaults up front, but an initializer that
//! reads `this` or has a side effect (a comma sequence, a `.push(...)`) must run
//! in the constructor: base-class fields first, then the base constructor body,
//! then the derived fields right after `super(...)` returns. This pass moves
//! such initializers into `this.field = <init>;` statements at the correct
//! point of each class's constructor (synthesizing an empty constructor when the
//! class has none), leaving the field with its `undefined` default.

use zun_common::Span;
use zun_syntax::ast;

fn is_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .any(|w| w == word)
}

/// Offset of the first standalone occurrence of `word` in `source`.
fn find_word(source: &str, word: &str) -> Option<Span> {
    let bytes = source.as_bytes();
    let mut from = 0;
    while let Some(found) = source[from..].find(word) {
        let start = from + found;
        let end = start + word.len();
        let before_ok = start == 0
            || !(bytes[start - 1].is_ascii_alphanumeric()
                || bytes[start - 1] == b'_'
                || bytes[start - 1] == b'$');
        let after_ok = end >= bytes.len()
            || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'$');
        if before_ok && after_ok {
            return Some(Span::new(start as u32, end as u32));
        }
        from = end;
    }
    None
}

/// A synthetic hidden slot (`__cfN`) made by the OXC adapter for a computed
/// class field key; its initializer defines the property on `this`, so it
/// always has to run inside the constructor.
fn is_computed_field_slot(field: &ast::ClassField) -> bool {
    ast::synthetic_ident_text(field.name).is_some_and(|text| text.starts_with("__cf"))
}

fn needs_hoist(source: &str, init: &ast::Expr) -> bool {
    let span = init.span();
    let Some(text) = source.get(span.start as usize..span.end as usize) else {
        return false;
    };
    if matches!(init, ast::Expr::Function(_)) && text.contains("=>") && is_word(text, "this") {
        return true;
    }
    if text.contains("=>") || text.starts_with("function") || text.starts_with("class") {
        return false;
    }
    is_word(text, "this")
        || text.contains(".push(")
        || (text.starts_with('(') && text.contains(','))
}

fn is_void_default(init: &ast::Expr) -> bool {
    matches!(
        init,
        ast::Expr::Unary {
            op: ast::UnaryOp::Void,
            ..
        }
    )
}

fn name_is(source: &str, name: &ast::Name, expected: &str) -> bool {
    source.get(name.span.start as usize..name.span.end as usize) == Some(expected)
}

/// Parameters of the nearest constructor up `class`'s base chain (empty when
/// no ancestor declares one). `None` when the chain cannot be resolved
/// statically or the constructor takes a rest parameter, which `super(...)`
/// forwarding cannot express.
fn inherited_ctor_params(
    source: &str,
    classes: &[ast::Class],
    class: &ast::Class,
) -> Option<Vec<ast::Param>> {
    let mut current = class;
    for _ in 0..classes.len() + 1 {
        let base = current.base.as_ref()?;
        let base_name = source.get(base.span.start as usize..base.span.end as usize)?;
        let parent = classes.iter().find(|c| {
            source.get(c.name.span.start as usize..c.name.span.end as usize) == Some(base_name)
        })?;
        if let Some(ctor) = parent
            .methods
            .iter()
            .find(|m| !m.is_static && name_is(source, &m.name, "constructor"))
        {
            if ctor.params.iter().any(|p| p.is_rest) {
                return None;
            }
            return Some(ctor.params.clone());
        }
        if parent.base.is_none() {
            return Some(Vec::new());
        }
        current = parent;
    }
    None
}

/// Rewrite `program` so order-sensitive instance field initializers run inside
/// constructors. Returns `None` when nothing needs rewriting.
pub(crate) fn hoist(source: &str, program: &ast::Program) -> Option<ast::Program> {
    let hoistable = |class: &ast::Class| {
        class.decorators.is_empty()
            && (class.base.is_none()
                || inherited_ctor_params(source, &program.classes, class).is_some()
                || class
                    .methods
                    .iter()
                    .any(|m| !m.is_static && name_is(source, &m.name, "constructor")))
            && class.fields.iter().any(|f| {
                !f.is_static
                    && f.decorators.is_empty()
                    && f.initializer.as_deref().is_some_and(|init| {
                        !is_void_default(init)
                            && (is_computed_field_slot(f)
                                || (!f.is_private && needs_hoist(source, init)))
                    })
            })
    };
    if !program.classes.iter().any(hoistable) {
        return None;
    }
    let this_name = find_word(source, "this").map_or_else(|| ast::synthetic_ident("this"), |span| ast::Name { span });
    let ctor_name = find_word(source, "constructor").map_or_else(|| ast::synthetic_ident("constructor"), |span| ast::Name { span });
    let mut out = program.clone();
    let mut changed = false;
    for class in &mut out.classes {
        if !hoistable(class) {
            continue;
        }
        // Collect and neutralize the order-sensitive initializers.
        let mut statements: Vec<ast::Stmt> = Vec::new();
        for field in &mut class.fields {
            let computed_slot = is_computed_field_slot(field);
            if field.is_static
                || (field.is_private && !computed_slot)
                || !field.decorators.is_empty()
            {
                continue;
            }
            let Some(init) = field.initializer.as_deref() else {
                continue;
            };
            if is_void_default(init) || !(computed_slot || needs_hoist(source, init)) {
                continue;
            }
            let value = *field.initializer.take().expect("checked above");
            let span = value.span();
            field.initializer = Some(Box::new(ast::Expr::Unary {
                op: ast::UnaryOp::Void,
                operand: Box::new(ast::Expr::Number {
                    value: 0.0,
                    span: field.name.span,
                }),
                span: field.name.span,
            }));
            if computed_slot {
                // The initializer only defines the property on `this`; the
                // hidden slot itself is never read.
                statements.push(ast::Stmt::Expr(value));
            } else {
                statements.push(ast::Stmt::Expr(ast::Expr::FieldAssign {
                    object: Box::new(ast::Expr::Ident(this_name)),
                    field: field.name,
                    value: Box::new(value),
                    span,
                }));
            }
        }
        if statements.is_empty() {
            continue;
        }
        changed = true;
        let ctor = class
            .methods
            .iter_mut()
            .find(|m| !m.is_static && name_is(source, &m.name, "constructor"));
        match ctor {
            Some(ctor) => {
                let super_at = ctor.body.statements.iter().position(|s| {
                    matches!(s, ast::Stmt::Expr(ast::Expr::Call { callee, .. }) if name_is(source, callee, "super"))
                });
                let at = super_at.map_or(0, |i| i + 1);
                ctor.body.statements.splice(at..at, statements);
            }
            None => {
                let mut statements = statements;
                let mut params = Vec::new();
                if class.base.is_some() {
                    params = inherited_ctor_params(source, &program.classes, class)
                        .unwrap_or_default();
                    let super_name = find_word(source, "super")
                        .map_or_else(|| ast::synthetic_ident("super"), |span| ast::Name { span });
                    let args = params.iter().map(|p| ast::Expr::Ident(p.name)).collect();
                    statements.insert(
                        0,
                        ast::Stmt::Expr(ast::Expr::Call {
                            callee: super_name,
                            args,
                            span: class.span,
                        }),
                    );
                }
                class.methods.push(ast::Function {
                    name: ctor_name,
                    type_params: Vec::new(),
                    decorators: Vec::new(),
                    params,
                    return_type: None,
                    is_extern: false,
                    is_static: false,
                    accessor: None,
                    external_module: None,
                    is_async: false,
                    is_generator: false,
                    body: ast::Block {
                        statements,
                        span: class.span,
                    },
                    span: class.span,
                });
            }
        }
    }
    changed.then_some(out)
}
