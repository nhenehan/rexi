//! String ([EXI 7.1.10](https://www.w3.org/TR/exi/#encodingString)): the length as an
//! Unsigned Integer, then each code point as an Unsigned Integer.
//!
//! The string-table forms ([EXI 7.3](https://www.w3.org/TR/exi/#stringTable)) reuse
//! these pieces, but add an offset to the length. That is why the length and the
//! characters can be written and read on their own.

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;

use super::unsigned::{read_unsigned, write_unsigned};
use crate::error::{ExiError, ExiResult};
use crate::io::{BitReader, BitWriter, ceil_log2};

/// Writes `length` and nothing else. The caller adds any offset first.
pub fn write_length<W: ByteSink>(w: &mut BitWriter<W>, length: u64) -> ExiResult<()> {
    write_unsigned(w, length)
}

/// Writes the code points of `s`, without a length prefix.
pub fn write_chars<W: ByteSink>(w: &mut BitWriter<W>, s: &str) -> ExiResult<()> {
    for ch in s.chars() {
        write_unsigned(w, u64::from(u32::from(ch)))?;
    }
    Ok(())
}

/// Writes the characters of `s` against a restricted character set
/// ([EXI 7.1.10](https://www.w3.org/TR/exi/#encodingString)): each one as an n-bit
/// index into `alphabet` (its characters in code point order), where n is
/// ceil(log2(N + 1)) for N characters; index N escapes a character outside the
/// set, whose code point follows as an Unsigned Integer. No length prefix.
pub fn write_rcs_chars<W: ByteSink>(
    w: &mut BitWriter<W>,
    s: &str,
    alphabet: &str,
) -> ExiResult<()> {
    let n = alphabet.chars().count();
    let bits = ceil_log2(n + 1);
    for ch in s.chars() {
        match alphabet.chars().position(|a| a == ch) {
            Some(i) => w.write_nbit_unsigned(i as u64, bits)?,
            None => {
                w.write_nbit_unsigned(n as u64, bits)?;
                write_unsigned(w, u64::from(u32::from(ch)))?;
            }
        }
    }
    Ok(())
}

/// Reads `count` characters written by [`write_rcs_chars`] into `out`. An index
/// past the escape, or an escaped value that is not a code point, is an error.
pub fn read_rcs_chars<R: ByteSource>(
    r: &mut BitReader<R>,
    count: u64,
    alphabet: &str,
    out: &mut String,
) -> ExiResult<()> {
    let n = alphabet.chars().count();
    let bits = ceil_log2(n + 1);
    out.reserve(count.min(4096) as usize);
    for _ in 0..count {
        let i = r.read_nbit_unsigned(bits)?;
        let ch = if (i as usize) < n {
            alphabet
                .chars()
                .nth(i as usize)
                .ok_or(ExiError::Internal("alphabet index"))?
        } else if i as usize == n {
            let value = read_unsigned(r)?;
            u32::try_from(value)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| ExiError::value("String", format!("invalid code point {value}")))?
        } else {
            return Err(ExiError::value(
                "String",
                format!("restricted character index {i} is past the escape {n}"),
            ));
        };
        out.push(ch);
    }
    Ok(())
}

/// Writes a whole String value: the character count (in code points, not bytes), then
/// the code points.
pub fn write_string<W: ByteSink>(w: &mut BitWriter<W>, s: &str) -> ExiResult<()> {
    write_length(w, s.chars().count() as u64)?;
    write_chars(w, s)
}

/// Reads a length (an Unsigned Integer). The caller removes any offset.
pub fn read_length<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<u64> {
    read_unsigned(r)
}

/// Reads `count` code points into `out`.
///
/// A code point is an Unsigned Integer of 7-bit groups
/// ([EXI 7.1.6](https://www.w3.org/TR/exi/#encodingUnsignedInteger)). Every valid
/// one fits in three groups (U+10FFFF is 21 bits), so one to three octets are
/// read with a few branches. A fourth octet means a value too large for a code
/// point; `read_long_code_point` reads the rest with the same checks as
/// [`read_unsigned`] and reports it.
pub fn read_chars<R: ByteSource>(
    r: &mut BitReader<R>,
    count: u64,
    out: &mut String,
) -> ExiResult<()> {
    // Up to four bytes per character, so text in any script fits at once (the
    // buffer is reused for the next value). Capped, so a bad count cannot ask
    // for a huge buffer up front.
    out.reserve(4 * count.min(4096) as usize);
    for _ in 0..count {
        let a = r.read_byte()?;
        let value = if a < 0x80 {
            u32::from(a)
        } else {
            let b = r.read_byte()?;
            if b < 0x80 {
                u32::from(a & 0x7F) | (u32::from(b) << 7)
            } else {
                let c = r.read_byte()?;
                if c < 0x80 {
                    u32::from(a & 0x7F) | (u32::from(b & 0x7F) << 7) | (u32::from(c) << 14)
                } else {
                    let low = u64::from(a & 0x7F)
                        | (u64::from(b & 0x7F) << 7)
                        | (u64::from(c & 0x7F) << 14);
                    return Err(read_long_code_point(r, low));
                }
            }
        };
        let ch = char::from_u32(value)
            .ok_or_else(|| ExiError::value("String", format!("invalid code point {value}")))?;
        out.push(ch);
    }
    Ok(())
}

/// Reads the rest of a code point that did not end in three octets (`low` holds
/// the first 21 bits) and returns the error to report: no such code point.
#[cold]
fn read_long_code_point<R: ByteSource>(r: &mut BitReader<R>, low: u64) -> ExiError {
    let mut value = low;
    let mut shift = 21u32;
    loop {
        let octet = match r.read_byte() {
            Ok(o) => o,
            Err(e) => return e,
        };
        let group = u64::from(octet & 0x7F);
        if shift >= 64 || (shift > 57 && group >> (64 - shift) != 0) {
            return ExiError::value("Unsigned Integer", "value exceeds 64 bits");
        }
        value |= group << shift;
        if octet & 0x80 == 0 {
            return ExiError::value("String", format!("invalid code point {value}"));
        }
        shift += 7;
    }
}

/// Reads a whole String value.
pub fn read_string<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<String> {
    let len = read_length(r)?;
    let mut s = String::new();
    read_chars(r, len, &mut s)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exip_vectors() {
        // check_streamIO.c :: test_encodeString
        let mut w = BitWriter::new(Vec::new());
        write_string(&mut w, "TEST encodeString()").unwrap_or_default();
        let bytes = w.finish().unwrap_or_default();
        assert_eq!(bytes[0], 19);
        assert_eq!(&bytes[1..3], b"TE");
        assert_eq!(bytes.len(), 20);
        // check_streamIO.c :: test_decodeString over 02 65 54 -> "eT"
        let mut r = BitReader::new([0x02u8, 0x65, 0x54].as_slice());
        assert_eq!(read_string(&mut r), Ok("eT".to_string()));
    }

    #[test]
    fn non_ascii_counts_code_points() {
        let mut w = BitWriter::new(Vec::new());
        write_string(&mut w, "héllo €").unwrap_or_default();
        let bytes = w.finish().unwrap_or_default();
        assert_eq!(bytes[0], 7);
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(read_string(&mut r), Ok("héllo €".to_string()));
    }

    /// Long strings mixing one- to three-octet code points, read on and off a
    /// byte boundary. The bits after each must still be there.
    #[test]
    fn long_mixed_strings_round_trip() {
        let pieces = ["a", "é", "€", "𝄞", "ж"];
        for n in [255, 256, 257, 300, 1000] {
            let text: String = (0..n).map(|i| pieces[i % pieces.len()]).collect();
            for offset in [0u8, 3] {
                let mut w = BitWriter::new(Vec::new());
                w.write_raw_bits(0, offset).unwrap_or_default();
                write_string(&mut w, &text).unwrap_or_default();
                w.write_raw_bits(0b101, 3).unwrap_or_default();
                let bytes = w.finish().unwrap_or_default();
                let mut r = BitReader::new(bytes.as_slice());
                assert_eq!(r.read_raw_bits(offset), Ok(0));
                assert_eq!(
                    read_string(&mut r),
                    Ok(text.clone()),
                    "{n} chars, {offset} bits"
                );
                assert_eq!(r.read_raw_bits(3), Ok(0b101));
            }
        }
    }

    #[test]
    fn code_point_over_three_octets_is_refused() {
        // One character: 0x80 0x80 0x80 0x01 is 2^21, above U+10FFFF.
        let mut r = BitReader::new([0x01u8, 0x80, 0x80, 0x80, 0x01].as_slice());
        assert!(matches!(read_string(&mut r), Err(ExiError::Value { .. })));
        // Truncated inside a character.
        let mut r = BitReader::new([0x02u8, 0x41, 0xC3].as_slice());
        assert_eq!(read_string(&mut r), Err(ExiError::Truncated));
    }
}
