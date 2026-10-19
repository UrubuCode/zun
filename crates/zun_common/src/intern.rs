//! String interning.
//!
//! Identifiers and string literals recur constantly across a module. The
//! [`Interner`] maps each distinct string to a small [`Symbol`] so that later
//! comparisons are integer equality and storage is deduplicated. The metadata
//! string heap (see `zun_metadata`) is a separate, serialized concept; this
//! interner is the in-memory working form used during compilation.

use std::collections::HashMap;

crate::define_id! {
    /// An interned string handle. Cheap to copy and compare.
    pub struct Symbol;
}

/// Deduplicating string store.
#[derive(Debug, Default)]
pub struct Interner {
    lookup: HashMap<Box<str>, Symbol>,
    strings: Vec<Box<str>>,
}

impl Interner {
    /// Create an empty interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern `s`, returning its symbol. Interning the same text twice yields
    /// the same symbol.
    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(&sym) = self.lookup.get(s) {
            return sym;
        }
        let sym = Symbol::from_index(self.strings.len());
        let boxed: Box<str> = s.into();
        self.strings.push(boxed.clone());
        self.lookup.insert(boxed, sym);
        sym
    }

    /// Resolve a symbol back to its string. Panics if the symbol did not come
    /// from this interner.
    pub fn resolve(&self, sym: Symbol) -> &str {
        &self.strings[sym.index()]
    }

    /// The number of distinct strings interned.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Whether no strings have been interned.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::Interner;

    #[test]
    fn same_text_same_symbol() {
        let mut i = Interner::new();
        let a = i.intern("hello");
        let b = i.intern("hello");
        let c = i.intern("world");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(i.len(), 2);
    }

    #[test]
    fn resolve_roundtrip() {
        let mut i = Interner::new();
        let sym = i.intern("String");
        assert_eq!(i.resolve(sym), "String");
    }
}
