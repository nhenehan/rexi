//! Index types shared by the string tables and the grammar tables.
//!
//! Every index is a plain `u32` into a flat table. That way a grammar set can be
//! `static` data built ahead of time, or `Vec`s built at run time, with the same
//! layout.

/// Index of a URI in the uri partition of the string table.
pub type UriId = u32;
/// Index of a local name inside its URI's local-name partition.
pub type LocalId = u32;
/// Index of a prefix inside its URI's prefix partition.
pub type PrefixId = u32;
/// Index into the type table.
pub type TypeIdx = u32;
/// Index into the element declaration table.
pub type ElemIdx = u32;
/// Index into the grammar table.
pub type GrammarIdx = u32;
/// Index into the grammar rule table.
pub type RuleIdx = u32;
/// Index into the enumeration table.
pub type EnumSetIdx = u32;
/// Index into the string pool of the grammar tables.
pub type StrIdx = u32;

/// Marker value that means "no index".
pub const NONE: u32 = u32::MAX;

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
