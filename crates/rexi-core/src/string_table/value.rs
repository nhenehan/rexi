//! Value partitions ([EXI 7.3.3](https://www.w3.org/TR/exi/#encodingOptimizedForMisses)):
//! the global value partition plus one "local" partition per context qname.
//!
//! A value is an attribute value or a piece of character data. Each value that is
//! added goes into two lists at once:
//!
//! * the **global** value partition, shared by the whole stream, where it gets a
//!   **global identifier**;
//! * the **local** value partition of its context qname (the attribute or element it
//!   appeared in), where it gets a **local identifier**.
//!
//! On the wire a value is one of:
//!
//! * Unsigned `0`, then its local id in `ceil_log2(m)` bits (`m` = local partition
//!   size): used when the value is found and was added under the same context;
//! * Unsigned `1`, then its global id in `ceil_log2(m)` bits (`m` = global partition
//!   size): used when it is found but under another context;
//! * Unsigned `length + 2`, then the characters: a miss. The value is then added,
//!   unless its length is 0, is above `valueMaxLength`, or `valuePartitionCapacity`
//!   is 0.
//!
//! **Eviction.** With `valuePartitionCapacity = c`, the global partition holds at
//! most `c` values. The spec's `globalID` counter says which global slot the next
//! value takes; it goes 0, 1, ..., c - 1 and then back to 0. When it comes back to a
//! slot already in use, the new value replaces the old one in the global partition,
//! and the old value's local identifier becomes "permanently unassigned": no value
//! will ever use that local id again, but it still counts in its local partition's
//! size `m`.
//!
//! Same behavior as EXIficient's `StringEncoderImpl` / `StringDecoderImpl` and
//! their `Bounded*` variants, in one structure for both directions. The storage
//! is laid out for few allocations:
//!
//! * `entries` is the append-only history of added values (new ones are only ever
//!   added at the end): text range in one arena (`text`, a single `String` that
//!   holds all the text back to back), context qname, local identifier, and the
//!   global identifier while the value is live. Without `valuePartitionCapacity`,
//!   entry `i` is global identifier `i`; with it, `global` maps each global slot to
//!   its current entry, and an evicted entry stays (as a permanently unassigned local
//!   identifier still counted in the partition size `m`);
//! * up to [`SMALL`] entries, lookups scan `entries`; beyond, a per-context
//!   index of entries is built, and a hash index (FNV-1a over the arena text,
//!   open addressing) at the first lookup by text; both are then kept up to
//!   date. Only the encoder looks values up by text, so a decoder never builds
//!   the hash index.
//!
//! A small table therefore costs two allocations (arena and entries), where the
//! previous layout (kept in `value_reference.rs` as the test model) cost two
//! `String`s and several `BTreeMap` nodes per value.

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;
use alloc::collections::BTreeMap;

use crate::datatype::string::{read_chars, write_chars, write_length};
use crate::datatype::unsigned::{read_unsigned, write_unsigned};
use crate::error::{ExiError, ExiResult};
use crate::ids::{LocalId, QNameId};
use crate::io::{BitReader, BitWriter, ceil_log2};

/// Tables up to this many entries are searched by a linear scan.
/// When the 17th entry is added, `by_context` is built; the hash index is built
/// at the first lookup by text after that (the encoder's).
const SMALL: usize = 16;

/// Marker value for an empty slot of the hash index.
const EMPTY: u32 = u32::MAX;

/// One value ever added. Entries are never removed; an evicted value keeps its
/// entry with `global_id = None`.
#[derive(Debug, Clone, Copy)]
struct Entry {
    /// Byte range in the arena (meaningless once evicted).
    start: u32,
    len: u32,
    /// The qname whose local partition holds this value.
    context: QNameId,
    /// Compact identifier in `context`'s local partition.
    local_id: LocalId,
    /// Global compact identifier while the value is in the table. `None` once the
    /// value has been evicted (its local id is then permanently unassigned).
    global_id: Option<u32>,
}

/// The global value partition and every local one, in one structure.
#[derive(Debug, Clone)]
pub(crate) struct ValuePartitions {
    value_max_length: Option<u32>,
    value_partition_capacity: Option<u32>,
    /// The text of every live value, back to back (the arena). Evicted text stays
    /// here until [`ValuePartitions::compact`] drops it.
    text: String,
    /// Bytes of `text` that belong to evicted values.
    dead: usize,
    entries: Vec<Entry>,
    /// With a capacity: entry index of each used global slot. It grows to the
    /// capacity and then stays that long. Empty and unused without a capacity.
    global: Vec<u32>,
    /// Hash index of live entries by text (built beyond `SMALL` entries). Each slot
    /// holds an entry index or [`EMPTY`]. Its length is always a power of two, so
    /// `fnv1a(text) & (len - 1)` picks the first slot to try. Collisions use linear
    /// probing (try the next slot). Empty means the table is still small and is
    /// scanned instead.
    hash: Vec<u32>,
    /// Occupied hash slots, live or evicted (for the load factor: how full the hash
    /// index is).
    hash_used: usize,
    /// Entries of each context in local-identifier order (built beyond `SMALL`), so
    /// position `k` in a context's list is local id `k`.
    by_context: Option<BTreeMap<QNameId, Vec<u32>>>,
    /// Spec `globalID` ([EXI 7.3.1](https://www.w3.org/TR/exi/#stringTablePartitions)):
    /// the identifier the next added value receives.
    next_global: u32,
}

/// FNV-1a, 32 bits.
fn fnv1a(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

impl ValuePartitions {
    /// Empty partitions with the given `valueMaxLength` and
    /// `valuePartitionCapacity`; `None` means no limit.
    pub(crate) fn new(
        value_max_length: Option<u32>,
        value_partition_capacity: Option<u32>,
    ) -> Self {
        Self {
            value_max_length,
            value_partition_capacity,
            text: String::new(),
            dead: 0,
            entries: Vec::new(),
            global: Vec::new(),
            hash: Vec::new(),
            hash_used: 0,
            by_context: None,
            next_global: 0,
        }
    }

    pub(crate) fn value_max_length(&self) -> Option<u32> {
        self.value_max_length
    }

    pub(crate) fn value_partition_capacity(&self) -> Option<u32> {
        self.value_partition_capacity
    }

    /// Number of entries in the global value partition. With a capacity this stops
    /// growing at the capacity; without one, nothing is ever evicted, so it is the
    /// number of entries.
    pub(crate) fn global_count(&self) -> usize {
        if self.value_partition_capacity.is_some() {
            self.global.len()
        } else {
            self.entries.len()
        }
    }

    /// Number of entries (assigned or evicted) in `context`'s local partition. Uses
    /// `by_context` when built, else counts by scanning.
    pub(crate) fn local_count(&self, context: QNameId) -> usize {
        match &self.by_context {
            Some(map) => map.get(&context).map_or(0, Vec::len),
            None => self.entries.iter().filter(|e| e.context == context).count(),
        }
    }

    /// Whether `value` is currently present in the global partition.
    pub(crate) fn contains(&self, value: &str) -> bool {
        self.find(value).is_some()
    }

    /// The text of entry `e`. Empty for an evicted entry after compaction.
    fn text_of(&self, e: &Entry) -> &str {
        self.text
            .get(e.start as usize..(e.start + e.len) as usize)
            .unwrap_or_default()
    }

    /// The entry index of the live value `value`. Evicted entries never match.
    ///
    /// Small table: linear scan. Otherwise: start at the slot picked by the hash and
    /// step to the next slot until the value or an empty slot turns up. Evicted
    /// entries still fill their slots (so the probe chain stays unbroken) but are
    /// skipped.
    fn find(&self, value: &str) -> Option<usize> {
        if self.hash.is_empty() {
            return self
                .entries
                .iter()
                .position(|e| e.global_id.is_some() && self.text_of(e) == value);
        }
        let mask = self.hash.len() - 1;
        let mut i = fnv1a(value.as_bytes()) as usize & mask;
        loop {
            let slot = *self.hash.get(i)?;
            if slot == EMPTY {
                return None;
            }
            let e = self.entries.get(slot as usize)?;
            if e.global_id.is_some() && self.text_of(e) == value {
                return Some(slot as usize);
            }
            i = (i + 1) & mask;
        }
    }

    /// The entry index currently holding global identifier `global_id`.
    fn entry_of_global(&self, global_id: usize) -> Option<usize> {
        if self.value_partition_capacity.is_some() {
            self.global.get(global_id).map(|&e| e as usize)
        } else {
            (global_id < self.entries.len()).then_some(global_id)
        }
    }

    /// The entry index of local identifier `local_id` of `context`.
    fn entry_of_local(&self, context: QNameId, local_id: usize) -> Option<usize> {
        match &self.by_context {
            Some(map) => map.get(&context)?.get(local_id).map(|&e| e as usize),
            None => self
                .entries
                .iter()
                .position(|e| e.context == context && e.local_id as usize == local_id),
        }
    }

    /// Whether a string of `length` code points would be added after a miss: not
    /// when it is empty, not when `valuePartitionCapacity` is 0, and not when it is
    /// longer than `valueMaxLength`.
    fn is_addable(&self, length: usize) -> bool {
        if length == 0 || self.value_partition_capacity == Some(0) {
            return false;
        }
        match self.value_max_length {
            Some(max) => length <= max as usize,
            None => true,
        }
    }

    /// Adds `value` to the global partition and to `context`'s local partition,
    /// evicting under `valuePartitionCapacity` exactly as
    /// [EXI 7.3.3](https://www.w3.org/TR/exi/#encodingOptimizedForMisses) requires.
    ///
    /// The new local id is the current local size (evicted ids included). The new
    /// global id is `next_global`, which then moves on by one, wrapping to 0 at the
    /// capacity.
    ///
    /// The caller has already checked [`Self::is_addable`] and that the value is
    /// not present.
    fn add(&mut self, context: QNameId, value: &str) {
        let index = self.entries.len() as u32;
        // With the per-context index, one lookup gives the local id and records
        // the new entry.
        let local_id = match &mut self.by_context {
            Some(map) => {
                let list = map.entry(context).or_insert_with(|| Vec::with_capacity(8));
                list.push(index);
                list.len() - 1
            }
            None => self.local_count(context),
        } as LocalId;
        let global_id = self.next_global;
        match self.value_partition_capacity {
            Some(capacity) if (global_id as usize) < self.global.len() => {
                // Slot already in use: the new value replaces the old one, whose
                // local identifier becomes permanently unassigned.
                if let Some(old) = self
                    .global
                    .get(global_id as usize)
                    .and_then(|&e| self.entries.get_mut(e as usize))
                {
                    old.global_id = None;
                    self.dead += old.len as usize;
                }
                if let Some(slot) = self.global.get_mut(global_id as usize) {
                    *slot = index;
                }
                self.next_global = (global_id + 1) % capacity;
            }
            Some(capacity) => {
                self.global.push(index);
                self.next_global = (global_id + 1) % capacity;
            }
            None => self.next_global = global_id + 1,
        }
        // Compact the arena once more than 1024 bytes and more than half of it are
        // dead. This keeps the arena bounded when values keep getting evicted.
        // The hash index stores entry indexes, not text positions, so it stays valid.
        if self.dead > 1024 && self.dead * 2 > self.text.len() {
            self.compact();
        }
        if self.entries.is_empty() {
            // The first value: make room for a small table at once, instead of
            // growing step by step (most streams stay small).
            self.entries.reserve(SMALL);
            self.text.reserve(16 * SMALL);
        }
        let start = self.text.len() as u32;
        self.text.push_str(value);
        self.entries.push(Entry {
            start,
            len: value.len() as u32,
            context,
            local_id,
            global_id: Some(global_id),
        });
        if !self.hash.is_empty() {
            self.hash_insert(index);
        }
        // Leaving the small range: build the per-context index once; from now on
        // it is kept up to date above. The hash index is built only when a value
        // is looked up by text (see `write`), which a decoder never does.
        if self.entries.len() > SMALL && self.by_context.is_none() {
            self.build_context_index();
        }
    }

    /// Builds the per-context index once the table outgrows a linear scan.
    fn build_context_index(&mut self) {
        let mut map: BTreeMap<QNameId, Vec<u32>> = BTreeMap::new();
        for (i, e) in self.entries.iter().enumerate() {
            map.entry(e.context).or_default().push(i as u32);
        }
        self.by_context = Some(map);
    }

    /// Rebuilds the hash index from the live entries, with room to grow: at least
    /// 4 slots per live entry, at least 64, rounded up to a power of two. Evicted
    /// entries are left out, which frees the slots they held.
    fn rehash(&mut self) {
        let live = self
            .entries
            .iter()
            .filter(|e| e.global_id.is_some())
            .count();
        let size = (live * 4).max(64).next_power_of_two();
        self.hash.clear();
        self.hash.resize(size, EMPTY);
        self.hash_used = 0;
        for i in 0..self.entries.len() {
            if self.entries.get(i).is_some_and(|e| e.global_id.is_some()) {
                self.hash_insert(i as u32);
            }
        }
    }

    /// Puts entry `index` in the first empty slot from its hash on. If that would
    /// make the index more than half full, it rebuilds instead.
    fn hash_insert(&mut self, index: u32) {
        if (self.hash_used + 1) * 2 > self.hash.len() {
            // Evicted entries keep their slots until a rebuild; this one is
            // live and is placed by the rebuild.
            self.rehash();
            return;
        }
        let Some(e) = self.entries.get(index as usize) else {
            return;
        };
        let mask = self.hash.len() - 1;
        let mut i = fnv1a(self.text_of(e).as_bytes()) as usize & mask;
        while let Some(slot) = self.hash.get_mut(i) {
            if *slot == EMPTY {
                *slot = index;
                self.hash_used += 1;
                return;
            }
            i = (i + 1) & mask;
        }
    }

    /// Drops the text of evicted values from the arena by copying the live text into
    /// a new `String` and updating each live entry's start. Evicted entries get an
    /// empty range.
    fn compact(&mut self) {
        let mut text = String::with_capacity(self.text.len() - self.dead);
        for e in &mut self.entries {
            if e.global_id.is_some() {
                let s = self
                    .text
                    .get(e.start as usize..(e.start + e.len) as usize)
                    .unwrap_or_default();
                e.start = text.len() as u32;
                text.push_str(s);
            } else {
                e.start = 0;
                e.len = 0;
            }
        }
        self.text = text;
        self.dead = 0;
    }

    /// Writes `value` in `context`: local hit, global hit or literal (see the module
    /// docs). A value found under the same context always uses the local form.
    pub(crate) fn write<W: ByteSink>(
        &mut self,
        w: &mut BitWriter<W>,
        context: QNameId,
        value: &str,
    ) -> ExiResult<()> {
        if self.hash.is_empty() && self.entries.len() > SMALL {
            self.rehash();
        }
        if let Some(index) = self.find(value) {
            let e = *self
                .entries
                .get(index)
                .ok_or(ExiError::Internal("value index points outside the table"))?;
            if e.context == context {
                write_unsigned(w, 0)?;
                let n = ceil_log2(self.local_count(context));
                w.write_nbit_unsigned(u64::from(e.local_id), n)
            } else {
                write_unsigned(w, 1)?;
                let n = ceil_log2(self.global_count());
                let global_id = e
                    .global_id
                    .ok_or(ExiError::Internal("found value is not live"))?;
                w.write_nbit_unsigned(u64::from(global_id), n)
            }
        } else {
            let length = value.chars().count();
            write_length(w, length as u64 + 2)?;
            write_chars(w, value)?;
            if self.is_addable(length) {
                self.add(context, value);
            }
            Ok(())
        }
    }

    /// Reads a value written by [`ValuePartitions::write`] into `out`, which is
    /// cleared first. Fails on a local id that is out of range or unassigned, or a
    /// global id that is out of range. After a literal, the value is added under the
    /// same rules as the encoder (without first looking for it, as
    /// EXIficient's decoder does).
    pub(crate) fn read<R: ByteSource>(
        &mut self,
        r: &mut BitReader<R>,
        context: QNameId,
        out: &mut String,
    ) -> ExiResult<()> {
        out.clear();
        match read_unsigned(r)? {
            0 => {
                let n = ceil_log2(self.local_count(context));
                let local_id = r.read_nbit_unsigned(n)? as usize;
                let index = self
                    .entry_of_local(context, local_id)
                    .ok_or(ExiError::StringTable("local value identifier out of range"))?;
                let e = self.entries.get(index).ok_or(ExiError::Internal(
                    "local partition points outside the table",
                ))?;
                if e.global_id.is_none() {
                    return Err(ExiError::StringTable(
                        "local value identifier is unassigned",
                    ));
                }
                out.push_str(self.text_of(e));
                Ok(())
            }
            1 => {
                let n = ceil_log2(self.global_count());
                let global_id = r.read_nbit_unsigned(n)? as usize;
                let e = self
                    .entry_of_global(global_id)
                    .and_then(|i| self.entries.get(i))
                    .ok_or(ExiError::StringTable(
                        "global value identifier out of range",
                    ))?;
                out.push_str(self.text_of(e));
                Ok(())
            }
            len_plus_two => {
                let length = len_plus_two - 2;
                read_chars(r, length, out)?;
                let length = usize::try_from(length)
                    .map_err(|_| ExiError::StringTable("value length exceeds address space"))?;
                // Added as EXIficient's decoder does, without a search: a literal
                // means the encoder did not find the value (EXI 7.3.3).
                if self.is_addable(length) {
                    self.add(context, out);
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: QNameId = QNameId::new(0, 0);
    const B: QNameId = QNameId::new(0, 1);
    const C: QNameId = QNameId::new(0, 2);

    fn encode(vp: &mut ValuePartitions, byte_aligned: bool, items: &[(QNameId, &str)]) -> Vec<u8> {
        let mut w = BitWriter::with_alignment(Vec::new(), byte_aligned);
        for &(ctx, v) in items {
            assert_eq!(vp.write(&mut w, ctx, v), Ok(()));
        }
        w.finish().unwrap_or_default()
    }

    fn decode_all(
        vp: &mut ValuePartitions,
        byte_aligned: bool,
        bytes: &[u8],
        ctxs: &[QNameId],
    ) -> Vec<String> {
        let mut r = BitReader::with_alignment(bytes, byte_aligned);
        let mut out = Vec::new();
        let mut s = String::new();
        for &ctx in ctxs {
            assert_eq!(vp.read(&mut r, ctx, &mut s), Ok(()));
            out.push(s.clone());
        }
        out
    }

    fn round_trip(
        max_len: Option<u32>,
        capacity: Option<u32>,
        byte_aligned: bool,
        items: &[(QNameId, &str)],
    ) {
        let mut enc = ValuePartitions::new(max_len, capacity);
        let bytes = encode(&mut enc, byte_aligned, items);
        let mut dec = ValuePartitions::new(max_len, capacity);
        let ctxs: Vec<QNameId> = items.iter().map(|&(c, _)| c).collect();
        let got = decode_all(&mut dec, byte_aligned, &bytes, &ctxs);
        let want: Vec<String> = items.iter().map(|&(_, v)| v.to_owned()).collect();
        assert_eq!(got, want);
        assert_eq!(enc.global_count(), dec.global_count());
        for &ctx in &ctxs {
            assert_eq!(enc.local_count(ctx), dec.local_count(ctx));
        }
    }

    #[test]
    fn miss_is_length_plus_two_then_chars() {
        let mut vp = ValuePartitions::new(None, None);
        let bytes = encode(&mut vp, false, &[(A, "ab")]);
        assert_eq!(bytes, vec![4, b'a', b'b']);
        assert_eq!(vp.global_count(), 1);
        assert_eq!(vp.local_count(A), 1);
    }

    #[test]
    fn empty_string_is_two_and_not_added() {
        let mut vp = ValuePartitions::new(None, None);
        let bytes = encode(&mut vp, false, &[(A, ""), (A, "")]);
        assert_eq!(bytes, vec![2, 2]);
        assert_eq!(vp.global_count(), 0);
        assert_eq!(vp.local_count(A), 0);
        round_trip(None, None, false, &[(A, ""), (B, ""), (A, "x"), (A, "")]);
    }

    #[test]
    fn local_hit_is_zero_then_local_id() {
        let mut vp = ValuePartitions::new(None, None);
        // "x" -> miss (3,'x'); "y" -> miss (3,'y'); "x" again in A -> 0 then id 0 in 1 bit.
        let bytes = encode(&mut vp, false, &[(A, "x"), (A, "y"), (A, "x"), (A, "y")]);
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(read_unsigned(&mut r), Ok(3));
        assert_eq!(r.read_byte(), Ok(b'x'));
        assert_eq!(read_unsigned(&mut r), Ok(3));
        assert_eq!(r.read_byte(), Ok(b'y'));
        assert_eq!(read_unsigned(&mut r), Ok(0));
        assert_eq!(r.read_nbit_unsigned(1), Ok(0));
        assert_eq!(read_unsigned(&mut r), Ok(0));
        assert_eq!(r.read_nbit_unsigned(1), Ok(1));
    }

    #[test]
    fn global_hit_is_one_then_global_id() {
        let mut vp = ValuePartitions::new(None, None);
        let bytes = encode(&mut vp, false, &[(A, "x"), (B, "y"), (C, "z"), (B, "z")]);
        let mut r = BitReader::new(bytes.as_slice());
        for ch in *b"xyz" {
            assert_eq!(read_unsigned(&mut r), Ok(3));
            assert_eq!(r.read_byte(), Ok(ch));
        }
        // 3 global entries -> 2 bits; "z" has global id 2.
        assert_eq!(read_unsigned(&mut r), Ok(1));
        assert_eq!(r.read_nbit_unsigned(2), Ok(2));
        // A hit never adds: B's local partition still has just "y".
        assert_eq!(vp.local_count(B), 1);
    }

    #[test]
    fn single_local_entry_hit_uses_zero_bits() {
        let mut vp = ValuePartitions::new(None, None);
        let bytes = encode(&mut vp, false, &[(A, "v"), (A, "v")]);
        // 3 'v' 0  -> three bytes, the local id takes ceil_log2(1) = 0 bits.
        assert_eq!(bytes, vec![3, b'v', 0]);
    }

    #[test]
    fn exificient_string_table_value0() {
        // StringTableCoreTest.testStringTableValue0: one context.
        round_trip(
            None,
            None,
            false,
            &[
                (A, "val1"),
                (A, "val2"),
                (A, "val1"),
                (A, "val3"),
                (A, "val3"),
                (A, "val3"),
            ],
        );
    }

    #[test]
    fn exificient_string_table_value1() {
        // StringTableCoreTest.testStringTableValue1: three contexts.
        round_trip(
            None,
            None,
            false,
            &[
                (A, "val1"),
                (C, "val2"),
                (B, "val1"),
                (C, "val3"),
                (A, "val3"),
                (C, "val3"),
            ],
        );
    }

    #[test]
    fn exificient_string_table_value() {
        // StringTableCoreTest.testStringTableValue.
        let at1 = QNameId::new(0, 0);
        let elx2 = QNameId::new(0, 1);
        let elx3 = QNameId::new(0, 2);
        let elxx1 = QNameId::new(0, 3);
        let elxx2 = QNameId::new(0, 4);
        round_trip(
            None,
            None,
            false,
            &[
                (at1, "at-ch1"),
                (elxx1, "ch1"),
                (elxx2, "ch2"),
                (elx2, "ch2"),
                (elx3, "ch3"),
                (at1, "at-ch2"),
                (elxx1, "ch1"),
                (elxx2, "ch2"),
                (elx2, "ch2"),
                (elx2, "ch3"),
            ],
        );
    }

    #[test]
    fn value_max_length_excludes_longer_strings() {
        // StringTableCoreTest.testStringTableValueMaxLength1 with valueMaxLength = 5.
        let mut enc = ValuePartitions::new(Some(5), None);
        let items = [
            (A, "123"),
            (B, "1234"),
            (A, "12345"),
            (B, "123456"),
            (A, "1234567"),
        ];
        let bytes = encode(&mut enc, false, &items);
        assert!(enc.contains("123"));
        assert!(enc.contains("12345"));
        assert!(!enc.contains("123456"));
        assert!(!enc.contains("1234567"));
        assert_eq!(enc.global_count(), 3);
        assert_eq!(enc.local_count(A), 2);
        assert_eq!(enc.local_count(B), 1);
        let mut dec = ValuePartitions::new(Some(5), None);
        let ctxs: Vec<QNameId> = items.iter().map(|&(c, _)| c).collect();
        let got = decode_all(&mut dec, false, &bytes, &ctxs);
        assert_eq!(got, ["123", "1234", "12345", "123456", "1234567"]);
        assert_eq!(dec.global_count(), 3);
        // Length is counted in code points, not bytes.
        let mut enc = ValuePartitions::new(Some(5), None);
        encode(&mut enc, false, &[(A, "héllo")]);
        assert!(enc.contains("héllo"));
    }

    #[test]
    fn value_partition_capacity_wraps_and_evicts() {
        // StringTableCoreTest.testStringTableValuePartitionCapacity, capacity 3.
        let mut enc = ValuePartitions::new(None, Some(3));
        let mut w = BitWriter::new(Vec::new());
        let step = |enc: &mut ValuePartitions, w: &mut BitWriter<Vec<u8>>, ctx, v| {
            assert_eq!(enc.write(w, ctx, v), Ok(()));
        };
        step(&mut enc, &mut w, A, "1"); // ["1"{a}]
        step(&mut enc, &mut w, B, "12"); // ["1"{a},"12"{b}]
        step(&mut enc, &mut w, A, "123"); // ["1"{a},"12"{b},"123"{a}]
        step(&mut enc, &mut w, C, "123"); // global hit, unchanged
        assert_eq!(enc.global_count(), 3);
        step(&mut enc, &mut w, C, "1234"); // ["1234"{c},"12"{b},"123"{a}]
        assert!(!enc.contains("1"));
        assert!(enc.contains("1234"));
        assert_eq!(enc.global_count(), 3);
        // "1" was evicted from A's local partition: id 0 is unassigned but still counted.
        assert_eq!(enc.local_count(A), 2);
        let first_of_a = enc.entry_of_local(A, 0).and_then(|i| enc.entries.get(i));
        assert_eq!(first_of_a.map(|e| e.global_id), Some(None));
        step(&mut enc, &mut w, A, "1"); // ["1234"{c},"1"{a},"123"{a}]
        assert!(enc.contains("1234"));
        assert!(enc.contains("1"));
        assert!(enc.contains("123"));
        assert!(!enc.contains("12"));
        assert_eq!(enc.local_count(A), 3);
        assert_eq!(enc.local_count(B), 1);
        step(&mut enc, &mut w, C, "12"); // ["1234"{c},"1"{a},"12"{c}]
        assert!(enc.contains("1234"));
        assert!(enc.contains("1"));
        assert!(enc.contains("12"));
        assert!(!enc.contains("123"));
        let bytes = w.finish().unwrap_or_default();

        let mut dec = ValuePartitions::new(None, Some(3));
        let got = decode_all(&mut dec, false, &bytes, &[A, B, A, C, C, A, C]);
        assert_eq!(got, ["1", "12", "123", "123", "1234", "1", "12"]);
        assert_eq!(dec.global_count(), 3);
        assert_eq!(dec.local_count(A), 3);
        assert_eq!(dec.local_count(C), 2);
    }

    #[test]
    fn capacity_zero_never_adds() {
        let mut enc = ValuePartitions::new(None, Some(0));
        let bytes = encode(&mut enc, false, &[(A, "x"), (A, "x")]);
        assert_eq!(bytes, vec![3, b'x', 3, b'x']);
        assert_eq!(enc.global_count(), 0);
        round_trip(None, Some(0), false, &[(A, "x"), (A, "x"), (B, "x")]);
    }

    #[test]
    fn capacity_one_replaces_every_time() {
        round_trip(
            None,
            Some(1),
            false,
            &[(A, "x"), (A, "y"), (A, "x"), (B, "x"), (A, "y"), (A, "y")],
        );
    }

    #[test]
    fn exificient_bounded_table_80() {
        // BoundedTableTestCase.testBoundedTable80: VAL_0..=VAL_100 in <a> then <b>,
        // then descending, with valuePartitionCapacity = 80.
        let values: Vec<String> = (0..=100).map(|i| format!("VAL_{i}")).collect();
        let mut items: Vec<(QNameId, &str)> = Vec::new();
        for v in &values {
            items.push((A, v));
            items.push((B, v));
        }
        for v in values.iter().rev() {
            items.push((A, v));
            items.push((B, v));
        }
        round_trip(None, Some(80), false, &items);
        round_trip(None, Some(80), true, &items);
    }

    /// Deterministic pseudo-random sequence generator (no dependencies).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    fn random_items(seed: u64, n: usize) -> Vec<(QNameId, String)> {
        let mut rng = Lcg(seed);
        let contexts = [A, B, C, QNameId::new(1, 0), QNameId::new(2, 5)];
        (0..n)
            .map(|_| {
                let ctx = contexts[(rng.next() % contexts.len() as u64) as usize];
                let len = (rng.next() % 9) as usize;
                let s: String = (0..len)
                    .map(|_| {
                        let alphabet = ['a', 'b', 'c', 'é', '€', '😀', '0', '1'];
                        alphabet[(rng.next() % alphabet.len() as u64) as usize]
                    })
                    .collect();
                (ctx, s)
            })
            .collect()
    }

    #[test]
    fn random_sequences_round_trip_under_every_limit_combination() {
        for seed in 1..=6u64 {
            let owned = random_items(seed, 400);
            let items: Vec<(QNameId, &str)> = owned.iter().map(|(c, s)| (*c, s.as_str())).collect();
            for max_len in [None, Some(0), Some(2), Some(5)] {
                for capacity in [None, Some(0), Some(1), Some(3), Some(7), Some(64)] {
                    for byte_aligned in [false, true] {
                        round_trip(max_len, capacity, byte_aligned, &items);
                    }
                }
            }
        }
    }

    /// The bits and values of the reference model (the earlier layout, in
    /// `value_reference.rs`) for the same items and limits.
    fn reference_bits(
        max_len: Option<u32>,
        capacity: Option<u32>,
        byte_aligned: bool,
        items: &[(QNameId, &str)],
    ) -> Vec<u8> {
        let mut vp = super::super::value_reference::ReferencePartitions::new(max_len, capacity);
        let mut w = BitWriter::with_alignment(Vec::new(), byte_aligned);
        for &(ctx, v) in items {
            assert_eq!(vp.write(&mut w, ctx, v), Ok(()));
        }
        w.finish().unwrap_or_default()
    }

    #[test]
    fn identical_bits_to_the_reference_model() {
        // Lengths that cross the scan/index threshold many times over, with and
        // without eviction, and enough distinct values to force compaction.
        for seed in 1..=8u64 {
            for n in [5, 16, 17, 40, 600] {
                let owned = random_items(seed, n);
                let items: Vec<(QNameId, &str)> =
                    owned.iter().map(|(c, s)| (*c, s.as_str())).collect();
                for max_len in [None, Some(3)] {
                    for capacity in [None, Some(1), Some(7), Some(16), Some(17), Some(50)] {
                        for byte_aligned in [false, true] {
                            let mut vp = ValuePartitions::new(max_len, capacity);
                            let ours = encode(&mut vp, byte_aligned, &items);
                            let theirs = reference_bits(max_len, capacity, byte_aligned, &items);
                            assert_eq!(ours, theirs, "seed {seed} n {n} {max_len:?} {capacity:?}");
                            let contexts: Vec<QNameId> = items.iter().map(|&(c, _)| c).collect();
                            let mut dec = ValuePartitions::new(max_len, capacity);
                            let got = decode_all(&mut dec, byte_aligned, &ours, &contexts);
                            let want: Vec<&str> = items.iter().map(|&(_, v)| v).collect();
                            assert_eq!(got, want, "seed {seed} n {n} {max_len:?} {capacity:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn long_values_and_heavy_eviction_compact_the_arena() {
        // 3000 distinct 100-byte values through a 10-slot table: the arena must
        // stay bounded (compaction) and the bits must match the reference.
        let owned: Vec<(QNameId, String)> = (0..3000)
            .map(|i| (if i % 3 == 0 { A } else { B }, format!("{i:0100}")))
            .collect();
        let items: Vec<(QNameId, &str)> = owned.iter().map(|(c, s)| (*c, s.as_str())).collect();
        let mut vp = ValuePartitions::new(None, Some(10));
        let ours = encode(&mut vp, false, &items);
        assert_eq!(ours, reference_bits(None, Some(10), false, &items));
        assert!(vp.text.len() < 4096, "arena {} bytes", vp.text.len());
    }

    #[test]
    fn decode_rejects_bad_identifiers() {
        let mut vp = ValuePartitions::new(None, None);
        let mut s = String::new();
        // Local hit in an empty partition: 0 bits read, id 0, but no entry.
        let mut r = BitReader::new([0u8].as_slice());
        assert_eq!(
            vp.read(&mut r, A, &mut s),
            Err(ExiError::StringTable("local value identifier out of range"))
        );
        // Global hit with no entries.
        let mut r = BitReader::new([1u8].as_slice());
        assert_eq!(
            vp.read(&mut r, A, &mut s),
            Err(ExiError::StringTable(
                "global value identifier out of range"
            ))
        );
        // Evicted local identifier.
        let mut vp = ValuePartitions::new(None, Some(1));
        let mut w = BitWriter::new(Vec::new());
        assert_eq!(vp.write(&mut w, A, "x"), Ok(()));
        assert_eq!(vp.write(&mut w, A, "y"), Ok(()));
        let mut dec = ValuePartitions::new(None, Some(1));
        let bytes = w.finish().unwrap_or_default();
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(dec.read(&mut r, A, &mut s), Ok(()));
        assert_eq!(dec.read(&mut r, A, &mut s), Ok(()));
        // Now local id 0 of A is unassigned; a stream claiming a local hit on it is invalid.
        let mut r = BitReader::new([0u8, 0x00].as_slice());
        assert_eq!(
            dec.read(&mut r, A, &mut s),
            Err(ExiError::StringTable(
                "local value identifier is unassigned"
            ))
        );
    }
}
