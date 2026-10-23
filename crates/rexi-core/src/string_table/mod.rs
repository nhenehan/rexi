//! String tables ([EXI 7.3](https://www.w3.org/TR/exi/#stringTable)): URI, prefix,
//! local-name and value partitions.
//!
//! A string table remembers strings already seen in the stream. The second time a string
//! comes up, the encoder can write a short number, its **compact identifier**, instead of
//! the full text. The table is split into **partitions** (separate lists, each with its
//! own numbering, [EXI 7.3.1](https://www.w3.org/TR/exi/#stringTablePartitions)): one uri
//! partition, and for each URI one prefix partition and one local-name partition, plus
//! the value partitions.
//!
//! One [`StringTable`] serves both the encoder and the decoder. Both sides make the
//! same changes (a miss adds the string), so the two tables stay in step.
//!
//! * The **uri** partition and each per-URI **prefix** partition are tuned for compact
//!   identifiers ([EXI 7.3.2](https://www.w3.org/TR/exi/#encodingOptimizedForHits)): a
//!   hit is `id + 1` in `ceil_log2(m + 1)` bits, a miss is `0` in that width followed by
//!   a String. (`m` is the number of entries in the partition right now.)
//! * Each per-URI **local-name** partition is tuned for string literals
//!   ([EXI 7.3.3](https://www.w3.org/TR/exi/#encodingOptimizedForMisses)): a hit is
//!   Unsigned `0` then `id` in `ceil_log2(m)` bits, a miss is a String whose length is
//!   increased by one.
//! * The **value** partitions live in the private `value` module and are described there.
//!
//! Compact identifiers never move, except through [`StringTable::sort_local_names`],
//! which exists for tables built by adding names. Initial entries of a schema-informed
//! stream (the ones listed in
//! [EXI Appendix D](https://www.w3.org/TR/exi/#initialStringValues) plus the schema's
//! names) are referenced from the grammar tables, not copied, and found by binary search.
//! Strings added during the stream are owned and found through `BTreeMap`s.

mod initial;
mod value;
#[cfg(test)]
mod value_reference;

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::ops::Range;
use core::sync::atomic::{AtomicU8, Ordering};

pub use initial::{
    EMPTY_URI_ID, XML_LOCAL_NAMES, XML_NS_URI, XML_NS_URI_ID, XSD_LOCAL_NAMES, XSD_NS_URI,
    XSD_NS_URI_ID, XSI_LOCAL_NAMES, XSI_NS_URI, XSI_NS_URI_ID,
};

use crate::datatype::string::{
    read_chars, read_length, read_string, write_chars, write_length, write_string,
};
use crate::datatype::unsigned::write_unsigned;
use crate::error::{ExiError, ExiResult};
use crate::grammar::GrammarTables;
use crate::ids::{LocalId, NONE, PrefixId, QNameId, StrIdx, UriId};
use crate::io::{BitReader, BitWriter, ceil_log2};
use value::ValuePartitions;

/// One entry of the uri partition, together with the prefix and local-name
/// partitions that belong to that URI (EXIficient's `RuntimeUriContext`).
///
/// Entries given by the grammar
/// ([EXI Appendix D](https://www.w3.org/TR/exi/#initialStringValues) and the schema's
/// names) are not copied: the URI is a [`StrIdx`], and the initial prefixes and
/// local names are ranges of [`GrammarTables::prefixes`] /
/// [`GrammarTables::local_names`], sorted by code point
/// ([EXI D.3](https://www.w3.org/TR/exi/#initialLocalNames)) and found by binary search.
/// Strings added during the stream are owned, with identifiers continuing after the
/// initial ones.
///
/// So a compact identifier `id` below `initial_*.len()` names an initial entry, and
/// `id - initial_*.len()` is the position in the owned `Vec` otherwise.
///
/// Most streams add no names, so the added ones sit behind one optional box: a
/// new table is one small entry per URI, cheap to build and to drop.
#[derive(Debug, Clone, Default)]
struct UriEntry {
    uri: Text,
    initial_prefixes: Range<usize>,
    initial_locals: Range<usize>,
    added: Option<Box<AddedNames>>,
}

/// The prefixes and local names a stream added to one URI's partitions.
#[derive(Debug, Clone, Default)]
struct AddedNames {
    prefixes: Vec<String>,
    prefix_index: BTreeMap<String, PrefixId>,
    local_names: Vec<String>,
    local_index: BTreeMap<String, LocalId>,
}

impl UriEntry {
    fn added_prefixes(&self) -> &[String] {
        self.added.as_ref().map_or(&[], |a| a.prefixes.as_slice())
    }

    fn added_locals(&self) -> &[String] {
        self.added
            .as_ref()
            .map_or(&[], |a| a.local_names.as_slice())
    }

    fn added_mut(&mut self) -> &mut AddedNames {
        self.added.get_or_insert_with(Box::default)
    }
}

/// The text of a URI: stored in the grammar tables, or added during the stream.
#[derive(Debug, Clone)]
enum Text {
    Grammar(StrIdx),
    Owned(String),
}

impl Default for Text {
    fn default() -> Self {
        Text::Owned(String::new())
    }
}

/// The complete string table of one EXI stream.
///
/// Methods that need the text of initial entries take the grammar tables the
/// table was built from ([`StringTable::from_grammar`]). A table built with
/// [`StringTable::new`] has no initial entries and accepts any tables (for example
/// [`GrammarSet::EMPTY`](crate::grammar::GrammarSet::EMPTY)).
#[derive(Debug, Clone)]
pub struct StringTable {
    uris: Vec<UriEntry>,
    /// URIs added during the stream (initial URIs are found by a linear scan).
    uri_index: BTreeMap<String, UriId>,
    /// Number of leading entries of `uris` whose URI is `Text::Grammar`.
    initial_uris: usize,
    /// Whether every initial prefix and local-name partition is sorted with no
    /// duplicates, so lookups by text can use binary search. Worked out on the
    /// first such lookup: most streams (every strict schema-informed decode)
    /// never make one, so they never pay for the check.
    sorted: LazyFlag,
    values: ValuePartitions,
}

/// A yes/no answer worked out on first use. Atomic, so a table stays `Sync`.
#[derive(Debug)]
struct LazyFlag(AtomicU8);

const FLAG_UNKNOWN: u8 = 0;
const FLAG_YES: u8 = 1;
const FLAG_NO: u8 = 2;

impl Default for LazyFlag {
    fn default() -> Self {
        Self(AtomicU8::new(FLAG_UNKNOWN))
    }
}

impl Clone for LazyFlag {
    fn clone(&self) -> Self {
        Self(AtomicU8::new(self.0.load(Ordering::Relaxed)))
    }
}

impl Default for StringTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the strings at `idx` only go up (sorted, no duplicates). Binary search
/// in [`search`] needs this.
///
/// Neighbors are told apart by their first byte when they can be (most sorted
/// names differ there), before a full compare.
fn strictly_sorted<G: GrammarTables + ?Sized>(tables: &G, idx: &[StrIdx]) -> bool {
    let Some((&first, rest)) = idx.split_first() else {
        return true;
    };
    let mut prev = tables.string(first).as_bytes();
    for &i in rest {
        let cur = tables.string(i).as_bytes();
        let less = match (prev.first(), cur.first()) {
            (Some(a), Some(b)) if a != b => a < b,
            _ => prev < cur,
        };
        if !less {
            return false;
        }
        prev = cur;
    }
    true
}

/// Binary search of `key` among the strings at `idx` (sorted, no duplicates).
/// Returns the position in `idx`, which is the compact identifier.
fn search<G: GrammarTables + ?Sized>(tables: &G, idx: &[StrIdx], key: &str) -> Option<usize> {
    idx.binary_search_by(|&i| tables.string(i).cmp(key)).ok()
}

impl StringTable {
    /// An empty table with no limits on the value partitions. The codec fills in the
    /// starting entries from
    /// [EXI Appendix D](https://www.w3.org/TR/exi/#initialStringValues)
    /// ([`StringTable::add_initial_entries`]).
    pub fn new() -> Self {
        Self::with_limits(None, None)
    }

    /// An empty table with the given `valueMaxLength` and `valuePartitionCapacity` header
    /// options ([EXI 5.4](https://www.w3.org/TR/exi/#options)); `None` means no limit.
    pub fn with_limits(
        value_max_length: Option<u32>,
        value_partition_capacity: Option<u32>,
    ) -> Self {
        Self {
            uris: Vec::new(),
            uri_index: BTreeMap::new(),
            initial_uris: 0,
            sorted: LazyFlag::default(),
            values: ValuePartitions::new(value_max_length, value_partition_capacity),
        }
    }

    /// The table of a schema-informed stream (one whose grammars come from an XML
    /// Schema): the grammar's URIs, prefixes and local names become the initial
    /// entries, referenced rather than copied, so a compact identifier is
    /// the entry's position in its partition.
    ///
    /// Lookups by text use binary search when the partitions are sorted with no
    /// duplicates, as the spec orders them
    /// ([EXI D.3](https://www.w3.org/TR/exi/#initialLocalNames)); that is checked
    /// on the first lookup. Hand-built tables that are not sorted still
    /// work, through a linear scan.
    pub fn from_grammar<G: GrammarTables + ?Sized>(
        tables: &G,
        value_max_length: Option<u32>,
        value_partition_capacity: Option<u32>,
    ) -> Self {
        let mut st = Self::with_limits(value_max_length, value_partition_capacity);
        st.uris = tables
            .uris()
            .iter()
            .map(|e| UriEntry {
                uri: Text::Grammar(e.uri),
                initial_prefixes: e.prefixes.range(),
                initial_locals: e.local_names.range(),
                added: None,
            })
            .collect();
        st.initial_uris = st.uris.len();
        st
    }

    /// Whether the initial entries are referenced from grammar tables:
    /// true for every table made by [`StringTable::from_grammar`] from tables with
    /// at least one URI.
    pub fn borrows_initial_entries(&self) -> bool {
        self.initial_uris > 0
    }

    /// Whether every initial prefix and local-name partition is sorted by code
    /// point with no duplicates, as
    /// [EXI D.3](https://www.w3.org/TR/exi/#initialLocalNames) orders them, so
    /// lookups by text use binary search. Worked out once, then remembered.
    pub fn initial_entries_sorted<G: GrammarTables + ?Sized>(&self, tables: &G) -> bool {
        match self.sorted.0.load(Ordering::Relaxed) {
            FLAG_YES => return true,
            FLAG_NO => return false,
            _ => {}
        }
        let sorted = self
            .uris
            .get(..self.initial_uris)
            .unwrap_or_default()
            .iter()
            .all(|e| {
                tables
                    .prefixes()
                    .get(e.initial_prefixes.clone())
                    .is_some_and(|p| strictly_sorted(tables, p))
                    && tables
                        .local_names()
                        .get(e.initial_locals.clone())
                        .is_some_and(|l| strictly_sorted(tables, l))
            });
        let flag = if sorted { FLAG_YES } else { FLAG_NO };
        self.sorted.0.store(flag, Ordering::Relaxed);
        sorted
    }

    /// Position of `key` among the initial strings `idx`: binary search when the
    /// partitions are sorted, else the first match of a linear scan.
    fn find_initial<G: GrammarTables + ?Sized>(
        &self,
        tables: &G,
        idx: &[StrIdx],
        key: &str,
    ) -> Option<usize> {
        if idx.is_empty() {
            return None;
        }
        if self.initial_entries_sorted(tables) {
            search(tables, idx, key)
        } else {
            idx.iter().position(|&i| tables.string(i) == key)
        }
    }

    /// The `valueMaxLength` limit this table was made with.
    pub fn value_max_length(&self) -> Option<u32> {
        self.values.value_max_length()
    }

    /// The `valuePartitionCapacity` limit this table was made with.
    pub fn value_partition_capacity(&self) -> Option<u32> {
        self.values.value_partition_capacity()
    }

    // ------------------------------------------------------------------ population

    /// Adds `uri` to the uri partition and returns its compact identifier. If the URI
    /// is already there, nothing is added and its existing identifier is returned.
    pub fn add_uri<G: GrammarTables + ?Sized>(&mut self, tables: &G, uri: &str) -> UriId {
        if let Some(id) = self.find_uri(tables, uri) {
            return id;
        }
        let id = self.uris.len() as UriId;
        self.uris.push(UriEntry {
            uri: Text::Owned(uri.to_owned()),
            ..UriEntry::default()
        });
        self.uri_index.insert(uri.to_owned(), id);
        id
    }

    /// Adds `prefix` to the prefix partition of `uri`, returning its compact
    /// identifier (the existing one if present). Returns [`NONE`] when `uri` is
    /// not a valid identifier.
    pub fn add_prefix<G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        uri: UriId,
        prefix: &str,
    ) -> PrefixId {
        if let Some(id) = self.find_prefix(tables, uri, prefix) {
            return id;
        }
        let Some(entry) = self.uris.get_mut(uri as usize) else {
            return NONE;
        };
        let id = (entry.initial_prefixes.len() + entry.added_prefixes().len()) as PrefixId;
        let added = entry.added_mut();
        added.prefixes.push(prefix.to_owned());
        added.prefix_index.insert(prefix.to_owned(), id);
        id
    }

    /// Adds `local` to the local-name partition of `uri`, returning its compact
    /// identifier (the existing one if present). Returns [`NONE`] when `uri` is
    /// not a valid identifier.
    pub fn add_local_name<G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        uri: UriId,
        local: &str,
    ) -> LocalId {
        if let Some(id) = self.find_local_name(tables, uri, local) {
            return id;
        }
        let Some(entry) = self.uris.get_mut(uri as usize) else {
            return NONE;
        };
        let id = (entry.initial_locals.len() + entry.added_locals().len()) as LocalId;
        let added = entry.added_mut();
        added.local_names.push(local.to_owned());
        added.local_index.insert(local.to_owned(), id);
        id
    }

    /// Sorts every local-name partition in dictionary order (by code point) and gives out
    /// compact identifiers again in that order, as
    /// [EXI D.3](https://www.w3.org/TR/exi/#initialLocalNames) requires after
    /// schema-declared names have been added. Identifiers handed out earlier are no
    /// longer valid. Only for tables built by adding names (initial entries referenced
    /// from a grammar are sorted already and are left alone).
    pub fn sort_local_names(&mut self) {
        for entry in &mut self.uris {
            if !entry.initial_locals.is_empty() {
                continue;
            }
            let Some(added) = entry.added.as_deref_mut() else {
                continue;
            };
            added.local_names.sort_unstable();
            added.local_index.clear();
            for (id, name) in added.local_names.iter().enumerate() {
                added.local_index.insert(name.clone(), id as LocalId);
            }
        }
    }

    // --------------------------------------------------------------------- lookup

    /// The compact identifier of `uri`, if it is in the uri partition. Initial URIs
    /// are checked first by a linear scan, then the map of URIs added later.
    pub fn find_uri<G: GrammarTables + ?Sized>(&self, tables: &G, uri: &str) -> Option<UriId> {
        self.uris
            .get(..self.initial_uris)
            .unwrap_or_default()
            .iter()
            .position(|e| matches!(e.uri, Text::Grammar(i) if tables.string(i) == uri))
            .map(|i| i as UriId)
            .or_else(|| self.uri_index.get(uri).copied())
    }

    /// The compact identifier of `prefix` in the prefix partition of `uri`. Initial
    /// prefixes are found by binary search, added ones through the map.
    pub fn find_prefix<G: GrammarTables + ?Sized>(
        &self,
        tables: &G,
        uri: UriId,
        prefix: &str,
    ) -> Option<PrefixId> {
        let entry = self.uris.get(uri as usize)?;
        let initial = tables
            .prefixes()
            .get(entry.initial_prefixes.clone())
            .unwrap_or_default();
        self.find_initial(tables, initial, prefix)
            .map(|i| i as PrefixId)
            .or_else(|| entry.added.as_ref()?.prefix_index.get(prefix).copied())
    }

    /// The compact identifier of `local` in the local-name partition of `uri`.
    /// Initial names are found by binary search, added ones through the map.
    pub fn find_local_name<G: GrammarTables + ?Sized>(
        &self,
        tables: &G,
        uri: UriId,
        local: &str,
    ) -> Option<LocalId> {
        let entry = self.uris.get(uri as usize)?;
        let initial = tables
            .local_names()
            .get(entry.initial_locals.clone())
            .unwrap_or_default();
        self.find_initial(tables, initial, local)
            .map(|i| i as LocalId)
            .or_else(|| entry.added.as_ref()?.local_index.get(local).copied())
    }

    /// The text of URI `id`, or `None` if there is no such entry.
    pub fn uri<'a, G: GrammarTables + ?Sized>(
        &'a self,
        tables: &'a G,
        id: UriId,
    ) -> Option<&'a str> {
        self.uris.get(id as usize).map(|e| match &e.uri {
            Text::Grammar(i) => tables.string(*i),
            Text::Owned(s) => s.as_str(),
        })
    }

    /// The text of local name `id` of `uri`, or `None` if there is no such entry.
    pub fn local_name<'a, G: GrammarTables + ?Sized>(
        &'a self,
        tables: &'a G,
        uri: UriId,
        id: LocalId,
    ) -> Option<&'a str> {
        let entry = self.uris.get(uri as usize)?;
        let id = id as usize;
        match id.checked_sub(entry.initial_locals.len()) {
            None => tables
                .local_names()
                .get(entry.initial_locals.start + id)
                .map(|&i| tables.string(i)),
            Some(added) => entry.added_locals().get(added).map(String::as_str),
        }
    }

    /// The text of prefix `id` of `uri`, or `None` if there is no such entry.
    pub fn prefix<'a, G: GrammarTables + ?Sized>(
        &'a self,
        tables: &'a G,
        uri: UriId,
        id: PrefixId,
    ) -> Option<&'a str> {
        let entry = self.uris.get(uri as usize)?;
        let id = id as usize;
        match id.checked_sub(entry.initial_prefixes.len()) {
            None => tables
                .prefixes()
                .get(entry.initial_prefixes.start + id)
                .map(|&i| tables.string(i)),
            Some(added) => entry.added_prefixes().get(added).map(String::as_str),
        }
    }

    /// Number of entries in the uri partition.
    pub fn uri_count(&self) -> usize {
        self.uris.len()
    }

    /// Number of entries in the local-name partition of `uri` (0 for an unknown URI).
    pub fn local_name_count(&self, uri: UriId) -> usize {
        self.uris
            .get(uri as usize)
            .map_or(0, |e| e.initial_locals.len() + e.added_locals().len())
    }

    /// Number of entries in the prefix partition of `uri` (0 for an unknown URI).
    pub fn prefix_count(&self, uri: UriId) -> usize {
        self.uris
            .get(uri as usize)
            .map_or(0, |e| e.initial_prefixes.len() + e.added_prefixes().len())
    }

    /// The identifiers of the qname (qualified name: a URI plus a local name)
    /// `uri`/`local`, if both strings are in the table.
    pub fn qname_id<G: GrammarTables + ?Sized>(
        &self,
        tables: &G,
        uri: &str,
        local: &str,
    ) -> Option<QNameId> {
        let uri_id = self.find_uri(tables, uri)?;
        let local_id = self.find_local_name(tables, uri_id, local)?;
        Some(QNameId::new(uri_id, local_id))
    }

    /// Both strings of a qname, if the identifiers are valid.
    pub fn qname<'a, G: GrammarTables + ?Sized>(
        &'a self,
        tables: &'a G,
        id: QNameId,
    ) -> Option<(&'a str, &'a str)> {
        Some((
            self.uri(tables, id.uri)?,
            self.local_name(tables, id.uri, id.local)?,
        ))
    }

    /// Number of entries in the global value partition.
    pub fn global_value_count(&self) -> usize {
        self.values.global_count()
    }

    /// Number of entries in the local value partition of `context`, counting both
    /// assigned and evicted ones. (An entry is evicted when a newer value takes its
    /// global slot under `valuePartitionCapacity`; see the `value` module.)
    pub fn local_value_count(&self, context: QNameId) -> usize {
        self.values.local_count(context)
    }

    /// Whether `value` is currently in the global value partition.
    pub fn has_value(&self, value: &str) -> bool {
        self.values.contains(value)
    }

    // ------------------------------------------------------------ uri (spec 7.3.2)
    // https://www.w3.org/TR/exi/#encodingOptimizedForHits

    /// Writes a URI: hit = `id + 1` in `ceil_log2(m + 1)` bits; miss = `0` in that
    /// width, then the URI as a String, and the URI is added. `m` is counted before
    /// the add. Returns the URI's compact identifier.
    pub fn write_uri<W: ByteSink, G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        w: &mut BitWriter<W>,
        uri: &str,
    ) -> ExiResult<UriId> {
        let n = ceil_log2(self.uris.len() + 1);
        match self.find_uri(tables, uri) {
            Some(id) => {
                w.write_nbit_unsigned(u64::from(id) + 1, n)?;
                Ok(id)
            }
            None => {
                w.write_nbit_unsigned(0, n)?;
                write_string(w, uri)?;
                Ok(self.add_uri(tables, uri))
            }
        }
    }

    /// Reads a URI written by [`StringTable::write_uri`]. Fails if the identifier
    /// names no entry.
    pub fn read_uri<R: ByteSource, G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        r: &mut BitReader<R>,
    ) -> ExiResult<UriId> {
        let n = ceil_log2(self.uris.len() + 1);
        match r.read_nbit_unsigned(n)? {
            0 => {
                let uri = read_string(r)?;
                Ok(self.add_uri(tables, &uri))
            }
            id_plus_one => {
                let id = id_plus_one - 1;
                if (id as usize) < self.uris.len() {
                    Ok(id as UriId)
                } else {
                    Err(ExiError::StringTable("uri identifier out of range"))
                }
            }
        }
    }

    // --------------------------------------------------------- prefix (spec 7.3.2)
    // https://www.w3.org/TR/exi/#encodingOptimizedForHits

    /// Writes a prefix in the prefix partition of `uri`, the same way as
    /// [`StringTable::write_uri`]. Fails if `uri` names no entry.
    pub fn write_prefix<W: ByteSink, G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        w: &mut BitWriter<W>,
        uri: UriId,
        prefix: &str,
    ) -> ExiResult<PrefixId> {
        if (uri as usize) >= self.uris.len() {
            return Err(ExiError::StringTable("uri identifier out of range"));
        }
        let n = ceil_log2(self.prefix_count(uri) + 1);
        match self.find_prefix(tables, uri, prefix) {
            Some(id) => {
                w.write_nbit_unsigned(u64::from(id) + 1, n)?;
                Ok(id)
            }
            None => {
                w.write_nbit_unsigned(0, n)?;
                write_string(w, prefix)?;
                Ok(self.add_prefix(tables, uri, prefix))
            }
        }
    }

    /// Reads a prefix written by [`StringTable::write_prefix`].
    pub fn read_prefix<R: ByteSource, G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        r: &mut BitReader<R>,
        uri: UriId,
    ) -> ExiResult<PrefixId> {
        if (uri as usize) >= self.uris.len() {
            return Err(ExiError::StringTable("uri identifier out of range"));
        }
        let count = self.prefix_count(uri);
        let n = ceil_log2(count + 1);
        match r.read_nbit_unsigned(n)? {
            0 => {
                let prefix = read_string(r)?;
                Ok(self.add_prefix(tables, uri, &prefix))
            }
            id_plus_one => {
                let id = id_plus_one - 1;
                if (id as usize) < count {
                    Ok(id as PrefixId)
                } else {
                    Err(ExiError::StringTable("prefix identifier out of range"))
                }
            }
        }
    }

    // ----------------------------------------------------- local-name (spec 7.3.3)
    // https://www.w3.org/TR/exi/#encodingOptimizedForMisses

    /// Writes a local name in the local-name partition of `uri`: hit = Unsigned `0`
    /// ([EXI 7.1.6](https://www.w3.org/TR/exi/#encodingUnsignedInteger)), then `id` in
    /// `ceil_log2(m)` bits; miss = the String length plus one as an Unsigned, then the
    /// characters, and the name is added. Fails if `uri` names no entry.
    pub fn write_local_name<W: ByteSink, G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        w: &mut BitWriter<W>,
        uri: UriId,
        local: &str,
    ) -> ExiResult<LocalId> {
        if (uri as usize) >= self.uris.len() {
            return Err(ExiError::StringTable("uri identifier out of range"));
        }
        match self.find_local_name(tables, uri, local) {
            Some(id) => {
                write_unsigned(w, 0)?;
                let n = ceil_log2(self.local_name_count(uri));
                w.write_nbit_unsigned(u64::from(id), n)?;
                Ok(id)
            }
            None => {
                write_length(w, local.chars().count() as u64 + 1)?;
                write_chars(w, local)?;
                Ok(self.add_local_name(tables, uri, local))
            }
        }
    }

    /// Reads a local name written by [`StringTable::write_local_name`]. A first
    /// number of 0 means a hit; any other number is the length plus one.
    pub fn read_local_name<R: ByteSource, G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        r: &mut BitReader<R>,
        uri: UriId,
    ) -> ExiResult<LocalId> {
        if (uri as usize) >= self.uris.len() {
            return Err(ExiError::StringTable("uri identifier out of range"));
        }
        let count = self.local_name_count(uri);
        match read_length(r)? {
            0 => {
                let n = ceil_log2(count);
                let id = r.read_nbit_unsigned(n)?;
                if (id as usize) < count {
                    Ok(id as LocalId)
                } else {
                    Err(ExiError::StringTable("local-name identifier out of range"))
                }
            }
            len_plus_one => {
                let mut local = String::new();
                read_chars(r, len_plus_one - 1, &mut local)?;
                Ok(self.add_local_name(tables, uri, &local))
            }
        }
    }

    // ---------------------------------------------------------- qname (spec 7.1.7)
    // https://www.w3.org/TR/exi/#encodingQName

    /// Writes a QName as uri then local-name. The prefix, when it is kept, is left to
    /// the codec ([`StringTable::write_prefix`]).
    pub fn write_qname<W: ByteSink, G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        w: &mut BitWriter<W>,
        uri: &str,
        local: &str,
    ) -> ExiResult<QNameId> {
        let uri_id = self.write_uri(tables, w, uri)?;
        let local_id = self.write_local_name(tables, w, uri_id, local)?;
        Ok(QNameId::new(uri_id, local_id))
    }

    /// Reads a QName written by [`StringTable::write_qname`].
    pub fn read_qname<R: ByteSource, G: GrammarTables + ?Sized>(
        &mut self,
        tables: &G,
        r: &mut BitReader<R>,
    ) -> ExiResult<QNameId> {
        let uri_id = self.read_uri(tables, r)?;
        let local_id = self.read_local_name(tables, r, uri_id)?;
        Ok(QNameId::new(uri_id, local_id))
    }

    // ---------------------------------------------------------- value (spec 7.3.3)
    // https://www.w3.org/TR/exi/#encodingOptimizedForMisses

    /// Writes a value content item (attribute value or character data) in the
    /// context of the attribute (AT) or element (CH) qname `context`. The context
    /// picks the "local" value partition.
    pub fn write_value<W: ByteSink>(
        &mut self,
        w: &mut BitWriter<W>,
        context: QNameId,
        value: &str,
    ) -> ExiResult<()> {
        self.values.write(w, context, value)
    }

    /// Reads a value content item into `out`, which is cleared first.
    pub fn read_value<R: ByteSource>(
        &mut self,
        r: &mut BitReader<R>,
        context: QNameId,
        out: &mut String,
    ) -> ExiResult<()> {
        self.values.read(r, context, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::GrammarSet;

    const NO: &GrammarSet<'static> = &GrammarSet::EMPTY;
    use crate::datatype::unsigned::read_unsigned;

    const DEFAULT_URIS: [&str; 4] = ["", XML_NS_URI, XSI_NS_URI, XSD_NS_URI];

    fn default_table() -> StringTable {
        let mut t = StringTable::new();
        for uri in DEFAULT_URIS {
            t.add_uri(NO, uri);
        }
        t
    }

    fn finish(w: BitWriter<Vec<u8>>) -> Vec<u8> {
        w.finish().unwrap_or_default()
    }

    #[test]
    fn add_uri_is_idempotent_and_sequential() {
        let mut t = default_table();
        assert_eq!(t.uri_count(), 4);
        assert_eq!(t.add_uri(NO, XSI_NS_URI), 2);
        assert_eq!(t.add_uri(NO, "urn:a"), 4);
        assert_eq!(t.add_uri(NO, "urn:a"), 4);
        assert_eq!(t.find_uri(NO, "urn:a"), Some(4));
        assert_eq!(t.find_uri(NO, "urn:b"), None);
        assert_eq!(t.uri(NO, 4), Some("urn:a"));
        assert_eq!(t.uri(NO, 5), None);
    }

    #[test]
    fn invalid_uri_id_yields_none_sentinel() {
        let mut t = default_table();
        assert_eq!(t.add_prefix(NO, 99, "p"), NONE);
        assert_eq!(t.add_local_name(NO, 99, "l"), NONE);
        assert_eq!(t.local_name_count(99), 0);
        assert_eq!(t.prefix_count(99), 0);
        assert_eq!(t.local_name(NO, 99, 0), None);
        assert_eq!(t.prefix(NO, 99, 0), None);
        let mut w = BitWriter::new(Vec::new());
        assert_eq!(
            t.write_local_name(NO, &mut w, 99, "l"),
            Err(ExiError::StringTable("uri identifier out of range"))
        );
        assert_eq!(
            t.write_prefix(NO, &mut w, 99, "p"),
            Err(ExiError::StringTable("uri identifier out of range"))
        );
    }

    #[test]
    fn uri_hit_is_id_plus_one_in_ceil_log2_count_plus_one_bits() {
        let mut t = default_table();
        let mut w = BitWriter::new(Vec::new());
        // 4 URIs -> ceil_log2(5) = 3 bits; id 2 is written as 3.
        assert_eq!(t.write_uri(NO, &mut w, XSI_NS_URI), Ok(2));
        assert_eq!(w.bit_position(), 3);
        assert_eq!(t.write_uri(NO, &mut w, ""), Ok(0));
        assert_eq!(w.bit_position(), 6);
        let bytes = finish(w);
        // 3 (id 2 + 1) in 3 bits, 1 (id 0 + 1) in 3 bits, two padding bits.
        assert_eq!(bytes, vec![0b0110_0100]);
        let mut r = BitReader::new(bytes.as_slice());
        let mut d = default_table();
        assert_eq!(d.read_uri(NO, &mut r), Ok(2));
        assert_eq!(d.read_uri(NO, &mut r), Ok(0));
    }

    #[test]
    fn uri_miss_is_zero_then_string_and_grows_the_partition() {
        let mut t = default_table();
        let mut w = BitWriter::new(Vec::new());
        assert_eq!(t.write_uri(NO, &mut w, "urn:x"), Ok(4));
        assert_eq!(t.uri_count(), 5);
        // 0 in 3 bits, then String "urn:x": length 5 then 5 chars = 48 bits.
        assert_eq!(w.bit_position(), 3 + 8 + 5 * 8);
        // Now 5 entries -> ceil_log2(6) is still 3 bits; the new id 4 is written as 5.
        assert_eq!(t.write_uri(NO, &mut w, "urn:x"), Ok(4));
        assert_eq!(w.bit_position(), 3 + 48 + 3);
        let bytes = finish(w);
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(r.read_nbit_unsigned(3), Ok(0));
        assert_eq!(read_string(&mut r), Ok("urn:x".to_owned()));
        assert_eq!(r.read_nbit_unsigned(3), Ok(5));

        let mut d = default_table();
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(d.read_uri(NO, &mut r), Ok(4));
        assert_eq!(d.uri(NO, 4), Some("urn:x"));
        assert_eq!(d.read_uri(NO, &mut r), Ok(4));
        assert_eq!(d.uri_count(), 5);
    }

    #[test]
    fn uri_widths_follow_partition_growth() {
        // Empty partition: ceil_log2(1) = 0 bits, every write is a miss.
        let mut t = StringTable::new();
        let mut w = BitWriter::new(Vec::new());
        assert_eq!(t.write_uri(NO, &mut w, "a"), Ok(0));
        assert_eq!(w.bit_position(), 8 + 8);
        // 1 entry -> ceil_log2(2) = 1 bit.
        assert_eq!(t.write_uri(NO, &mut w, "a"), Ok(0));
        assert_eq!(w.bit_position(), 17);
        for (i, uri) in ["b", "c", "d", "e", "f", "g"].iter().enumerate() {
            assert_eq!(t.write_uri(NO, &mut w, uri), Ok(i as UriId + 1));
        }
        // 7 entries -> ceil_log2(8) = 3 bits; 8 entries -> ceil_log2(9) = 4 bits.
        let before = w.bit_position();
        assert_eq!(t.write_uri(NO, &mut w, "g"), Ok(6));
        assert_eq!(w.bit_position() - before, 3);
        assert_eq!(t.write_uri(NO, &mut w, "h"), Ok(7));
        let before = w.bit_position();
        assert_eq!(t.write_uri(NO, &mut w, "h"), Ok(7));
        assert_eq!(w.bit_position() - before, 4);
        let bytes = finish(w);
        let mut d = StringTable::new();
        let mut r = BitReader::new(bytes.as_slice());
        let got: Vec<ExiResult<UriId>> = (0..11).map(|_| d.read_uri(NO, &mut r)).collect();
        assert_eq!(got, [0, 0, 1, 2, 3, 4, 5, 6, 6, 7, 7].map(Ok));
    }

    #[test]
    fn read_uri_rejects_out_of_range_identifier() {
        let mut d = default_table();
        // 3 bits: 6 would mean id 5, but only 4 entries exist.
        let mut r = BitReader::new([0b110_00000u8].as_slice());
        assert_eq!(
            d.read_uri(NO, &mut r),
            Err(ExiError::StringTable("uri identifier out of range"))
        );
    }

    #[test]
    fn local_name_hit_and_miss() {
        let mut t = StringTable::with_initial_entries(false);
        let xml = XML_NS_URI_ID;
        let mut w = BitWriter::new(Vec::new());
        // Hit: Unsigned 0 (one octet), then id 2 ("lang") in ceil_log2(4) = 2 bits.
        assert_eq!(t.write_local_name(NO, &mut w, xml, "lang"), Ok(2));
        assert_eq!(w.bit_position(), 10);
        // Miss: length 3 + 1 = 4 as Unsigned, then 3 chars; added as id 4.
        assert_eq!(t.write_local_name(NO, &mut w, xml, "foo"), Ok(4));
        assert_eq!(w.bit_position(), 10 + 8 + 24);
        assert_eq!(t.local_name_count(xml), 5);
        assert_eq!(t.local_name(NO, xml, 4), Some("foo"));
        // Hit on the new entry: 5 entries -> 3 bits.
        assert_eq!(t.write_local_name(NO, &mut w, xml, "foo"), Ok(4));
        assert_eq!(w.bit_position(), 42 + 8 + 3);
        // Empty partition (uri 0): a hit would need ceil_log2(1) = 0 bits after a miss of one entry.
        assert_eq!(t.write_local_name(NO, &mut w, 0, "root"), Ok(0));
        assert_eq!(t.write_local_name(NO, &mut w, 0, "root"), Ok(0));
        let bytes = finish(w);

        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(read_unsigned(&mut r), Ok(0));
        assert_eq!(r.read_nbit_unsigned(2), Ok(2));
        assert_eq!(read_unsigned(&mut r), Ok(4));
        let mut s = String::new();
        assert_eq!(read_chars(&mut r, 3, &mut s), Ok(()));
        assert_eq!(s, "foo");
        assert_eq!(read_unsigned(&mut r), Ok(0));
        assert_eq!(r.read_nbit_unsigned(3), Ok(4));
        assert_eq!(read_unsigned(&mut r), Ok(5));

        let mut d = StringTable::with_initial_entries(false);
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(d.read_local_name(NO, &mut r, xml), Ok(2));
        assert_eq!(d.read_local_name(NO, &mut r, xml), Ok(4));
        assert_eq!(d.local_name(NO, xml, 4), Some("foo"));
        assert_eq!(d.read_local_name(NO, &mut r, xml), Ok(4));
        assert_eq!(d.read_local_name(NO, &mut r, 0), Ok(0));
        assert_eq!(d.read_local_name(NO, &mut r, 0), Ok(0));
        assert_eq!(d.local_name(NO, 0, 0), Some("root"));
    }

    #[test]
    fn local_name_length_counts_code_points() {
        let mut t = default_table();
        let mut w = BitWriter::new(Vec::new());
        assert_eq!(t.write_local_name(NO, &mut w, 0, "é€"), Ok(0));
        let bytes = finish(w);
        assert_eq!(bytes[0], 3);
        let mut d = default_table();
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(d.read_local_name(NO, &mut r, 0), Ok(0));
        assert_eq!(d.local_name(NO, 0, 0), Some("é€"));
    }

    #[test]
    fn read_local_name_rejects_out_of_range_identifier() {
        let mut d = StringTable::with_initial_entries(false);
        // XSI has 2 names -> 1 bit; XML has 4 -> 2 bits. A hit with id 3 is fine
        // for XML but there is no way to express an invalid id there, so use an
        // empty partition where a hit must fail.
        let mut r = BitReader::new([0u8].as_slice());
        assert_eq!(
            d.read_local_name(NO, &mut r, 0),
            Err(ExiError::StringTable("local-name identifier out of range"))
        );
    }

    #[test]
    fn prefix_hit_and_miss() {
        let mut t = StringTable::with_initial_entries(false);
        let xml = XML_NS_URI_ID;
        let mut w = BitWriter::new(Vec::new());
        // 1 prefix -> ceil_log2(2) = 1 bit; "xml" is id 0, written as 1.
        assert_eq!(t.write_prefix(NO, &mut w, xml, "xml"), Ok(0));
        assert_eq!(w.bit_position(), 1);
        // Miss: 0 in 1 bit, then String "x" (length 1 + 1 char), added as id 1.
        assert_eq!(t.write_prefix(NO, &mut w, xml, "x"), Ok(1));
        assert_eq!(w.bit_position(), 1 + 1 + 16);
        // 2 prefixes -> ceil_log2(3) = 2 bits.
        assert_eq!(t.write_prefix(NO, &mut w, xml, "x"), Ok(1));
        assert_eq!(w.bit_position(), 20);
        assert_eq!(t.prefix_count(xml), 2);
        assert_eq!(t.find_prefix(NO, xml, "x"), Some(1));
        let bytes = finish(w);
        let mut d = StringTable::with_initial_entries(false);
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(d.read_prefix(NO, &mut r, xml), Ok(0));
        assert_eq!(d.read_prefix(NO, &mut r, xml), Ok(1));
        assert_eq!(d.read_prefix(NO, &mut r, xml), Ok(1));
        assert_eq!(d.prefix(NO, xml, 1), Some("x"));
        assert_eq!(
            d.read_prefix(NO, &mut r, 99),
            Err(ExiError::StringTable("uri identifier out of range"))
        );
    }

    #[test]
    fn qname_round_trip_and_lookup() {
        let mut t = StringTable::with_initial_entries(true);
        let mut w = BitWriter::new(Vec::new());
        let seq = [
            (XSI_NS_URI, "type"),
            ("urn:ex", "root"),
            ("urn:ex", "child"),
            ("urn:ex", "root"),
            ("", "a"),
            (XSD_NS_URI, "dateTime"),
        ];
        let mut ids = Vec::new();
        for (uri, local) in seq {
            ids.push(
                t.write_qname(NO, &mut w, uri, local)
                    .unwrap_or(QNameId::new(NONE, NONE)),
            );
        }
        assert_eq!(ids[0], QNameId::new(2, 1));
        assert_eq!(ids[1], QNameId::new(4, 0));
        assert_eq!(ids[2], QNameId::new(4, 1));
        assert_eq!(ids[3], ids[1]);
        assert_eq!(ids[4], QNameId::new(0, 0));
        assert_eq!(ids[5], QNameId::new(3, 18));
        assert_eq!(t.qname_id(NO, "urn:ex", "child"), Some(ids[2]));
        assert_eq!(t.qname_id(NO, "urn:ex", "nope"), None);
        assert_eq!(t.qname_id(NO, "urn:nope", "root"), None);
        assert_eq!(t.qname(NO, ids[2]), Some(("urn:ex", "child")));
        assert_eq!(t.qname(NO, QNameId::new(4, 9)), None);
        let bytes = finish(w);
        let mut d = StringTable::with_initial_entries(true);
        let mut r = BitReader::new(bytes.as_slice());
        for (i, (uri, local)) in seq.iter().enumerate() {
            assert_eq!(d.read_qname(NO, &mut r), Ok(ids[i]));
            assert_eq!(d.qname(NO, ids[i]), Some((*uri, *local)));
        }
    }

    #[test]
    fn value_first_miss_then_local_then_global_hit() {
        let mut t = default_table();
        let at = QNameId::new(0, 0);
        let el = QNameId::new(0, 1);
        let mut w = BitWriter::new(Vec::new());
        // Miss: length 3 + 2 = 5, then "abc".
        assert_eq!(t.write_value(&mut w, at, "abc"), Ok(()));
        assert_eq!(w.bit_position(), 32);
        // Same context: local hit, Unsigned 0 then id 0 in ceil_log2(1) = 0 bits.
        assert_eq!(t.write_value(&mut w, at, "abc"), Ok(()));
        assert_eq!(w.bit_position(), 40);
        // Different context: global hit, Unsigned 1 then id 0 in ceil_log2(1) = 0 bits.
        assert_eq!(t.write_value(&mut w, el, "abc"), Ok(()));
        assert_eq!(w.bit_position(), 48);
        let bytes = finish(w);
        assert_eq!(bytes, vec![5, b'a', b'b', b'c', 0, 1]);
        assert_eq!(t.global_value_count(), 1);
        assert_eq!(t.local_value_count(at), 1);
        assert_eq!(t.local_value_count(el), 0);
        assert!(t.has_value("abc"));

        let mut d = default_table();
        let mut r = BitReader::new(bytes.as_slice());
        let mut s = String::from("stale");
        for ctx in [at, at, el] {
            assert_eq!(d.read_value(&mut r, ctx, &mut s), Ok(()));
            assert_eq!(s, "abc");
        }
    }

    #[test]
    fn value_limits_are_reported() {
        let t = StringTable::with_limits(Some(10), Some(100));
        assert_eq!(t.value_max_length(), Some(10));
        assert_eq!(t.value_partition_capacity(), Some(100));
        let t = StringTable::default();
        assert_eq!(t.value_max_length(), None);
        assert_eq!(t.value_partition_capacity(), None);
    }

    #[test]
    fn byte_aligned_mode_round_trips_everything() {
        let mut t = StringTable::with_initial_entries(true);
        let mut w = BitWriter::with_alignment(Vec::new(), true);
        let ctx = QNameId::new(0, 0);
        assert_eq!(
            t.write_qname(NO, &mut w, "urn:ex", "root"),
            Ok(QNameId::new(4, 0))
        );
        assert_eq!(
            t.write_qname(NO, &mut w, "urn:ex", "root"),
            Ok(QNameId::new(4, 0))
        );
        assert_eq!(t.write_prefix(NO, &mut w, 4, "ex"), Ok(0));
        assert_eq!(t.write_prefix(NO, &mut w, 4, "ex"), Ok(0));
        assert_eq!(
            t.write_qname(NO, &mut w, XML_NS_URI, "lang"),
            Ok(QNameId::new(1, 2))
        );
        for v in ["one", "two", "one", "", "three", "two"] {
            assert_eq!(t.write_value(&mut w, ctx, v), Ok(()));
        }
        assert_eq!(t.write_value(&mut w, QNameId::new(4, 0), "three"), Ok(()));
        // In byte-aligned mode the 3-bit uri hit (5 entries) takes a whole octet.
        let bytes = finish(w);
        let mut r = BitReader::with_alignment(bytes.as_slice(), true);
        assert_eq!(r.read_nbit_unsigned(3), Ok(0));
        assert_eq!(r.bit_position(), 8);

        let mut d = StringTable::with_initial_entries(true);
        let mut r = BitReader::with_alignment(bytes.as_slice(), true);
        assert_eq!(d.read_qname(NO, &mut r), Ok(QNameId::new(4, 0)));
        assert_eq!(d.read_qname(NO, &mut r), Ok(QNameId::new(4, 0)));
        assert_eq!(d.read_prefix(NO, &mut r, 4), Ok(0));
        assert_eq!(d.read_prefix(NO, &mut r, 4), Ok(0));
        assert_eq!(d.read_qname(NO, &mut r), Ok(QNameId::new(1, 2)));
        let mut s = String::new();
        for v in ["one", "two", "one", "", "three", "two"] {
            assert_eq!(d.read_value(&mut r, ctx, &mut s), Ok(()));
            assert_eq!(s, v);
        }
        assert_eq!(d.read_value(&mut r, QNameId::new(4, 0), &mut s), Ok(()));
        assert_eq!(s, "three");
        assert_eq!(d.uri(NO, 4), Some("urn:ex"));
        assert_eq!(d.prefix(NO, 4, 0), Some("ex"));
    }

    /// Deterministic pseudo-random generator (no dependencies).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[(self.next() % items.len() as u64) as usize]
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Op {
        Qname(String, String),
        Prefix(UriId, String),
        Value(QNameId, String),
    }

    fn random_ops(seed: u64, n: usize) -> Vec<Op> {
        let uris = ["", XML_NS_URI, "urn:a", "urn:b", "urn:c"];
        let locals = ["x", "y", "z", "lang", "élan", "w"];
        let prefixes = ["p", "q", "xml", ""];
        let values = ["", "1", "22", "333", "4444", "€uro", "xyzzy", "a b"];
        let mut rng = Lcg(seed);
        (0..n)
            .map(|_| match rng.next() % 4 {
                0 => Op::Qname(rng.pick(&uris).to_owned(), rng.pick(&locals).to_owned()),
                1 => Op::Prefix((rng.next() % 3) as UriId, rng.pick(&prefixes).to_owned()),
                _ => Op::Value(
                    QNameId::new((rng.next() % 3) as UriId, (rng.next() % 3) as LocalId),
                    rng.pick(&values).to_owned(),
                ),
            })
            .collect()
    }

    fn check_random_sequence(
        seed: u64,
        byte_aligned: bool,
        max_len: Option<u32>,
        capacity: Option<u32>,
    ) {
        let ops = random_ops(seed, 300);
        let mut enc = StringTable::with_limits(max_len, capacity);
        enc.add_initial_entries(false);
        let mut w = BitWriter::with_alignment(Vec::new(), byte_aligned);
        let mut expected = Vec::new();
        for op in &ops {
            match op {
                Op::Qname(u, l) => {
                    expected.push(format!("{:?}", enc.write_qname(NO, &mut w, u, l)))
                }
                Op::Prefix(u, p) => {
                    expected.push(format!("{:?}", enc.write_prefix(NO, &mut w, *u, p)))
                }
                Op::Value(c, v) => {
                    assert_eq!(enc.write_value(&mut w, *c, v), Ok(()));
                    expected.push(v.clone());
                }
            }
        }
        let bytes = finish(w);
        let mut dec = StringTable::with_limits(max_len, capacity);
        dec.add_initial_entries(false);
        let mut r = BitReader::with_alignment(bytes.as_slice(), byte_aligned);
        let mut s = String::new();
        for (op, want) in ops.iter().zip(&expected) {
            let got = match op {
                Op::Qname(u, l) => {
                    let id = dec.read_qname(NO, &mut r);
                    if let Ok(id) = id {
                        assert_eq!(dec.qname(NO, id), Some((u.as_str(), l.as_str())));
                    }
                    format!("{id:?}")
                }
                Op::Prefix(u, p) => {
                    let id = dec.read_prefix(NO, &mut r, *u);
                    if let Ok(id) = id {
                        assert_eq!(dec.prefix(NO, *u, id), Some(p.as_str()));
                    }
                    format!("{id:?}")
                }
                Op::Value(c, _) => {
                    assert_eq!(dec.read_value(&mut r, *c, &mut s), Ok(()));
                    s.clone()
                }
            };
            assert_eq!(&got, want, "seed {seed} aligned {byte_aligned} op {op:?}");
        }
        assert_eq!(enc.uri_count(), dec.uri_count());
        assert_eq!(enc.global_value_count(), dec.global_value_count());
        assert_eq!(r.bit_position().div_ceil(8), bytes.len() as u64);
    }

    #[test]
    fn random_sequences_round_trip() {
        for seed in 1..=8 {
            for byte_aligned in [false, true] {
                check_random_sequence(seed, byte_aligned, None, None);
                check_random_sequence(seed, byte_aligned, Some(3), Some(5));
                check_random_sequence(seed, byte_aligned, None, Some(0));
            }
        }
    }
}
