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
    let mut query_names: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for statement in body {
        if let Statement::FunctionDeclaration(function) = statement {
            if let Some(id) = function.id.as_ref() {
                let next = query_names.len();
                query_names.entry(id.name.as_str()).or_insert(next);
            }
        }
    }
    if query_names.is_empty() {
        return body.iter().collect();
    }
    // One tokenizing pass over the top-level statements records which hoisted
    // names each statement mentions (duplicates are harmless).
    let mut mentions: Vec<Vec<usize>> = Vec::with_capacity(body.len());
    for statement in body {
        let span = statement.span();
        let mut ids = Vec::new();
        if let Some(text) = source.get(span.start as usize..span.end as usize) {
            for word in identifier_words(text) {
                if let Some(&id) = query_names.get(word) {
                    ids.push(id);
                }
            }
        }
        mentions.push(ids);
    }
    // Statements in original order, keyed by a total-order label.
    let mut order: std::collections::BTreeMap<OrderKey, usize> =
        std::collections::BTreeMap::new();
    // Left-most label of an inserted statement mentioning each name.
    let mut earliest: Vec<Option<f64>> = vec![None; query_names.len()];
    let mut last = -1.0_f64;
    for (index, statement) in body.iter().enumerate() {
        let query = match statement {
            Statement::FunctionDeclaration(function) => function
                .id
                .as_ref()
                .and_then(|id| query_names.get(id.name.as_str()).copied()),
            _ => None,
        };
        let label = match query {
            Some(query) => match earliest[query] {
                Some(p) => {
                    let prev = order.range(..OrderKey(p)).next_back().map(|(k, _)| k.0);
                    let mut new = prev.map_or(p - 1.0, |prev| prev + (p - prev) / 2.0);
                    if !(new < p && prev.map_or(true, |prev| new > prev)) {
                        // f64 exhaustion between adjacent labels: relabel the
                        // current order `0, 1, 2, ...` and recompute `earliest`.
                        let current: Vec<usize> = order.values().copied().collect();
                        order.clear();
                        earliest.iter_mut().for_each(|slot| *slot = None);
                        for (label, value) in current.into_iter().enumerate() {
                            let label = label as f64;
                            order.insert(OrderKey(label), value);
                            for &id in &mentions[value] {
                                if earliest[id].map_or(true, |earliest| label < earliest) {
                                    earliest[id] = Some(label);
                                }
                            }
                        }
                        last = order.len() as f64 - 1.0;
                        let p = earliest[query].expect("relabel preserves the earliest use");
                        let prev = order.range(..OrderKey(p)).next_back().map(|(k, _)| k.0);
                        new = prev.map_or(p - 1.0, |prev| prev + (p - prev) / 2.0);
                    }
                    new
                }
                None => last + 1.0,
            },
            None => last + 1.0,
        };
        order.insert(OrderKey(label), index);
        if label > last {
            last = label;
        }
        for &id in &mentions[index] {
            if earliest[id].map_or(true, |earliest| label < earliest) {
                earliest[id] = Some(label);
            }
        }
    }
    order.values().map(|&index| &body[index]).collect()
}

/// Ordering label for the intermediate statement order: `f64` compared
/// totally, so `BTreeMap` can order inserted statements.
#[derive(Clone, Copy, PartialEq)]
struct OrderKey(f64);

impl Eq for OrderKey {}

impl PartialOrd for OrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Split `text` into maximal identifier words, using the same boundary rule as
/// [`contains_word`].
fn identifier_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    for (index, ch) in text.char_indices() {
        if is_identifier_char(ch) {
            start.get_or_insert(index);
        } else if let Some(from) = start.take() {
            words.push(&text[from..index]);
        }
    }
    if let Some(from) = start {
        words.push(&text[from..]);
    }
    words
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
    let mut hoisted_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let param_set: std::collections::HashSet<&str> =
        params.iter().map(String::as_str).collect();
    let in_head_names: std::collections::HashSet<&str> = sites
        .iter()
        .filter(|site| site.in_head)
        .map(|site| site.name.as_str())
        .collect();
    // First top-level statement mentioning each identifier (one pass).
    let mut first_index: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for (index, statement) in body.iter().enumerate() {
        let span = statement.span();
        if let Some(text) = source.get(span.start as usize..span.end as usize) {
            for word in identifier_words(text) {
                first_index.entry(word).or_insert(index);
            }
        }
    }
    for site in &sites {
        if site.in_head
            || param_set.contains(site.name.as_str())
            || hoisted_seen.contains(&site.name)
            || in_head_names.contains(site.name.as_str())
        {
            continue;
        }
        let used_early = !site.nested
            && first_index
                .get(site.name.as_str())
                .is_some_and(|&first| first < site.top_index);
        if site.nested || used_early {
            if hoisted_seen.insert(site.name.clone()) {
                hoisted.push(site.name.clone());
            }
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
    // One pre-order pass over the body collects every hoisted name's span at
    // once (first declaration in statement order wins).
    let mut var_spans: std::collections::HashMap<&str, Span> =
        std::collections::HashMap::new();
    collect_var_name_spans(body, &mut var_spans);
    let mut statements = Vec::with_capacity(converted.len() + hoisted.len());
    for name in &hoisted {
        let span = var_spans
            .get(name.as_str())
            .copied()
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

/// Collect every `var` binding identifier's name and span in one pre-order pass
/// over statement bodies (ignoring `for` heads). The first encounter of a name
/// wins.
pub(super) fn collect_var_name_spans<'s>(
    body: &[Statement<'s>],
    out: &mut std::collections::HashMap<&'s str, Span>,
) {
    fn in_declaration<'s>(
        declaration: &oxc_ast::ast::VariableDeclaration<'s>,
        out: &mut std::collections::HashMap<&'s str, Span>,
    ) {
        for declarator in &declaration.declarations {
            if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
                out.entry(identifier.name.as_str())
                    .or_insert(Span::new(identifier.span.start, identifier.span.end));
            }
        }
    }
    fn in_statement<'s>(
        statement: &Statement<'s>,
        out: &mut std::collections::HashMap<&'s str, Span>,
    ) {
        match statement {
            Statement::VariableDeclaration(declaration) => in_declaration(declaration, out),
            Statement::BlockStatement(block) => {
                for inner in &block.body {
                    in_statement(inner, out);
                }
            }
            Statement::IfStatement(if_statement) => {
                in_statement(&if_statement.consequent, out);
                if let Some(alternate) = &if_statement.alternate {
                    in_statement(alternate, out);
                }
            }
            Statement::ForStatement(for_statement) => in_statement(&for_statement.body, out),
            Statement::ForInStatement(for_in) => in_statement(&for_in.body, out),
            Statement::ForOfStatement(for_of) => in_statement(&for_of.body, out),
            Statement::WhileStatement(while_statement) => {
                in_statement(&while_statement.body, out)
            }
            Statement::DoWhileStatement(do_while) => in_statement(&do_while.body, out),
            Statement::LabeledStatement(labeled) => in_statement(&labeled.body, out),
            Statement::TryStatement(try_statement) => {
                for inner in &try_statement.block.body {
                    in_statement(inner, out);
                }
                if let Some(handler) = &try_statement.handler {
                    for inner in &handler.body.body {
                        in_statement(inner, out);
                    }
                }
                if let Some(finalizer) = &try_statement.finalizer {
                    for inner in &finalizer.body {
                        in_statement(inner, out);
                    }
                }
            }
            Statement::SwitchStatement(switch) => {
                for case in &switch.cases {
                    for inner in &case.consequent {
                        in_statement(inner, out);
                    }
                }
            }
            _ => {}
        }
    }
    for statement in body {
        in_statement(statement, out);
    }
}
