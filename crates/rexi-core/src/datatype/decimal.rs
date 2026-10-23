//! Decimal ([EXI 7.1.3](https://www.w3.org/TR/exi/#encodingDecimal)): a sign bit,
//! the integral part (digits before the point) as an Unsigned Integer, then the
//! fractional digits (after the point) in reverse order as an Unsigned Integer.
//! Reversing keeps leading zeros of the fraction: `.05` is sent as 50, not 5.
//!
//! Text handling works the same as EXIficient's `DecimalValue`. Parsing accepts an
//! optional sign, an optional integral part and an optional fractional part (see
//! [`xsd:decimal`](https://www.w3.org/TR/xmlschema-2/#decimal)). `-0.0` is turned
//! into positive zero. Zeros at the end of the fraction disappear, because after
//! reversing they become leading zeros of an integer. The standard output form is
//! always `[-]integral.fraction`, with `.0` when there is no fraction (`"210"` prints
//! as `"210.0"`, `"110.74080"` as `"110.7408"`).

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;

use super::unsigned::{Uint, read_unsigned_any};
use crate::error::ExiResult;
use crate::io::{BitReader, BitWriter};
use crate::value::{Decimal, Int};

impl Decimal {
    /// Builds a decimal, turning `-0.0` into `0.0` as EXIficient does ("normalize
    /// \"-0.0\" to \"0.0\"",
    /// [`DecimalValue.java#L51-L55`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/values/DecimalValue.java#L51-L55)).
    pub fn new(negative: bool, integral: Uint, fraction_reversed: Uint) -> Self {
        let negative = negative && !(integral.is_zero() && fraction_reversed.is_zero());
        Self {
            negative,
            integral,
            fraction_reversed,
        }
    }

    /// The decimal with the value of an integer (`-7` -> `-7.0`).
    pub fn from_int(value: &Int) -> Self {
        match value {
            Int::Small(v) => Self::new(*v < 0, Uint::Small(v.unsigned_abs()), Uint::Small(0)),
            Int::Big {
                negative,
                magnitude,
            } => Self::new(*negative, Uint::Big(magnitude.clone()), Uint::Small(0)),
        }
    }

    /// The decimal with the value of an unsigned integer (`7` -> `7.0`).
    pub fn from_uint(value: &Uint) -> Self {
        Self::new(false, value.clone(), Uint::Small(0))
    }

    /// Parses an `xsd:decimal` text value (no spaces around it).
    ///
    /// Differences from EXIficient's `DecimalValue.parse`: `"5."` is accepted (as
    /// `5.0`, which XSD allows) where EXIficient rejects it. A sign character inside
    /// the digits is rejected, where Java's `parseInt` would accept it.
    pub fn from_lexical(s: &str) -> Option<Self> {
        let (negative, rest) = match s.as_bytes().first() {
            Some(b'-') => (true, &s[1..]),
            Some(b'+') => (false, &s[1..]),
            _ => (false, s),
        };
        let (int_part, frac_part) = match rest.find('.') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return None;
        }
        let integral = if int_part.is_empty() {
            Uint::Small(0)
        } else {
            digits_to_uint(int_part)?
        };
        let fraction_reversed = if frac_part.is_empty() {
            Uint::Small(0)
        } else {
            let reversed: String = frac_part.chars().rev().collect();
            digits_to_uint(&reversed)?
        };
        Some(Self::new(negative, integral, fraction_reversed))
    }

    /// Standard text form, as EXIficient's `DecimalValue.toString` prints it. The
    /// fraction is stored reversed, so it is reversed again here.
    pub fn to_lexical(&self) -> String {
        let mut s = String::new();
        if self.negative {
            s.push('-');
        }
        s.push_str(&self.integral.to_string());
        s.push('.');
        s.extend(self.fraction_reversed.to_string().chars().rev());
        s
    }
}

impl core::fmt::Display for Decimal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_lexical())
    }
}

/// Parses a run of ASCII digits (leading zeros allowed) of any length.
pub(crate) fn digits_to_uint(s: &str) -> Option<Uint> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Uint::from_decimal_str(s)
}

/// Writes a Decimal
/// ([EXI 7.1.3](https://www.w3.org/TR/exi/#encodingDecimal)): a sign bit, the
/// integral part, then the fraction digits in reverse order, both as Unsigned
/// Integers.
pub fn write_decimal<W: ByteSink>(w: &mut BitWriter<W>, value: &Decimal) -> ExiResult<()> {
    w.write_bool(value.negative)?;
    value.integral.write(w)?;
    value.fraction_reversed.write(w)
}

/// Reads a Decimal: sign bit, integral part, reversed fraction digits.
pub fn read_decimal<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<Decimal> {
    let negative = r.read_bool()?;
    let integral = read_unsigned_any(r)?;
    let fraction_reversed = read_unsigned_any(r)?;
    Ok(Decimal::new(negative, integral, fraction_reversed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(aligned: bool, d: &Decimal) -> Vec<u8> {
        let mut w = BitWriter::with_alignment(Vec::new(), aligned);
        write_decimal(&mut w, d).unwrap_or_default();
        w.finish().unwrap_or_default()
    }

    fn round_trip(aligned: bool, d: &Decimal) -> String {
        let bytes = enc(aligned, d);
        let mut r = BitReader::with_alignment(bytes.as_slice(), aligned);
        read_decimal(&mut r)
            .map(|d| d.to_lexical())
            .unwrap_or_default()
    }

    fn parts(s: &str) -> (bool, String, String) {
        let d = Decimal::from_lexical(s).unwrap_or_else(|| Decimal::new(false, 0.into(), 0.into()));
        (
            d.negative,
            d.integral.to_string(),
            d.fraction_reversed.to_string(),
        )
    }

    #[test]
    fn exip_vectors() {
        // check_streamIO.c :: test_decodeDecimalValue, 02 B2 12 -> 5.001
        let mut r = BitReader::new([0x02u8, 0xB2, 0x12].as_slice());
        let d = read_decimal(&mut r);
        assert_eq!(d, Ok(Decimal::new(false, 5.into(), 100.into())));
        assert_eq!(d.map(|d| d.to_lexical()), Ok("5.001".to_string()));
        assert_eq!(r.bit_position(), 17);
        // check_streamIO.c :: test_encodeDecimalValue(5.001): 17 bits, bitPointer 1.
        let bytes = enc(false, &Decimal::new(false, 5.into(), 100.into()));
        assert_eq!(bytes.len(), 3);
        assert_eq!(&bytes[..2], &[0x02, 0xB2]);
        assert_eq!(bytes[2] >> 7, 0);
    }

    #[test]
    fn parse_examples_from_task() {
        assert_eq!(parts("31.50"), (false, "31".into(), "5".into()));
        assert_eq!(parts("42.95"), (false, "42".into(), "59".into()));
        assert_eq!(parts("-0.5"), (true, "0".into(), "5".into()));
        assert_eq!(parts("5"), (false, "5".into(), "0".into()));
        assert_eq!(
            parts("+100000.0012"),
            (false, "100000".into(), "2100".into())
        );
        assert_eq!(parts("0.001359"), (false, "0".into(), "953100".into()));
        assert_eq!(
            Decimal::new(false, 31.into(), 5.into()).to_lexical(),
            "31.5"
        );
        assert_eq!(Decimal::new(true, 0.into(), 5.into()).to_lexical(), "-0.5");
        assert_eq!(Decimal::from_int(&Int::Small(-7)).to_lexical(), "-7.0");
        assert_eq!(Decimal::from_int(&Int::Small(0)).to_lexical(), "0.0");
        assert_eq!(Decimal::from_uint(&Uint::Small(42)).to_lexical(), "42.0");
    }

    #[test]
    fn exificient_decimal_core_test() {
        let cases = [
            ("-1.23", "-1.23"),
            ("12678967.543233", "12678967.543233"),
            ("+100000.0012", "100000.0012"),
            ("210", "210.0"),
            ("380", "380.0"),
            ("0.001359", "0.001359"),
            ("110.74080", "110.7408"),
            ("55000.0", "55000.0"),
            ("3.141592653589", "3.141592653589"),
            ("-.1", "-0.1"),
            ("-.234", "-0.234"),
            (
                "36.087139166666670000000000000000001",
                "36.087139166666670000000000000000001",
            ),
            (
                "123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890.1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678912345678901234567890123456789012345678901234567890123456789012345678901234567890123456789",
                "123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890.1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678912345678901234567890123456789012345678901234567890123456789012345678901234567890123456789",
            ),
            // testBigDecimal1..3 go through BigDecimal in Java; the lexical path agrees.
            ("-1.3", "-1.3"),
            (
                "1236.087139166666670000000000000000001",
                "1236.087139166666670000000000000000001",
            ),
            ("-65.12033898980989080", "-65.1203389898098908"),
        ];
        for (input, expected) in cases {
            let d = Decimal::from_lexical(input);
            assert!(d.is_some(), "{input}");
            let d = d.unwrap_or_else(|| Decimal::new(false, 0.into(), 0.into()));
            assert_eq!(d.to_lexical(), expected, "{input}");
            assert_eq!(round_trip(false, &d), expected, "{input} bit");
            assert_eq!(round_trip(true, &d), expected, "{input} byte");
        }
    }

    #[test]
    fn exificient_decimal_equals_and_failures() {
        // testDecimalEquals: -0.0, +0.0, 0.0, +.0 are all the same (positive) zero.
        let zero = Decimal::new(false, 0.into(), 0.into());
        for s in ["-0.0", "+0.0", "0.0", "+.0", "-0", "0"] {
            assert_eq!(Decimal::from_lexical(s), Some(zero.clone()), "{s}");
        }
        assert_eq!(zero.to_lexical(), "0.0");
        // testDecimalFail1
        assert_eq!(Decimal::from_lexical("9.213.456"), None);
        for s in ["", "-", "+", ".", "1a", "1.2a", "--1", "1.-2", " 1", "1 "] {
            assert_eq!(Decimal::from_lexical(s), None, "{s:?}");
        }
        // "5." is accepted here (XSD allows it) although EXIficient rejects it.
        assert_eq!(
            Decimal::from_lexical("5.").map(|d| d.to_lexical()),
            Some("5.0".into())
        );
    }

    #[test]
    fn big_parts_round_trip() {
        let d = Decimal::from_lexical("-18446744073709551616.00000000000000000000123")
            .unwrap_or_else(|| Decimal::new(false, 0.into(), 0.into()));
        assert!(matches!(d.integral, Uint::Big(_)));
        assert!(matches!(d.fraction_reversed, Uint::Big(_)));
        assert_eq!(
            round_trip(false, &d),
            "-18446744073709551616.00000000000000000000123"
        );
    }

    #[test]
    fn wire_layout() {
        // -1.23: sign 1, varint(1) = 01, varint(32) = 20.
        let bytes = enc(
            true,
            &Decimal::from_lexical("-1.23")
                .unwrap_or_else(|| Decimal::new(false, 0.into(), 0.into())),
        );
        assert_eq!(bytes, vec![0x01, 0x01, 0x20]);
        // Decoding a negative zero gives positive zero.
        let mut r = BitReader::with_alignment([0x01u8, 0x00, 0x00].as_slice(), true);
        assert_eq!(
            read_decimal(&mut r),
            Ok(Decimal::new(false, 0.into(), 0.into()))
        );
    }
}
