//! Typed index newtypes.
//!
//! Many tables in the compiler and runtime are dense arrays indexed by a small
//! integer. Using a bare `u32` everywhere invites mixing an index into one table
//! with an index into another. [`define_id`] stamps out a distinct, zero-cost
//! newtype per table so the type checker keeps them apart.

/// Define a typed index newtype backed by a `u32`.
///
/// The generated type is `Copy`, totally ordered, hashable, and convertible to
/// and from `u32`/`usize`. Index `u32::MAX` is reserved as a niche so that
/// `Option<Id>` stays the same size as the id itself.
#[macro_export]
macro_rules! define_id {
    ($(#[$meta:meta])* $vis:vis struct $name:ident;) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis struct $name(::core::num::NonZeroU32);

        impl $name {
            /// Build an id from a zero-based index. Panics if `index == u32::MAX`.
            #[inline]
            $vis fn from_index(index: usize) -> Self {
                let raw = u32::try_from(index).expect("id index out of range");
                // Shift by one so that index 0 maps to NonZero 1, keeping the
                // `u32::MAX` niche free for `Option`.
                let bumped = raw.checked_add(1).expect("id index out of range");
                // SAFETY: `bumped` is >= 1, so it is non-zero.
                Self(unsafe { ::core::num::NonZeroU32::new_unchecked(bumped) })
            }

            /// Return the zero-based index this id represents.
            #[inline]
            $vis fn index(self) -> usize {
                (self.0.get() - 1) as usize
            }

            /// Return the raw (bumped) `u32`, e.g. for serialization.
            #[inline]
            $vis fn as_u32(self) -> u32 {
                self.0.get() - 1
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                write!(f, "{}#{}", stringify!($name), self.index())
            }
        }
    };
}

#[cfg(test)]
mod tests {
    define_id! {
        /// Test id.
        pub struct TestId;
    }

    #[test]
    fn roundtrip_index() {
        let id = TestId::from_index(0);
        assert_eq!(id.index(), 0);
        assert_eq!(id.as_u32(), 0);

        let id = TestId::from_index(41);
        assert_eq!(id.index(), 41);
        assert_eq!(id.as_u32(), 41);
    }

    #[test]
    fn option_has_niche() {
        assert_eq!(
            core::mem::size_of::<TestId>(),
            core::mem::size_of::<Option<TestId>>()
        );
    }

    #[test]
    fn ordering_follows_index() {
        assert!(TestId::from_index(1) < TestId::from_index(2));
    }
}
