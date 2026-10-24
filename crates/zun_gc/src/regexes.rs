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

    /// JS-style flags string (`"gi"`, …), or `None` if not a regex.
    pub fn regex_flags(&self, object: Obj) -> Option<&str> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(zun_core::RegexValue::flags)
    }

    /// `String.prototype.matchAll`: an array of match arrays (each shaped like
    /// `regex_exec`'s result — index 0 the full match, 1.. the capture groups,
    /// `undefined` for non-participating groups). `None` if `object` is not a
    /// regex; an inner `Err` if the engine failed.
    pub fn regex_exec_all(
        &mut self,
        object: Obj,
        input: &str,
    ) -> Option<Result<Obj, zun_core::RegexError>> {
        let matches = match self
            .regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.captures_all(input))?
        {
            Ok(matches) => matches,
            Err(error) => return Some(Err(error)),
        };
        let outer = self.alloc_array(matches.len());
        for (index, groups) in matches.into_iter().enumerate() {
            let inner = self.alloc_array(groups.len());
            for (group_index, group) in groups.into_iter().enumerate() {
                let value = match group {
                    Some(text) => zun_core::Any::string(self.intern_string(text.into())),
                    None => zun_core::Any::void(),
                };
                self.array_set(inner, group_index, value);
            }
            self.array_set(outer, index, zun_core::Any::object(inner.addr() as usize));
        }
        Some(Ok(outer))
    }

    pub fn regex_source(&self, object: Obj) -> Option<&str> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(zun_core::RegexValue::source)
    }

    /// Which engine the regex object selected (`None` if not a regex).
    pub fn regex_engine_kind(&self, object: Obj) -> Option<zun_core::EngineKind> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(zun_core::RegexValue::engine_kind)
    }

    /// Match operations return `None` if `object` is not a regex, and an inner
    /// `Err` if the engine failed (e.g. the backtracking step limit).
    pub fn regex_is_match(
        &self,
        object: Obj,
        input: &str,
    ) -> Option<Result<bool, zun_core::RegexError>> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.is_match(input))
    }

    /// First match as UTF-8 byte offsets (both engines).
    #[allow(clippy::type_complexity)]
    pub fn regex_find(
        &self,
        object: Obj,
        input: &str,
    ) -> Option<Result<Option<(usize, usize)>, zun_core::RegexError>> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.find(input))
    }

    /// `String.prototype.search`: UTF-16 index of the first match, or -1.
    pub fn regex_search(&self, object: Obj, input: &str) -> Option<Result<i64, zun_core::RegexError>> {
        self.regexes.get(&(object.addr() as usize)).map(|regex| {
            regex
                .search(input)
                .map(|index| index.map_or(-1, |index| index as i64))
        })
    }

    /// `String.prototype.split(regex)`: array of Any strings (`undefined` for
    /// non-participating capture groups).
    pub fn regex_split(
        &mut self,
        object: Obj,
        input: &str,
    ) -> Option<Result<Obj, zun_core::RegexError>> {
        let pieces = match self
            .regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.split(input))?
        {
            Ok(pieces) => pieces,
            Err(error) => return Some(Err(error)),
        };
        let array = self.alloc_array(pieces.len());
        for (index, piece) in pieces.into_iter().enumerate() {
            let value = match piece {
                Some(text) => zun_core::Any::string(self.intern_string(text.into())),
                None => zun_core::Any::void(),
            };
            self.array_set(array, index, value);
        }
        Some(Ok(array))
    }

    /// `String.prototype.replace(regex, string)`: first match only.
    pub fn regex_replace_first(
        &self,
        object: Obj,
        input: &str,
        replacement: &str,
    ) -> Option<Result<String, zun_core::RegexError>> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.replace_first(input, replacement))
    }

    /// Run `exec`: `None` if `object` is not a regex, `Some(None)` when there is
    /// no match, otherwise an array of Any: index 0 is the full match and
    /// indices 1.. are capture-group strings (`undefined` for non-participating
    /// groups).
    pub fn regex_exec(
        &mut self,
        object: Obj,
        input: &str,
    ) -> Option<Result<Option<Obj>, zun_core::RegexError>> {
        let groups = match self
            .regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.captures(input))?
        {
            Ok(groups) => groups,
            Err(error) => return Some(Err(error)),
        };
        let Some(groups) = groups else {
            return Some(Ok(None));
        };
        let array = self.alloc_array(groups.len());
        for (index, group) in groups.into_iter().enumerate() {
            let value = match group {
                Some(text) => zun_core::Any::string(self.intern_string(text.into())),
                None => zun_core::Any::void(),
            };
            self.array_set(array, index, value);
        }
        Some(Ok(Some(array)))
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
        assert_eq!(heap.regex_is_match(regex, "zun-42"), Some(Ok(true)));
        assert_eq!(heap.regex_find(regex, "x zun-42"), Some(Ok(Some((2, 8)))));
        assert_eq!(heap.regex_is_match(regex, "zun-x"), Some(Ok(false)));
        assert_eq!(heap.regex_engine_kind(regex), Some(zun_core::EngineKind::Re2));
    }

    #[test]
    fn backreference_and_lookaround_use_engine_b_through_the_heap() {
        let mut heap = Heap::new();
        let backref = heap.alloc_regex(r"(\w)\1").expect("valid regex");
        assert_eq!(heap.regex_engine_kind(backref), Some(zun_core::EngineKind::Backtracking));
        assert_eq!(heap.regex_is_match(backref, "hello"), Some(Ok(true)));
        assert_eq!(heap.regex_is_match(backref, "helo"), Some(Ok(false)));
        // Capture spans in exec: "é😀" precedes the match; strings are exact.
        let array = heap.regex_exec(backref, "é😀xyy").unwrap().unwrap().expect("match");
        let first = heap.array_get(array, 0).unwrap().as_string().unwrap();
        assert_eq!(&**heap.string(first), "yy");
        let group = heap.array_get(array, 1).unwrap().as_string().unwrap();
        assert_eq!(&**heap.string(group), "y");
        // Same observable units as Engine A: bytes for find, UTF-16 for search.
        assert_eq!(heap.regex_find(backref, "é😀xyy"), Some(Ok(Some((7, 9)))));
        assert_eq!(heap.regex_search(backref, "é😀xyy"), Some(Ok(4)));
        let plain = heap.alloc_regex(r"yy").expect("valid regex");
        assert_eq!(heap.regex_find(plain, "é😀xyy"), Some(Ok(Some((7, 9)))));
        assert_eq!(heap.regex_search(plain, "é😀xyy"), Some(Ok(4)));
        let ahead = heap.alloc_regex(r"a(?=b)").expect("valid regex");
        assert_eq!(heap.regex_is_match(ahead, "ab"), Some(Ok(true)));
        assert_eq!(heap.regex_is_match(ahead, "ac"), Some(Ok(false)));
        let behind = heap.alloc_regex(r"(?<!a)b").expect("valid regex");
        assert_eq!(heap.regex_is_match(behind, "ab"), Some(Ok(false)));
        assert_eq!(heap.regex_is_match(behind, "cb"), Some(Ok(true)));
    }

    #[test]
    fn catastrophic_backtracking_surfaces_as_an_error() {
        let mut heap = Heap::new();
        let regex = heap.alloc_regex(r"(a+)+\1b").expect("valid regex");
        let input = format!("{}!", "a".repeat(60));
        assert!(matches!(heap.regex_is_match(regex, &input), Some(Err(_))));
        assert!(matches!(heap.regex_exec(regex, &input), Some(Err(_))));
        assert!(matches!(heap.regex_search(regex, &input), Some(Err(_))));
        assert!(matches!(heap.regex_split(regex, &input), Some(Err(_))));
        assert!(matches!(heap.regex_replace_first(regex, &input, "x"), Some(Err(_))));
    }

    #[test]
    fn managed_regex_exec_builds_group_array() {
        let mut heap = Heap::new();
        let regex = heap.alloc_regex_with_flags(r"(a)(x)?(b)", "i").expect("valid regex");
        let array = heap.regex_exec(regex, "zAB").unwrap().unwrap().expect("match");
        assert_eq!(heap.array_len(array), Some(4));
        let first = heap.array_get(array, 0).unwrap().as_string().unwrap();
        assert_eq!(&**heap.string(first), "AB");
        assert_eq!(heap.array_get(array, 2).unwrap().tag(), zun_core::AnyTag::Void);
        assert!(heap.regex_exec(regex, "zzz").unwrap().unwrap().is_none());
    }

    #[test]
    fn exec_all_collects_every_match_with_groups() {
        let mut heap = Heap::new();
        let regex = heap.alloc_regex_with_flags(r"(\d)(\d)", "g").expect("valid regex");
        assert_eq!(heap.regex_flags(regex), Some("g"));
        let outer = heap.regex_exec_all(regex, "12 34 56").unwrap().unwrap();
        assert_eq!(heap.array_len(outer), Some(3));
        let first = heap.array_get(outer, 0).unwrap().as_object().unwrap();
        let first = unsafe { super::Obj::from_addr(first as *mut u64) };
        let whole = heap.array_get(first, 0).unwrap().as_string().unwrap();
        assert_eq!(&**heap.string(whole), "12");
        let group = heap.array_get(first, 2).unwrap().as_string().unwrap();
        assert_eq!(&**heap.string(group), "2");
    }

    #[test]
    fn invalid_managed_regex_is_rejected_before_allocation() {
        let mut heap = Heap::new();
        assert!(heap.alloc_regex("[").is_err());
        assert_eq!(heap.regexes.len(), 0);
    }
}
