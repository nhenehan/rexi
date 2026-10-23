//! Binary ([EXI 7.1.1](https://www.w3.org/TR/exi/#encodingBinary)): the length as an
//! Unsigned Integer, then the raw octets.
//!
//! The base64 and hex text forms
//! ([`xsd:base64Binary`](https://www.w3.org/TR/xmlschema-2/#base64Binary),
//! [`xsd:hexBinary`](https://www.w3.org/TR/xmlschema-2/#hexBinary)) work the same as
//! EXIficient's `BinaryBase64Value` and `BinaryHexValue` (both taken from Xerces):
//!
//! - base64 decoding drops whitespace anywhere in the value and needs padding up to a
//!   multiple of four characters;
//! - base64 output uses the standard alphabet
//!   ([RFC 4648 section 4](https://www.rfc-editor.org/rfc/rfc4648#section-4)) with `=`
//!   padding and no line breaks;
//! - hex decoding accepts upper or lower case, and hex output is upper case.

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;

use super::unsigned::{read_unsigned, write_unsigned};
use crate::error::{ExiError, ExiResult};
use crate::io::{BitReader, BitWriter};

const NAME: &str = "Binary";
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const HEX_ALPHABET: &[u8; 16] = b"0123456789ABCDEF";

/// Writes a Binary value
/// ([EXI 7.1.1](https://www.w3.org/TR/exi/#encodingBinary)): the byte count as an
/// Unsigned Integer, then the bytes as they are.
pub fn write_binary<W: ByteSink>(w: &mut BitWriter<W>, bytes: &[u8]) -> ExiResult<()> {
    write_unsigned(w, bytes.len() as u64)?;
    w.write_bytes(bytes)
}

/// Reads a Binary value: the byte count, then that many bytes.
pub fn read_binary<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<Vec<u8>> {
    let mut out = Vec::new();
    read_binary_into(r, &mut out)?;
    Ok(out)
}

/// Reads a Binary value into `out`, replacing what it held. Reusing one buffer
/// for every value saves an allocation each time.
pub fn read_binary_into<R: ByteSource>(r: &mut BitReader<R>, out: &mut Vec<u8>) -> ExiResult<()> {
    out.clear();
    let len = read_unsigned(r)?;
    let mut remaining =
        usize::try_from(len).map_err(|_| ExiError::value(NAME, "length exceeds usize"))?;
    // Grow in steps of at most 64 KiB, so a broken length gives `Truncated` when the
    // stream runs out, not a failed allocation.
    out.reserve(remaining.min(1 << 16));
    while remaining > 0 {
        let take = remaining.min(1 << 16);
        let start = out.len();
        out.resize(start + take, 0);
        r.read_bytes(&mut out[start..])?;
        remaining -= take;
    }
    Ok(())
}

fn base64_value(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn is_xml_whitespace(c: u8) -> bool {
    matches!(c, b' ' | b'\r' | b'\n' | b'\t')
}

/// Decodes `xsd:base64Binary`; whitespace anywhere is ignored.
///
/// Each group of 4 characters (6 bits each) gives 3 bytes. Only the last group may
/// hold `=` padding: `xx==` gives 1 byte and `xxx=` gives 2. With padding, the bits
/// left over in the last real character must be zero, or the input is rejected.
pub fn parse_base64(s: &str) -> Option<Vec<u8>> {
    let data: Vec<u8> = s.bytes().filter(|c| !is_xml_whitespace(*c)).collect();
    if !data.len().is_multiple_of(4) {
        return None;
    }
    let quads = data.len() / 4;
    if quads == 0 {
        return Some(Vec::new());
    }
    let mut out = Vec::with_capacity(quads * 3);
    for quad in data[..(quads - 1) * 4].as_chunks::<4>().0 {
        let b1 = base64_value(quad[0])?;
        let b2 = base64_value(quad[1])?;
        let b3 = base64_value(quad[2])?;
        let b4 = base64_value(quad[3])?;
        out.push((b1 << 2) | (b2 >> 4));
        out.push(((b2 & 0x0F) << 4) | ((b3 >> 2) & 0x0F));
        out.push((b3 << 6) | b4);
    }
    let last = &data[(quads - 1) * 4..];
    let b1 = base64_value(last[0])?;
    let b2 = base64_value(last[1])?;
    match (
        base64_value(last[2]),
        base64_value(last[3]),
        last[2] == b'=',
        last[3] == b'=',
    ) {
        // `xx==`: one byte. The low 4 bits of the second character are unused.
        (None, None, true, true) => {
            if b2 & 0x0F != 0 {
                return None;
            }
            out.push((b1 << 2) | (b2 >> 4));
        }
        // `xxx=`: two bytes. The low 2 bits of the third character are unused.
        (Some(b3), None, false, true) => {
            if b3 & 0x03 != 0 {
                return None;
            }
            out.push((b1 << 2) | (b2 >> 4));
            out.push(((b2 & 0x0F) << 4) | ((b3 >> 2) & 0x0F));
        }
        (Some(b3), Some(b4), _, _) => {
            out.push((b1 << 2) | (b2 >> 4));
            out.push(((b2 & 0x0F) << 4) | ((b3 >> 2) & 0x0F));
            out.push((b3 << 6) | b4);
        }
        _ => return None,
    }
    Some(out)
}

/// Encodes bytes as base64: each 3 bytes become 4 characters, and a last group of 1
/// or 2 bytes is padded with `==` or `=`.
pub fn format_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let (chunks, remainder) = bytes.as_chunks::<3>();
    for chunk in chunks {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        for shift in [18, 12, 6, 0] {
            out.push(char::from(BASE64_ALPHABET[((n >> shift) & 0x3F) as usize]));
        }
    }
    match remainder {
        [a] => {
            let n = u32::from(*a) << 16;
            out.push(char::from(BASE64_ALPHABET[((n >> 18) & 0x3F) as usize]));
            out.push(char::from(BASE64_ALPHABET[((n >> 12) & 0x3F) as usize]));
            out.push_str("==");
        }
        [a, b] => {
            let n = (u32::from(*a) << 16) | (u32::from(*b) << 8);
            out.push(char::from(BASE64_ALPHABET[((n >> 18) & 0x3F) as usize]));
            out.push(char::from(BASE64_ALPHABET[((n >> 12) & 0x3F) as usize]));
            out.push(char::from(BASE64_ALPHABET[((n >> 6) & 0x3F) as usize]));
            out.push('=');
        }
        _ => {}
    }
    out
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Decodes `xsd:hexBinary` (no whitespace allowed, upper or lower case letters).
pub fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Some((hex_value(pair[0])? << 4) | hex_value(pair[1])?))
        .collect()
}

/// The bytes as upper-case hex text (`[0xAB, 0x01]` -> `"AB01"`), the standard
/// form of `xsd:hexBinary`.
pub fn format_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(HEX_ALPHABET[usize::from(b >> 4)]));
        out.push(char::from(HEX_ALPHABET[usize::from(b & 0x0F)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(aligned: bool, f: impl FnOnce(&mut BitWriter<Vec<u8>>) -> ExiResult<()>) -> Vec<u8> {
        let mut w = BitWriter::with_alignment(Vec::new(), aligned);
        f(&mut w).unwrap_or_default();
        w.finish().unwrap_or_default()
    }

    fn round_trip(aligned: bool, bytes: &[u8]) -> Vec<u8> {
        let stream = enc(aligned, |w| write_binary(w, bytes));
        let mut r = BitReader::with_alignment(stream.as_slice(), aligned);
        read_binary(&mut r).unwrap_or_default()
    }

    #[test]
    fn exip_vectors() {
        // check_streamIO.c :: test_encodeBinary: 22 65 D4 5A D7 -> 05 22 65 D4 5A D7
        let data = [0x22u8, 0x65, 0xD4, 0x5A, 0xD7];
        assert_eq!(
            enc(false, |w| write_binary(w, &data)),
            vec![0x05, 0x22, 0x65, 0xD4, 0x5A, 0xD7]
        );
        assert_eq!(
            enc(true, |w| write_binary(w, &data)),
            vec![0x05, 0x22, 0x65, 0xD4, 0x5A, 0xD7]
        );
        // check_streamIO.c :: test_decodeBinary: two values back to back.
        let stream = [
            0x05u8, 0xF0, 0xCC, 0xAA, 0x55, 0x33, 0x08, 0x6E, 0xCA, 0x59, 0xD8, 0x59, 0xCA, 0x6C,
            0xD8, 0x07,
        ];
        let mut r = BitReader::new(stream.as_slice());
        assert_eq!(read_binary(&mut r), Ok(vec![0xF0, 0xCC, 0xAA, 0x55, 0x33]));
        assert_eq!(r.bit_position(), 6 * 8);
        assert_eq!(
            read_binary(&mut r),
            Ok(vec![0x6E, 0xCA, 0x59, 0xD8, 0x59, 0xCA, 0x6C, 0xD8])
        );
        assert_eq!(r.bit_position(), 15 * 8);
    }

    #[test]
    fn exificient_hex_binary() {
        // BinaryCoreTest :: testHexBinaryAsString0FB7, testHexBinary_1 (mixed case -> upper).
        assert_eq!(parse_hex("0FB7"), Some(vec![0x0F, 0xB7]));
        assert_eq!(format_hex(&[0x0F, 0xB7]), "0FB7");
        let input = "0123456789ABCDEFabcdef".repeat(20);
        let expected = "0123456789ABCDEFABCDEF".repeat(20);
        let bytes = parse_hex(&input).unwrap_or_default();
        assert_eq!(bytes.len(), 220);
        assert_eq!(format_hex(&bytes), expected);
        assert_eq!(format_hex(&round_trip(false, &bytes)), expected);
        assert_eq!(format_hex(&round_trip(true, &bytes)), expected);
        // testHexBinaryFailure1..4
        for s in [
            "ZHM=",
            "RGFzIGlzIGphIGVpbiBmZXN0ZXIgQmxlZHNpbm4sIHdlaWwgVW1sYXV0ZSB3aWUg9iB1bmQg/CBtYWNoZW4gU2lubiwgd2llIGF1Y2ggZWluIHNjaGFyZmVzIN8u",
            "R0lGODlhAgSzzs7O3t7e",
            "SMOkdHRlbiBIw7x0ZSBlaW4gw58gaW0gTmFtZW4sIHfDpHJlbiBzaWUgbcO2Z2xpY2hlcndlaXNlIGtlaW5lIEjDvHRlIG1laHIsDQpzb25kZXJuIEjDvMOfZS4NCg==",
            "0FB",
            "0F B7",
            "0G",
        ] {
            assert_eq!(parse_hex(s), None, "{s}");
        }
        assert_eq!(parse_hex(""), Some(Vec::new()));
        assert_eq!(format_hex(&[]), "");
    }

    #[test]
    fn exificient_base64_binary() {
        let s1 = "R0lGODdhWAK+ov////v7++fn58DAwI6Ojl5eXjExMQMDAyxYAr5AA/8Iutz+MMpJq7046827/2Ao";
        let s2 = "jmRpnmiqPsKxvvBqCIxgxHg=";
        let cases = [
            ("0FB7", "0FB7"),
            ("  0  F B 7 ", "0FB7"),
            ("ZHM=", "ZHM="),
            (
                "RGFzIGlzIGphIGVpbiBmZXN0ZXIgQmxlZHNpbm4sIHdlaWwgVW1sYXV0ZSB3aWUg9iB1bmQg/CBtYWNoZW4gU2lubiwgd2llIGF1Y2ggZWluIHNjaGFyZmVzIN8u",
                "RGFzIGlzIGphIGVpbiBmZXN0ZXIgQmxlZHNpbm4sIHdlaWwgVW1sYXV0ZSB3aWUg9iB1bmQg/CBtYWNoZW4gU2lubiwgd2llIGF1Y2ggZWluIHNjaGFyZmVzIN8u",
            ),
            (
                "SMOkdHRlbiBIw7x0ZSBlaW4gw58gaW0gTmFtZW4sIHfDpHJlbiBzaWUgbcO2Z2xpY2hlcndlaXNlIGtlaW5lIEjDvHRlIG1laHIsDQpzb25kZXJuIEjDvMOfZS4NCg==",
                "SMOkdHRlbiBIw7x0ZSBlaW4gw58gaW0gTmFtZW4sIHfDpHJlbiBzaWUgbcO2Z2xpY2hlcndlaXNlIGtlaW5lIEjDvHRlIG1laHIsDQpzb25kZXJuIEjDvMOfZS4NCg==",
            ),
        ];
        let joined_in = format!("{s1}\n \n {s2}");
        let joined_out = format!("{s1}{s2}");
        for (input, expected) in cases
            .iter()
            .copied()
            .chain([(joined_in.as_str(), joined_out.as_str())])
        {
            let bytes = parse_base64(input);
            assert!(bytes.is_some(), "{input}");
            let bytes = bytes.unwrap_or_default();
            assert_eq!(format_base64(&bytes), expected, "{input}");
            assert_eq!(format_base64(&round_trip(false, &bytes)), expected);
            assert_eq!(format_base64(&round_trip(true, &bytes)), expected);
        }
        assert_eq!(parse_base64("ZHM="), Some(b"ds".to_vec()));
        // testBase64Failure1 and other malformed inputs.
        for s in [
            "0FB7 x", "0FB", "ZHM", "Z===", "ZH=M", "ZHM-", "0FB7=", "ZHN=", "ZQ=A",
        ] {
            assert_eq!(parse_base64(s), None, "{s}");
        }
        assert_eq!(parse_base64(""), Some(Vec::new()));
        assert_eq!(parse_base64(" \n"), Some(Vec::new()));
        assert_eq!(format_base64(&[]), "");
        assert_eq!(format_base64(b"f"), "Zg==");
        assert_eq!(format_base64(b"fo"), "Zm8=");
        assert_eq!(format_base64(b"foo"), "Zm9v");
        assert_eq!(format_base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(parse_base64("Zm9vYmFy"), Some(b"foobar".to_vec()));
        assert_eq!(parse_base64("Zg=="), Some(b"f".to_vec()));
    }

    #[test]
    fn exificient_binary_framing() {
        // BinaryCoreTest :: testBinary_1..3: binary between n-bit values, both alignments.
        for s in ["blabla", "X", ""] {
            for aligned in [false, true] {
                let stream = enc(aligned, |w| {
                    w.write_nbit_unsigned(2, 3)?;
                    write_binary(w, s.as_bytes())?;
                    w.write_nbit_unsigned(5, 7)
                });
                let mut r = BitReader::with_alignment(stream.as_slice(), aligned);
                assert_eq!(r.read_nbit_unsigned(3), Ok(2));
                assert_eq!(read_binary(&mut r), Ok(s.as_bytes().to_vec()));
                assert_eq!(r.read_nbit_unsigned(7), Ok(5));
            }
        }
        assert_eq!(round_trip(false, b"blabla"), b"blabla");
        assert_eq!(round_trip(true, b"blabla"), b"blabla");
    }

    #[test]
    fn large_and_truncated() {
        let big: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        assert_eq!(round_trip(false, &big), big);
        // A length larger than the stream reports truncation.
        let mut r = BitReader::new([0xFFu8, 0xFF, 0x7F, 0x00].as_slice());
        assert_eq!(read_binary(&mut r), Err(ExiError::Truncated));
    }
}
