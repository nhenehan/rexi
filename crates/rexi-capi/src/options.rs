//! EXI stream options as a plain C struct
//! ([EXI 5.4](https://www.w3.org/TR/exi/#options)).

use std::ffi::c_char;

use rexi_core::{Alignment, ExiOptions, HeaderConfig, SchemaId};

use crate::status::{Fail, c_str};

/// `RexiOptions::alignment`: bit-packed, the default. Values are packed with no
/// gaps between them, not padded to byte edges.
pub const REXI_ALIGN_BIT_PACKED: u32 = 0;
/// `RexiOptions::alignment`: byte-aligned. Each event code and value starts on a
/// byte edge ([EXI 5.4](https://www.w3.org/TR/exi/#options)).
pub const REXI_ALIGN_BYTE_ALIGNED: u32 = 1;

/// EXI options ([EXI 5.4](https://www.w3.org/TR/exi/#options)) and header
/// settings ([EXI 5](https://www.w3.org/TR/exi/#header)). Start from
/// `rexi_options_strict()` and change the fields you need.
///
/// The struct is passed by pointer and only read during the call; rexi copies
/// what it needs, including the `schema_id` string.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RexiOptions {
    /// `REXI_ALIGN_BIT_PACKED` or `REXI_ALIGN_BYTE_ALIGNED`.
    pub alignment: u32,
    /// Strict schema-informed grammars: the stream may hold only what the schema
    /// allows. Only `true` is supported.
    pub strict: bool,
    /// Encoder only: write the options document into the header, so a decoder
    /// does not need to be told the options separately.
    pub include_options: bool,
    /// Encoder only: write the four-byte `$EXI` cookie at the start
    /// ([EXI 5.1](https://www.w3.org/TR/exi/#EXICookie)).
    pub include_cookie: bool,
    /// The `schemaId` option, a NUL-terminated UTF-8 string. NULL means the option
    /// is absent. An empty string `""` is kept as an empty `schemaId`, which is not
    /// the same as absent.
    pub schema_id: *const c_char,
    /// `schemaId` is `xsi:nil="true"`, which says no schema was used. When set,
    /// `schema_id` is ignored.
    pub schema_id_nil: bool,
    /// `valueMaxLength`: the longest string value that is added to the string
    /// table. Negative means no limit. Values above 2^32-1 are rejected.
    pub value_max_length: i64,
    /// `valuePartitionCapacity`: how many values the string table's value
    /// partitions can hold. Negative means no limit. Values above 2^32-1 are
    /// rejected.
    pub value_partition_capacity: i64,
}

/// Default options: strict, bit-packed, no options document, no cookie, no
/// `schemaId`, no limits on the value partitions.
#[unsafe(no_mangle)]
pub extern "C" fn rexi_options_strict() -> RexiOptions {
    RexiOptions {
        alignment: REXI_ALIGN_BIT_PACKED,
        strict: true,
        include_options: false,
        include_cookie: false,
        schema_id: std::ptr::null(),
        schema_id_nil: false,
        value_max_length: -1,
        value_partition_capacity: -1,
    }
}

fn bound(v: i64, what: &str) -> Result<Option<u32>, Fail> {
    if v < 0 {
        return Ok(None);
    }
    u32::try_from(v)
        .map(Some)
        .map_err(|_| Fail::invalid(format!("{what} {v} exceeds 2^32-1")))
}

impl RexiOptions {
    /// Converts to the core options and header settings. Fails with
    /// `ErrInvalidArgument` for an unknown alignment, a `schema_id` that is not
    /// UTF-8, or a limit above 2^32-1.
    ///
    /// # Safety
    /// `schema_id` is NULL or a NUL-terminated string.
    pub(crate) unsafe fn core(self) -> Result<(ExiOptions, HeaderConfig), Fail> {
        let mut o = if self.strict {
            ExiOptions::strict()
        } else {
            ExiOptions::default()
        };
        o.alignment = match self.alignment {
            REXI_ALIGN_BIT_PACKED => Alignment::BitPacked,
            REXI_ALIGN_BYTE_ALIGNED => Alignment::ByteAligned,
            other => return Err(Fail::invalid(format!("unknown alignment {other}"))),
        };
        o.schema_id = if self.schema_id_nil {
            SchemaId::Nil
        } else if self.schema_id.is_null() {
            SchemaId::Absent
        } else {
            // SAFETY: per the contract.
            let id = unsafe { c_str(self.schema_id, "schema_id") }?;
            if id.is_empty() {
                SchemaId::Empty
            } else {
                SchemaId::Id(id.to_string())
            }
        };
        o.value_max_length = bound(self.value_max_length, "value_max_length")?;
        o.value_partition_capacity =
            bound(self.value_partition_capacity, "value_partition_capacity")?;
        let header = HeaderConfig::default()
            .with_cookie(self.include_cookie)
            .with_include_options(self.include_options);
        Ok((o, header))
    }
}
