//! Index types shared by the string tables and the grammar tables.
//!
//! Every index is a `u32` into a flat table, wrapped in its own type. That way a grammar set can be
//! `static` data built ahead of time, or `Vec`s built at run time, with the same
//! layout.

/// Defines an index type: a `u32` wrapped in its own type, so the compiler
/// stops an index into one table being used for another.
macro_rules! index_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// The field is the raw `u32` (public, so generated tables can be
        /// `static` data). [`Self::NONE`] means "no index".
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
        #[repr(transparent)]
        pub struct $name(pub u32);

        impl $name {
            /// Marker value that means "no index".
            pub const NONE: Self = Self(u32::MAX);

            /// The index `i`.
            #[inline]
            pub const fn new(i: u32) -> Self {
                Self(i)
            }

            /// The index as a `usize`, for indexing a slice.
            #[inline]
            pub const fn index(self) -> usize {
                self.0 as usize
            }

            /// Whether this is [`Self::NONE`].
            #[inline]
            pub const fn is_none(self) -> bool {
                self.0 == u32::MAX
            }
        }

        impl From<$name> for u32 {
            #[inline]
            fn from(i: $name) -> u32 {
                i.0
            }
        }

        impl From<$name> for u64 {
            #[inline]
            fn from(i: $name) -> u64 {
                u64::from(i.0)
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

index_type!(
    /// Index of a URI in the uri partition of the string table.
    UriId
);
index_type!(
    /// Index of a local name inside its URI's local-name partition.
    LocalId
);
index_type!(
    /// Index of a prefix inside its URI's prefix partition.
    PrefixId
);
index_type!(
    /// Index into the type table.
    TypeIdx
);
index_type!(
    /// Index into the element declaration table.
    ElemIdx
);
index_type!(
    /// Index into the grammar table.
    GrammarIdx
);
index_type!(
    /// Index into the grammar rule table.
    RuleIdx
);
index_type!(
    /// Index into the enumeration table.
    EnumSetIdx
);
index_type!(
    /// Index into the string pool of the grammar tables.
    StrIdx
);

/// A qualified name (qname: a namespace URI plus a local name) stored as two
/// string-table compact identifiers. A compact identifier is the position of a
/// string in its string-table partition: `uri` in the URI partition, `local` in
/// that URI's local-name partition
/// ([EXI 7.3.1](https://www.w3.org/TR/exi/#stringTablePartitions)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QNameId {
    /// The URI.
    pub uri: UriId,
    /// The local name, inside the URI's partition.
    pub local: LocalId,
}

impl QNameId {
    /// The qname `{uri}local`.
    pub const fn new(uri: UriId, local: LocalId) -> Self {
        Self { uri, local }
    }
}
