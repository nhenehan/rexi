#![allow(dead_code)]
//! The earlier value-partition layout, kept as the reference model
//! that `value.rs` is tested against (both must give identical identifiers and bits).
//!
//! Original description: the global value partition plus one "local" partition per
//! context qname ([EXI 7.3.3](https://www.w3.org/TR/exi/#encodingOptimizedForMisses)).
//!
//! Does the same as EXIficient's `StringEncoderImpl` / `StringDecoderImpl` and their
//! `Bounded*` variants, in a single structure usable for both directions:
//!
//! * the global partition is a `Vec` indexed by global compact identifier; under
//!   `valuePartitionCapacity` the next identifier wraps around to 0 and the slot's
//!   old value is evicted (thrown out);
//! * each local partition is a `Vec<Option<global id>>` so that local compact
//!   identifiers never change; an evicted entry becomes `None` ("permanently
//!   unassigned") but still counts toward the partition size `m`;
//! * a `BTreeMap<String, global id>` gives the encoder its lookup.

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;
use alloc::collections::BTreeMap;

use crate::datatype::string::{read_chars, write_chars, write_length};
use crate::datatype::unsigned::{read_unsigned, write_unsigned};
use crate::error::{ExiError, ExiResult};
use crate::ids::{LocalId, QNameId};
use crate::io::{BitReader, BitWriter, ceil_log2};

/// One used slot of the global value partition.
#[derive(Debug, Clone)]
struct GlobalSlot {
    value: String,
    /// The qname whose local partition also holds this value.
    context: QNameId,
    /// Compact identifier of this value in `context`'s local partition.
    local_id: LocalId,
}

/// The reference value partitions (global plus every local one).
#[derive(Debug, Clone)]
pub(crate) struct ReferencePartitions {
    value_max_length: Option<u32>,
    value_partition_capacity: Option<u32>,
    /// Global partition, indexed by global compact identifier.
    global: Vec<GlobalSlot>,
    /// Encoder lookup: value -> global compact identifier.
    index: BTreeMap<String, u32>,
    /// Local partitions: context qname -> local id -> global id (`None` = evicted).
    local: BTreeMap<QNameId, Vec<Option<u32>>>,
    /// Spec `globalID`: the identifier the next added value receives.
    next_global: u32,
}

impl ReferencePartitions {
    pub(crate) fn new(
        value_max_length: Option<u32>,
        value_partition_capacity: Option<u32>,
    ) -> Self {
        Self {
            value_max_length,
            value_partition_capacity,
            global: Vec::new(),
            index: BTreeMap::new(),
            local: BTreeMap::new(),
            next_global: 0,
        }
    }

    pub(crate) fn value_max_length(&self) -> Option<u32> {
        self.value_max_length
    }

    pub(crate) fn value_partition_capacity(&self) -> Option<u32> {
        self.value_partition_capacity
    }

    /// Number of entries in the global value partition.
    pub(crate) fn global_count(&self) -> usize {
        self.global.len()
    }

    /// Number of entries (assigned or evicted) in `context`'s local partition.
    pub(crate) fn local_count(&self, context: QNameId) -> usize {
        self.local.get(&context).map_or(0, Vec::len)
    }

    /// Whether `value` is currently present in the global partition.
    pub(crate) fn contains(&self, value: &str) -> bool {
        self.index.contains_key(value)
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
    /// The caller has already checked [`Self::is_addable`] and that the value is
    /// not present.
    fn add(&mut self, context: QNameId, value: String) {
        let local_id = self.local_count(context) as LocalId;
        let global_id = self.next_global;
        let slot = GlobalSlot {
            value: value.clone(),
            context,
            local_id,
        };
        match self.value_partition_capacity {
            Some(capacity) if (global_id as usize) < self.global.len() => {
                // Slot already in use: S replaces V (the spec's names for the new and
                // old values), and V's local identifier becomes permanently unassigned.
                let old = core::mem::replace(&mut self.global[global_id as usize], slot);
                self.index.remove(&old.value);
                if let Some(old_local) = self.local.get_mut(&old.context)
                    && let Some(entry) = old_local.get_mut(old.local_id as usize)
                {
                    *entry = None;
                }
                self.next_global = (global_id + 1) % capacity;
            }
            Some(capacity) => {
                self.global.push(slot);
                self.next_global = (global_id + 1) % capacity;
            }
            None => {
                self.global.push(slot);
                self.next_global = global_id + 1;
            }
        }
        // Looked up after the eviction, which may have changed this same partition.
        self.local.entry(context).or_default().push(Some(global_id));
        self.index.insert(value, global_id);
    }

    /// Writes `value`: local hit (Unsigned 0 + local id), global hit (Unsigned 1 +
    /// global id), or literal (Unsigned length + 2, then the characters).
    pub(crate) fn write<W: ByteSink>(
        &mut self,
        w: &mut BitWriter<W>,
        context: QNameId,
        value: &str,
    ) -> ExiResult<()> {
        if let Some(&global_id) = self.index.get(value) {
            let slot = self
                .global
                .get(global_id as usize)
                .ok_or(ExiError::Internal(
                    "value index points outside global partition",
                ))?;
            if slot.context == context {
                write_unsigned(w, 0)?;
                let n = ceil_log2(self.local_count(context));
                w.write_nbit_unsigned(u64::from(slot.local_id), n)
            } else {
                write_unsigned(w, 1)?;
                let n = ceil_log2(self.global.len());
                w.write_nbit_unsigned(u64::from(global_id), n)
            }
        } else {
            let length = value.chars().count();
            write_length(w, length as u64 + 2)?;
            write_chars(w, value)?;
            if self.is_addable(length) {
                self.add(context, value.to_owned());
            }
            Ok(())
        }
    }

    /// Reads a value written by [`ReferencePartitions::write`] into `out`, which is
    /// cleared first.
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
                let global_id = self
                    .local
                    .get(&context)
                    .and_then(|p| p.get(local_id))
                    .ok_or(ExiError::StringTable("local value identifier out of range"))?
                    .ok_or(ExiError::StringTable(
                        "local value identifier is unassigned",
                    ))?;
                let slot = self
                    .global
                    .get(global_id as usize)
                    .ok_or(ExiError::Internal(
                        "local partition points outside global partition",
                    ))?;
                out.push_str(&slot.value);
                Ok(())
            }
            1 => {
                let n = ceil_log2(self.global.len());
                let global_id = r.read_nbit_unsigned(n)? as usize;
                let slot = self.global.get(global_id).ok_or(ExiError::StringTable(
                    "global value identifier out of range",
                ))?;
                out.push_str(&slot.value);
                Ok(())
            }
            len_plus_two => {
                let length = len_plus_two - 2;
                read_chars(r, length, out)?;
                let length = usize::try_from(length)
                    .map_err(|_| ExiError::StringTable("value length exceeds address space"))?;
                if self.is_addable(length) && !self.index.contains_key(out.as_str()) {
                    self.add(context, out.clone());
                }
                Ok(())
            }
        }
    }
}
