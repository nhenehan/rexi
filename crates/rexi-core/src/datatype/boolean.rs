//! Boolean ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)): one bit, or a
//! 2-bit number that picks the text form when the schema type has pattern facets
//! (0 = "false", 1 = "0", 2 = "true", 3 = "1"). The 2-bit form exists because a
//! pattern can allow "true" but not "1", so the exact text must survive.

use crate::io::{ByteSink, ByteSource};

use crate::error::ExiResult;
use crate::io::{BitReader, BitWriter};

/// Text forms picked by the 2-bit pattern representation, indexed by its value.
pub const PATTERN_LEXICAL: [&str; 4] = ["false", "0", "true", "1"];

/// Writes a Boolean as one bit: 1 for true, 0 for false
/// ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)).
pub fn write_boolean<W: ByteSink>(w: &mut BitWriter<W>, value: bool) -> ExiResult<()> {
    w.write_bool(value)
}

/// Reads a one-bit Boolean.
pub fn read_boolean<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<bool> {
    r.read_bool()
}

/// Writes the 2-bit pattern form. `id` is an index into [`PATTERN_LEXICAL`].
pub fn write_boolean_pattern<W: ByteSink>(w: &mut BitWriter<W>, id: u8) -> ExiResult<()> {
    w.write_nbit_unsigned(u64::from(id & 0b11), 2)
}

/// Reads the 2-bit pattern form and returns its id (an index into
/// [`PATTERN_LEXICAL`]).
pub fn read_boolean_pattern<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<u8> {
    Ok(r.read_nbit_unsigned(2)? as u8)
}

/// Parses an [`xsd:boolean`](https://www.w3.org/TR/xmlschema-2/#boolean) text value
/// ("true", "false", "1", "0"). Works the same as EXIficient's `BooleanValue.parse`:
/// upper and lower case matter, and no spaces are allowed around the value.
pub fn parse_boolean(s: &str) -> Option<bool> {
    match s {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Parses a boolean text value into its pattern id (see [`PATTERN_LEXICAL`]).
pub fn parse_boolean_pattern(s: &str) -> Option<u8> {
    PATTERN_LEXICAL
        .iter()
        .position(|p| *p == s)
        .map(|i| i as u8)
}

/// Pattern id EXIficient's typed encoder uses for a typed boolean: "false" or "true".
pub fn pattern_id_for(value: bool) -> u8 {
    if value { 2 } else { 0 }
}

/// The boolean value of a pattern id: ids 2 and 3 ("true", "1") are true.
pub fn pattern_to_bool(id: u8) -> bool {
    id >= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    fn enc(aligned: bool, f: impl FnOnce(&mut BitWriter<Vec<u8>>) -> ExiResult<()>) -> Vec<u8> {
        let mut w = BitWriter::with_alignment(Vec::new(), aligned);
        f(&mut w).unwrap_or_default();
        w.finish().unwrap_or_default()
    }

    #[test]
    fn exip_vectors() {
        // check_streamIO.c :: test_decodeBoolean over D4 60 -> first bit is 1.
        let mut r = BitReader::new([0xD4u8, 0x60].as_slice());
        assert_eq!(read_boolean(&mut r), Ok(true));
        assert_eq!(r.bit_position(), 1);
        // check_streamIO.c :: test_encodeBoolean(1) -> top bit set.
        let bytes = enc(false, |w| write_boolean(w, true));
        assert_eq!(bytes[0] >> 7, 1);
    }

    #[test]
    fn exificient_boolean_core_test() {
        // BooleanCoreTest: "0" / "1" / "false" / "true" through the 1-bit form.
        for (s, expected) in [("0", false), ("1", true), ("false", false), ("true", true)] {
            let v = parse_boolean(s);
            assert_eq!(v, Some(expected), "{s}");
            for aligned in [false, true] {
                let bytes = enc(aligned, |w| write_boolean(w, expected));
                let mut r = BitReader::with_alignment(bytes.as_slice(), aligned);
                assert_eq!(read_boolean(&mut r), Ok(expected));
            }
        }
        // Failures: "00", "fAlse".
        assert_eq!(parse_boolean("00"), None);
        assert_eq!(parse_boolean("fAlse"), None);
        assert_eq!(parse_boolean(" true"), None);
    }

    #[test]
    fn exificient_boolean_pattern_facet() {
        // BooleanCoreTest :: FalsePatternFalse / TruePatternFalse keep the lexical form.
        for (s, id) in [("false", 0u8), ("0", 1), ("true", 2), ("1", 3)] {
            assert_eq!(parse_boolean_pattern(s), Some(id));
            for aligned in [false, true] {
                let bytes = enc(aligned, |w| write_boolean_pattern(w, id));
                let mut r = BitReader::with_alignment(bytes.as_slice(), aligned);
                let decoded = read_boolean_pattern(&mut r);
                assert_eq!(decoded, Ok(id));
                assert_eq!(PATTERN_LEXICAL[usize::from(id)], s);
            }
        }
        assert_eq!(parse_boolean_pattern("TRUE"), None);
        assert_eq!(pattern_id_for(true), 2);
        assert_eq!(pattern_id_for(false), 0);
        assert!(pattern_to_bool(3));
        assert!(!pattern_to_bool(1));
    }

    #[test]
    fn pattern_form_is_two_bits_or_one_octet() {
        let bytes = enc(false, |w| {
            write_boolean_pattern(w, 3)?;
            write_boolean_pattern(w, 1)
        });
        assert_eq!(bytes, vec![0b1101_0000]);
        let bytes = enc(true, |w| write_boolean_pattern(w, 2));
        assert_eq!(bytes, vec![0x02]);
        let bytes = enc(true, |w| write_boolean(w, true));
        assert_eq!(bytes, vec![0x01]);
    }
}
