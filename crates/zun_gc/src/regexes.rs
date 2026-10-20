use super::{Heap, Obj};

impl Heap {
    /// Allocate a managed regular-expression object with a compiled pattern.
    pub fn alloc_regex(&mut self, source: impl Into<String>) -> Result<Obj, zun_core::RegexError> {
        let regex = zun_core::RegexValue::new(source)?;
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
    fn invalid_managed_regex_is_rejected_before_allocation() {
        let mut heap = Heap::new();
        assert!(heap.alloc_regex("[").is_err());
        assert_eq!(heap.regexes.len(), 0);
    }
}
