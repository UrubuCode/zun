//! `ZStr` — the engine's managed string value.
//!
//! `ZStr` is an **opaque** reference-counted string. Today it wraps an
//! `Rc<str>` and carries no extra data (PR a: a behavior-neutral newtype), but
//! its internal representation is deliberately private so a later change — a
//! cached `{ascii, len16}` header (PR b), a GC-handle string, or WTF-16 storage
//! — stays internal to this type and never touches the call sites again.
//!
//! Construct with [`From<&str>`], [`From<String>`], or [`From<Rc<str>>`]; read
//! as a `&str` through [`Deref`]. There is intentionally no accessor that hands
//! back the inner `Rc`, so the storage can evolve freely.

use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::rc::Rc;

/// A managed, reference-counted string. Opaque: see the module docs.
#[derive(Clone)]
pub struct ZStr {
    // PR a: a plain `Rc<str>`. PR b replaces this with a header-carrying
    // representation without changing this type's public surface.
    data: Rc<str>,
}

impl ZStr {
    /// The string contents as a `&str`.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.data
    }
}

impl Deref for ZStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        &self.data
    }
}

impl AsRef<str> for ZStr {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.data
    }
}

impl Borrow<str> for ZStr {
    #[inline]
    fn borrow(&self) -> &str {
        &self.data
    }
}

impl From<&str> for ZStr {
    #[inline]
    fn from(text: &str) -> Self {
        ZStr {
            data: Rc::from(text),
        }
    }
}

impl From<String> for ZStr {
    #[inline]
    fn from(text: String) -> Self {
        ZStr {
            data: Rc::from(text),
        }
    }
}

impl From<Rc<str>> for ZStr {
    #[inline]
    fn from(data: Rc<str>) -> Self {
        ZStr { data }
    }
}

impl From<&String> for ZStr {
    #[inline]
    fn from(text: &String) -> Self {
        ZStr {
            data: Rc::from(text.as_str()),
        }
    }
}

impl PartialEq for ZStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        *self.data == *other.data
    }
}

impl Eq for ZStr {}

impl PartialEq<str> for ZStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        &*self.data == other
    }
}

impl PartialEq<&str> for ZStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        &*self.data == *other
    }
}

impl Hash for ZStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        (*self.data).hash(state);
    }
}

impl fmt::Display for ZStr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.data, f)
    }
}

impl fmt::Debug for ZStr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.data, f)
    }
}
