//! Decoder limits as C sees them: a plain struct of four numbers, where 0 means
//! "no limit". They keep a hostile stream from using up time, memory or stack.
//! See `DecodeLimits` in `rexi-core` for what each one stops.

use rexi_core::DecodeLimits;

/// Limits on what one stream may ask the decoder for. A field of 0 means no limit
/// for that item. Start from `rexi_limits_default()` and change the fields you
/// need. When a stream goes over a limit, decoding stops with
/// `REXI_STATUS_ERR_LIMIT`.
///
/// `max_depth` and `max_integer_bits` are 32-bit numbers inside rexi. A value
/// above `UINT32_MAX` (more than 4 billion) is treated as no limit.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RexiLimits {
    /// Most elements open at once. Default 256.
    pub max_depth: u64,
    /// Most items in one list value. Default 1000000.
    pub max_list_items: u64,
    /// Most bits in one integer value. Default 4096.
    pub max_integer_bits: u64,
    /// Most bytes (UTF-8) of all decoded string values together. Default
    /// 268435456 (256 MiB).
    pub max_string_bytes: u64,
}

impl RexiLimits {
    /// The core limits these numbers stand for.
    pub(crate) fn core(&self) -> DecodeLimits {
        let opt64 = |v: u64| (v != 0).then_some(v);
        // Above u32::MAX is no limit, as the struct docs say.
        let opt32 = |v: u64| (v != 0).then(|| u32::try_from(v).ok()).flatten();
        DecodeLimits::default()
            .with_max_depth(opt32(self.max_depth))
            .with_max_list_items(opt64(self.max_list_items))
            .with_max_integer_bits(opt32(self.max_integer_bits))
            .with_max_string_bytes(opt64(self.max_string_bytes))
    }

    /// The C form of core limits, `None` written as 0.
    pub(crate) fn from_core(l: &DecodeLimits) -> Self {
        RexiLimits {
            max_depth: l.max_depth.map_or(0, u64::from),
            max_list_items: l.max_list_items.unwrap_or(0),
            max_integer_bits: l.max_integer_bits.map_or(0, u64::from),
            max_string_bytes: l.max_string_bytes.unwrap_or(0),
        }
    }
}

/// The default limits: depth 256, 1000000 list items, 4096 integer bits and
/// 256 MiB of string values. These are what `rexi_decoder_new` and `rexi_decode`
/// use.
#[unsafe(no_mangle)]
pub extern "C" fn rexi_limits_default() -> RexiLimits {
    RexiLimits::from_core(&DecodeLimits::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_huge_mean_no_limit() {
        let all_zero = RexiLimits {
            max_depth: 0,
            max_list_items: 0,
            max_integer_bits: 0,
            max_string_bytes: 0,
        };
        assert_eq!(all_zero.core(), DecodeLimits::unlimited());
        let huge = RexiLimits {
            max_depth: u64::from(u32::MAX) + 1,
            ..rexi_limits_default()
        };
        assert_eq!(huge.core().max_depth, None);
        assert_eq!(rexi_limits_default().core(), DecodeLimits::default());
    }
}
