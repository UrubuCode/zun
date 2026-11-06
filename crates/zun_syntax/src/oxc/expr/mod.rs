use super::*;

mod assign;
mod call;
mod chain;
mod function_expr;
mod literal;
mod member;
mod object;
mod operators;
mod update;
mod yield_star;

use assign::*;
use call::*;
use chain::*;
use function_expr::*;
use literal::*;
use member::*;
use object::*;
use operators::*;
use update::*;
use yield_star::*;
pub(super) use call::convert_argument;
pub(super) use update::convert_effect_expression;

pub(super) fn convert_expression(source: &str, expression: &Expression<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    match expression {
        Expression::NumericLiteral(number) => Ok(Expr::Number {
            value: number.value,
            span: span(number.span),
        }),
        Expression::BigIntLiteral(bigint) => Ok(Expr::BigInt {
            value: bigint.value.as_str().to_owned(),
            span: span(bigint.span),
        }),
        Expression::StringLiteral(string) => {
            let value = js_str_to_string(string.value);
            Ok(Expr::String {
                value: value.to_owned(),
                span: span(string.span),
            })
        }
        Expression::BooleanLiteral(boolean) => Ok(Expr::Bool {
            value: boolean.value,
            span: span(boolean.span),
        }),
        Expression::NullLiteral(null) => Ok(Expr::Null {
            span: span(null.span),
        }),
        Expression::RegExpLiteral(regex) => Ok(Expr::RegexLiteral {
            pattern: regex.regex.pattern.text.to_string(),
            flags: regex.regex.flags.to_string(),
            span: span(regex.span),
        }),
        Expression::TemplateLiteral(template) => convert_template_literal(source, template),
        Expression::TaggedTemplateExpression(tagged) => convert_tagged_template(source, tagged),
        Expression::Identifier(identifier) => convert_identifier(identifier),
        // `super` is modelled as an identifier whose text is `super`; sema
        // recognises it in call/method-call receiver position and lowers the
        // base-class dispatch. The span covers the `super` keyword in source.
        Expression::Super(super_keyword) => Ok(Expr::Ident(Name {
            span: span(super_keyword.span),
        })),
        Expression::ThisExpression(this_expression) => Ok(Expr::Ident(Name {
            span: span(this_expression.span),
        })),
        Expression::ParenthesizedExpression(parenthesized) => {
            convert_expression(source, &parenthesized.expression)
        }
        Expression::SequenceExpression(sequence) => convert_sequence(source, sequence),
        Expression::TSAsExpression(assertion) => convert_ts_cast(
            source,
            &assertion.expression,
            assertion.type_annotation.span(),
            assertion.span,
        ),
        Expression::TSTypeAssertion(assertion) => convert_ts_cast(
            source,
            &assertion.expression,
            assertion.type_annotation.span(),
            assertion.span,
        ),
        Expression::TSSatisfiesExpression(assertion) => convert_ts_cast(
            source,
            &assertion.expression,
            assertion.type_annotation.span(),
            assertion.span,
        ),
        Expression::TSNonNullExpression(assertion) => convert_expression(source, &assertion.expression),
        Expression::StaticMemberExpression(member) => convert_static_member(source, member),
        Expression::ComputedMemberExpression(member) => convert_computed_member(source, member),
        Expression::PrivateFieldExpression(member) => convert_private_field(source, member),
        Expression::ChainExpression(chain) => convert_chain(source, chain),
        Expression::PrivateInExpression(private_in) => convert_private_in(source, private_in),
        Expression::BinaryExpression(binary) => convert_binary(source, binary),
        Expression::LogicalExpression(logical) => convert_logical(source, logical),
        Expression::ConditionalExpression(conditional) => Ok(Expr::Conditional {
            condition: Box::new(convert_expression(source, &conditional.test)?),
            when_true: Box::new(convert_expression(source, &conditional.consequent)?),
            when_false: Box::new(convert_expression(source, &conditional.alternate)?),
            span: span(conditional.span),
        }),
        Expression::UnaryExpression(unary) => convert_unary(source, unary),
        Expression::ImportExpression(import) => convert_import_expression(source, import),
        Expression::AwaitExpression(await_expression) => Ok(Expr::Await {
            operand: Box::new(convert_expression(source, &await_expression.argument)?),
            span: span(await_expression.span),
        }),
        Expression::YieldExpression(yield_expression) if yield_expression.delegate => {
            convert_yield_star(source, yield_expression)
        }
        Expression::YieldExpression(yield_expression) => convert_yield(source, yield_expression),
        Expression::AssignmentExpression(assignment) if is_destructuring_assignment(assignment) => {
            convert_destructuring_assignment(
                source,
                &assignment.left,
                convert_expression(source, &assignment.right)?,
                span(assignment.span),
            )
        }
        Expression::AssignmentExpression(assignment) => {
            convert_assignment(source, assignment)
        }
        Expression::CallExpression(call) => convert_call(source, call),
        Expression::ObjectExpression(object) => convert_object_literal(source, object),
        Expression::ArrayExpression(array) => convert_array_literal(source, array),
        Expression::NewExpression(new_expression) => convert_new_expression(source, new_expression),
        Expression::ArrowFunctionExpression(arrow) => convert_arrow(source, arrow),
        // A named function expression binds its own name inside its body:
        // `(() => { let name = function name() {...}; return name; })()` gives the
        // closure its self-reference (the `let` initializer already supports it).
        Expression::FunctionExpression(function) if refers_to_own_name(source, function) => {
            convert_named_function_expression(source, function)
        }
        Expression::FunctionExpression(function) => convert_oxc_function(source, function),
        // A class expression is lifted to a hoisted class declaration (Zun
        // classes are program-global) and the expression becomes a reference
        // to it, which sema reads as the class constructor descriptor. An
        // anonymous class takes the name of the binding it initializes (set by
        // the declarator converter), else the `class` keyword span.
        Expression::ClassExpression(class_expression) => {
            convert_class_expression(source, class_expression)
        }
        Expression::UpdateExpression(update) => convert_update(source, update, true),
        // `new.target`: first-class class values are not modelled yet, so it
        // evaluates to `undefined` (the value it has in every plain call).
        Expression::NewTarget(meta) => {
            // The identifier text is `new.target`; the checker resolves it.
            Ok(Expr::Ident(Name { span: span(meta.span) }))
        }
        other => {
            let dbg = format!("{other:?}");
            let kind = dbg.split(|c: char| !c.is_alphanumeric()).find(|s| !s.is_empty()).unwrap_or("?");
            Err(format!("OXC adapter: unsupported expression `{kind}`"))
        }
    }
}
