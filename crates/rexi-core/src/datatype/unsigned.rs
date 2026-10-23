//! Unsigned Integer
//! ([EXI 7.1.6](https://www.w3.org/TR/exi/#encodingUnsignedInteger)): 7-bit groups,
//! least significant group first. The high bit of each octet is 1 when more octets
//! follow and 0 on the last one.

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;

use super::biguint::BigUint;
use crate::error::{ExiError, ExiResult};
use crate::io::{BitReader, BitWriter};
use crate::limits::Limit;

/// Writes an Unsigned Integer
/// ([EXI 7.1.6](https://www.w3.org/TR/exi/#encodingUnsignedInteger)): 7-bit groups,
/// least significant first. Values below 128 (most lengths and small values) fit in
/// one group, which is one octet. Longer values are built up in one 64-bit word and
/// written in a single call.
#[inline]
pub fn write_unsigned<W: ByteSink>(w: &mut BitWriter<W>, value: u64) -> ExiResult<()> {
    if value < 0x80 {
        return w.write_byte(value as u8);
    }
    // Each new octet is shifted in at the bottom of `word`, so the first octet in the
    // stream (the least significant group) ends up as the most significant byte of
    // `word`. Writing `word` from the top then gives the right stream order. A u64 holds
    // at most 8 octets, that is 8 groups of 7 bits = 56 value bits.
    let mut word = 0u64;
    let mut len = 0u8;
    let mut v = value;
    while len < 8 {
        let mut octet = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            octet |= 0x80;
        }
        word = (word << 8) | u64::from(octet);
        len += 1;
        if v == 0 {
            return w.write_raw_bits(word, 8 * len);
        }
    }
    // More than 56 value bits: write the eight octets built so far, then the rest one
    // octet at a time.
    w.write_raw_bits(word, 64)?;
    loop {
        let mut octet = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            octet |= 0x80;
        }
        w.write_byte(octet)?;
        if v == 0 {
            return Ok(());
        }
    }
}

/// Writes an Unsigned Integer of any size, in 7-bit groups like
/// [`write_unsigned`].
pub fn write_unsigned_big<W: ByteSink>(w: &mut BitWriter<W>, value: &BigUint) -> ExiResult<()> {
    if let Some(v) = value.to_u64() {
        return write_unsigned(w, v);
    }
    let mut n = value.clone();
    loop {
        let mut octet = n.low7();
        n.shr7();
        if !n.is_zero() {
            octet |= 0x80;
        }
        w.write_byte(octet)?;
        if n.is_zero() {
            return Ok(());
        }
    }
}

/// Reads an unsigned integer that must fit in `u64`. Returns an error if it does not.
#[inline]
pub fn read_unsigned<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<u64> {
    // Most values (lengths, ASCII code points) are one octet.
    let first = r.read_byte()?;
    if first & 0x80 == 0 {
        return Ok(u64::from(first));
    }
    let mut value = u64::from(first & 0x7F);
    let mut shift = 7u32;
    loop {
        let octet = r.read_byte()?;
        let low = u64::from(octet & 0x7F);
        // `shift` is always a multiple of 7. At shift 63 only the lowest bit of the
        // group still fits in 64 bits; any higher bit set means overflow.
        if shift >= 64 || (shift > 57 && low >> (64 - shift) != 0) {
            return Err(ExiError::value("Unsigned Integer", "value exceeds 64 bits"));
        }
        value |= low << shift;
        if octet & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
}

/// Fails with [`Limit::IntegerBits`] when an integer of `bits` bits is over the
/// reader's limit.
#[inline]
fn check_integer_bits<R: ByteSource>(r: &BitReader<R>, bits: u64) -> ExiResult<()> {
    let max = r.integer_bits_limit();
    if bits > max {
        return Err(ExiError::LimitExceeded {
            limit: Limit::IntegerBits,
            max,
        });
    }
    Ok(())
}

/// Reads an unsigned integer of any size, using [`BigUint`]. The reader's
/// [`max_integer_bits`](BitReader::max_integer_bits) applies.
#[cfg(test)]
pub fn read_unsigned_big<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<BigUint> {
    let mut groups = Vec::new();
    loop {
        check_integer_bits(r, 7 * (groups.len() as u64 + 1))?;
        let octet = r.read_byte()?;
        groups.push(octet & 0x7F);
        if octet & 0x80 == 0 {
            break;
        }
    }
    Ok(BigUint::from_7bit_groups(&groups))
}

/// Reads an unsigned integer, returning `u64` when it fits and the big form otherwise.
///
/// Values that fit in `u64` (nearly all) are read without allocating. Once a
/// value needs more than 64 bits, the groups go to a [`BigUint`]. Groups of all zero
/// bits past the 64th bit (allowed, though not the standard short form) still read as
/// the same value.
///
/// The reader's [`max_integer_bits`](BitReader::max_integer_bits) is checked as
/// each group arrives, so an endless run of groups fails fast, before any
/// [`BigUint`] is built (building one costs time that grows with the square of
/// its length).
pub fn read_unsigned_any<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<Uint> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        check_integer_bits(r, u64::from(shift) + 7)?;
        let octet = r.read_byte()?;
        let low = u64::from(octet & 0x7F);
        // A zero group always fits, since it adds nothing. While `shift` is 57 or less
        // (in practice up to 56) every whole group fits; at shift 63 only the lowest
        // bit may be set.
        let fits = low == 0 || shift <= 57 || (shift < 64 && low >> (64 - shift) == 0);
        if !fits {
            // Switch to the big form: rebuild the groups read so far from `value`,
            // then add this group and the rest of the stream.
            let mut groups = Vec::with_capacity(shift as usize / 7 + 4);
            for s in (0..shift).step_by(7) {
                groups.push((value.checked_shr(s).unwrap_or(0) & 0x7F) as u8);
            }
            groups.push(octet & 0x7F);
            let mut more = octet & 0x80 != 0;
            while more {
                check_integer_bits(r, 7 * (groups.len() as u64 + 1))?;
                let octet = r.read_byte()?;
                groups.push(octet & 0x7F);
                more = octet & 0x80 != 0;
            }
            let big = BigUint::from_7bit_groups(&groups);
            return Ok(match big.to_u64() {
                Some(v) => Uint::Small(v),
                None => Uint::Big(big),
            });
        }
        if low != 0 {
            value |= low << shift;
        }
        if octet & 0x80 == 0 {
            return Ok(Uint::Small(value));
        }
        // Saturating, so a long run of zero groups cannot overflow `shift`.
        shift = shift.saturating_add(7);
    }
}

/// An unsigned integer value of any size: `Small` when it fits in `u64`, `Big`
/// otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Uint {
    /// A value that fits in `u64`.
    Small(u64),
    /// A value too big for `u64`.
    Big(BigUint),
}

impl Uint {
    /// Writes the value as an Unsigned Integer.
    pub fn write<W: ByteSink>(&self, w: &mut BitWriter<W>) -> ExiResult<()> {
        match self {
            Uint::Small(v) => write_unsigned(w, *v),
            Uint::Big(b) => write_unsigned_big(w, b),
        }
    }

    /// Parses base-10 digits; `None` when the text is not a plain unsigned number.
    pub fn from_decimal_str(s: &str) -> Option<Self> {
        if let Ok(v) = s.parse::<u64>() {
            return Some(Uint::Small(v));
        }
        BigUint::from_decimal_str(s).map(Uint::Big)
    }

    /// The value as a `u64`, or `None` when it does not fit.
    pub fn to_u64(&self) -> Option<u64> {
        match self {
            Uint::Small(v) => Some(*v),
            Uint::Big(b) => b.to_u64(),
        }
    }

    /// Whether the value is zero.
    pub fn is_zero(&self) -> bool {
        match self {
            Uint::Small(v) => *v == 0,
            Uint::Big(b) => b.is_zero(),
        }
    }
}

impl core::fmt::Display for Uint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Uint::Small(v) => write!(f, "{v}"),
            Uint::Big(b) => write!(f, "{b}"),
        }
    }
}

impl From<u64> for Uint {
    fn from(v: u64) -> Self {
        Uint::Small(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(f: impl FnOnce(&mut BitWriter<Vec<u8>>) -> ExiResult<()>) -> Vec<u8> {
        let mut w = BitWriter::new(Vec::new());
        f(&mut w).unwrap_or_default();
        w.finish().unwrap_or_default()
    }

    #[test]
    fn exip_vectors() {
        // check_streamIO.c :: test_encodeUnsignedInteger
        assert_eq!(enc(|w| write_unsigned(w, 421)), vec![0xA5, 0x03]);
        assert_eq!(enc(|w| write_unsigned(w, 0)), vec![0x00]);
        // check_streamIO.c :: test_decodeUnsignedInteger over D4 60 48 -> 12372
        let mut r = BitReader::new([0xD4u8, 0x60, 0x48].as_slice());
        assert_eq!(read_unsigned(&mut r), Ok(12372));
    }

    #[test]
    fn exificient_unsigned_integer_values_round_trip() {
        for v in [
            0u64,
            1,
            2,
            127,
            128,
            200,
            2000,
            20000,
            200000,
            2000000,
            u32::MAX as u64,
            u64::MAX,
        ] {
            let bytes = enc(|w| write_unsigned(w, v));
            let mut r = BitReader::new(bytes.as_slice());
            assert_eq!(read_unsigned(&mut r), Ok(v), "{v}");
        }
    }

    #[test]
    fn big_values_round_trip_and_match_small_path() {
        let big = BigUint::from_decimal_str("12678967543233").unwrap_or_default();
        let a = enc(|w| write_unsigned_big(w, &big));
        let b = enc(|w| write_unsigned(w, 12678967543233));
        assert_eq!(a, b);
        let huge = BigUint::from_decimal_str("123456789012345678901234567890").unwrap_or_default();
        let bytes = enc(|w| write_unsigned_big(w, &huge));
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(read_unsigned_big(&mut r), Ok(huge.clone()));
        let mut r = BitReader::new(bytes.as_slice());
        assert!(read_unsigned(&mut r).is_err());
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(read_unsigned_any(&mut r), Ok(Uint::Big(huge)));
    }

    #[test]
    fn byte_aligned_is_identical_for_varints() {
        let mut w = BitWriter::with_alignment(Vec::new(), true);
        write_unsigned(&mut w, 421).unwrap_or_default();
        assert_eq!(w.finish().unwrap_or_default(), vec![0xA5, 0x03]);
    }

    /// The earlier path: every value through `BigUint`.
    fn read_any_reference(bytes: &[u8]) -> ExiResult<Uint> {
        let mut r = BitReader::new(bytes);
        let big = read_unsigned_big(&mut r)?;
        Ok(match big.to_u64() {
            Some(v) => Uint::Small(v),
            None => Uint::Big(big),
        })
    }

    fn read_any(bytes: &[u8]) -> ExiResult<Uint> {
        read_unsigned_any(&mut BitReader::new(bytes))
    }

    #[test]
    fn integer_bits_limit_counts_7_bits_per_group() {
        let read = |bytes: &[u8], max: u32| {
            let mut r = BitReader::new(bytes);
            r.set_max_integer_bits(Some(max));
            read_unsigned_any(&mut r)
        };
        let over = |max: u64| {
            Err(ExiError::LimitExceeded {
                limit: Limit::IntegerBits,
                max,
            })
        };
        // A cap below 7 refuses even a one-group integer.
        assert_eq!(read(&[0x05], 6), over(6));
        assert_eq!(read(&[0x05], 7), Ok(Uint::Small(5)));
        assert_eq!(read(&[0x85, 0x01], 13), over(13));
        assert_eq!(read(&[0x85, 0x01], 14), Ok(Uint::Small(133)));
    }

    #[test]
    fn read_unsigned_any_matches_the_big_path() {
        let mut cases: Vec<Vec<u8>> = Vec::new();
        // Standard (shortest) encodings of values around every 7-bit and the 64-bit
        // boundary.
        let mut values: Vec<u64> = vec![0, 1, 127, 128, 16_383, 16_384, u64::MAX, u64::MAX - 1];
        for b in 0..64 {
            values.push(1u64 << b);
            values.push((1u64 << b).wrapping_sub(1));
        }
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..2000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            values.push(state >> (state % 64));
        }
        for v in values {
            let mut w = BitWriter::new(Vec::new());
            assert_eq!(write_unsigned(&mut w, v), Ok(()));
            cases.push(w.finish().unwrap_or_default());
        }
        // Beyond 64 bits, and longer-than-needed encodings that end in zero groups.
        cases.push(vec![0xff; 9].into_iter().chain([0x7f]).collect());
        cases.push(vec![0xff; 10].into_iter().chain([0x01]).collect());
        cases.push(vec![0x80; 12].into_iter().chain([0x00]).collect());
        cases.push(
            vec![0x81]
                .into_iter()
                .chain(vec![0x80; 12])
                .chain([0x00])
                .collect(),
        );
        cases.push(
            vec![0x81]
                .into_iter()
                .chain(vec![0x80; 12])
                .chain([0x01])
                .collect(),
        );
        cases.push(vec![
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80, 0x01,
        ]);
        cases.push(vec![
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01,
        ]);
        cases.push(vec![
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02,
        ]);
        cases.push(vec![0x80, 0x80]); // truncated
        for bytes in cases {
            assert_eq!(read_any(&bytes), read_any_reference(&bytes), "{bytes:02x?}");
        }
    }

    #[test]
    fn write_unsigned_matches_one_octet_at_a_time() {
        // The earlier writer: one write_byte per 7-bit group.
        fn reference(w: &mut BitWriter<Vec<u8>>, mut value: u64) {
            loop {
                let mut octet = (value & 0x7F) as u8;
                value >>= 7;
                if value != 0 {
                    octet |= 0x80;
                }
                assert_eq!(w.write_byte(octet), Ok(()));
                if value == 0 {
                    return;
                }
            }
        }
        let mut values: Vec<u64> = vec![0, 1, 127, 128, u64::MAX];
        for b in 0..64 {
            values.push(1 << b);
            values.push((1u64 << b) - 1);
            values.push((1u64 << b) + 1);
        }
        for offset in 0..8u8 {
            for &v in &values {
                let mut a = BitWriter::new(Vec::new());
                let mut b = BitWriter::new(Vec::new());
                assert_eq!(a.write_raw_bits(0x55, offset), Ok(()));
                assert_eq!(b.write_raw_bits(0x55, offset), Ok(()));
                assert_eq!(write_unsigned(&mut a, v), Ok(()));
                reference(&mut b, v);
                assert_eq!(a.bit_position(), b.bit_position());
                assert_eq!(a.finish(), b.finish(), "{v} at offset {offset}");
            }
        }
    }
}
