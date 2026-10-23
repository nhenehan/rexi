/// Number of bits needed to encode `m` different values: `ceil(log2(m))`, with 0 for
/// `m <= 1`.
///
/// This is the width of one part of an event code (the number that tells the decoder
/// which grammar choice comes next,
/// [EXI 6.2](https://www.w3.org/TR/exi/#encodingEventCodes)). It is also the width of
/// a compact identifier (the number that stands for a string already in a
/// string-table partition, [EXI 7.3](https://www.w3.org/TR/exi/#stringTable)).
///
/// It works as `bits in usize - leading zeros of (m - 1)`: the bit length of the
/// largest index `m - 1`.
pub const fn ceil_log2(m: usize) -> u8 {
    if m <= 1 {
        0
    } else {
        (usize::BITS - (m - 1).leading_zeros()) as u8
    }
}

/// Number of bits needed to write the value `max` itself: `ceil(log2(max + 1))`, with 0
/// for `max == 0`.
///
/// Used for n-bit unsigned integers whose range is `0..=max`
/// ([EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)).
pub const fn bits_to_represent(max: u64) -> u8 {
    if max == 0 {
        0
    } else {
        (u64::BITS - max.leading_zeros()) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_log2_matches_spec_table() {
        assert_eq!(ceil_log2(0), 0);
        assert_eq!(ceil_log2(1), 0);
        assert_eq!(ceil_log2(2), 1);
        assert_eq!(ceil_log2(3), 2);
        assert_eq!(ceil_log2(4), 2);
        assert_eq!(ceil_log2(5), 3);
        assert_eq!(ceil_log2(8), 3);
        assert_eq!(ceil_log2(9), 4);
        assert_eq!(ceil_log2(4096), 12);
        assert_eq!(ceil_log2(4097), 13);
    }

    #[test]
    fn bits_to_represent_values() {
        assert_eq!(bits_to_represent(0), 0);
        assert_eq!(bits_to_represent(1), 1);
        assert_eq!(bits_to_represent(2), 2);
        assert_eq!(bits_to_represent(255), 8);
        assert_eq!(bits_to_represent(256), 9);
        assert_eq!(bits_to_represent(4095), 12);
        assert_eq!(bits_to_represent(u64::MAX), 64);
    }
}
