use super::bytes::ByteSink;

use crate::error::{ExiError, ExiResult};

/// Writes bits MSB-first (highest bit of each byte first) to any [`ByteSink`].
///
/// It can work bit-packed or byte-aligned (see the module docs).
///
/// When bytes reach the sink:
///
/// * A byte goes to the sink as soon as all 8 of its bits are written. One call to
///   `write_raw_bits` sends every byte it completes in a single `put_bytes` call.
/// * A partly filled last byte stays in `pending` until later writes fill it, or until
///   `align_to_byte` or `finish` pads it with zero bits.
/// * `flush` only asks the sink to flush its own buffer. It does **not** send the
///   partly filled byte; use `finish` (or `align_to_byte` first) for that.
#[derive(Debug)]
pub struct BitWriter<W: ByteSink> {
    inner: W,
    /// The bits waiting to be written, right-aligned: they sit in the low `pending_len`
    /// bits, the oldest bit is the highest of them, and every bit above them is zero.
    pending: u8,
    /// How many bits are waiting in `pending` (0 to 7). It is never 8, because a full
    /// byte is sent to the sink right away.
    pending_len: u8,
    byte_aligned: bool,
    bits_written: u64,
}

impl<W: ByteSink> BitWriter<W> {
    /// Creates a bit-packed writer.
    pub fn new(inner: W) -> Self {
        Self::with_alignment(inner, false)
    }

    /// Creates a writer in bit-packed (`false`) or byte-aligned (`true`) mode.
    pub fn with_alignment(inner: W, byte_aligned: bool) -> Self {
        Self {
            inner,
            pending: 0,
            pending_len: 0,
            byte_aligned,
            bits_written: 0,
        }
    }

    /// Whether the writer is in byte-aligned mode.
    pub fn is_byte_aligned(&self) -> bool {
        self.byte_aligned
    }

    /// Switches alignment mode. Used once, after the header, when the options
    /// document says byte alignment is on. Pads to a byte boundary with zero bits
    /// first.
    pub fn set_byte_aligned(&mut self, byte_aligned: bool) -> ExiResult<()> {
        self.align_to_byte()?;
        self.byte_aligned = byte_aligned;
        Ok(())
    }

    /// Total number of bits written so far, padding included.
    pub fn bit_position(&self) -> u64 {
        self.bits_written
    }

    /// Writes the low `n` bits of `value`, most significant first, ignoring alignment.
    /// Bits of `value` above `n` are ignored.
    ///
    /// The pending bits and `value` are joined in one word, and every byte that word
    /// completes goes to the sink in a single call. The steps:
    ///
    /// 1. `total` = waiting bits + new bits (at most 7 + 64 = 71).
    /// 2. Under 8 bits in all: no byte is complete, so just add the bits to `pending`.
    /// 3. Otherwise build `acc = (pending << n) | value`. Its top `total / 8` bytes are
    ///    complete (at most 8); they are copied out highest first. The low
    ///    `total % 8` bits become the new `pending`.
    /// 4. `acc` is a `u64` when it fits (`total <= 64`, and `n < 64` so the shift is
    ///    legal). Otherwise it is a `u128`.
    ///
    /// `bits_written` is counted before the sink call, so if the sink fails it already
    /// includes these bits.
    #[inline]
    pub fn write_raw_bits(&mut self, value: u64, n: u8) -> ExiResult<()> {
        if n > 64 {
            return Err(ExiError::Internal("bit width exceeds 64"));
        }
        if n == 0 {
            return Ok(());
        }
        let value = if n == 64 {
            value
        } else {
            value & ((1u64 << n) - 1)
        };
        let total = u32::from(self.pending_len) + u32::from(n); // at most 71
        self.bits_written += u64::from(n);
        if total < 8 {
            // No byte completes (most event codes): stay in the pending byte.
            self.pending = ((u64::from(self.pending) << n) | value) as u8;
            self.pending_len = total as u8;
            return Ok(());
        }
        let full = (total / 8) as usize;
        let rest = total % 8;
        // At most 71 / 8 = 8 complete bytes, so 9 slots is always enough.
        let mut buf = [0u8; 9];
        if total <= 64 && n < 64 {
            let acc = (u64::from(self.pending) << n) | value;
            for (i, b) in buf.iter_mut().take(full).enumerate() {
                *b = (acc >> (total - 8 * (i as u32 + 1))) as u8;
            }
            self.pending = (acc & ((1u64 << rest) - 1)) as u8;
        } else {
            let acc = (u128::from(self.pending) << n) | u128::from(value);
            for (i, b) in buf.iter_mut().take(full).enumerate() {
                *b = (acc >> (total - 8 * (i as u32 + 1))) as u8;
            }
            self.pending = (acc & ((1u128 << rest) - 1)) as u8;
        }
        self.pending_len = rest as u8;
        // Fixed lengths for the common cases let the compiler inline the sink's copy.
        match buf {
            [a, ..] if full == 1 => self.inner.put_bytes(&[a])?,
            [a, b, ..] if full == 2 => self.inner.put_bytes(&[a, b])?,
            [a, b, c, ..] if full == 3 => self.inner.put_bytes(&[a, b, c])?,
            _ => self.inner.put_bytes(buf.get(..full).unwrap_or_default())?,
        }
        Ok(())
    }

    /// Writes an n-bit unsigned integer and follows the alignment mode
    /// ([EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)).
    ///
    /// Bit-packed: exactly `n` bits. Byte-aligned: `ceil(n / 8)` whole bytes, least
    /// significant byte first. Fails if `value` does not fit in `n` bits.
    /// `n == 0` writes nothing.
    #[inline]
    pub fn write_nbit_unsigned(&mut self, value: u64, n: u8) -> ExiResult<()> {
        if n == 0 {
            return Ok(());
        }
        if n < 64 && value >> n != 0 {
            return Err(ExiError::Internal(
                "n-bit unsigned integer value exceeds width",
            ));
        }
        if self.byte_aligned {
            // ceil(n / 8) octets, least significant first, in one write.
            let octets = value.to_le_bytes();
            let len = usize::from(n.div_ceil(8));
            self.write_bytes(octets.get(..len).unwrap_or_default())
        } else {
            self.write_raw_bits(value, n)
        }
    }

    /// Writes a boolean ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)): one
    /// bit, or one octet when byte-aligned.
    pub fn write_bool(&mut self, value: bool) -> ExiResult<()> {
        self.write_nbit_unsigned(u64::from(value), 1)
    }

    /// Writes one octet, ignoring alignment. In bit-packed mode it may cross a byte
    /// boundary (part in one output byte, part in the next).
    pub fn write_byte(&mut self, byte: u8) -> ExiResult<()> {
        if self.pending_len == 0 {
            self.inner.put_bytes(&[byte])?;
            self.bits_written += 8;
            Ok(())
        } else {
            self.write_raw_bits(u64::from(byte), 8)
        }
    }

    /// Writes all of `bytes`, ignoring alignment.
    ///
    /// On a byte boundary it hands the slice to the sink in one call. Off a boundary
    /// every byte must be shifted, so it packs up to eight bytes into one `u64` and
    /// writes that with `write_raw_bits`.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> ExiResult<()> {
        if self.pending_len == 0 {
            self.inner.put_bytes(bytes)?;
            self.bits_written += 8 * bytes.len() as u64;
            Ok(())
        } else {
            // Off a byte boundary: shift eight octets at a time.
            for chunk in bytes.chunks(8) {
                let word = chunk.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
                self.write_raw_bits(word, (8 * chunk.len()) as u8)?;
            }
            Ok(())
        }
    }

    /// Pads with zero bits up to the next byte boundary. Does nothing when already on
    /// one.
    pub fn align_to_byte(&mut self) -> ExiResult<()> {
        if self.pending_len > 0 {
            let pad = 8 - self.pending_len;
            self.write_raw_bits(0, pad)?;
        }
        Ok(())
    }

    /// Pads the final byte with zeros, flushes, and returns the underlying writer.
    pub fn finish(mut self) -> ExiResult<W> {
        self.align_to_byte()?;
        self.inner.flush_bytes()?;
        Ok(self.inner)
    }

    /// Flushes the sink. Does not pad or send the partly filled byte in `pending`.
    pub fn flush(&mut self) -> ExiResult<()> {
        self.inner.flush_bytes()?;
        Ok(())
    }

    /// The sink, holding only the complete bytes written so far.
    pub fn get_ref(&self) -> &W {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::bit_writer_reference::ReferenceWriter;
    use crate::prelude::*;

    /// Speed of both writers on an EXI-like mix (event codes of 0-4 bits, 7-bit
    /// groups, short strings). Run with `--ignored --nocapture` in release mode.
    #[test]
    #[ignore]
    #[cfg(feature = "std")] // timing needs std::time
    fn throughput_against_the_reference_writer() {
        macro_rules! mix {
            ($w:expr) => {
                for i in 0..200_000u64 {
                    let _ = $w.write_raw_bits(i & 3, 2);
                    let _ = $w.write_raw_bits(i & 1, 1);
                    let _ = $w.write_raw_bits(0x5a, 7);
                    let _ = $w.write_bytes(&[0x81, 0x22]);
                    let _ = $w.write_raw_bits(i & 0xf, 4);
                    let _ = $w.write_bytes(b"lat");
                }
            };
        }
        for round in 0..3 {
            let t = std::time::Instant::now();
            let mut a = BitWriter::new(Vec::with_capacity(4 << 20));
            mix!(a);
            let new = t.elapsed();
            let t = std::time::Instant::now();
            let mut b = ReferenceWriter::new(Vec::with_capacity(4 << 20));
            mix!(b);
            let old = t.elapsed();
            std::println!("round {round}: new {new:?} old {old:?}");
            assert_eq!(a.get_ref(), b.get_ref());
        }
    }

    /// Random sequences of every write operation give the same bytes and bit
    /// positions as the earlier writer.
    #[test]
    fn identical_output_to_the_reference_writer() {
        let mut state = 0x5eed_1234_abcd_ef01u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..2000 {
            let aligned = next() % 2 == 0;
            let mut a = BitWriter::with_alignment(Vec::new(), aligned);
            let mut b = ReferenceWriter::with_alignment(Vec::new(), aligned);
            for _ in 0..(next() % 40) {
                let r = next();
                let v = next();
                match r % 7 {
                    0 => {
                        let n = (r >> 8) as u8 % 65;
                        assert_eq!(a.write_raw_bits(v, n), b.write_raw_bits(v, n));
                    }
                    1 => {
                        let n = (r >> 8) as u8 % 65;
                        let x = if n == 64 {
                            v
                        } else if n == 0 {
                            0
                        } else {
                            v & ((1u64 << n) - 1)
                        };
                        assert_eq!(a.write_nbit_unsigned(x, n), b.write_nbit_unsigned(x, n));
                    }
                    2 => assert_eq!(a.write_bool(v % 2 == 1), b.write_bool(v % 2 == 1)),
                    3 => assert_eq!(a.write_byte(v as u8), b.write_byte(v as u8)),
                    4 => {
                        let bytes: Vec<u8> = (0..(v % 20)).map(|i| (v >> (i % 8)) as u8).collect();
                        assert_eq!(a.write_bytes(&bytes), b.write_bytes(&bytes));
                    }
                    5 => assert_eq!(a.align_to_byte(), b.align_to_byte()),
                    _ => {
                        let flag = v % 2 == 0;
                        assert_eq!(a.set_byte_aligned(flag), b.set_byte_aligned(flag));
                    }
                }
                assert_eq!(a.bit_position(), b.bit_position());
                assert_eq!(a.get_ref(), b.get_ref());
            }
            assert_eq!(a.finish(), b.finish());
        }
    }

    fn packed(f: impl FnOnce(&mut BitWriter<Vec<u8>>)) -> Vec<u8> {
        let mut w = BitWriter::new(Vec::new());
        f(&mut w);
        w.finish().unwrap_or_default()
    }

    #[test]
    fn writes_msb_first_across_boundaries() {
        let bytes = packed(|w| {
            w.write_raw_bits(0b101, 3).unwrap_or_default();
            w.write_raw_bits(0b11_0000_1111, 10).unwrap_or_default();
        });
        assert_eq!(bytes, vec![0b1011_1000, 0b0111_1000]);
    }

    #[test]
    fn exip_write_nbits_vector() {
        // exip check_streamIO.c :: test_writeNBits: 19 in 7 bits, then 9 in 5 bits.
        let bytes = packed(|w| {
            w.write_raw_bits(19, 7).unwrap_or_default();
            w.write_raw_bits(9, 5).unwrap_or_default();
        });
        assert_eq!(bytes[0] & 0x01, 0);
        assert_eq!((bytes[1] & 0xF0) >> 4, 9);
    }

    #[test]
    fn nbit_412_in_9_bits_matches_exip() {
        let bytes = packed(|w| w.write_nbit_unsigned(412, 9).unwrap_or_default());
        assert_eq!(bytes[0], 206);
        assert_eq!(bytes[1] >> 7, 0);
    }

    #[test]
    fn all_widths_round_trip_through_vec() {
        for width in 1u8..=64 {
            let value = if width == 64 {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            let bytes = packed(|w| {
                w.write_raw_bits(1, 1).unwrap_or_default();
                w.write_raw_bits(value, width).unwrap_or_default();
            });
            let mut r = crate::io::BitReader::new(bytes.as_slice());
            assert_eq!(r.read_raw_bits(1), Ok(1));
            assert_eq!(r.read_raw_bits(width), Ok(value));
        }
    }

    #[test]
    fn byte_aligned_nbit_is_little_endian_octets() {
        let mut w = BitWriter::with_alignment(Vec::new(), true);
        w.write_nbit_unsigned(0x0123, 12).unwrap_or_default();
        w.write_bool(true).unwrap_or_default();
        w.write_nbit_unsigned(0, 0).unwrap_or_default();
        let bytes = w.finish().unwrap_or_default();
        assert_eq!(bytes, vec![0x23, 0x01, 0x01]);
    }

    #[test]
    fn finish_pads_with_zeros() {
        let bytes = packed(|w| w.write_raw_bits(0b1, 1).unwrap_or_default());
        assert_eq!(bytes, vec![0x80]);
    }

    #[test]
    fn rejects_value_wider_than_n() {
        let mut w = BitWriter::new(Vec::new());
        assert!(w.write_nbit_unsigned(8, 3).is_err());
    }
}
