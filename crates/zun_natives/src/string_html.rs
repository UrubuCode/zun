//! The Annex B legacy HTML methods of `String.prototype` (ECMA-262 B.2.2.2 to
//! B.2.2.14): `anchor`, `big`, `blink`, `bold`, `fixed`, `fontcolor`,
//! `fontsize`, `italics`, `link`, `small`, `strike`, `sub` and `sup`.
//!
//! One table, read by the checker (which methods exist on a string), the
//! interpreter (what each one emits) and the prototype reflection (their
//! `length`). Each method is `CreateHTML(this, tag, attribute, value)`.

/// `(method, tag, attribute)`. An empty attribute means the method takes no
/// argument, so its `length` is 0; otherwise it takes the attribute value and
/// its `length` is 1.
pub const STRING_HTML_METHODS: &[(&str, &str, &str)] = &[
    ("anchor", "a", "name"),
    ("big", "big", ""),
    ("blink", "blink", ""),
    ("bold", "b", ""),
    ("fixed", "tt", ""),
    ("fontcolor", "font", "color"),
    ("fontsize", "font", "size"),
    ("italics", "i", ""),
    ("link", "a", "href"),
    ("small", "small", ""),
    ("strike", "strike", ""),
    ("sub", "sub", ""),
    ("sup", "sup", ""),
];

/// The `(tag, attribute)` of the legacy HTML method `name`, if it is one.
pub fn string_html_method(name: &str) -> Option<(&'static str, &'static str)> {
    STRING_HTML_METHODS
        .iter()
        .find(|(method, _, _)| *method == name)
        .map(|(_, tag, attribute)| (*tag, *attribute))
}

/// ECMA-262 B.2.2.2.1 `CreateHTML`, after both operands were converted with
/// ToString: `<tag attribute="value">text</tag>`. The only escaping the
/// specification performs is on the attribute value, where every `"` becomes
/// `&quot;`; the text and every other character pass through unchanged.
pub fn create_html(text: &str, tag: &str, attribute: &str, value: Option<&str>) -> String {
    let mut html = String::with_capacity(text.len() + 2 * tag.len() + 5);
    html.push('<');
    html.push_str(tag);
    if !attribute.is_empty() {
        html.push(' ');
        html.push_str(attribute);
        html.push_str("=\"");
        html.push_str(&value.unwrap_or("undefined").replace('"', "&quot;"));
        html.push('"');
    }
    html.push('>');
    html.push_str(text);
    html.push_str("</");
    html.push_str(tag);
    html.push('>');
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_html_escapes_only_the_quote_of_the_attribute_value() {
        assert_eq!(create_html("x", "big", "", None), "<big>x</big>");
        assert_eq!(
            create_html("x", "a", "name", Some("a&b")),
            "<a name=\"a&b\">x</a>"
        );
        assert_eq!(
            create_html("a\"b", "a", "href", Some("\"<>\"")),
            "<a href=\"&quot;<>&quot;\">a\"b</a>"
        );
        assert_eq!(
            create_html("x", "font", "size", None),
            "<font size=\"undefined\">x</font>"
        );
    }

    #[test]
    fn every_method_resolves_to_its_tag() {
        assert_eq!(string_html_method("bold"), Some(("b", "")));
        assert_eq!(string_html_method("fixed"), Some(("tt", "")));
        assert_eq!(string_html_method("link"), Some(("a", "href")));
        assert_eq!(string_html_method("trim"), None);
        assert_eq!(STRING_HTML_METHODS.len(), 13);
    }
}
