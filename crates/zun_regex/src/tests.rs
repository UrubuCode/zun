use super::*;

type Spans = Vec<Option<(usize, usize)>>;

fn run(pat: &str, flags: &str, input: &str) -> Option<Spans> {
    let re = Regex::new(pat, flags).unwrap_or_else(|e| panic!("compile {pat:?}: {e}"));
    re.find(input, 0).unwrap().map(|m| m.groups().to_vec())
}

fn span(pat: &str, flags: &str, input: &str) -> Option<(usize, usize)> {
    run(pat, flags, input).map(|g| g[0].unwrap())
}

fn s(a: usize, b: usize) -> Option<(usize, usize)> {
    Some((a, b))
}

fn bad(pat: &str) {
    assert!(Regex::new(pat, "").is_err(), "expected parse error for {pat:?}");
}

// ---- quantifiers ----

#[test]
fn greedy_quantifiers() {
    assert_eq!(span("a*", "", "aaab"), s(0, 3));
    assert_eq!(span("a+", "", "baaa"), s(1, 4));
    assert_eq!(span("a?", "", "b"), s(0, 0));
    assert_eq!(span("a?", "", "ab"), s(0, 1));
    assert_eq!(span("a{2}", "", "aaaa"), s(0, 2));
    assert_eq!(span("a{2,}", "", "aaaa"), s(0, 4));
    assert_eq!(span("a{2,3}", "", "aaaa"), s(0, 3));
    assert_eq!(span("a{2,3}", "", "a"), None);
}

#[test]
fn lazy_quantifiers() {
    assert_eq!(span("a*?", "", "aaa"), s(0, 0));
    assert_eq!(span("a+?", "", "aaa"), s(0, 1));
    assert_eq!(span("a??", "", "a"), s(0, 0));
    assert_eq!(span("a{2,3}?", "", "aaaa"), s(0, 2));
    assert_eq!(span("a{2,}?", "", "aaaa"), s(0, 2));
    assert_eq!(span("<.+?>", "", "<a><b>"), s(0, 3));
    assert_eq!(span("<.+>", "", "<a><b>"), s(0, 6));
    assert_eq!(span("a+?b", "", "aaab"), s(0, 4));
}

#[test]
fn counted_groups() {
    assert_eq!(span("(?:ab){2,3}", "", "ababababab"), s(0, 6));
    assert_eq!(span("(a{2}){2}", "", "aaaaa"), s(0, 4));
    assert_eq!(span("a{0}b", "", "ab"), s(1, 2));
}

#[test]
fn brace_literal_when_not_quantifier() {
    assert_eq!(span("a{,3}", "", "a{,3}"), s(0, 5));
    assert_eq!(span("a{", "", "a{"), s(0, 2));
    assert_eq!(span("x{1", "", "x{1"), s(0, 3));
}

#[test]
fn long_input_does_not_overflow_stack() {
    let input = "x".repeat(200_000);
    let re = Regex::new("x*", "").unwrap();
    assert_eq!(re.find(&input, 0).unwrap().unwrap().span(), (0, 200_000));
}

// ---- classes ----

#[test]
fn classes() {
    assert_eq!(span("[a-c]+", "", "xxabcabd"), s(2, 7));
    assert_eq!(span("[^a-c]+", "", "abxyc"), s(2, 4));
    assert_eq!(span("[abc-]+", "", "x-ab-"), s(1, 5));
    assert_eq!(span("[]a]", "", "a"), None); // empty class (never matches), then `a]`
    assert_eq!(span("[^]", "", "\n"), s(0, 1));
    assert_eq!(span("\\d+", "", "ab123c"), s(2, 5));
    assert_eq!(span("\\D+", "", "12ab3"), s(2, 4));
    assert_eq!(span("\\w+", "", "  ab_9 "), s(2, 6));
    assert_eq!(span("\\W", "", "ab c"), s(2, 3));
    assert_eq!(span("\\s+", "", "a \t\nb"), s(1, 4));
    assert_eq!(span("\\S+", "", "  ab "), s(2, 4));
    assert_eq!(span("[\\d.]+", "", "v1.25x"), s(1, 5));
    assert_eq!(span("[\\w-]+", "", "!a-b!"), s(1, 4));
    assert_eq!(span("[\\d-z]+", "", "a-9z"), s(1, 4));
    assert_eq!(span("[\\]]", "", "a]"), s(1, 2));
    assert_eq!(span("[\\b]", "", "a\u{8}"), s(1, 2));
}

#[test]
fn escapes() {
    assert_eq!(span("\\n\\r\\t", "", "x\n\r\t"), s(1, 4));
    assert_eq!(span("\\x41\\u0042", "", "zAB"), s(1, 3));
    assert_eq!(span("\\0", "", "a\0"), s(1, 2));
    assert_eq!(span("\\f\\v", "", "\u{c}\u{b}"), s(0, 2));
    assert_eq!(span("\\.", "", "a.b"), s(1, 2));
    assert_eq!(span("a\\\\b", "", "a\\b"), s(0, 3));
    assert_eq!(span("\\/", "", "a/"), s(1, 2));
    assert_eq!(span("\\cJ", "", "\n"), s(0, 1));
    assert_eq!(span("\\uD83D\\uDE00", "", "x\u{1F600}"), s(1, 2));
}

#[test]
fn dot() {
    assert_eq!(span(".", "", "\nab"), s(1, 2));
    assert_eq!(span("a.b", "", "a\nb"), None);
    assert_eq!(span("a.b", "s", "a\nb"), s(0, 3));
    assert_eq!(span(".", "", "\u{1F600}"), s(0, 1));
}

// ---- anchors ----

#[test]
fn anchors() {
    assert_eq!(span("^abc$", "", "abc"), s(0, 3));
    assert_eq!(span("^abc$", "", "xabc"), None);
    assert_eq!(span("^b", "", "a\nb"), None);
    assert_eq!(span("^b$", "m", "a\nb\nc"), s(2, 3));
    assert_eq!(span("a$", "", "a\nb"), None);
    assert_eq!(span("a$", "m", "a\nb"), s(0, 1));
    assert_eq!(span("$", "", "abc"), s(3, 3));
}

#[test]
fn word_boundaries() {
    assert_eq!(span("\\bfoo\\b", "", "a foo b"), s(2, 5));
    assert_eq!(span("\\bfoo\\b", "", "afoo b"), None);
    assert_eq!(span("\\Boo", "", "foo"), s(1, 3));
    assert_eq!(span("\\b", "", "  "), None);
    assert_eq!(span("\\b", "", "ab"), s(0, 0));
    assert_eq!(span("o\\b", "", "foo!"), s(2, 3));
}

// ---- alternation ----

#[test]
fn alternation() {
    assert_eq!(span("cat|dog", "", "hotdog"), s(3, 6));
    assert_eq!(span("a|ab", "", "ab"), s(0, 1)); // leftmost alternative wins
    assert_eq!(span("ab|a", "", "ab"), s(0, 2));
    assert_eq!(span("(?:a|b)+c", "", "xabbac"), s(1, 6));
    assert_eq!(span("a||b", "", "b"), s(0, 0));
    assert_eq!(span("(|a)+", "", "aa"), s(0, 2));
}

// ---- groups ----

#[test]
fn capture_groups() {
    assert_eq!(run("(a)(b)?", "", "ac"), Some(vec![s(0, 1), s(0, 1), None]));
    assert_eq!(run("(a|ab)(c|bcd)(d*)", "", "abcd"), Some(vec![s(0, 4), s(0, 1), s(1, 4), s(4, 4)]));
    assert_eq!(run("((a)(b))", "", "ab"), Some(vec![s(0, 2), s(0, 2), s(0, 1), s(1, 2)]));
    assert_eq!(run("(?:a)(b)", "", "ab"), Some(vec![s(0, 2), s(1, 2)]));
}

#[test]
fn captures_reset_each_iteration() {
    // From the spec: group 1 is cleared when the second iteration takes `b`.
    assert_eq!(run("(?:(a)|b)+", "", "ab"), Some(vec![s(0, 2), None]));
    assert_eq!(
        run("(z)((a+)?(b+)?(c))*", "", "zaacbbbcac"),
        Some(vec![s(0, 10), s(0, 1), s(8, 10), s(8, 9), None, s(9, 10)])
    );
}

#[test]
fn empty_iteration_rules() {
    assert_eq!(run("(a*)*", "", "b"), Some(vec![s(0, 0), None]));
    assert_eq!(run("(a*)+", "", "b"), Some(vec![s(0, 0), s(0, 0)]));
    assert_eq!(span("(a*)*b", "", "aaac"), None);
    assert_eq!(span("(?:)*x", "", "x"), s(0, 1));
}

#[test]
fn named_groups() {
    let re = Regex::new("(?<y>\\d{4})-(?<m>\\d\\d)", "").unwrap();
    let m = re.find("on 2024-05.", 0).unwrap().unwrap();
    assert_eq!(m.span(), (3, 10));
    assert_eq!(m.named("y"), s(3, 7));
    assert_eq!(m.named("m"), s(8, 10));
    assert_eq!(m.group(1), s(3, 7));
    assert_eq!(m.named("nope"), None);
    assert_eq!(re.group_names(), &[("y".to_string(), 1), ("m".to_string(), 2)]);
    assert_eq!(re.group_count(), 2);
}

// ---- backreferences ----

#[test]
fn backrefs_numbered() {
    assert_eq!(run("(a+)\\1", "", "aaaa"), Some(vec![s(0, 4), s(0, 2)]));
    assert_eq!(span("(\\w)\\1", "", "abccd"), s(2, 4));
    assert_eq!(span("(a)\\1", "", "aA"), None);
    assert_eq!(span("^(.+)\\1$", "", "abcabc"), s(0, 6));
    assert_eq!(span("^(.+)\\1$", "", "abcabd"), None);
}

#[test]
fn backrefs_named() {
    assert_eq!(span("(?<q>['\"]).*?\\k<q>", "", "say \"hi\" 'x'"), s(4, 8));
    assert_eq!(span("\\k<a>(?<a>x)", "", "x"), s(0, 1)); // forward reference is empty
}

#[test]
fn backrefs_ignore_case() {
    assert_eq!(span("(a)\\1", "i", "aA"), s(0, 2));
    assert_eq!(span("(?<w>hello) \\k<w>", "i", "hello HELLO"), s(0, 11));
    assert_eq!(span("(?<w>hello) \\k<w>", "", "hello HELLO"), None);
}

#[test]
fn backrefs_unset_and_forward() {
    assert_eq!(span("\\1(a)", "", "a"), s(0, 1));
    assert_eq!(span("(a)|b\\1", "", "b"), s(0, 1));
}

#[test]
fn backrefs_multi_digit() {
    assert_eq!(span("(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)\\10", "", "abcdefghijj"), s(0, 11));
    // Fewer groups than the number: legacy octal / identity escape (Annex B).
    assert_eq!(span("\\8", "", "8"), s(0, 1));
    assert_eq!(span("\\101", "", "A"), s(0, 1));
    assert_eq!(span("(a)\\2", "", "a\u{2}"), s(0, 2));
}

// ---- lookaround ----

#[test]
fn lookahead_positive() {
    assert_eq!(span("foo(?=bar)", "", "foobaz foobar"), s(7, 10));
    assert_eq!(run("(?=(a+))a", "", "aaa"), Some(vec![s(0, 1), s(0, 3)]));
    assert_eq!(span("\\d+(?=%)", "", "10 20%"), s(3, 5));
}

#[test]
fn lookahead_negative() {
    assert_eq!(span("foo(?!bar)", "", "foobar foobaz"), s(7, 10));
    assert_eq!(run("(?!(a))b", "", "b"), Some(vec![s(0, 1), None]));
    assert_eq!(span("^(?!.*x).*$", "", "abc"), s(0, 3));
    assert_eq!(span("^(?!.*x).*$", "", "abxc"), None);
}

#[test]
fn lookahead_is_atomic() {
    // Spec example: backtracking never re-enters the lookahead.
    assert_eq!(run("(?=(a+))a*b\\1", "", "baaabac"), Some(vec![s(3, 6), s(3, 4)]));
    assert_eq!(span("(?=(a+))a*b\\1", "", "baaac"), None);
}

#[test]
fn lookahead_does_not_consume() {
    assert_eq!(span("(?=a)a", "", "a"), s(0, 1));
    assert_eq!(span("(?=ab)(?=abc)abc", "", "abc"), s(0, 3));
    assert_eq!(span("a(?=b)", "", "ab"), s(0, 1));
}

#[test]
fn lookbehind_positive() {
    assert_eq!(span("(?<=\\$)\\d+", "", "cost $42"), s(6, 8));
    assert_eq!(span("(?<=a)b", "", "b"), None);
    assert_eq!(span("(?<=ab)c", "", "abc"), s(2, 3));
    assert_eq!(span("(?<=é)x", "", "éx"), s(1, 2));
}

#[test]
fn lookbehind_negative() {
    assert_eq!(span("(?<!\\$)\\b\\d+", "", "$42 17"), s(4, 6));
    assert_eq!(span("(?<!a)b", "", "ab cb"), s(4, 5));
    assert_eq!(span("(?<!a)b", "", "b"), s(0, 1));
}

#[test]
fn lookbehind_variable_length() {
    assert_eq!(span("(?<=a+)b", "", "aab"), s(2, 3));
    assert_eq!(span("(?<=a|bc)d", "", "bcd"), s(2, 3));
    assert_eq!(span("(?<=^a*)b", "", "aab"), s(2, 3));
    assert_eq!(span("(?<=^a*)b", "", "xab"), None);
}

#[test]
fn lookbehind_captures_match_backward() {
    // Spec examples: lookbehind matches right-to-left.
    assert_eq!(run("(?<=(\\d+)(\\d+))$", "", "1053"), Some(vec![s(4, 4), s(0, 1), s(1, 4)]));
    assert_eq!(run("(?<=\\1(a))b", "", "aab"), Some(vec![s(2, 3), s(1, 2)]));
}

#[test]
fn lookaround_nesting() {
    assert_eq!(span("(?<=(?=a)a)b", "", "ab"), s(1, 2));
    assert_eq!(span("a(?=(?<=a)b)", "", "ab"), s(0, 1));
    assert_eq!(span("a(?=(?<=b)b)", "", "ab"), None);
}

// ---- flags ----

#[test]
fn flag_ignore_case() {
    assert_eq!(span("abc", "i", "xABC"), s(1, 4));
    assert_eq!(span("[a-z]+", "i", "ABC"), s(0, 3));
    assert_eq!(span("[^a-z]", "i", "A"), None);
    assert_eq!(span("[A-Z]+", "i", "abc"), s(0, 3));
    assert_eq!(span("é", "i", "É"), s(0, 1));
    assert_eq!(span("abc", "", "ABC"), None);
}

#[test]
fn flag_multiline_and_dotall() {
    assert_eq!(span("^b", "m", "a\r\nb"), s(3, 4));
    assert_eq!(span("a$", "m", "a\u{2028}b"), s(0, 1));
    assert_eq!(span(".+", "s", "a\nb"), s(0, 3));
    assert_eq!(span(".+", "", "a\nb"), s(0, 1));
}

#[test]
fn flag_parsing() {
    let f = Flags::parse("gimsuyd").unwrap();
    assert!(f.global && f.ignore_case && f.multiline && f.dot_all && f.unicode && f.sticky && f.has_indices);
    assert!(Flags::parse("").unwrap() == Flags::default());
    assert!(Flags::parse("gg").is_err());
    assert!(Flags::parse("x").is_err());
}

// ---- find / API ----

#[test]
fn find_from_start_index() {
    let re = Regex::new("a", "").unwrap();
    assert_eq!(re.find("aXa", 1).unwrap().unwrap().span(), (2, 3));
    assert!(re.find("aXa", 3).unwrap().is_none());
    assert!(re.find("aXa", 99).unwrap().is_none());
    assert!(re.is_match("zza").unwrap());
    assert!(!re.is_match("zzz").unwrap());
}

#[test]
fn match_at_is_anchored() {
    let re = Regex::new("b", "").unwrap();
    let chars: Vec<char> = "ab".chars().collect();
    assert!(re.match_at_chars(&chars, 0).unwrap().is_none());
    assert_eq!(re.match_at_chars(&chars, 1).unwrap().unwrap().span(), (1, 2));
}

#[test]
fn char_indices_not_bytes() {
    let re = Regex::new("b", "").unwrap();
    assert_eq!(re.find("éé😀b", 0).unwrap().unwrap().span(), (3, 4));
}

#[test]
fn ast_shape() {
    let p = parse("a|b").unwrap();
    assert_eq!(p.node, RegexNode::Alt(vec![RegexNode::Char('a'), RegexNode::Char('b')]));
    let p = parse("(?<n>a)+?").unwrap();
    assert_eq!(p.group_count, 1);
    match p.node {
        RegexNode::Repeat { min: 1, max: None, greedy: false, node } => {
            assert!(matches!(*node, RegexNode::Group { index: 1, name: Some(_), .. }));
        }
        other => panic!("unexpected {other:?}"),
    }
}

// ---- invalid patterns ----

#[test]
fn invalid_patterns() {
    for p in [
        "(", "(a", "a)", ")", "[a", "[b-a]", "*a", "+", "?", "a**", "a+*", "^*", "$+", "\\b+",
        "(?<n>a)(?<n>b)", "(?<a>x)\\k<b>", "(?<a>x)\\k", "a{2,1}", "(?z)", "(?<>a)", "(?<1a>a)",
        "\\", "(?<=a)*", "{1}", "a{1}{2}", "(?",
    ] {
        bad(p);
    }
}

#[test]
fn parse_errors_have_position_and_display() {
    let e = Regex::new("ab(", "").unwrap_err();
    assert_eq!(e.position, 3);
    assert!(e.to_string().contains("unterminated group"));
    let e = Regex::new("a", "q").unwrap_err();
    assert!(e.to_string().contains("unknown flag"));
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let p = format!("{}a{}", "(".repeat(5000), ")".repeat(5000));
    bad(&p);
}

// ---- step limit ----

#[test]
fn catastrophic_pattern_hits_step_limit() {
    let re = Regex::new("(a+)+$", "").unwrap().with_step_limit(1_000_000);
    let r = re.find("aaaaaaaaaaaaaaaaaaaaaaaa!", 0);
    assert_eq!(r.unwrap_err(), RegexError::StepLimitExceeded);
    let re = Regex::new("(a+)+$", "").unwrap();
    assert_eq!(re.is_match("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!"), Err(RegexError::StepLimitExceeded));
}

#[test]
fn step_limit_is_per_call_and_benign_inputs_pass() {
    let re = Regex::new("(a+)+$", "").unwrap().with_step_limit(1_000_000);
    assert!(re.is_match("aaaa").unwrap());
    assert!(re.is_match("aaaa").unwrap());
    let tiny = Regex::new("a*b", "").unwrap().with_step_limit(5);
    assert_eq!(tiny.is_match("aaaaaaaaaaaaaaa"), Err(RegexError::StepLimitExceeded));
}

// ---- prefilter equivalence ----

/// For a corpus of patterns over several inputs, the prefilter-driven scan must
/// return exactly the same result as the naive "try every position" scan.
#[test]
fn prefilter_matches_naive_scan() {
    let patterns: &[(&str, &str)] = &[
        // literals
        ("hello", ""),
        ("hello", "i"),
        ("abc", ""),
        // classes
        ("[a-z]+", ""),
        ("[a-z]+", "i"),
        ("[^a-c]+", ""),
        ("\\d{3,}", ""),
        ("\\w+@\\w+\\.\\w+", ""),
        ("\\s+", ""),
        ("[A-Z]", "i"),
        // quantifiers (incl. empty-match)
        ("a*", ""),
        ("a+", ""),
        ("a?", ""),
        ("a{2,3}", ""),
        ("(?:)", ""),
        ("a*b", ""),
        ("x*y", ""),
        // anchors
        ("^abc", ""),
        ("^abc$", ""),
        ("abc$", ""),
        ("^$", ""),
        ("$", ""),
        ("^b$", "m"),
        ("^a", "m"),
        ("a$", "m"),
        // alternation
        ("cat|dog|bird|fish", ""),
        ("a|ab", ""),
        ("^a|b", ""),
        ("foo|", ""),
        // word boundary (leading zero-width then literal)
        ("\\bfoo\\b", ""),
        ("\\Boo", ""),
        // lookaround
        ("foo(?=bar)", ""),
        ("foo(?!bar)", ""),
        ("(?=a)a", ""),
        ("(?<=\\$)\\d+", ""),
        ("(?<!a)b", ""),
        ("^(?!.*x).*$", ""),
        // backrefs
        ("(a+)\\1", ""),
        ("(\\w)\\1", ""),
        ("\\1(a)", ""),
        // dotAll / any
        (".+", "s"),
        (".+", ""),
        ("a.b", "s"),
        // non-ascii
        ("é", "i"),
        ("(?<=é)x", ""),
        ("\\u{1F600}", ""),
    ];
    let inputs: &[&str] = &[
        "",
        "a",
        "b",
        "hello",
        "say hello to HELLO",
        "The quick brown fox. cat dog bird fish!",
        "foobar foobaz foobar",
        "cost $42 and 17 more 1234",
        "user@example.com",
        "aaa\nbbb\nccc",
        "a\r\nb\u{2028}c",
        "éxésé 😀 end",
        "abcabcabd",
        "   \t spaces \n here",
        "aaaaaaaaaaaaaab",
        "xxxxxyyyy",
    ];
    for (pat, flags) in patterns {
        // Skip patterns that are invalid under a given flag set gracefully.
        let re = match Regex::new(pat, flags) {
            Ok(re) => re,
            Err(e) => panic!("compile {pat:?}/{flags:?}: {e}"),
        };
        for input in inputs {
            let chars: Vec<char> = input.chars().collect();
            for start in 0..=chars.len() {
                let with = re.find_chars(&chars, start);
                let naive = re.find_chars_naive(&chars, start);
                assert_eq!(
                    with.as_ref().map(|o| o.as_ref().map(Match::groups)),
                    naive.as_ref().map(|o| o.as_ref().map(Match::groups)),
                    "prefilter mismatch for pattern {pat:?} flags {flags:?} input {input:?} start {start}"
                );
            }
        }
    }
}

#[test]
fn nested_quantifier_alternation_blowup_is_bounded() {
    let re = Regex::new("(a|aa)+$", "").unwrap().with_step_limit(500_000);
    assert_eq!(re.is_match(&format!("{}!", "a".repeat(60))), Err(RegexError::StepLimitExceeded));
}
