use super::*;

/// JS hoists function declarations to the top of their block. The adapter
/// lowers a nested declaration to a `let` of a closure, so a declaration that
/// an earlier statement mentions is moved in front of the first such
/// statement (not to the very top, so it still captures the bindings declared
/// before that use).
pub(super) fn hoist_function_declarations<'a, 'b>(
    source: &str,
    body: &'a [Statement<'b>],
) -> Vec<&'a Statement<'b>> {
    let mut order: Vec<&Statement> = body.iter().collect();
    let mut index = 0;
    while index < order.len() {
        if let Statement::FunctionDeclaration(function) = order[index] {
            if let Some(id) = function.id.as_ref() {
                let name = id.name.as_str();
                let first_use = order[..index].iter().position(|earlier| {
                    let span = earlier.span();
                    source
                        .get(span.start as usize..span.end as usize)
                        .map(|text| contains_word(text, name))
                        .unwrap_or(false)
                });
                if let Some(first) = first_use {
                    let moved = order.remove(index);
                    order.insert(first, moved);
                }
            }
        }
        index += 1;
    }
    order
}

/// A `var` binding found in a function body.
pub(super) struct VarSite {
    pub(super) name: String,
    /// Span of the binding identifier.
    pub(super) id_span: oxc_span::Span,
    /// Index of the enclosing top-level statement of the function body.
    pub(super) top_index: usize,
    /// Declared inside a nested block / loop / branch.
    pub(super) nested: bool,
    /// Declared in a `for` / `for-in` / `for-of` head (converted separately).
    pub(super) in_head: bool,
}

pub(super) fn collect_var_declaration(
    declaration: &oxc_ast::ast::VariableDeclaration<'_>,
    top_index: usize,
    nested: bool,
    in_head: bool,
    out: &mut Vec<VarSite>,
) {
    if declaration.kind != oxc_ast::ast::VariableDeclarationKind::Var {
        return;
    }
    for declarator in &declaration.declarations {
        if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
            out.push(VarSite {
                name: identifier.name.to_string(),
                id_span: identifier.span,
                top_index,
                nested,
                in_head,
            });
        }
    }
}

pub(super) fn collect_vars_in_statement(
    statement: &Statement<'_>,
    top_index: usize,
    nested: bool,
    out: &mut Vec<VarSite>,
) {
    match statement {
        Statement::VariableDeclaration(declaration) => {
            collect_var_declaration(declaration, top_index, nested, false, out)
        }
        Statement::BlockStatement(block) => {
            for inner in &block.body {
                collect_vars_in_statement(inner, top_index, true, out);
            }
        }
        Statement::IfStatement(if_statement) => {
            collect_vars_in_statement(&if_statement.consequent, top_index, true, out);
            if let Some(alternate) = &if_statement.alternate {
                collect_vars_in_statement(alternate, top_index, true, out);
            }
        }
        Statement::ForStatement(for_statement) => {
            if let Some(ForStatementInit::VariableDeclaration(declaration)) = &for_statement.init {
                collect_var_declaration(declaration, top_index, true, true, out);
            }
            collect_vars_in_statement(&for_statement.body, top_index, true, out);
        }
        Statement::ForInStatement(for_in) => {
            if let ForStatementLeft::VariableDeclaration(declaration) = &for_in.left {
                collect_var_declaration(declaration, top_index, true, true, out);
            }
            collect_vars_in_statement(&for_in.body, top_index, true, out);
        }
        Statement::ForOfStatement(for_of) => {
            if let ForStatementLeft::VariableDeclaration(declaration) = &for_of.left {
                collect_var_declaration(declaration, top_index, true, true, out);
            }
            collect_vars_in_statement(&for_of.body, top_index, true, out);
        }
        Statement::WhileStatement(while_statement) => {
            collect_vars_in_statement(&while_statement.body, top_index, true, out)
        }
        Statement::DoWhileStatement(do_while) => {
            collect_vars_in_statement(&do_while.body, top_index, true, out)
        }
        Statement::LabeledStatement(labeled) => {
            collect_vars_in_statement(&labeled.body, top_index, nested, out)
        }
        Statement::TryStatement(try_statement) => {
            for inner in &try_statement.block.body {
                collect_vars_in_statement(inner, top_index, true, out);
            }
            if let Some(handler) = &try_statement.handler {
                for inner in &handler.body.body {
                    collect_vars_in_statement(inner, top_index, true, out);
                }
            }
            if let Some(finalizer) = &try_statement.finalizer {
                for inner in &finalizer.body {
                    collect_vars_in_statement(inner, top_index, true, out);
                }
            }
        }
        Statement::SwitchStatement(switch) => {
            for case in &switch.cases {
                for inner in &case.consequent {
                    collect_vars_in_statement(inner, top_index, true, out);
                }
            }
        }
        _ => {}
    }
}

/// Convert a function body, hoisting `var` declarations to function scope when
/// they are declared in a nested block or read before their declaration.
pub(super) fn convert_function_statements(
    source: &str,
    body: &[Statement<'_>],
    params: &[String],
) -> Result<Vec<Stmt>, String> {
    let simple_params = LAST_PARAMS_SIMPLE.with(|flag| flag.replace(false));
    let mut sites = Vec::new();
    for (index, statement) in body.iter().enumerate() {
        collect_vars_in_statement(statement, index, false, &mut sites);
    }
    let mut hoisted: Vec<String> = Vec::new();
    for site in &sites {
        if site.in_head
            || params.contains(&site.name)
            || hoisted.contains(&site.name)
            || sites
                .iter()
                .any(|other| other.in_head && other.name == site.name)
        {
            continue;
        }
        let used_early = !site.nested
            && body[..site.top_index].iter().any(|earlier| {
                let span = earlier.span();
                source
                    .get(span.start as usize..span.end as usize)
                    .map(|text| contains_word(text, &site.name))
                    .unwrap_or(false)
            });
        if site.nested || used_early {
            hoisted.push(site.name.clone());
        }
    }
    let mut hoisted_set: std::collections::HashSet<String> = hoisted.iter().cloned().collect();
    if simple_params {
        for site in &sites {
            if !site.in_head && params.contains(&site.name) {
                hoisted_set.insert(site.name.clone());
            }
        }
    }
    HOISTED_VARS.with(|stack| stack.borrow_mut().push(hoisted_set));
    let shadowing: Vec<&Statement<'_>> = body
        .iter()
        .filter(|statement| {
            matches!(statement, Statement::FunctionDeclaration(function)
            if function.id.as_ref().is_some_and(|id| params.iter().any(|p| p == id.name.as_str())))
        })
        .collect();
    let outer_shadows = PARAM_SHADOWS.with(|names| {
        std::mem::replace(
            &mut *names.borrow_mut(),
            shadowing
                .iter()
                .filter_map(|statement| match statement {
                    Statement::FunctionDeclaration(function) => {
                        function.id.as_ref().map(|id| id.name.to_string())
                    }
                    _ => None,
                })
                .collect(),
        )
    });
    let converted = convert_statement_list(source, body);
    let mut head = Vec::new();
    if converted.is_ok() && !shadowing.is_empty() {
        EMIT_SHADOW.with(|flag| flag.set(true));
        for statement in &shadowing {
            match convert_statement_list(source, std::slice::from_ref(*statement)) {
                Ok(list) => head.extend(list),
                Err(error) => {
                    EMIT_SHADOW.with(|flag| flag.set(false));
                    PARAM_SHADOWS.with(|names| *names.borrow_mut() = outer_shadows);
                    HOISTED_VARS.with(|stack| stack.borrow_mut().pop());
                    return Err(error);
                }
            }
        }
        EMIT_SHADOW.with(|flag| flag.set(false));
    }
    PARAM_SHADOWS.with(|names| *names.borrow_mut() = outer_shadows);
    HOISTED_VARS.with(|stack| stack.borrow_mut().pop());
    let mut converted = converted?;
    if !head.is_empty() {
        head.append(&mut converted);
        converted = head;
    }
    if hoisted.is_empty() {
        return Ok(converted);
    }
    let mut statements = Vec::with_capacity(converted.len() + hoisted.len());
    for name in &hoisted {
        let span = find_var_name_span(body, name)
            .ok_or_else(|| "OXC adapter lost a hoisted var span".to_string())?;
        statements.push(Stmt::Let {
            name: Name { span },
            ty: None,
            init: Expr::Unary {
                op: crate::ast::UnaryOp::Void,
                operand: Box::new(Expr::Number { value: 0.0, span }),
                span,
            },
            span,
        });
    }
    statements.extend(converted);
    Ok(statements)
}

pub(super) fn find_var_name_span(body: &[Statement<'_>], name: &str) -> Option<Span> {
    fn in_declaration(
        declaration: &oxc_ast::ast::VariableDeclaration<'_>,
        name: &str,
    ) -> Option<Span> {
        for declarator in &declaration.declarations {
            if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
                if identifier.name == name {
                    return Some(Span::new(identifier.span.start, identifier.span.end));
                }
            }
        }
        None
    }
    fn in_statement(statement: &Statement<'_>, name: &str) -> Option<Span> {
        match statement {
            Statement::VariableDeclaration(declaration) => in_declaration(declaration, name),
            Statement::BlockStatement(block) => {
                block.body.iter().find_map(|s| in_statement(s, name))
            }
            Statement::IfStatement(s) => in_statement(&s.consequent, name)
                .or_else(|| s.alternate.as_ref().and_then(|a| in_statement(a, name))),
            Statement::ForStatement(s) => in_statement(&s.body, name),
            Statement::ForInStatement(s) => in_statement(&s.body, name),
            Statement::ForOfStatement(s) => in_statement(&s.body, name),
            Statement::WhileStatement(s) => in_statement(&s.body, name),
            Statement::DoWhileStatement(s) => in_statement(&s.body, name),
            Statement::LabeledStatement(s) => in_statement(&s.body, name),
            Statement::TryStatement(s) => s
                .block
                .body
                .iter()
                .find_map(|x| in_statement(x, name))
                .or_else(|| {
                    s.handler
                        .as_ref()
                        .and_then(|h| h.body.body.iter().find_map(|x| in_statement(x, name)))
                })
                .or_else(|| {
                    s.finalizer
                        .as_ref()
                        .and_then(|f| f.body.iter().find_map(|x| in_statement(x, name)))
                }),
            Statement::SwitchStatement(s) => s
                .cases
                .iter()
                .find_map(|c| c.consequent.iter().find_map(|x| in_statement(x, name))),
            _ => None,
        }
    }
    body.iter()
        .find_map(|statement| in_statement(statement, name))
}
