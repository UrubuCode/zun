use super::{Heap, Obj};

impl Heap {
    /// Allocate a managed regular-expression object with a compiled pattern.
    pub fn alloc_regex(&mut self, source: impl Into<String>) -> Result<Obj, zun_core::RegexError> {
        self.alloc_regex_with_flags(source, "")
    }

    /// Allocate a managed regular-expression object with JS-style flags.
    pub fn alloc_regex_with_flags(
        &mut self,
        source: impl Into<String>,
        flags: impl Into<String>,
    ) -> Result<Obj, zun_core::RegexError> {
        let regex = zun_core::RegexValue::with_flags(source, flags)?;
        let object = self.alloc(0, 0);
        self.regexes.insert(object.addr() as usize, regex);
        Ok(object)
    }

    pub fn regex_source(&self, object: Obj) -> Option<&str> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(zun_core::RegexValue::source)
    }

    pub fn regex_is_match(&self, object: Obj, input: &str) -> Option<bool> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.is_match(input))
    }

    pub fn regex_find(&self, object: Obj, input: &str) -> Option<Option<(usize, usize)>> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.find(input))
    }

    /// `String.prototype.search`: UTF-16 index of the first match, or -1.
    pub fn regex_search(&self, object: Obj, input: &str) -> Option<i64> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.search(input).map_or(-1, |index| index as i64))
    }

    /// `String.prototype.split(regex)`: array of Any strings (`undefined` for
    /// non-participating capture groups).
    pub fn regex_split(&mut self, object: Obj, input: &str) -> Option<Obj> {
        let pieces = self
            .regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.split(input))?;
        let array = self.alloc_array(pieces.len());
        for (index, piece) in pieces.into_iter().enumerate() {
            let value = match piece {
                Some(text) => zun_core::Any::string(self.intern_string(text.into())),
                None => zun_core::Any::void(),
            };
            self.array_set(array, index, value);
        }
        Some(array)
    }

    /// `String.prototype.replace(regex, string)`: first match only.
    pub fn regex_replace_first(&self, object: Obj, input: &str, replacement: &str) -> Option<String> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.replace_first(input, replacement))
    }

    /// Run `exec`: `None` if `object` is not a regex, `Some(None)` when there is
    /// no match, otherwise an array of Any: index 0 is the full match and
    /// indices 1.. are capture-group strings (`undefined` for non-participating
    /// groups).
    pub fn regex_exec(&mut self, object: Obj, input: &str) -> Option<Option<Obj>> {
        let groups = self
            .regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.captures(input))?;
        let Some(groups) = groups else {
            return Some(None);
        };
        let array = self.alloc_array(groups.len());
        for (index, group) in groups.into_iter().enumerate() {
            let value = match group {
                Some(text) => zun_core::Any::string(self.intern_string(text.into())),
                None => zun_core::Any::void(),
            };
            self.array_set(array, index, value);
        }
        Some(Some(array))
    }
}

#[cfg(test)]
mod tests {
    use super::Heap;

    #[test]
    fn managed_regex_compiles_and_matches() {
        let mut heap = Heap::new();
        let regex = heap.alloc_regex(r"zun-[0-9]+").expect("valid regex");
        assert_eq!(heap.regex_source(regex), Some(r"zun-[0-9]+"));
        assert_eq!(heap.regex_is_match(regex, "zun-42"), Some(true));
        assert_eq!(heap.regex_find(regex, "x zun-42"), Some(Some((2, 8))));
        assert_eq!(heap.regex_is_match(regex, "zun-x"), Some(false));
    }

    #[test]
    fn managed_regex_exec_builds_group_array() {
        let mut heap = Heap::new();
        let regex = heap.alloc_regex_with_flags(r"(a)(x)?(b)", "i").expect("valid regex");
        let array = heap.regex_exec(regex, "zAB").unwrap().expect("match");
        assert_eq!(heap.array_len(array), Some(4));
        let first = heap.array_get(array, 0).unwrap().as_string().unwrap();
        assert_eq!(&**heap.string(first), "AB");
        assert_eq!(heap.array_get(array, 2).unwrap().tag(), zun_core::AnyTag::Void);
        assert!(heap.regex_exec(regex, "zzz").unwrap().is_none());
    }

    #[test]
    fn invalid_managed_regex_is_rejected_before_allocation() {
        let mut heap = Heap::new();
        assert!(heap.alloc_regex("[").is_err());
        assert_eq!(heap.regexes.len(), 0);
    }
}
