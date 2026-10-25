use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SymbolId(u64);

impl SymbolId {
    pub const fn get(self) -> u64 { self.0 }
    pub const fn from_raw(value: u64) -> Self { SymbolId(value) }
}

/// The well-known symbols, seeded at fixed ids so their identities are stable
/// across a build and match the constants the compiler lowers against. User
/// symbols (`Symbol(desc)`/`Symbol.for(key)`) are allocated after these.
///
/// Each variant's discriminant is its [`SymbolId`]; the ordering here is the
/// ABI the checker depends on (see `zun_sema` well-known lowering).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum WellKnownSymbol {
    Iterator = 0,
    AsyncIterator = 1,
    HasInstance = 2,
    ToPrimitive = 3,
    ToStringTag = 4,
    // Seeded as stable *values* only (no behavioral dispatch yet): they let
    // user code read `Symbol.match` etc. as distinct identities. Appended at
    // the end so ids 0..4 stay fixed.
    //
    // Behavior status: `Match`/`Replace`/`Search`/`Split` are wired on the
    // String methods for dynamic arguments (`stringSymbolDispatch`).
    // `IsConcatSpreadable` is honored by `Array.prototype.concat` (`arrayConcat`).
    Match = 5,
    Replace = 6,
    Search = 7,
    Split = 8,
    /// Intentionally a no-op: Array is a frozen built-in and its copy methods
    /// (`map/filter/slice/...`) always produce a plain `Array`, so a
    /// species-constructor override has no expressible effect in the typed
    /// model.
    Species = 9,
    IsConcatSpreadable = 10,
    /// N/A: `Symbol.unscopables` only affects the `with` statement, which Zun
    /// does not have.
    Unscopables = 11,
}

impl WellKnownSymbol {
    /// All well-known symbols in id order; the seed order for the registry.
    pub const ALL: [WellKnownSymbol; 12] = [
        WellKnownSymbol::Iterator,
        WellKnownSymbol::AsyncIterator,
        WellKnownSymbol::HasInstance,
        WellKnownSymbol::ToPrimitive,
        WellKnownSymbol::ToStringTag,
        WellKnownSymbol::Match,
        WellKnownSymbol::Replace,
        WellKnownSymbol::Search,
        WellKnownSymbol::Split,
        WellKnownSymbol::Species,
        WellKnownSymbol::IsConcatSpreadable,
        WellKnownSymbol::Unscopables,
    ];

    pub const fn id(self) -> SymbolId { SymbolId(self as u64) }

    /// The `.description` string of the well-known symbol (ECMAScript form,
    /// e.g. `Symbol.iterator`).
    pub const fn description(self) -> &'static str {
        match self {
            WellKnownSymbol::Iterator => "Symbol.iterator",
            WellKnownSymbol::AsyncIterator => "Symbol.asyncIterator",
            WellKnownSymbol::HasInstance => "Symbol.hasInstance",
            WellKnownSymbol::ToPrimitive => "Symbol.toPrimitive",
            WellKnownSymbol::ToStringTag => "Symbol.toStringTag",
            WellKnownSymbol::Match => "Symbol.match",
            WellKnownSymbol::Replace => "Symbol.replace",
            WellKnownSymbol::Search => "Symbol.search",
            WellKnownSymbol::Split => "Symbol.split",
            WellKnownSymbol::Species => "Symbol.species",
            WellKnownSymbol::IsConcatSpreadable => "Symbol.isConcatSpreadable",
            WellKnownSymbol::Unscopables => "Symbol.unscopables",
        }
    }

    /// The member name that selects this symbol on the `Symbol` global
    /// (`Symbol.iterator` → `iterator`).
    pub fn from_member(member: &str) -> Option<Self> {
        Some(match member {
            "iterator" => WellKnownSymbol::Iterator,
            "asyncIterator" => WellKnownSymbol::AsyncIterator,
            "hasInstance" => WellKnownSymbol::HasInstance,
            "toPrimitive" => WellKnownSymbol::ToPrimitive,
            "toStringTag" => WellKnownSymbol::ToStringTag,
            "match" => WellKnownSymbol::Match,
            "replace" => WellKnownSymbol::Replace,
            "search" => WellKnownSymbol::Search,
            "split" => WellKnownSymbol::Split,
            "species" => WellKnownSymbol::Species,
            "isConcatSpreadable" => WellKnownSymbol::IsConcatSpreadable,
            "unscopables" => WellKnownSymbol::Unscopables,
            _ => return None,
        })
    }
}

/// The isolate's symbol table. It owns every symbol identity: the well-known
/// symbols (seeded eagerly at fixed ids), the unique symbols created by
/// `Symbol(desc)`, and the cross-realm registry behind `Symbol.for`/`keyFor`.
#[derive(Debug)]
pub struct SymbolRegistry {
    descriptions: Vec<Option<String>>,
    /// `Symbol.for` registry: key → id, and the reverse for `Symbol.keyFor`.
    global: HashMap<String, SymbolId>,
    global_keys: HashMap<SymbolId, String>,
}

impl Default for SymbolRegistry {
    fn default() -> Self {
        let mut registry = Self {
            descriptions: Vec::new(),
            global: HashMap::new(),
            global_keys: HashMap::new(),
        };
        // Seed the well-known symbols so ids 0..N are stable and match the
        // compiler's `WellKnownSymbol` constants. They are deliberately *not*
        // entered in the `Symbol.for` registry (so `Symbol.keyFor` returns
        // undefined for them, per spec).
        for well_known in WellKnownSymbol::ALL {
            let id = registry.create(Some(well_known.description()));
            debug_assert_eq!(id, well_known.id());
        }
        registry
    }
}

impl SymbolRegistry {
    pub fn new() -> Self { Self::default() }

    /// Allocate a fresh, unique symbol. Two calls never collide, even with the
    /// same description — this backs `Symbol(desc)`.
    pub fn create(&mut self, description: Option<impl Into<String>>) -> SymbolId {
        let id = SymbolId(self.descriptions.len() as u64);
        self.descriptions.push(description.map(Into::into));
        id
    }

    /// The canonical symbol for `key` in the cross-realm registry — the same id
    /// every time (`Symbol.for`).
    pub fn for_key(&mut self, key: impl Into<String>) -> SymbolId {
        let key = key.into();
        if let Some(&id) = self.global.get(&key) { return id; }
        let id = self.create(Some(key.clone()));
        self.global.insert(key.clone(), id);
        self.global_keys.insert(id, key);
        id
    }

    /// The registry key a `Symbol.for` symbol was created under, or `None` for
    /// a unique or well-known symbol (`Symbol.keyFor`).
    pub fn key_for(&self, id: SymbolId) -> Option<&str> {
        self.global_keys.get(&id).map(String::as_str)
    }

    pub fn iterator(&mut self) -> SymbolId { WellKnownSymbol::Iterator.id() }

    pub fn description(&self, id: SymbolId) -> Option<&str> {
        self.descriptions.get(id.0 as usize).and_then(Option::as_deref)
    }

    pub fn len(&self) -> usize { self.descriptions.len() }

    pub fn is_empty(&self) -> bool { self.descriptions.is_empty() }
}
