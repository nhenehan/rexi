//! The earlier, simpler bit writer. It is kept as the reference model that
//! `bit_writer.rs` is tested against: both must give identical output.
//!
//! It is simpler and slower: it fills one byte at a time and sends each byte to the
//! sink on its own as soon as it is full.
#![allow(dead_code)]

use super::bytes::ByteSink;

use crate::error::{ExiError, ExiResult};

/// Writes bits MSB-first (highest bit of each byte first) to any [`ByteSink`], in
/// bit-packed or byte-aligned mode.
#[derive(Debug)]
pub(crate) struct ReferenceWriter<W: ByteSink> {
    inner: W,
    /// The bits waiting to be written, right-aligned: they sit in the low `pending_len`
    /// bits, the oldest bit is the highest of them, and every bit above them is zero.
    pending: u8,
    pending_len: u8,
    byte_aligned: bool,
    bits_written: u64,
}

impl<W: ByteSink> ReferenceWriter<W> {
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
    ///
    /// Each loop pass moves as many of the highest remaining bits as fit into the
    /// current byte, and sends the byte when it reaches 8 bits.
    pub fn write_raw_bits(&mut self, value: u64, n: u8) -> ExiResult<()> {
        if n > 64 {
            return Err(ExiError::Internal("bit width exceeds 64"));
        }
        let mut remaining = n;
        while remaining > 0 {
            let room = 8 - self.pending_len;
            let take = remaining.min(room);
            let shift = remaining - take;
            let chunk = (value >> shift) & ((1u64 << take) - 1);
            self.pending = ((u16::from(self.pending) << take) | chunk as u16) as u8;
            self.pending_len += take;
            remaining -= take;
            if self.pending_len == 8 {
                self.inner.put_bytes(&[self.pending])?;
                self.pending = 0;
                self.pending_len = 0;
            }
        }
        self.bits_written += u64::from(n);
        Ok(())
    }

    /// Writes an n-bit unsigned integer and follows the alignment mode
    /// ([EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)). Byte-aligned
    /// mode writes `ceil(n / 8)` bytes, least significant byte first.
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
            let mut v = value;
            let mut bits = n;
            while bits > 0 {
                self.write_raw_bits(v & 0xFF, 8)?;
                v >>= 8;
                bits = bits.saturating_sub(8);
            }
            Ok(())
        } else {
            self.write_raw_bits(value, n)
        }
    }

    /// Writes a boolean ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)): one
    /// bit, or one octet when byte-aligned.
    pub fn write_bool(&mut self, value: bool) -> ExiResult<()> {
        self.write_nbit_unsigned(u64::from(value), 1)
    }

    /// Writes one octet. In bit-packed mode it may cross a byte boundary (part in one
    /// output byte, part in the next).
    pub fn write_byte(&mut self, byte: u8) -> ExiResult<()> {
        if self.pending_len == 0 {
            self.inner.put_bytes(&[byte])?;
            self.bits_written += 8;
            Ok(())
        } else {
            self.write_raw_bits(u64::from(byte), 8)
        }
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> ExiResult<()> {
        if self.pending_len == 0 {
            self.inner.put_bytes(bytes)?;
            self.bits_written += 8 * bytes.len() as u64;
            Ok(())
        } else {
            for &b in bytes {
                self.write_raw_bits(u64::from(b), 8)?;
            }
            Ok(())
        }
    }

    /// Pads with zero bits up to the next byte boundary.
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

    pub fn flush(&mut self) -> ExiResult<()> {
        self.inner.flush_bytes()?;
        Ok(())
    }

    pub fn get_ref(&self) -> &W {
        &self.inner
    }
}
