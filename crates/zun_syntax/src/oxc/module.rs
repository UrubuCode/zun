use super::*;

/// Record the names bound by top-level variable declarations of the module.
pub(super) fn record_top_level_names(body: &[Statement<'_>]) {
    TOP_LEVEL_NAMES.with(|names| {
        let mut names = names.borrow_mut();
        names.clear();
        for statement in body.iter() {
            if let Statement::VariableDeclaration(decl) = statement {
                for d in decl.declarations.iter() {
                    if let BindingPattern::BindingIdentifier(id) = &d.id {
                        names.insert(id.name.to_string());
                    }
                }
            }
        }
    });
}

/// `var` declared inside a nested block / loop / branch of the module body
/// is module-scoped: hoist a `let x = undefined` to the head of the module
/// and turn the nested declaration into an assignment. Returns the hoisted
/// `let`s and registers the hoisted names for the statement converter.
pub(super) fn hoist_module_vars(body: &[Statement<'_>]) -> Vec<Stmt> {
    let mut module_hoisted_lets: Vec<Stmt> = Vec::new();
    let mut sites = Vec::new();
    for (index, statement) in body.iter().enumerate() {
        collect_vars_in_statement(statement, index, false, &mut sites);
    }
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for site in &sites {
        let top_declared = sites.iter().any(|other| other.name == site.name && (!other.nested || other.in_head))
            || TOP_LEVEL_NAMES.with(|names| names.borrow().contains(&site.name));
        if site.nested && !site.in_head && !top_declared && names.insert(site.name.clone()) {
            module_hoisted_lets.push(Stmt::Let {
                name: Name { span: Span::new(site.id_span.start, site.id_span.end) },
                ty: None,
                init: Expr::Unary {
                    op: crate::ast::UnaryOp::Void,
                    operand: Box::new(Expr::Number {
                        value: 0.0,
                        span: Span::new(site.id_span.start, site.id_span.end),
                    }),
                    span: Span::new(site.id_span.start, site.id_span.end),
                },
                span: Span::new(site.id_span.start, site.id_span.end),
            });
        }
    }
    HOISTED_VARS.with(|stack| stack.borrow_mut().push(names));
    module_hoisted_lets
}

/// Convert a top-level `import` declaration.
pub(super) fn convert_import_declaration(
    import_declaration: &oxc_ast::ast::ImportDeclaration<'_>,
) -> Result<crate::ast::Import, Vec<String>> {
    let Some(module) = import_declaration.source.value.as_str() else {
        return Err(vec![
            "OXC adapter cannot represent non-UTF-8 import sources".into(),
        ]);
    };
    let mut names = Vec::new();
    if let Some(specifiers) = import_declaration.specifiers.as_deref() {
        for specifier in specifiers {
            match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                    let ModuleExportName::IdentifierName(imported) = &specifier.imported
                    else {
                        return Err(vec![
                            "OXC adapter supports identifier named imports only".into(),
                        ]);
                    };
                    names.push(crate::ast::ImportName {
                        imported: Name {
                            span: Span::new(imported.span.start, imported.span.end),
                        },
                        local: Name {
                            span: Span::new(
                                specifier.local.span.start,
                                specifier.local.span.end,
                            ),
                        },
                        is_default: false,
                        is_namespace: false,
                    });
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                    names.push(crate::ast::ImportName {
                        imported: Name {
                            span: Span::new(
                                specifier.local.span.start,
                                specifier.local.span.end,
                            ),
                        },
                        local: Name {
                            span: Span::new(
                                specifier.local.span.start,
                                specifier.local.span.end,
                            ),
                        },
                        is_default: true,
                        is_namespace: false,
                    });
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                    names.push(crate::ast::ImportName {
                        imported: Name {
                            span: Span::new(specifier.span.start, specifier.span.start),
                        },
                        local: Name {
                            span: Span::new(
                                specifier.local.span.start,
                                specifier.local.span.end,
                            ),
                        },
                        is_default: false,
                        is_namespace: true,
                    });
                }
            }
        }
    }
    Ok(crate::ast::Import {
        names,
        module: module.to_owned(),
        span: Span::new(import_declaration.span.start, import_declaration.span.end),
    })
}

/// Accept `export { a, b }` lists that keep their local names; reject aliases.
pub(super) fn check_export_specifiers(
    source: &str,
    export: &oxc_ast::ast::ExportNamedDeclaration<'_>,
) -> Result<(), Vec<String>> {
    for specifier in &export.specifiers {
        let local = specifier.local.span();
        let exported = specifier.exported.span();
        if source.get(local.start as usize..local.end as usize)
            != source.get(exported.start as usize..exported.end as usize)
        {
            return Err(vec![
                "OXC adapter supports unchanged local export names only".into(),
            ]);
        }
    }
    Ok(())
}

/// A top-level function declaration that the program reassigns
/// (`f = function () {}`, the string-array obfuscator idiom) is a mutable
/// binding: lower it to a `let` hoisted to the head of the module body.
/// Returns whether the declaration was handled here.
pub(super) fn lower_reassigned_function(
    source: &str,
    statement: &Statement<'_>,
    function: &OxcFunction<'_>,
    module_code_spans: &[(usize, usize)],
    hoisted_functions: &mut Vec<Stmt>,
    top_level: &mut Vec<Stmt>,
) -> Result<bool, Vec<String>> {
    if let Some(id) = function.id.as_ref() {
        let name = &source[id.span.start as usize..id.span.end as usize];
        if !function.generator
            && !function.r#async
            && (name_is_reassigned(&source[function.span.start as usize..function.span.end as usize], name)
                // Reassigned by top-level module code (never shadowed
                // there, unlike text inside other functions).
                || module_code_spans.iter().any(|&(start, end)| {
                    source.get(start..end).is_some_and(|text| name_is_reassigned(text, name))
                })) {
            let converted = convert_statement_list(source, std::slice::from_ref(statement))
                .map_err(|error| vec![error])?;
            // Hoist to the module head only when earlier text uses the
            // name; otherwise keep the textual position so the body
            // can capture the globals declared before it.
            if source[..function.span.start as usize]
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .any(|line| contains_word(line, name))
            {
                hoisted_functions.extend(converted);
            } else {
                top_level.extend(converted);
            }
            return Ok(true);
        }
    }
    Ok(false)
}

/// Convert a named top-level function declaration (or a bodiless `extern`
/// signature) into the program's function list.
pub(super) fn convert_top_level_function(
    source: &str,
    function: &OxcFunction<'_>,
    functions: &mut Vec<Function>,
) -> Result<(), Vec<String>> {
    let Some(id) = function.id.as_ref() else {
        return Err(vec!["OXC adapter requires named functions".into()]);
    };
    let Some(body) = function.body.as_ref() else {
        // `extern function f(..): T from "lib";` arrives as a bodiless
        // overload signature (see `preprocess_externs`).
        if let Some(module) = extern_mark(id.span.start) {
            let mut prologue = Vec::new();
            let mut params = Vec::new();
            for parameter in &function.params.items {
                params.push(
                    convert_formal_param(source, parameter, &mut prologue)
                        .map_err(|error| vec![error])?,
                );
            }
            functions.push(Function {
                name: Name { span: Span::new(id.span.start, id.span.end) },
                type_params: Vec::new(),
                decorators: Vec::new(),
                params,
                return_type: convert_type_annotation(source, function.return_type.as_deref())
                    .map_err(|error| vec![error])?,
                is_extern: true,
                is_static: false,
                accessor: None,
                external_module: module,
                is_async: false,
                is_generator: false,
                body: Block { statements: Vec::new(), span: Span::new(function.span.start, function.span.end) },
                span: Span::new(function.span.start, function.span.end),
            });
            return Ok(());
        }
        return Err(vec!["OXC adapter requires function bodies".into()]);
    };
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let function_span = span(function.span);
    let mut params = Vec::with_capacity(function.params.items.len());
    let mut prologue = Vec::new();
    for parameter in &function.params.items {
        params.push(
            convert_formal_param(source, parameter, &mut prologue)
                .map_err(|error| vec![error])?,
        );
    }
    // The function body converts through the single shared statement
    // converter (switch, labelled statements, loops, etc.), after any
    // destructured-parameter prologue.
    let mut statements = prologue;
    statements.extend(
        convert_function_statements(source, &body.statements, &param_names(&function.params)).map_err(|error| vec![error])?,
    );
    if let Some(rest) = function.params.rest.as_ref() {
        let BindingPattern::BindingIdentifier(identifier) = &rest.rest.argument else {
            return Err(vec![
                "OXC adapter supports identifier rest parameters only".into()
            ]);
        };
        params.push(crate::ast::Param {
            name: Name {
                span: span(identifier.span),
            },
            ty: convert_type_annotation(source, rest.type_annotation.as_deref())
                .map_err(|error| vec![error])?,
            default_value: None,
            is_rest: true,
        });
    }
    inject_arguments_object(source, function_span, &mut params, &mut statements);
    functions.push(Function {
        name: Name {
            span: span(id.span),
        },
        // OXC adapter erases generic type parameters (see above).
        type_params: Vec::new(),
        decorators: Vec::new(),
        params,
        return_type: convert_type_annotation(source, function.return_type.as_deref())
            .map_err(|error| vec![error])?,
        is_extern: false,
        is_static: false,
        accessor: None,
        external_module: None,
        is_async: function.r#async,
        is_generator: function.generator,
        body: Block {
            statements,
            span: span(body.span),
        },
        span: function_span,
    });
    Ok(())
}

/// Source ranges of the module-level statements that are neither function nor
/// class declarations (the executable module code).
pub(super) fn collect_module_code_spans(body: &[Statement<'_>]) -> Vec<(usize, usize)> {
    body.iter()
        .filter(|other| !matches!(other, Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_)))
        .map(|other| (other.span().start as usize, other.span().end as usize))
        .collect()
}
