thread_local! {
    /// Name-start offsets of Zun `extern` declarations in the source being
    /// parsed, with the native library of an `extern function ... from "lib"`.
    static EXTERN_MARKS: std::cell::RefCell<std::collections::HashMap<u32, Option<String>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

pub(super) fn extern_mark(name_start: u32) -> Option<Option<String>> {
    EXTERN_MARKS.with(|marks| marks.borrow().get(&name_start).cloned())
}

/// Rewrite Zun `extern` declarations into forms OXC parses, preserving every
/// byte offset: the `extern` keyword and a trailing `from "lib"` become
/// spaces, leaving `function f(..): T;` / `m(..): T;` bodiless signatures. The
/// offsets of the declared names (and the library) go to `EXTERN_MARKS`.
/// Returns `None` when the source has no `extern` declaration.
pub(super) fn preprocess_externs(source: &str) -> Option<String> {
    EXTERN_MARKS.with(|marks| marks.borrow_mut().clear());
    if !source.contains("extern") {
        return None;
    }
    let bytes = source.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let word_end = |mut k: usize| {
        while k < bytes.len() && is_ident(bytes[k]) {
            k += 1;
        }
        k
    };
    let mut out = bytes.to_vec();
    let mut marks: Vec<(u32, Option<String>)> = Vec::new();
    let mut i = 0;
    let mut last_significant = b';';
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'"' | b'\'' | b'`' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                last_significant = b'"';
                continue;
            }
            _ => {}
        }
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let at_statement_start = matches!(last_significant, b';' | b'{' | b'}');
        if at_statement_start
            && bytes[i..].starts_with(b"extern")
            && bytes.get(i + 6).is_some_and(|c| c.is_ascii_whitespace())
        {
            // `extern` + whitespace + (`function` NAME | NAME)
            let mut j = i + 6;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let first_end = word_end(j);
            let is_function = &source[j..first_end] == "function"
                && bytes
                    .get(first_end)
                    .is_some_and(|c| c.is_ascii_whitespace());
            if is_function {
                let mut k = first_end;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                let name_end = word_end(k);
                if name_end > k {
                    if let Some(semi_rel) = source[name_end..].find(';') {
                        let semi = name_end + semi_rel;
                        let head = &source[name_end..semi];
                        let mut module = None;
                        if let Some(from_rel) = head.rfind(" from ") {
                            let tail = head[from_rel + 6..].trim();
                            if tail.len() >= 2 && (tail.starts_with('"') || tail.starts_with('\''))
                            {
                                module = Some(tail[1..tail.len() - 1].to_owned());
                                for slot in &mut out[name_end + from_rel..semi] {
                                    *slot = b' ';
                                }
                            }
                        }
                        for slot in &mut out[i..i + 6] {
                            *slot = b' ';
                        }
                        marks.push((k as u32, module));
                        last_significant = b';';
                        i = semi + 1;
                        continue;
                    }
                }
            } else if first_end > j
                && bytes
                    .get(first_end)
                    .is_some_and(|c| *c == b'(' || *c == b'<')
            {
                for slot in &mut out[i..i + 6] {
                    *slot = b' ';
                }
                marks.push((j as u32, None));
                i = first_end;
                last_significant = b'a';
                continue;
            }
        }
        last_significant = b;
        i += 1;
    }
    if marks.is_empty() {
        return None;
    }
    EXTERN_MARKS.with(|cell| {
        let mut cell = cell.borrow_mut();
        for (offset, module) in marks {
            cell.insert(offset, module);
        }
    });
    String::from_utf8(out).ok()
}
