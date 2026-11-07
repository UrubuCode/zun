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
        let names = self.regexes.get(&(object.addr() as usize))?.group_names();
        let has_indices = self
            .regexes
            .get(&(object.addr() as usize))?
            .flags()
            .contains('d');
        let spans = match self
            .regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.match_all_full(input, true))?
        {
            Ok(spans) => spans,
            Err(error) => return Some(Err(error)),
        };
        let outer = self.alloc_array(spans.len());
        for (index, found) in spans.into_iter().enumerate() {
            let inner = self.alloc_match_array(
                input,
                found.start16,
                found.groups,
                &names,
                has_indices.then_some(&found.group_spans16[..]),
            );
            self.array_set(outer, index, zun_core::Any::object(inner.addr() as usize));
        }
        Some(Ok(outer))
    }

    /// Build a match-result array: the groups plus the `index`, `input` and
    /// `groups` expando properties of an `exec` / `matchAll` result.
    fn alloc_match_array(
        &mut self,
        input: &str,
        start16: usize,
        groups: Vec<Option<String>>,
        names: &[Option<String>],
        spans: Option<&[Option<(usize, usize)>]>,
    ) -> Obj {
        let array = self.alloc_array(groups.len());
        let mut named: Vec<(String, zun_core::Any)> = Vec::new();
        for (index, group) in groups.into_iter().enumerate() {
            let value = match group {
                Some(text) => zun_core::Any::string(self.intern_string(text.into())),
                None => zun_core::Any::void(),
            };
            if let Some(Some(name)) = names.get(index) {
                named.push((name.clone(), value));
            }
            self.array_set(array, index, value);
        }
        self.ensure_dynamic_entry(array);
        self.dynamic_set(
            array,
            "index".to_owned(),
            zun_core::Any::float(start16 as f64),
        );
        let input_any = zun_core::Any::string(self.intern_string(input.into()));
        self.dynamic_set(array, "input".to_owned(), input_any);
        let groups_any = if named.is_empty() {
            zun_core::Any::void()
        } else {
            let object = self.alloc_dynamic();
            self.dynamic_set_prototype(object, Some(zun_core::Any::null()));
            for (name, value) in named {
                self.dynamic_set(object, name, value);
            }
            zun_core::Any::object(object.addr() as usize)
        };
        self.dynamic_set(array, "groups".to_owned(), groups_any);
        if let Some(spans) = spans {
            // The `d` flag: `indices` holds `[start, end]` per group (and a
            // null-prototype `groups` object keyed by group name).
            let indices = self.alloc_array(spans.len());
            let mut named_spans: Vec<(String, zun_core::Any)> = Vec::new();
            for (index, span) in spans.iter().enumerate() {
                let value = match span {
                    Some((start, end)) => {
                        let pair = self.alloc_array(2);
                        self.array_set(pair, 0, zun_core::Any::float(*start as f64));
                        self.array_set(pair, 1, zun_core::Any::float(*end as f64));
                        zun_core::Any::object(pair.addr() as usize)
                    }
                    None => zun_core::Any::void(),
                };
                if let Some(Some(name)) = names.get(index) {
                    named_spans.push((name.clone(), value));
                }
                self.array_set(indices, index, value);
            }
            self.ensure_dynamic_entry(indices);
            let indices_groups = if named_spans.is_empty() {
                zun_core::Any::void()
            } else {
                let object = self.alloc_dynamic();
                self.dynamic_set_prototype(object, Some(zun_core::Any::null()));
                for (name, value) in named_spans {
                    self.dynamic_set(object, name, value);
                }
                zun_core::Any::object(object.addr() as usize)
            };
            self.dynamic_set(indices, "groups".to_owned(), indices_groups);
            self.dynamic_set(
                array,
                "indices".to_owned(),
                zun_core::Any::object(indices.addr() as usize),
            );
        }
        array
    }

    /// The `lastIndex` value as stored: a number, or whatever user code put there.
    pub fn regex_last_index_any(&self, object: Obj) -> Option<zun_core::Any> {
        let regex = self.regexes.get(&(object.addr() as usize))?;
        if regex.is_last_index_materialized() {
            Some(
                self.dynamic_get(object, "lastIndex")
                    .unwrap_or_else(zun_core::Any::void),
            )
        } else {
            Some(zun_core::Any::float(regex.last_index()))
        }
    }

    /// ToLength(ToNumber(`lastIndex`)) -- what `exec` reads (`None` if not a regex).
    pub fn regex_last_index(&self, object: Obj) -> Option<f64> {
        let any = self.regex_last_index_any(object)?;
        let number = match any.tag() {
            zun_core::AnyTag::Float => any.as_float().unwrap_or(f64::NAN),
            zun_core::AnyTag::Int => any.as_int().unwrap_or(0) as f64,
            zun_core::AnyTag::Bool => f64::from(u8::from(any.as_bool().unwrap_or(false))),
            zun_core::AnyTag::Null => 0.0,
            zun_core::AnyTag::String => {
                let text = self.string(any.as_string().unwrap_or(0)).trim().to_owned();
                if text.is_empty() {
                    0.0
                } else {
                    text.parse::<f64>().unwrap_or(f64::NAN)
                }
            }
            _ => f64::NAN,
        };
        Some(if number.is_nan() || number <= 0.0 {
            0.0
        } else {
            number.min(9007199254740991.0).floor()
        })
    }

    /// Overwrite the `lastIndex` slot without attribute checks (internal seeding);
    /// `false` if `object` is not a regex.
    pub fn regex_set_last_index(&mut self, object: Obj, value: f64) -> bool {
        let materialized = match self.regexes.get_mut(&(object.addr() as usize)) {
            Some(regex) => {
                regex.set_last_index(value);
                regex.is_last_index_materialized()
            }
            None => return false,
        };
        if materialized {
            self.dynamic_set(object, "lastIndex".to_owned(), zun_core::Any::float(value));
        }
        true
    }

    /// Move `lastIndex` into the object's property table (a writable,
    /// non-enumerable, non-configurable data property) so reflection and
    /// `Object.freeze` can act on it.
    pub fn regex_materialize_last_index(&mut self, object: Obj) {
        let Some(regex) = self.regexes.get_mut(&(object.addr() as usize)) else {
            return;
        };
        if regex.is_last_index_materialized() {
            return;
        }
        let value = regex.last_index();
        regex.mark_last_index_materialized();
        self.ensure_dynamic_entry(object);
        if let Some(properties) = self.dynamic_objects.get_mut(&(object.addr() as usize)) {
            properties.force_define_data(
                "lastIndex",
                zun_core::Any::float(value),
                true,
                false,
                false,
            );
        }
    }

    /// The checked write of `exec` / `replace` (`Set(R, "lastIndex", v, true)`):
    /// a `TypeError` when the property is not writable.
    pub fn regex_store_last_index(
        &mut self,
        object: Obj,
        value: f64,
    ) -> Result<(), zun_core::RegexError> {
        let materialized = match self.regexes.get_mut(&(object.addr() as usize)) {
            Some(regex) if !regex.is_last_index_materialized() => {
                regex.set_last_index(value);
                return Ok(());
            }
            Some(_) => true,
            None => false,
        };
        if materialized {
            if !self.dynamic_can_set(object, "lastIndex") {
                return Err(zun_core::RegexError::new(
                    "TypeError: Cannot assign to read only property 'lastIndex' of object '[object RegExp]'",
                ));
            }
            self.dynamic_set(object, "lastIndex".to_owned(), zun_core::Any::float(value));
        }
        Ok(())
    }

    /// A user assignment `re.lastIndex = value` of any value.
    pub fn regex_assign_last_index(
        &mut self,
        object: Obj,
        value: zun_core::Any,
    ) -> Result<(), zun_core::RegexError> {
        match value.tag() {
            zun_core::AnyTag::Float => {
                self.regex_store_last_index(object, value.as_float().unwrap_or(0.0))
            }
            zun_core::AnyTag::Int => {
                self.regex_store_last_index(object, value.as_int().unwrap_or(0) as f64)
            }
            _ => {
                self.regex_materialize_last_index(object);
                if !self.dynamic_can_set(object, "lastIndex") {
                    return Err(zun_core::RegexError::new(
                        "TypeError: Cannot assign to read only property 'lastIndex' of object '[object RegExp]'",
                    ));
                }
                self.dynamic_set(object, "lastIndex".to_owned(), value);
                Ok(())
            }
        }
    }

    /// The stateful core of `exec` / `test` (RegExpBuiltinExec): honors and
    /// updates `lastIndex` for `g` / `y` regexes. Returns the match as
    /// (byte start, groups, group names).
    #[allow(clippy::type_complexity)]
    pub fn regex_builtin_exec(
        &mut self,
        object: Obj,
        input: &str,
    ) -> Option<Result<Option<(zun_core::MatchFull, Vec<Option<String>>)>, zun_core::RegexError>>
    {
        let (stateful, sticky) = {
            let flags = self.regexes.get(&(object.addr() as usize))?.flags();
            (
                flags.contains('g') || flags.contains('y'),
                flags.contains('y'),
            )
        };
        let raw = self.regex_last_index(object)?;
        let last_index = if stateful { raw } else { 0.0 };
        let total = zun_core::wtf16::len16(input);
        if last_index > total as f64 {
            if stateful {
                if let Err(error) = self.regex_store_last_index(object, 0.0) {
                    return Some(Err(error));
                }
            }
            return Some(Ok(None));
        }
        let regex = self.regexes.get(&(object.addr() as usize))?;
        let found = match regex.match_full(input, last_index as usize, sticky) {
            Ok(found) => found,
            Err(error) => return Some(Err(error)),
        };
        let names = regex.group_names();
        match found {
            None => {
                if stateful {
                    if let Err(error) = self.regex_store_last_index(object, 0.0) {
                        return Some(Err(error));
                    }
                }
                Some(Ok(None))
            }
            Some(found) => {
                if stateful {
                    if let Err(error) = self.regex_store_last_index(object, found.end16 as f64) {
                        return Some(Err(error));
                    }
                }
                Some(Ok(Some((found, names))))
            }
        }
    }

    /// `RegExp.prototype.test` with `lastIndex` semantics.
    pub fn regex_test_stateful(
        &mut self,
        object: Obj,
        input: &str,
    ) -> Option<Result<bool, zun_core::RegexError>> {
        Some(
            self.regex_builtin_exec(object, input)?
                .map(|found| found.is_some()),
        )
    }

    /// `RegExp.prototype.exec` with `lastIndex` semantics.
    pub fn regex_exec_stateful(
        &mut self,
        object: Obj,
        input: &str,
    ) -> Option<Result<Option<Obj>, zun_core::RegexError>> {
        let has_indices = self
            .regexes
            .get(&(object.addr() as usize))?
            .flags()
            .contains('d');
        match self.regex_builtin_exec(object, input)? {
            Ok(Some((found, names))) => Some(Ok(Some(self.alloc_match_array(
                input,
                found.start16,
                found.groups,
                &names,
                has_indices.then_some(&found.group_spans16[..]),
            )))),
            Ok(None) => Some(Ok(None)),
            Err(error) => Some(Err(error)),
        }
    }

    /// Capture-group names by group number (index 0 unused), `None` if not a regex.
    pub fn regex_group_names(&self, object: Obj) -> Option<Vec<Option<String>>> {
        self.regexes
            .get(&(object.addr() as usize))
            .map(zun_core::RegexValue::group_names)
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
    pub fn regex_search(
        &self,
        object: Obj,
        input: &str,
    ) -> Option<Result<i64, zun_core::RegexError>> {
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
        limit: Option<usize>,
    ) -> Option<Result<Obj, zun_core::RegexError>> {
        let pieces = match self
            .regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.split(input))?
        {
            Ok(mut pieces) => {
                if let Some(limit) = limit {
                    pieces.truncate(limit);
                }
                pieces
            }
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
        &mut self,
        object: Obj,
        input: &str,
        replacement: &str,
    ) -> Option<Result<String, zun_core::RegexError>> {
        let spans = match self.regex_replace_spans(object, input, false)? {
            Ok(spans) => spans,
            Err(error) => return Some(Err(error)),
        };
        let regex = self.regexes.get(&(object.addr() as usize))?;
        Some(Ok(regex.expand_matches(input, &spans, replacement)))
    }

    /// Match spans (byte offsets plus group texts) for a callback-driven
    /// replace: all matches when the regex has the `g` flag, else the first.
    #[allow(clippy::type_complexity)]
    pub fn regex_replace_spans(
        &mut self,
        object: Obj,
        input: &str,
        force_global: bool,
    ) -> Option<Result<Vec<(usize, usize, Vec<Option<String>>)>, zun_core::RegexError>> {
        let (global, sticky) = {
            let regex = self.regexes.get(&(object.addr() as usize))?;
            (
                force_global || regex.flags().contains('g'),
                regex.flags().contains('y'),
            )
        };
        if !global && sticky {
            // A sticky, non-global replace is a single `exec` driven by `lastIndex`.
            return Some(match self.regex_builtin_exec(object, input)? {
                Ok(Some((found, _))) => Ok(vec![(found.start16, found.end16, found.groups)]),
                Ok(None) => Ok(Vec::new()),
                Err(error) => Err(error),
            });
        }
        if global {
            if let Err(error) = self.regex_store_last_index(object, 0.0) {
                return Some(Err(error));
            }
        }
        self.regexes
            .get(&(object.addr() as usize))
            .map(|regex| regex.match_spans16(input, global))
    }

    /// `String.prototype.match` with a global regex: every match's text as an
    /// array of strings, or `None` for no match (`null`).
    pub fn regex_match_global(
        &mut self,
        object: Obj,
        input: &str,
    ) -> Option<Result<Option<Obj>, zun_core::RegexError>> {
        let spans = match self
            .regexes
            .get(&(object.addr() as usize))?
            .match_spans(input, true)
        {
            Ok(spans) => spans,
            Err(error) => return Some(Err(error)),
        };
        if spans.is_empty() {
            return Some(Ok(None));
        }
        let array = self.alloc_array(spans.len());
        for (index, (start, end, _)) in spans.into_iter().enumerate() {
            let value = zun_core::Any::string(self.intern_string(input[start..end].into()));
            self.array_set(array, index, value);
        }
        Some(Ok(Some(array)))
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
        let (spans, names, has_indices) = {
            let regex = self.regexes.get(&(object.addr() as usize))?;
            match regex.match_all_full(input, false) {
                Ok(spans) => (spans, regex.group_names(), regex.flags().contains('d')),
                Err(error) => return Some(Err(error)),
            }
        };
        let Some(found) = spans.into_iter().next() else {
            return Some(Ok(None));
        };
        Some(Ok(Some(self.alloc_match_array(
            input,
            found.start16,
            found.groups,
            &names,
            has_indices.then_some(&found.group_spans16[..]),
        ))))
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
        assert_eq!(
            heap.regex_engine_kind(regex),
            Some(zun_core::EngineKind::Re2)
        );
    }

    #[test]
    fn backreference_and_lookaround_use_engine_b_through_the_heap() {
        let mut heap = Heap::new();
        let backref = heap.alloc_regex(r"(\w)\1").expect("valid regex");
        assert_eq!(
            heap.regex_engine_kind(backref),
            Some(zun_core::EngineKind::Backtracking)
        );
        assert_eq!(heap.regex_is_match(backref, "hello"), Some(Ok(true)));
        assert_eq!(heap.regex_is_match(backref, "helo"), Some(Ok(false)));
        // Capture spans in exec: "é😀" precedes the match; strings are exact.
        let array = heap
            .regex_exec(backref, "é😀xyy")
            .unwrap()
            .unwrap()
            .expect("match");
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
        assert!(matches!(
            heap.regex_split(regex, &input, None),
            Some(Err(_))
        ));
        assert!(matches!(
            heap.regex_replace_first(regex, &input, "x"),
            Some(Err(_))
        ));
    }

    #[test]
    fn managed_regex_exec_builds_group_array() {
        let mut heap = Heap::new();
        let regex = heap
            .alloc_regex_with_flags(r"(a)(x)?(b)", "i")
            .expect("valid regex");
        let array = heap
            .regex_exec(regex, "zAB")
            .unwrap()
            .unwrap()
            .expect("match");
        assert_eq!(heap.array_len(array), Some(4));
        let first = heap.array_get(array, 0).unwrap().as_string().unwrap();
        assert_eq!(&**heap.string(first), "AB");
        assert_eq!(
            heap.array_get(array, 2).unwrap().tag(),
            zun_core::AnyTag::Void
        );
        assert!(heap.regex_exec(regex, "zzz").unwrap().unwrap().is_none());
    }

    #[test]
    fn exec_all_collects_every_match_with_groups() {
        let mut heap = Heap::new();
        let regex = heap
            .alloc_regex_with_flags(r"(\d)(\d)", "g")
            .expect("valid regex");
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
