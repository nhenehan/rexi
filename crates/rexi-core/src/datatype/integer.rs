//! Integer ([EXI 7.1.5](https://www.w3.org/TR/exi/#encodingInteger)): a sign bit,
//! then the magnitude (the value without its sign) as an Unsigned Integer. Negative
//! values store `|v| - 1`, so -1 is sent as 0 and there is no "minus zero". This
//! module also has the bounded n-bit form
//! ([EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)).
//!
//! The sign bit is a Boolean
//! ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)), so in byte-aligned mode
//! it takes a whole octet. This is exactly what EXIficient's `encodeBoolean` +
//! `encodeUnsignedInteger` do.

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;

use super::biguint::BigUint;
use super::unsigned::{Uint, read_unsigned_any, write_unsigned, write_unsigned_big};
use crate::error::{ExiError, ExiResult};
use crate::io::{BitReader, BitWriter};
use crate::value::Int;

const NAME: &str = "Integer";

/// Writes an Integer
/// ([EXI 7.1.5](https://www.w3.org/TR/exi/#encodingInteger)): a sign bit (1 for
/// negative), then the size as an Unsigned Integer. A negative value `v` is sent as
/// `-v - 1`, so no number has two forms.
pub fn write_integer<W: ByteSink>(w: &mut BitWriter<W>, value: &Int) -> ExiResult<()> {
    match value {
        Int::Small(v) => {
            if *v < 0 {
                w.write_bool(true)?;
                // |v| - 1 == -(v + 1), which cannot overflow for any negative v.
                write_unsigned(w, (-(v + 1)) as u64)
            } else {
                w.write_bool(false)?;
                write_unsigned(w, *v as u64)
            }
        }
        Int::Big {
            negative,
            magnitude,
        } => {
            if *negative && magnitude.is_zero() {
                return Err(ExiError::value(NAME, "negative zero"));
            }
            w.write_bool(*negative)?;
            if *negative {
                write_unsigned_big(w, &magnitude.sub_one())
            } else {
                write_unsigned_big(w, magnitude)
            }
        }
    }
}

/// Reads an Integer: sign bit, then the size (`-(n + 1)` when negative).
pub fn read_integer<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<Int> {
    let negative = r.read_bool()?;
    Ok(from_wire(negative, read_unsigned_any(r)?))
}

/// Builds an `Int` from the wire form (the form in the stream): sign and, for
/// negatives, `|v| - 1`. So a negative value is `-m - 1`.
fn from_wire(negative: bool, magnitude: Uint) -> Int {
    match (negative, magnitude) {
        (false, Uint::Small(m)) => match i64::try_from(m) {
            Ok(v) => Int::Small(v),
            Err(_) => Int::Big {
                negative: false,
                magnitude: BigUint::from_u64(m),
            },
        },
        (true, Uint::Small(m)) => match i64::try_from(m) {
            Ok(v) => Int::Small(-v - 1),
            Err(_) => Int::Big {
                negative: true,
                magnitude: BigUint::from_u64(m).add_one(),
            },
        },
        (false, Uint::Big(b)) => Int::Big {
            negative: false,
            magnitude: b,
        },
        (true, Uint::Big(b)) => Int::Big {
            negative: true,
            magnitude: b.add_one(),
        },
    }
}

/// Writes `value - min` as an n-bit unsigned integer, for integer types with a bounded
/// range of 4096 values or fewer
/// ([EXI 7.1.5](https://www.w3.org/TR/exi/#encodingInteger),
/// [EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)). The math is done
/// in `i128` so `value - min` cannot overflow. Only the `bits`-wide window is checked
/// here, not the type's upper bound.
pub fn write_nbit<W: ByteSink>(
    w: &mut BitWriter<W>,
    value: &Int,
    min: i64,
    bits: u8,
) -> ExiResult<()> {
    let offset = value
        .to_i64()
        .map(|v| i128::from(v) - i128::from(min))
        .filter(|offset| *offset >= 0 && *offset <= nbit_max(bits))
        .ok_or_else(|| {
            ExiError::value(
                "n-bit Unsigned Integer",
                format!(
                    "{value} is outside the bounded range {min}..={}",
                    i128::from(min) + nbit_max(bits)
                ),
            )
        })?;
    w.write_nbit_unsigned(offset as u64, bits)
}

/// Reads a bounded integer: a `bits`-wide offset that is added to `min`
/// ([EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)).
pub fn read_nbit<R: ByteSource>(r: &mut BitReader<R>, min: i64, bits: u8) -> ExiResult<Int> {
    let offset = r.read_nbit_unsigned(bits)?;
    i64::try_from(i128::from(min) + i128::from(offset))
        .map(Int::Small)
        .map_err(|_| ExiError::value("n-bit Unsigned Integer", "decoded value exceeds i64"))
}

/// Largest offset that fits in `bits` bits. `bits` is capped at 64.
fn nbit_max(bits: u8) -> i128 {
    (1i128 << bits.min(64)) - 1
}

/// Converts a non-negative `Int` to a `Uint`; `None` for negative values.
pub fn int_to_uint(value: &Int) -> Option<Uint> {
    match value {
        Int::Small(v) => u64::try_from(*v).ok().map(Uint::Small),
        Int::Big {
            negative: false,
            magnitude,
        } => Some(Uint::Big(magnitude.clone())),
        Int::Big { negative: true, .. } => None,
    }
}

/// Converts a `Uint` to an `Int`. This always works: values too big for `i64`
/// become `Int::Big`.
pub fn uint_to_int(value: &Uint) -> Int {
    match value {
        Uint::Small(v) => match i64::try_from(*v) {
            Ok(v) => Int::Small(v),
            Err(_) => Int::Big {
                negative: false,
                magnitude: BigUint::from_u64(*v),
            },
        },
        Uint::Big(b) => Int::Big {
            negative: false,
            magnitude: b.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(aligned: bool, f: impl FnOnce(&mut BitWriter<Vec<u8>>) -> ExiResult<()>) -> Vec<u8> {
        let mut w = BitWriter::with_alignment(Vec::new(), aligned);
        f(&mut w).unwrap_or_default();
        w.finish().unwrap_or_default()
    }

    fn round_trip(aligned: bool, v: &Int) -> Int {
        let bytes = enc(aligned, |w| write_integer(w, v));
        let mut r = BitReader::with_alignment(bytes.as_slice(), aligned);
        read_integer(&mut r).unwrap_or(Int::Small(i64::MIN))
    }

    #[test]
    fn exip_decode_vectors() {
        // check_streamIO.c :: test_decodeIntegerValue, 94 60 48 -> -41
        let mut r = BitReader::new([0x94u8, 0x60, 0x48].as_slice());
        assert_eq!(read_integer(&mut r), Ok(Int::Small(-41)));
        assert_eq!(r.bit_position(), 9);
        // ... then 14 (followed by the untouched 60 48) -> 40
        let mut r = BitReader::new([0x14u8, 0x60, 0x48].as_slice());
        assert_eq!(read_integer(&mut r), Ok(Int::Small(40)));
        assert_eq!(r.bit_position(), 9);
    }

    #[test]
    fn exip_encode_minus_913() {
        // check_streamIO.c :: test_encodeIntegerValue: sign 1, then varint(912) = 90 07.
        let bytes = enc(false, |w| write_integer(w, &Int::Small(-913)));
        assert_eq!(bytes, vec![0xC8, 0x03, 0x80]);
        assert_eq!(round_trip(false, &Int::Small(-913)), Int::Small(-913));
    }

    #[test]
    fn exificient_integer_core_values() {
        for v in [
            0i64,
            1,
            i64::from(i32::MIN),
            -128,
            12_131_321_321,
            12_678_967_543_233,
        ] {
            assert_eq!(round_trip(false, &Int::Small(v)), Int::Small(v), "{v}");
            assert_eq!(
                round_trip(true, &Int::Small(v)),
                Int::Small(v),
                "{v} aligned"
            );
        }
        for s in [
            "0",
            "1",
            "-128",
            "12131321321",
            "2137000000000000000000000000001",
            "12678967543233",
            "-5153135115135135135135153153135135153",
        ] {
            let v = Int::from_decimal_str(s).unwrap_or(Int::Small(0));
            assert_eq!(round_trip(false, &v).to_string(), s);
            assert_eq!(round_trip(true, &v).to_string(), s);
        }
    }

    #[test]
    fn exificient_integer_sequence() {
        let mut w = BitWriter::new(Vec::new());
        let mut wb = BitWriter::with_alignment(Vec::new(), true);
        for i in 0..20_000i64 {
            write_integer(&mut w, &Int::Small(i)).unwrap_or_default();
            write_integer(&mut wb, &Int::Small(i)).unwrap_or_default();
        }
        let bytes = w.finish().unwrap_or_default();
        let bytes_b = wb.finish().unwrap_or_default();
        let mut r = BitReader::new(bytes.as_slice());
        let mut rb = BitReader::with_alignment(bytes_b.as_slice(), true);
        for i in 0..20_000i64 {
            assert_eq!(read_integer(&mut r), Ok(Int::Small(i)));
            assert_eq!(read_integer(&mut rb), Ok(Int::Small(i)));
        }
    }

    #[test]
    fn i64_boundaries_and_big_values() {
        for v in [i64::MIN, i64::MAX, -1, i64::MIN + 1] {
            assert_eq!(round_trip(false, &Int::Small(v)), Int::Small(v));
        }
        // 2^63 does not fit i64; its wire form is sign 0 then varint(2^63).
        let big = Int::Big {
            negative: false,
            magnitude: BigUint::from_u128(1u128 << 63),
        };
        assert_eq!(round_trip(false, &big), big);
        // -(2^63) - 1: sign 1, then varint(2^63).
        let neg = Int::Big {
            negative: true,
            magnitude: BigUint::from_u128((1u128 << 63) + 1),
        };
        assert_eq!(round_trip(false, &neg), neg);
        assert_eq!(round_trip(true, &neg), neg);
        // Big values that happen to fit i64 decode as Small.
        let small_as_big = Int::Big {
            negative: true,
            magnitude: BigUint::from_u64(5),
        };
        assert_eq!(round_trip(false, &small_as_big), Int::Small(-5));
        let bytes_a = enc(false, |w| write_integer(w, &small_as_big));
        let bytes_b = enc(false, |w| write_integer(w, &Int::Small(-5)));
        assert_eq!(bytes_a, bytes_b);
    }

    #[test]
    fn byte_aligned_sign_is_one_octet() {
        let bytes = enc(true, |w| write_integer(w, &Int::Small(-913)));
        assert_eq!(bytes, vec![0x01, 0x90, 0x07]);
        let bytes = enc(true, |w| write_integer(w, &Int::Small(40)));
        assert_eq!(bytes, vec![0x00, 0x28]);
    }

    #[test]
    fn exificient_nbit_values() {
        for (value, bits) in [
            (0u64, 1u8),
            (1, 1),
            (8, 4),
            (33, 9),
            (78935, 20),
            (8448, 20),
        ] {
            for aligned in [false, true] {
                let v = Int::Small(value as i64);
                let bytes = enc(aligned, |w| write_nbit(w, &v, 0, bits));
                let mut r = BitReader::with_alignment(bytes.as_slice(), aligned);
                assert_eq!(read_nbit(&mut r, 0, bits), Ok(v));
            }
        }
    }

    #[test]
    fn exificient_nbit_facets() {
        // NBitUnsignedIntegerCoreTest :: Facet1: range 2..=10 (9 values, 4 bits). The kind
        // carries no upper bound, so only values outside the 4-bit offset window are
        // rejected here (18 and 1); "12" is rejected by the schema layer, not this one.
        let mut w = BitWriter::new(Vec::new());
        assert!(write_nbit(&mut w, &Int::Small(18), 2, 4).is_err());
        assert!(write_nbit(&mut w, &Int::Small(1), 2, 4).is_err());
        assert!(write_nbit(&mut w, &Int::Small(10), 2, 4).is_ok());
        // Facet2/3: range -200..=-10 (191 values -> 8 bits), "-12" round-trips.
        for aligned in [false, true] {
            let bytes = enc(aligned, |w| write_nbit(w, &Int::Small(-12), -200, 8));
            let mut r = BitReader::with_alignment(bytes.as_slice(), aligned);
            assert_eq!(read_nbit(&mut r, -200, 8), Ok(Int::Small(-12)));
        }
        assert_eq!(
            enc(false, |w| write_nbit(w, &Int::Small(-12), -200, 8)),
            vec![188]
        );
    }

    #[test]
    fn nbit_rejects_big_and_out_of_range() {
        let mut w = BitWriter::new(Vec::new());
        let big = Int::Big {
            negative: false,
            magnitude: BigUint::from_u128(1u128 << 70),
        };
        assert!(write_nbit(&mut w, &big, 0, 12).is_err());
        assert!(write_nbit(&mut w, &Int::Small(i64::MIN), i64::MAX, 1).is_err());
        assert!(write_nbit(&mut w, &Int::Small(i64::MAX), i64::MIN, 1).is_err());
        assert!(write_nbit(&mut w, &Int::Small(0), 0, 0).is_ok());
    }

    #[test]
    fn conversions() {
        assert_eq!(int_to_uint(&Int::Small(5)), Some(Uint::Small(5)));
        assert_eq!(int_to_uint(&Int::Small(-1)), None);
        assert_eq!(
            uint_to_int(&Uint::Small(u64::MAX)).to_string(),
            "18446744073709551615"
        );
        assert_eq!(uint_to_int(&Uint::Small(7)), Int::Small(7));
    }
}
