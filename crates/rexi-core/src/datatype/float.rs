//! Float ([EXI 7.1.4](https://www.w3.org/TR/exi/#encodingFloat)): the mantissa (the
//! digits) then the base-10 exponent, each written as an Integer
//! ([EXI 7.1.5](https://www.w3.org/TR/exi/#encodingInteger)). The value is
//! `mantissa × 10^exponent`. The mantissa must fit in an `i64` and the exponent must
//! be between -(2^14 - 1) and 2^14 - 1. The exponent -(2^14) is kept as a marker
//! value for the special values: with mantissa 1 it means `INF`, with -1 `-INF`, and
//! with anything else `NaN`.
//!
//! Text handling works the same as EXIficient's `FloatValue`
//! ([`FloatValue.java#L55-L94`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/values/FloatValue.java#L55-L94), citing
//! <http://www.w3.org/TR/exi-c14n/#dt-float>: "If the mantissa is not 0, mantissas MUST
//! have no trailing zeros"):
//!
//! * parsing accepts `INF`, `-INF`, `NaN`, or `[+-]digits[.digits][(e|E)[+-]digits]`
//!   (see [`xsd:float`](https://www.w3.org/TR/xmlschema-2/#float) and
//!   [`xsd:double`](https://www.w3.org/TR/xmlschema-2/#double)). All digits, before
//!   and after the point, go into one mantissa, so the exponent is lowered by the
//!   number of digits after the point (`73.25` is `7325 × 10^-2`). Zeros at the end
//!   of the digits go into the exponent as they are read, so a whole number past
//!   2^63 written out in full (`10000000000000000000`) still fits; EXIficient
//!   overflows there;
//! * building the value then puts it into its one standard form, as Canonical EXI
//!   asks: a zero mantissa forces the exponent to 0. A non-zero mantissa has the zeros
//!   at its end moved into the exponent (`12300E0` -> `123E2`). The special exponent
//!   with a mantissa other than ±1 becomes NaN (mantissa 0);
//! * the standard output is `<mantissa>E<exponent>` (`"73.25"` -> `"7325E-2"`,
//!   `"0"` -> `"0E0"`, `"1.0E3"` -> `"1E3"`), or `INF` / `-INF` / `NaN`.

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;

use super::integer::{read_integer, write_integer};
use crate::error::{ExiError, ExiResult};
use crate::io::{BitReader, BitWriter};
use crate::value::{Float, Int};

const NAME: &str = "Float";

/// Largest exponent size (ignoring sign) the spec allows: 2^14 - 1.
pub const EXPONENT_LIMIT: i64 = 16383;

impl Float {
    /// Builds a float in its standard form, or `None` when the result is outside the
    /// spec's ranges. Moving zeros into the exponent can push a valid exponent past
    /// the limit (`10E16383` becomes `1E16384`), which is why the range check comes
    /// last.
    pub fn normalized(mantissa: i64, exponent: i64) -> Option<Self> {
        let mut mantissa = mantissa;
        let mut exponent = exponent;
        if mantissa == 0 {
            if exponent != i64::from(Self::SPECIAL_EXPONENT) {
                exponent = 0;
            }
        } else {
            while mantissa % 10 == 0 {
                mantissa /= 10;
                exponent += 1;
            }
        }
        if exponent == i64::from(Self::SPECIAL_EXPONENT) && mantissa != 1 && mantissa != -1 {
            mantissa = 0;
        }
        let in_range = exponent == i64::from(Self::SPECIAL_EXPONENT)
            || (-EXPONENT_LIMIT..=EXPONENT_LIMIT).contains(&exponent);
        if !in_range {
            return None;
        }
        Some(Self {
            mantissa,
            exponent: exponent as i16,
        })
    }

    /// Whether this is one of the special values INF, -INF or NaN, which use the
    /// special exponent.
    pub fn is_special(&self) -> bool {
        self.exponent == Self::SPECIAL_EXPONENT
    }

    /// Parses an `xsd:float`/`xsd:double` text value (no spaces around it).
    ///
    /// Stricter than EXIficient in three places where its parser accepts input that
    /// XSD does not allow: the mantissa needs at least one digit (EXIficient reads
    /// `"-"` as 0), an exponent marker needs at least one digit after it, and an
    /// exponent sign is only accepted as the first character of the exponent.
    pub fn from_lexical(s: &str) -> Option<Self> {
        match s {
            "INF" => return Some(Self::INFINITY),
            "-INF" => return Some(Self::NEG_INFINITY),
            "NaN" => return Some(Self::NAN),
            _ => {}
        }
        let bytes = s.as_bytes();
        let index_e = s.find('E').or_else(|| s.find('e'));
        let mantissa_end = index_e.unwrap_or(bytes.len());
        let negative = bytes.first() == Some(&b'-');
        let mut i = usize::from(matches!(bytes.first(), Some(b'-' | b'+')));
        if i >= mantissa_end {
            return None;
        }
        let mut magnitude = 0u64;
        let mut dec_point = false;
        let mut decimal_digits = 0i64;
        let mut digits = 0usize;
        // Zeros are only multiplied in when a later digit is not zero. Zeros at the
        // end go into the exponent instead (they would be moved there anyway, see
        // `normalized`), so a whole number like 1e19 written out in full,
        // `10000000000000000000`, is mantissa 1 and does not overflow the i64.
        let mut pending_zeros = 0u32;
        while i < mantissa_end {
            match bytes[i] {
                b'0' => {
                    pending_zeros = pending_zeros.checked_add(1)?;
                    if dec_point {
                        decimal_digits += 1;
                    }
                    digits += 1;
                }
                b'1'..=b'9' => {
                    let digit = u64::from(bytes[i] - b'0');
                    // Zeros before the first other digit change nothing.
                    magnitude = if magnitude == 0 {
                        digit
                    } else {
                        magnitude
                            .checked_mul(10u64.checked_pow(pending_zeros + 1)?)?
                            .checked_add(digit)?
                    };
                    pending_zeros = 0;
                    if dec_point {
                        decimal_digits += 1;
                    }
                    digits += 1;
                }
                b'.' if !dec_point => dec_point = true,
                _ => return None,
            }
            i += 1;
        }
        if digits == 0 {
            return None;
        }
        // The mantissa must fit in i64. A negative value may reach 2^63 (that is,
        // i64::MIN); `wrapping_neg` turns 2^63 into i64::MIN without overflow.
        let mantissa = if negative {
            if magnitude > 1u64 << 63 {
                return None;
            }
            (magnitude as i64).wrapping_neg()
        } else {
            i64::try_from(magnitude).ok()?
        };
        let mut exponent = 0i64;
        if let Some(e) = index_e {
            let exp = &bytes[e + 1..];
            let negative_exp = exp.first() == Some(&b'-');
            let start = usize::from(matches!(exp.first(), Some(b'-' | b'+')));
            if start >= exp.len() {
                return None;
            }
            for &b in &exp[start..] {
                if !b.is_ascii_digit() {
                    return None;
                }
                exponent = exponent.checked_mul(10)?.checked_add(i64::from(b - b'0'))?;
            }
            if negative_exp {
                exponent = -exponent;
            }
        }
        // Move the point: each digit after the point lowers the exponent by one. The
        // range is checked here, before zeros are stripped from the mantissa, and
        // again in `normalized`.
        exponent -= decimal_digits;
        if magnitude != 0 {
            exponent += i64::from(pending_zeros);
        }
        if !(-EXPONENT_LIMIT..=EXPONENT_LIMIT).contains(&exponent) {
            return None;
        }
        Self::normalized(mantissa, exponent)
    }

    /// Standard text form, as EXIficient's `FloatValue.toString` prints it.
    pub fn to_lexical(&self) -> String {
        if self.is_special() {
            match self.mantissa {
                1 => "INF".to_string(),
                -1 => "-INF".to_string(),
                _ => "NaN".to_string(),
            }
        } else {
            format!("{}E{}", self.mantissa, self.exponent)
        }
    }
}

impl core::fmt::Display for Float {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_lexical())
    }
}

/// Writes a Float
/// ([EXI 7.1.4](https://www.w3.org/TR/exi/#encodingFloat)): the mantissa, then the
/// exponent, both as Integers.
pub fn write_float<W: ByteSink>(w: &mut BitWriter<W>, value: Float) -> ExiResult<()> {
    write_integer(w, &Int::Small(value.mantissa))?;
    write_integer(w, &Int::Small(i64::from(value.exponent)))
}

/// Reads a float and puts it in standard form exactly as EXIficient's `FloatValue`
/// constructor does (so `1200E-2` in the stream reads as `12E0`).
pub fn read_float<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<Float> {
    let mantissa = read_integer(r)?
        .to_i64()
        .ok_or_else(|| ExiError::value(NAME, "mantissa exceeds 64 bits"))?;
    let exponent = read_integer(r)?
        .to_i64()
        .ok_or_else(|| ExiError::value(NAME, "exponent exceeds 64 bits"))?;
    Float::normalized(mantissa, exponent)
        .ok_or_else(|| ExiError::value(NAME, format!("exponent {exponent} out of range")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(aligned: bool, f: Float) -> Vec<u8> {
        let mut w = BitWriter::with_alignment(Vec::new(), aligned);
        write_float(&mut w, f).unwrap_or_default();
        w.finish().unwrap_or_default()
    }

    fn round_trip(aligned: bool, f: Float) -> Float {
        let bytes = enc(aligned, f);
        let mut r = BitReader::with_alignment(bytes.as_slice(), aligned);
        read_float(&mut r).unwrap_or(Float {
            mantissa: i64::MIN,
            exponent: 0,
        })
    }

    fn parse(s: &str) -> Float {
        Float::from_lexical(s).unwrap_or(Float {
            mantissa: i64::MIN,
            exponent: 0,
        })
    }

    #[test]
    fn exip_vectors() {
        // check_streamIO.c :: test_decodeFloat, 02 80 92 -> 5 x 10^2, 18 bits consumed.
        let mut r = BitReader::new([0x02u8, 0x80, 0x92].as_slice());
        assert_eq!(
            read_float(&mut r),
            Ok(Float {
                mantissa: 5,
                exponent: 2
            })
        );
        assert_eq!(r.bit_position(), 18);
        // check_streamIO.c :: test_encodeFloatValue(5, 2): bufferIndx 2, bitPointer 2.
        let bytes = enc(
            false,
            Float {
                mantissa: 5,
                exponent: 2,
            },
        );
        assert_eq!(bytes.len(), 3);
        assert_eq!(&bytes[..2], &[0x02, 0x80]);
        assert_eq!(bytes[2] & 0xC0, 0x80);
    }

    #[test]
    fn exificient_float_core_test_strings() {
        // (input, mantissa, exponent, canonical) derived from FloatValue.parse + constructor.
        let cases: [(&str, i64, i16, &str); 22] = [
            ("NaN", 0, -16384, "NaN"),
            ("INF", 1, -16384, "INF"),
            ("-INF", -1, -16384, "-INF"),
            ("-1E4", -1, 4, "-1E4"),
            ("1267.43233E12", 126_743_233, 7, "126743233E7"),
            ("12.78e-2", 1278, -4, "1278E-4"),
            ("12", 12, 0, "12E0"),
            ("0", 0, 0, "0E0"),
            ("-0", 0, 0, "0E0"),
            ("-1", -1, 0, "-1E0"),
            (
                "119.999999999929",
                119_999_999_999_929,
                -12,
                "119999999999929E-12",
            ),
            ("000123400.0031200", 12_340_000_312, -5, "12340000312E-5"),
            (
                "562949953421312",
                562_949_953_421_312,
                0,
                "562949953421312E0",
            ),
            (
                "-9223372036854775808E123",
                i64::MIN,
                123,
                "-9223372036854775808E123",
            ),
            (
                "9223372036854775807E123",
                i64::MAX,
                123,
                "9223372036854775807E123",
            ),
            ("456E-16383", 456, -16383, "456E-16383"),
            ("456E16383", 456, 16383, "456E16383"),
            // Task examples.
            ("85.00", 85, 0, "85E0"),
            ("44.10", 441, -1, "441E-1"),
            ("73.25", 7325, -2, "7325E-2"),
            ("19.5", 195, -1, "195E-1"),
            ("1.0E3", 1, 3, "1E3"),
        ];
        for (input, mantissa, exponent, canonical) in cases {
            let f = Float::from_lexical(input);
            assert_eq!(f, Some(Float { mantissa, exponent }), "{input}");
            let f = f.unwrap_or(Float::NAN);
            assert_eq!(f.to_lexical(), canonical, "{input}");
            assert_eq!(round_trip(false, f), f, "{input} bit");
            assert_eq!(round_trip(true, f), f, "{input} byte");
        }
    }

    #[test]
    fn exificient_float_equals_tests() {
        // FloatCoreTest :: testFloatEquals1..9 and NotEquals1: equality after putting
        // values in standard form.
        assert_eq!(parse("234E2"), parse("2340E1"));
        assert_eq!(parse("30E0"), parse("3E1"));
        assert_eq!(
            parse("123.012300"),
            Float {
                mantissa: 1_230_123,
                exponent: -4
            }
        );
        assert_eq!(
            parse("0.0"),
            Float {
                mantissa: 0,
                exponent: 0
            }
        );
        assert_eq!(
            parse("-0.0"),
            Float {
                mantissa: 0,
                exponent: 0
            }
        );
        assert_eq!(
            parse("1.0"),
            Float {
                mantissa: 1,
                exponent: 0
            }
        );
        assert_eq!(
            parse("-1230.01"),
            Float {
                mantissa: -123_001,
                exponent: -2
            }
        );
        assert_eq!(
            parse("0.1230"),
            Float {
                mantissa: 123,
                exponent: -3
            }
        );
        for s in ["12.0", "120E-1", "1.2E1"] {
            assert_eq!(
                parse(s),
                Float {
                    mantissa: 12,
                    exponent: 0
                },
                "{s}"
            );
        }
        assert_eq!(
            parse("12300"),
            Float {
                mantissa: 123,
                exponent: 2
            }
        );
        assert_ne!(parse("234E3"), parse("2340E1"));
        // testFloatEquals10: the constructor puts values in standard form.
        assert_eq!(
            Float::normalized(0, 12),
            Some(Float {
                mantissa: 0,
                exponent: 0
            })
        );
        assert_eq!(
            Float::normalized(2, 120),
            Some(Float {
                mantissa: 2,
                exponent: 120
            })
        );
        assert_eq!(
            Float::normalized(12300, 0),
            Some(Float {
                mantissa: 123,
                exponent: 2
            })
        );
        assert_eq!(Float::normalized(7, -16384), Some(Float::NAN));
        assert_eq!(Float::normalized(-1, -16384), Some(Float::NEG_INFINITY));
        assert_eq!(Float::normalized(1, 16384), None);
        assert_eq!(Float::normalized(1, -16385), None);
    }

    #[test]
    fn exificient_float_invalid_tests() {
        for s in [
            "x11.1",
            "",
            "1.1223x",
            "E",
            "9223372036854775808E3",
            "9223372036854775809E3",
            "123E16384",
            "-9223372036854775809E3",
            "123E-16384",
            "123E-+16384",
            // Stricter than EXIficient:
            "-",
            "+",
            ".",
            "1E",
            "1E1+2",
            "1.2.3",
            " 1",
            "1 ",
            "inf",
            "nan",
            "99999999999999999999",
        ] {
            assert_eq!(Float::from_lexical(s), None, "{s:?}");
        }
        // Trailing-zero stripping may push a valid exponent past the limit.
        assert_eq!(Float::from_lexical("10E16383"), None);
    }

    #[test]
    fn whole_numbers_past_2_to_63_parse() {
        // Trailing zeros go into the exponent as they are read, so a big whole
        // number written out in full no longer overflows the mantissa.
        let f = |m, e| Float {
            mantissa: m,
            exponent: e,
        };
        assert_eq!(parse("10000000000000000000"), f(1, 19));
        assert_eq!(parse("-10000000000000000000.0"), f(-1, 19));
        assert_eq!(parse("10000000000000000000"), parse("1E19"));
        let max = format!("{}", f64::MAX);
        assert_eq!(parse(&max), f(17_976_931_348_623_157, 292));
        assert_eq!(parse(&format!("0.{}1", "0".repeat(40))), f(1, -41));
        assert_eq!(parse("1001000"), f(1001, 3));
        // EXIficient's test list calls this invalid only because its 64-bit
        // mantissa overflows; with the trailing zero moved, it fits.
        assert_eq!(
            parse("-9223372036854775810E3"),
            f(-922_337_203_685_477_581, 4)
        );
        // Too many digits that matter is still refused.
        assert_eq!(Float::from_lexical("3.141592653589793238462643"), None);
    }

    #[test]
    fn decoder_normalises_like_exificient() {
        // 1200E-2 on the wire decodes as 12E0.
        let mut w = BitWriter::new(Vec::new());
        write_integer(&mut w, &Int::Small(1200)).unwrap_or_default();
        write_integer(&mut w, &Int::Small(-2)).unwrap_or_default();
        let bytes = w.finish().unwrap_or_default();
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(
            read_float(&mut r),
            Ok(Float {
                mantissa: 12,
                exponent: 0
            })
        );
    }

    #[test]
    fn wire_layout_byte_aligned() {
        // 7325E-2: sign 0, varint(7325) = 9D 39, sign 1, varint(1) = 01.
        assert_eq!(
            enc(
                true,
                Float {
                    mantissa: 7325,
                    exponent: -2
                }
            ),
            vec![0x00, 0x9D, 0x39, 0x01, 0x01]
        );
        // INF: sign 0, 01, sign 1, varint(16383) = FF 7F.
        assert_eq!(
            enc(true, Float::INFINITY),
            vec![0x00, 0x01, 0x01, 0xFF, 0x7F]
        );
        assert_eq!(round_trip(true, Float::NAN), Float::NAN);
        assert_eq!(round_trip(false, Float::NEG_INFINITY), Float::NEG_INFINITY);
    }
}
