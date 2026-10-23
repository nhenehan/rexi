use super::bytes::ByteSource;

use crate::error::{ExiError, ExiResult};

/// Reads bits MSB-first (highest bit of each byte first) from any [`ByteSource`].
///
/// It can work bit-packed or byte-aligned (see the module docs). It pulls one byte at
/// a time from the source and hands out its bits as callers ask for them.
#[derive(Debug)]
pub struct BitReader<R: ByteSource> {
    inner: R,
    /// The unread bits of the current byte, right-aligned: they sit in the low
    /// `pending_len` bits, the next bit to read is the highest of them, and every bit
    /// above them is zero.
    pending: u8,
    /// How many unread bits are left in `pending` (0 to 8). 0 means the next read
    /// must pull a new byte from the source.
    pending_len: u8,
    byte_aligned: bool,
    bits_read: u64,
    /// Most bits an unsigned integer may have (7 per group); `u64::MAX` for no
    /// limit. Kept here so every integer reader sees it. See
    /// [`DecodeLimits::max_integer_bits`](crate::DecodeLimits::max_integer_bits).
    max_integer_bits: u64,
}

/// The low `bits` (0 to 8) bits of a byte set.
#[inline]
const fn low_mask(bits: u8) -> u8 {
    if bits >= 8 { 0xFF } else { (1u8 << bits) - 1 }
}

impl<R: ByteSource> BitReader<R> {
    /// Creates a bit-packed reader.
    pub fn new(inner: R) -> Self {
        Self::with_alignment(inner, false)
    }

    /// Creates a reader in bit-packed (`false`) or byte-aligned (`true`) mode.
    pub fn with_alignment(inner: R, byte_aligned: bool) -> Self {
        Self {
            inner,
            pending: 0,
            pending_len: 0,
            byte_aligned,
            bits_read: 0,
            max_integer_bits: u64::MAX,
        }
    }

    /// Sets the most bits one unsigned integer may have, counted as 7 bits per
    /// group ([EXI 7.1.6](https://www.w3.org/TR/exi/#encodingUnsignedInteger)).
    /// `None` means no limit, which is the default for a new reader. The
    /// [`Decoder`](crate::Decoder) sets this from its
    /// [`DecodeLimits`](crate::DecodeLimits).
    pub fn set_max_integer_bits(&mut self, max: Option<u32>) {
        self.max_integer_bits = max.map_or(u64::MAX, u64::from);
    }

    /// The limit set by [`set_max_integer_bits`](Self::set_max_integer_bits).
    pub fn max_integer_bits(&self) -> Option<u32> {
        u32::try_from(self.max_integer_bits).ok()
    }

    /// The same limit as a plain number, `u64::MAX` for none. Cheaper to compare
    /// on every integer group.
    #[inline]
    pub(crate) fn integer_bits_limit(&self) -> u64 {
        self.max_integer_bits
    }

    /// Whether the reader is in byte-aligned mode.
    pub fn is_byte_aligned(&self) -> bool {
        self.byte_aligned
    }

    /// Switches alignment mode after the header. It skips to the next byte boundary
    /// first, throwing away any unread bits of the current byte.
    pub fn set_byte_aligned(&mut self, byte_aligned: bool) {
        self.align_to_byte();
        self.byte_aligned = byte_aligned;
    }

    /// Number of bits used up so far, including skipped padding.
    pub fn bit_position(&self) -> u64 {
        self.bits_read
    }

    /// Pulls the next byte from the source.
    #[inline]
    fn source_byte(&mut self) -> ExiResult<u8> {
        let mut buf = [0u8; 1];
        match self.inner.get_bytes(&mut buf)? {
            0 => Err(ExiError::Truncated),
            _ => Ok(buf[0]),
        }
    }

    /// Reads `n` bits (0..=64), most significant first, ignoring alignment.
    ///
    /// When the waiting bits are enough, they are all it uses. Otherwise it takes
    /// the waiting bits, then whole bytes from the source, then the high bits of one
    /// more byte, whose low bits wait for the next read. If the input ends partway,
    /// it returns [`ExiError::Truncated`]; the bits already taken in that call are
    /// lost and `bit_position` does not count them, since the count is only added at
    /// the end.
    #[inline]
    pub fn read_raw_bits(&mut self, n: u8) -> ExiResult<u64> {
        if n > 64 {
            return Err(ExiError::Internal("bit width exceeds 64"));
        }
        let pl = self.pending_len;
        if n <= pl {
            // All in the waiting bits (most event codes).
            let rest = pl - n;
            let value = u64::from(self.pending) >> rest;
            self.pending &= low_mask(rest);
            self.pending_len = rest;
            self.bits_read += u64::from(n);
            return Ok(value);
        }
        // `value` never holds more than `n` <= 64 bits.
        let mut value = u64::from(self.pending);
        let mut need = n - pl;
        while need >= 8 {
            value = (value << 8) | u64::from(self.source_byte()?);
            need -= 8;
        }
        if need > 0 {
            let b = self.source_byte()?;
            let keep = 8 - need;
            value = (value << need) | u64::from(b >> keep);
            self.pending = b & low_mask(keep);
            self.pending_len = keep;
        } else {
            self.pending = 0;
            self.pending_len = 0;
        }
        self.bits_read += u64::from(n);
        Ok(value)
    }

    /// Reads an n-bit unsigned integer and follows the alignment mode
    /// ([EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)).
    ///
    /// Bit-packed: exactly `n` bits. Byte-aligned: `ceil(n / 8)` whole bytes, least
    /// significant byte first; any extra high bits in the last byte are masked off so
    /// the result fits in `n` bits. `n == 0` reads nothing and returns 0.
    #[inline]
    pub fn read_nbit_unsigned(&mut self, n: u8) -> ExiResult<u64> {
        if n == 0 {
            return Ok(0);
        }
        if self.byte_aligned {
            let mut value = 0u64;
            let mut shift = 0u32;
            let mut bits = n;
            while bits > 0 {
                let byte = u64::from(self.read_byte()?);
                value |= byte << shift;
                shift += 8;
                bits = bits.saturating_sub(8);
            }
            if n < 64 {
                value &= (1u64 << n) - 1;
            }
            Ok(value)
        } else {
            self.read_raw_bits(n)
        }
    }

    /// Reads a boolean ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)): one
    /// bit, or one byte when byte-aligned. In byte-aligned mode only the lowest bit of
    /// that byte counts, because `read_nbit_unsigned` masks the rest off.
    #[inline]
    pub fn read_bool(&mut self) -> ExiResult<bool> {
        Ok(self.read_nbit_unsigned(1)? != 0)
    }

    /// Reads 8 bits as one byte, ignoring alignment. In bit-packed mode they may span
    /// two input bytes.
    ///
    /// The next 8 bits are the waiting bits followed by the high bits of the next
    /// source byte, whose low bits then wait in turn: one shift and one mask.
    #[inline]
    pub fn read_byte(&mut self) -> ExiResult<u8> {
        let b = self.source_byte()?;
        self.bits_read += 8;
        let pl = self.pending_len;
        if pl == 0 {
            return Ok(b);
        }
        if pl >= 8 {
            return Ok(core::mem::replace(&mut self.pending, b));
        }
        let out = (self.pending << (8 - pl)) | (b >> pl);
        self.pending = b & low_mask(pl);
        Ok(out)
    }

    /// Fills `out` with bytes, ignoring alignment.
    ///
    /// On a byte boundary it copies straight from the source in one call. Off a
    /// boundary it also fetches them in one call, then shifts them into place.
    pub fn read_bytes(&mut self, out: &mut [u8]) -> ExiResult<()> {
        let pl = self.pending_len;
        if pl == 0 {
            self.inner.get_bytes_exact(out)?;
            self.bits_read += 8 * out.len() as u64;
            return Ok(());
        }
        if pl >= 8 {
            for slot in out.iter_mut() {
                *slot = self.read_byte()?;
            }
            return Ok(());
        }
        // Off a boundary: fetch the source bytes in one call, then shift them in
        // place. Each output byte is the waiting bits followed by the high bits of
        // the next source byte, whose low bits wait in turn.
        self.inner.get_bytes_exact(out)?;
        let mut carry = self.pending;
        for slot in out.iter_mut() {
            let b = *slot;
            *slot = (carry << (8 - pl)) | (b >> pl);
            carry = b & low_mask(pl);
        }
        self.pending = carry;
        self.bits_read += 8 * out.len() as u64;
        Ok(())
    }

    /// Throws away the unread bits of the current byte. They still count toward
    /// `bit_position`. Does nothing when already on a byte boundary.
    pub fn align_to_byte(&mut self) {
        self.bits_read += u64::from(self.pending_len);
        self.pending_len = 0;
        self.pending = 0;
    }

    /// Gives back the byte source.
    ///
    /// Note: a generic reader cannot peek to see whether more input exists without
    /// using it up. Callers that need to detect the end of the stream use the grammar.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exip_read_bits_vector() {
        // exip check_streamIO.c :: test_readBits over D4 60.
        let mut r = BitReader::new([0xD4u8, 0x60].as_slice());
        assert_eq!(r.read_raw_bits(4), Ok(13));
        assert_eq!(r.bit_position(), 4);
        assert_eq!(r.read_raw_bits(3), Ok(0b010));
        assert_eq!(r.read_raw_bits(5), Ok(6));
        assert_eq!(r.bit_position(), 12);
    }

    #[test]
    fn exip_decode_nbit_6_from_d4_60() {
        let mut r = BitReader::new([0xD4u8, 0x60].as_slice());
        assert_eq!(r.read_nbit_unsigned(6), Ok(53));
    }

    #[test]
    fn truncated_read_reports_truncated() {
        let mut r = BitReader::new([0xA0u8].as_slice());
        assert_eq!(r.read_raw_bits(3), Ok(0b101));
        assert_eq!(r.read_raw_bits(6), Err(ExiError::Truncated));
    }

    #[test]
    fn byte_aligned_reads_little_endian() {
        let mut r = BitReader::with_alignment([0x23u8, 0x01, 0x01].as_slice(), true);
        assert_eq!(r.read_nbit_unsigned(12), Ok(0x0123));
        assert_eq!(r.read_bool(), Ok(true));
    }

    #[test]
    fn align_skips_padding() {
        let mut r = BitReader::new([0b1010_0000u8, 0x42].as_slice());
        assert_eq!(r.read_raw_bits(3), Ok(0b101));
        r.align_to_byte();
        assert_eq!(r.read_byte(), Ok(0x42));
        assert_eq!(r.bit_position(), 16);
    }
}
