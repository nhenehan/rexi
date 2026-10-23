//! Limits that keep the decoder safe on input you do not trust.
//!
//! EXI is compact, and that cuts both ways: a few hostile bytes can ask the decoder
//! for a huge amount of work or memory. A 7-byte list can claim four billion items
//! of zero bits each. Two bytes can open elements without end when the schema has
//! a required self-recursive element. One long string can be repeated a million
//! times through one-byte string-table hits
//! ([EXI 7.3](https://www.w3.org/TR/exi/#stringTable)). A very long integer costs
//! time that grows with the square of its length.
//!
//! [`DecodeLimits`] caps each of these. The defaults are far above what normal
//! documents need, so they only stop hostile or broken streams. When a limit is
//! hit, the decoder stops with [`ExiError::LimitExceeded`], naming the [`Limit`]
//! and its maximum.
//!
//! The limits are not part of [`ExiOptions`](crate::ExiOptions). Those mirror the
//! EXI options document and travel with the stream; limits are a choice of the
//! program that reads it. The encoder has no limits: its input comes from the
//! program itself, so it is trusted.
//!
//! [`ExiError::LimitExceeded`]: crate::ExiError::LimitExceeded

use core::fmt;

/// How much a [`Decoder`](crate::Decoder) is willing to do for one stream.
///
/// Each field is `None` for "no limit". [`DecodeLimits::default`] gives safe
/// values, and [`DecodeLimits::unlimited`] turns every limit off.
///
/// ```
/// use rexi_core::grammar::options_grammar::OPTIONS_GRAMMARS;
/// use rexi_core::{DecodeLimits, Decoder, ExiOptions};
///
/// # fn main() -> rexi_core::ExiResult<()> {
/// // The defaults, with deeper nesting allowed and no cap on list length.
/// let limits = DecodeLimits::default()
///     .with_max_depth(Some(1024))
///     .with_max_list_items(None);
/// assert_eq!(limits.max_integer_bits, Some(4096));
///
/// let stream = [0x80u8]; // a header with no options in it
/// let dec = Decoder::with_limits(
///     &stream[..],
///     &OPTIONS_GRAMMARS,
///     Some(ExiOptions::strict()),
///     limits,
/// )?;
/// assert_eq!(dec.limits().max_depth, Some(1024));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeLimits {
    /// Most elements open at once. Stops endless nesting, such as a required
    /// element that contains itself. Default 256.
    pub max_depth: Option<u32>,
    /// Most items in one list value
    /// ([EXI 7.1.11](https://www.w3.org/TR/exi/#encodingList)). Checked right after
    /// the item count is read, before any item. Default 1,000,000.
    pub max_list_items: Option<u64>,
    /// Most bits in one unsigned integer in the stream, counted as 7 bits per
    /// group ([EXI 7.1.6](https://www.w3.org/TR/exi/#encodingUnsignedInteger)).
    /// It covers Integer, Unsigned Integer, Decimal, Float and the year of a date.
    /// Default 4096.
    pub max_integer_bits: Option<u32>,
    /// Most bytes (UTF-8) of all decoded string values together, whether they came
    /// as text or as a string-table hit. Names (QNames and URIs) are not counted.
    /// Default 256 MiB.
    pub max_string_bytes: Option<u64>,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_depth: Some(256),
            max_list_items: Some(1_000_000),
            max_integer_bits: Some(4096),
            max_string_bytes: Some(256 * 1024 * 1024),
        }
    }
}

impl DecodeLimits {
    /// No limits at all. Use this only for input you trust.
    pub const fn unlimited() -> Self {
        Self {
            max_depth: None,
            max_list_items: None,
            max_integer_bits: None,
            max_string_bytes: None,
        }
    }

    /// Sets [`max_depth`](Self::max_depth).
    #[must_use]
    pub const fn with_max_depth(mut self, max: Option<u32>) -> Self {
        self.max_depth = max;
        self
    }

    /// Sets [`max_list_items`](Self::max_list_items).
    #[must_use]
    pub const fn with_max_list_items(mut self, max: Option<u64>) -> Self {
        self.max_list_items = max;
        self
    }

    /// Sets [`max_integer_bits`](Self::max_integer_bits).
    #[must_use]
    pub const fn with_max_integer_bits(mut self, max: Option<u32>) -> Self {
        self.max_integer_bits = max;
        self
    }

    /// Sets [`max_string_bytes`](Self::max_string_bytes).
    #[must_use]
    pub const fn with_max_string_bytes(mut self, max: Option<u64>) -> Self {
        self.max_string_bytes = max;
        self
    }
}

/// Which of the [`DecodeLimits`] a stream went over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Limit {
    /// [`DecodeLimits::max_depth`].
    Depth,
    /// [`DecodeLimits::max_list_items`].
    ListItems,
    /// [`DecodeLimits::max_integer_bits`].
    IntegerBits,
    /// [`DecodeLimits::max_string_bytes`].
    StringBytes,
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Limit::Depth => "nesting depth",
            Limit::ListItems => "list items",
            Limit::IntegerBits => "integer bits",
            Limit::StringBytes => "string bytes",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ExiError;
    use alloc::string::ToString;

    #[test]
    fn defaults_and_builders() {
        let d = DecodeLimits::default();
        assert_eq!(d.max_depth, Some(256));
        assert_eq!(d.max_list_items, Some(1_000_000));
        assert_eq!(d.max_integer_bits, Some(4096));
        assert_eq!(d.max_string_bytes, Some(268_435_456));
        let u = DecodeLimits::unlimited();
        assert_eq!(
            (
                u.max_depth,
                u.max_list_items,
                u.max_integer_bits,
                u.max_string_bytes
            ),
            (None, None, None, None)
        );
        let b = DecodeLimits::unlimited()
            .with_max_depth(Some(1))
            .with_max_list_items(Some(2))
            .with_max_integer_bits(Some(3))
            .with_max_string_bytes(Some(4));
        assert_eq!(
            (
                b.max_depth,
                b.max_list_items,
                b.max_integer_bits,
                b.max_string_bytes
            ),
            (Some(1), Some(2), Some(3), Some(4))
        );
    }

    #[test]
    fn error_text() {
        let e = ExiError::LimitExceeded {
            limit: Limit::Depth,
            max: 256,
        };
        assert_eq!(
            e.to_string(),
            "decoder limit exceeded: nesting depth is over 256"
        );
        assert_eq!(Limit::ListItems.to_string(), "list items");
        assert_eq!(Limit::IntegerBits.to_string(), "integer bits");
        assert_eq!(Limit::StringBytes.to_string(), "string bytes");
    }
}
