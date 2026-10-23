//! Picks the right encoder or decoder for each [`DatatypeKind`], and converts between
//! lexical strings (the text form a value has in XML) and typed values.
//!
//! [`write_scalar`] does one of two things, then encodes the value:
//!
//! - for a [`Value::Lexical`], it applies the whitespace facet (collapse for every
//!   non-string kind, see
//!   [XML Schema 1.0 Part 2, whiteSpace](https://www.w3.org/TR/xmlschema-2/#rf-whiteSpace))
//!   and then parses the text;
//! - for a typed value, it checks that the value fits the kind.
//!
//! [`read_scalar`] does the reverse. [`parse_lexical`] and [`format_value`] offer the
//! text conversions on their own. They are also how enumeration values are matched
//! ([EXI 7.2](https://www.w3.org/TR/exi/#encodingEnumerations)): the value being
//! looked up and each allowed value are put into their one standard form through the
//! enumeration's item type, and then compared as strings.

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;
use alloc::borrow::Cow;

use super::binary::{
    format_base64, format_hex, parse_base64, parse_hex, read_binary, write_binary,
};
use super::boolean::{
    parse_boolean, parse_boolean_pattern, pattern_id_for, pattern_to_bool, read_boolean,
    read_boolean_pattern, write_boolean, write_boolean_pattern,
};
use super::datetime::{read_datetime, write_datetime};
use super::decimal::{read_decimal, write_decimal};
use super::float::{read_float, write_float};
use super::integer::{
    int_to_uint, read_integer, read_nbit, uint_to_int, write_integer, write_nbit,
};
use super::unsigned::read_unsigned_any;
use super::{DatatypeKind, TypeResolver, WhiteSpace};
use crate::error::{ExiError, ExiResult};
use crate::io::{BitReader, BitWriter, ceil_log2};
use crate::value::{DateTime, Decimal, Float, Int, OwnedValue, Uint, Value};

/// The four XML space characters. Only these separate list items and count for
/// the whiteSpace facet; other Unicode spaces (no-break space, ideographic space,
/// form feed) are ordinary characters
/// ([XML 1.0 S](https://www.w3.org/TR/xml/#NT-S),
/// [XSD whiteSpace](https://www.w3.org/TR/xmlschema-2/#rf-whiteSpace)).
pub(crate) fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// Applies the XSD `whiteSpace` facet
/// ([XML Schema 1.0 Part 2](https://www.w3.org/TR/xmlschema-2/#rf-whiteSpace)).
/// Borrows the input, without copying, when nothing changes.
///
/// - `Replace`: each tab, line feed or carriage return becomes a space.
/// - `Collapse`: split on the four XML space characters, drop empty pieces, and join
///   the rest with one space. That removes spaces at both ends and joins runs of
///   spaces. The quick check first finds out if the text is already collapsed.
pub fn apply_whitespace(ws: WhiteSpace, s: &str) -> Cow<'_, str> {
    let is_other_space = |c: char| matches!(c, '\t' | '\n' | '\r');
    match ws {
        WhiteSpace::Preserve => Cow::Borrowed(s),
        WhiteSpace::Replace => {
            if s.contains(is_other_space) {
                Cow::Owned(
                    s.chars()
                        .map(|c| if is_other_space(c) { ' ' } else { c })
                        .collect(),
                )
            } else {
                Cow::Borrowed(s)
            }
        }
        WhiteSpace::Collapse => {
            let unchanged = !s.starts_with(is_xml_space)
                && !s.ends_with(is_xml_space)
                && !s.contains(is_other_space)
                && !s.contains("  ");
            if unchanged {
                return Cow::Borrowed(s);
            }
            let mut out = String::with_capacity(s.len());
            for word in s.split(is_xml_space).filter(|w| !w.is_empty()) {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(word);
            }
            Cow::Owned(out)
        }
    }
}

fn collapse(s: &str) -> Cow<'_, str> {
    apply_whitespace(WhiteSpace::Collapse, s)
}

fn invalid(kind: DatatypeKind, s: &str) -> ExiError {
    ExiError::value(kind.name(), format!("{s:?} is not a valid lexical value"))
}

fn mismatch(kind: DatatypeKind, value: &Value<'_>) -> ExiError {
    ExiError::value(
        kind.name(),
        format!(
            "typed {} value does not match the datatype",
            value_name(value)
        ),
    )
}

fn value_name(value: &Value<'_>) -> &'static str {
    match value {
        Value::Lexical(_) => "lexical",
        Value::String(_) => "String",
        Value::Boolean(_) => "Boolean",
        Value::Integer(_) => "Integer",
        Value::Unsigned(_) => "Unsigned Integer",
        Value::Decimal(_) => "Decimal",
        Value::Float(_) => "Float",
        Value::DateTime(..) => "Date-Time",
        Value::Binary(_) => "Binary",
        Value::Enum(_) => "Enumeration",
        Value::List(_) => "List",
    }
}

fn not_scalar(kind: DatatypeKind) -> ExiError {
    match kind {
        DatatypeKind::String { .. }
        | DatatypeKind::RestrictedString { .. }
        | DatatypeKind::Untyped => ExiError::Internal(
            "string values are encoded through the string table, not the datatype layer",
        ),
        DatatypeKind::QName => {
            ExiError::Internal("QName values are encoded by the codec, not the datatype layer")
        }
        DatatypeKind::List { .. } => {
            ExiError::Internal("list values are iterated by the codec, not the datatype layer")
        }
        _ => ExiError::Internal("datatype kind is not a scalar"),
    }
}

/// Whether `value - min` fits in `bits` bits. Done in `i128` so the subtraction
/// cannot overflow.
fn nbit_in_range(value: &Int, min: i64, bits: u8) -> bool {
    value
        .to_i64()
        .map(|v| i128::from(v) - i128::from(min))
        .is_some_and(|offset| offset >= 0 && offset < (1i128 << bits.min(64)))
}

/// Parses a lexical value of `kind` into its typed form. The whitespace facet is
/// applied first (collapse for every non-string kind). Untyped text is kept as it is.
/// A list ([EXI 7.1.11](https://www.w3.org/TR/exi/#encodingList)) is split on XML
/// whitespace and each piece is parsed as the item type.
pub fn parse_lexical(
    kind: DatatypeKind,
    s: &str,
    types: &dyn TypeResolver,
) -> ExiResult<OwnedValue> {
    match kind {
        DatatypeKind::String { whitespace } | DatatypeKind::RestrictedString { whitespace } => Ok(
            OwnedValue::String(apply_whitespace(whitespace, s).into_owned()),
        ),
        DatatypeKind::Untyped => Ok(OwnedValue::String(s.to_string())),
        DatatypeKind::QName => Err(ExiError::value(
            kind.name(),
            "QName values are resolved by the codec",
        )),
        DatatypeKind::List { item } => {
            let item_kind = types.datatype(item);
            s.split(is_xml_space)
                .filter(|tok| !tok.is_empty())
                .map(|tok| parse_lexical(item_kind, tok, types))
                .collect::<ExiResult<Vec<_>>>()
                .map(OwnedValue::List)
        }
        DatatypeKind::Enumeration { set, item } => {
            enum_ordinal(kind, set, item, &Value::Lexical(s), types).map(OwnedValue::Enum)
        }
        _ => {
            let s = collapse(s);
            parse_scalar(kind, &s).ok_or_else(|| invalid(kind, &s))
        }
    }
}

/// Parses a lexical value of a scalar kind whose whitespace is already collapsed.
fn parse_scalar(kind: DatatypeKind, s: &str) -> Option<OwnedValue> {
    Some(match kind {
        DatatypeKind::Boolean => OwnedValue::Boolean(parse_boolean(s)?),
        DatatypeKind::BooleanPattern => {
            OwnedValue::Boolean(pattern_to_bool(parse_boolean_pattern(s)?))
        }
        DatatypeKind::Integer => OwnedValue::Integer(Int::from_decimal_str(s)?),
        DatatypeKind::UnsignedInteger => {
            OwnedValue::Unsigned(int_to_uint(&Int::from_decimal_str(s)?)?)
        }
        DatatypeKind::NBit { min, bits } => {
            OwnedValue::Integer(Int::from_decimal_str(s).filter(|v| nbit_in_range(v, min, bits))?)
        }
        DatatypeKind::Decimal => OwnedValue::Decimal(Decimal::from_lexical(s)?),
        DatatypeKind::Float => OwnedValue::Float(Float::from_lexical(s)?),
        DatatypeKind::DateTime(k) => OwnedValue::DateTime(k, DateTime::from_lexical(k, s)?),
        DatatypeKind::Base64Binary => OwnedValue::Binary(parse_base64(s)?),
        DatatypeKind::HexBinary => OwnedValue::Binary(parse_hex(s)?),
        _ => return None,
    })
}

/// Standard lexical form of `value` under `kind`. A [`Value::Lexical`] is parsed
/// and formatted again. An enumeration ordinal (position in the list of allowed
/// values) becomes the allowed value's string. List items are joined with a single
/// space.
pub fn format_value(
    kind: DatatypeKind,
    value: &Value<'_>,
    types: &dyn TypeResolver,
) -> ExiResult<String> {
    match value {
        Value::Lexical(s) => {
            let owned = parse_lexical(kind, s, types)?;
            format_value(kind, &owned.as_value(), types)
        }
        Value::String(s) => Ok((*s).to_string()),
        Value::Boolean(b) => Ok(if *b { "true" } else { "false" }.to_string()),
        Value::Integer(i) => Ok(match kind {
            DatatypeKind::Decimal => Decimal::from_int(i).to_lexical(),
            _ => i.to_string(),
        }),
        Value::Unsigned(u) => Ok(match kind {
            DatatypeKind::Decimal => Decimal::from_uint(u).to_lexical(),
            _ => u.to_string(),
        }),
        Value::Decimal(d) => Ok(d.to_lexical()),
        Value::Float(f) => Ok(f.to_lexical()),
        Value::DateTime(k, dt) => Ok(dt.to_lexical(*k)),
        Value::Binary(bytes) => match kind {
            DatatypeKind::HexBinary => Ok(format_hex(bytes)),
            DatatypeKind::Base64Binary => Ok(format_base64(bytes)),
            DatatypeKind::Enumeration { item, .. } => {
                format_value(types.datatype(item), value, types)
            }
            _ => Err(mismatch(kind, value)),
        },
        Value::Enum(ordinal) => match kind {
            DatatypeKind::Enumeration { set, .. } => types
                .enum_value(set, *ordinal as usize)
                .map(str::to_string)
                .ok_or_else(|| {
                    ExiError::value(kind.name(), format!("ordinal {ordinal} out of range"))
                }),
            _ => Err(mismatch(kind, value)),
        },
        Value::List(items) => {
            let DatatypeKind::List { item } = kind else {
                return Err(mismatch(kind, value));
            };
            let item_kind = types.datatype(item);
            let parts = items
                .iter()
                .map(|v| format_value(item_kind, v, types))
                .collect::<ExiResult<Vec<_>>>()?;
            Ok(parts.join(" "))
        }
    }
}

/// Finds the ordinal (position) of `value` in enumeration `set`
/// ([EXI 7.2](https://www.w3.org/TR/exi/#encodingEnumerations)), putting values in
/// standard form through `item` first. So for an integer enumeration, `"010"` matches
/// the allowed value `"10"`. Each allowed value is parsed and formatted the same way,
/// and the first one that matches wins. This is a linear scan over the allowed
/// values.
fn enum_ordinal(
    kind: DatatypeKind,
    set: crate::ids::EnumSetIdx,
    item: crate::ids::TypeIdx,
    value: &Value<'_>,
    types: &dyn TypeResolver,
) -> ExiResult<u32> {
    let count = types.enum_count(set);
    if let Value::Enum(ordinal) = value {
        return if (*ordinal as usize) < count {
            Ok(*ordinal)
        } else {
            Err(ExiError::value(
                kind.name(),
                format!("ordinal {ordinal} out of range (count {count})"),
            ))
        };
    }
    let item_kind = types.datatype(item);
    let canonical = format_value(item_kind, value, types)?;
    let matches = (0..count).find(|&i| {
        types
            .enum_value(set, i)
            .and_then(|s| parse_lexical(item_kind, s, types).ok())
            .and_then(|owned| format_value(item_kind, &owned.as_value(), types).ok())
            .is_some_and(|candidate| candidate == canonical)
    });
    matches.map(|i| i as u32).ok_or_else(|| {
        ExiError::value(
            kind.name(),
            format!("{canonical:?} is not an enumerated value"),
        )
    })
}

fn as_int(kind: DatatypeKind, value: &Value<'_>) -> ExiResult<Int> {
    match value {
        Value::Integer(i) => Ok(i.clone()),
        Value::Unsigned(u) => Ok(uint_to_int(u)),
        Value::Lexical(s) => {
            let s = collapse(s);
            Int::from_decimal_str(&s).ok_or_else(|| invalid(kind, &s))
        }
        _ => Err(mismatch(kind, value)),
    }
}

fn as_uint(kind: DatatypeKind, value: &Value<'_>) -> ExiResult<Uint> {
    match value {
        Value::Unsigned(u) => Ok(u.clone()),
        Value::Integer(i) => {
            int_to_uint(i).ok_or_else(|| ExiError::value(kind.name(), format!("{i} is negative")))
        }
        Value::Lexical(s) => {
            let s = collapse(s);
            let i = Int::from_decimal_str(&s).ok_or_else(|| invalid(kind, &s))?;
            int_to_uint(&i).ok_or_else(|| ExiError::value(kind.name(), format!("{i} is negative")))
        }
        _ => Err(mismatch(kind, value)),
    }
}

/// Encodes one scalar value of `kind`. See the module docs for how text and typed
/// values are handled. An enumeration is written as its ordinal in
/// `ceil(log2(count))` bits ([EXI 7.2](https://www.w3.org/TR/exi/#encodingEnumerations)).
pub fn write_scalar<W: ByteSink>(
    w: &mut BitWriter<W>,
    kind: DatatypeKind,
    value: &Value<'_>,
    types: &dyn TypeResolver,
) -> ExiResult<()> {
    match kind {
        DatatypeKind::String { .. }
        | DatatypeKind::RestrictedString { .. }
        | DatatypeKind::Untyped
        | DatatypeKind::QName
        | DatatypeKind::List { .. } => Err(not_scalar(kind)),
        DatatypeKind::Boolean => {
            let b = match value {
                Value::Boolean(b) => *b,
                Value::Lexical(s) => {
                    let s = collapse(s);
                    parse_boolean(&s).ok_or_else(|| invalid(kind, &s))?
                }
                _ => return Err(mismatch(kind, value)),
            };
            write_boolean(w, b)
        }
        DatatypeKind::BooleanPattern => {
            let id = match value {
                Value::Boolean(b) => pattern_id_for(*b),
                Value::Lexical(s) => {
                    let s = collapse(s);
                    parse_boolean_pattern(&s).ok_or_else(|| invalid(kind, &s))?
                }
                _ => return Err(mismatch(kind, value)),
            };
            write_boolean_pattern(w, id)
        }
        DatatypeKind::Integer => write_integer(w, &as_int(kind, value)?),
        DatatypeKind::UnsignedInteger => as_uint(kind, value)?.write(w),
        DatatypeKind::NBit { min, bits } => write_nbit(w, &as_int(kind, value)?, min, bits),
        DatatypeKind::Decimal => {
            let d = match value {
                Value::Decimal(d) => Cow::Borrowed(d),
                Value::Lexical(s) => {
                    let s = collapse(s);
                    Cow::Owned(Decimal::from_lexical(&s).ok_or_else(|| invalid(kind, &s))?)
                }
                Value::Integer(i) => Cow::Owned(Decimal::from_int(i)),
                Value::Unsigned(u) => Cow::Owned(Decimal::from_uint(u)),
                _ => return Err(mismatch(kind, value)),
            };
            write_decimal(w, &d)
        }
        DatatypeKind::Float => {
            let f = match value {
                Value::Float(f) => *f,
                Value::Lexical(s) => {
                    let s = collapse(s);
                    Float::from_lexical(&s).ok_or_else(|| invalid(kind, &s))?
                }
                _ => return Err(mismatch(kind, value)),
            };
            write_float(w, f)
        }
        DatatypeKind::DateTime(k) => match value {
            Value::DateTime(vk, dt) if *vk == k => write_datetime(w, k, dt),
            Value::DateTime(vk, _) => Err(ExiError::value(
                kind.name(),
                format!("{} value where {} is expected", vk.name(), k.name()),
            )),
            Value::Lexical(s) => {
                let s = collapse(s);
                let dt = DateTime::from_lexical(k, &s).ok_or_else(|| invalid(kind, &s))?;
                write_datetime(w, k, &dt)
            }
            _ => Err(mismatch(kind, value)),
        },
        DatatypeKind::Base64Binary | DatatypeKind::HexBinary => match value {
            Value::Binary(bytes) => write_binary(w, bytes),
            Value::Lexical(s) => {
                let s = collapse(s);
                let bytes = if kind == DatatypeKind::HexBinary {
                    parse_hex(&s)
                } else {
                    parse_base64(&s)
                };
                write_binary(w, &bytes.ok_or_else(|| invalid(kind, &s))?)
            }
            _ => Err(mismatch(kind, value)),
        },
        DatatypeKind::Enumeration { set, item } => {
            let ordinal = enum_ordinal(kind, set, item, value, types)?;
            w.write_nbit_unsigned(u64::from(ordinal), ceil_log2(types.enum_count(set)))
        }
    }
}

/// Decodes one scalar value of `kind`. An enumeration ordinal past the last allowed
/// value is an error.
pub fn read_scalar<R: ByteSource>(
    r: &mut BitReader<R>,
    kind: DatatypeKind,
    types: &dyn TypeResolver,
) -> ExiResult<OwnedValue> {
    Ok(match kind {
        DatatypeKind::String { .. }
        | DatatypeKind::RestrictedString { .. }
        | DatatypeKind::Untyped
        | DatatypeKind::QName
        | DatatypeKind::List { .. } => {
            return Err(not_scalar(kind));
        }
        DatatypeKind::Boolean => OwnedValue::Boolean(read_boolean(r)?),
        DatatypeKind::BooleanPattern => {
            OwnedValue::Boolean(pattern_to_bool(read_boolean_pattern(r)?))
        }
        DatatypeKind::Integer => OwnedValue::Integer(read_integer(r)?),
        DatatypeKind::UnsignedInteger => OwnedValue::Unsigned(read_unsigned_any(r)?),
        DatatypeKind::NBit { min, bits } => OwnedValue::Integer(read_nbit(r, min, bits)?),
        DatatypeKind::Decimal => OwnedValue::Decimal(read_decimal(r)?),
        DatatypeKind::Float => OwnedValue::Float(read_float(r)?),
        DatatypeKind::DateTime(k) => OwnedValue::DateTime(k, read_datetime(r, k)?),
        DatatypeKind::Base64Binary | DatatypeKind::HexBinary => OwnedValue::Binary(read_binary(r)?),
        DatatypeKind::Enumeration { set, .. } => {
            let count = types.enum_count(set);
            let ordinal = r.read_nbit_unsigned(ceil_log2(count))?;
            if ordinal as usize >= count {
                return Err(ExiError::value(
                    kind.name(),
                    format!("ordinal {ordinal} out of range (count {count})"),
                ));
            }
            OwnedValue::Enum(ordinal as u32)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{EnumSetIdx, TypeIdx};
    use crate::value::DateTimeKind;

    /// Types: 0 Integer, 1 String(collapse), 2 Float, 3 Decimal, 4 List of Integer,
    /// 5 NBit 2..=10, 6 Enumeration of strings, 7 Enumeration of integers, 8 Enumeration of decimals.
    struct Mock;

    const COLOURS: [&str; 3] = ["red", "green", "blue"];
    const NUMBERS: [&str; 4] = ["1", "2", "3", "10"];
    const DECIMALS: [&str; 2] = ["1.0", "2.5"];

    impl TypeResolver for Mock {
        fn datatype(&self, t: TypeIdx) -> DatatypeKind {
            match t {
                0 => DatatypeKind::Integer,
                1 => DatatypeKind::String {
                    whitespace: WhiteSpace::Collapse,
                },
                2 => DatatypeKind::Float,
                3 => DatatypeKind::Decimal,
                4 => DatatypeKind::List { item: 0 },
                5 => DatatypeKind::NBit { min: 2, bits: 4 },
                6 => DatatypeKind::Enumeration { set: 0, item: 1 },
                7 => DatatypeKind::Enumeration { set: 1, item: 0 },
                _ => DatatypeKind::Enumeration { set: 2, item: 3 },
            }
        }
        fn enum_count(&self, set: EnumSetIdx) -> usize {
            match set {
                0 => COLOURS.len(),
                1 => NUMBERS.len(),
                _ => DECIMALS.len(),
            }
        }
        fn enum_value(&self, set: EnumSetIdx, ordinal: usize) -> Option<&str> {
            match set {
                0 => COLOURS.get(ordinal).copied(),
                1 => NUMBERS.get(ordinal).copied(),
                _ => DECIMALS.get(ordinal).copied(),
            }
        }
    }

    fn enc(aligned: bool, kind: DatatypeKind, value: &Value<'_>) -> ExiResult<Vec<u8>> {
        let mut w = BitWriter::with_alignment(Vec::new(), aligned);
        write_scalar(&mut w, kind, value, &Mock)?;
        w.finish()
    }

    fn dec(aligned: bool, kind: DatatypeKind, bytes: &[u8]) -> ExiResult<OwnedValue> {
        let mut r = BitReader::with_alignment(bytes, aligned);
        read_scalar(&mut r, kind, &Mock)
    }

    #[test]
    fn whitespace_facets() {
        assert!(matches!(
            apply_whitespace(WhiteSpace::Preserve, " a\tb "),
            Cow::Borrowed(" a\tb ")
        ));
        assert!(matches!(
            apply_whitespace(WhiteSpace::Replace, "ab"),
            Cow::Borrowed("ab")
        ));
        assert_eq!(
            apply_whitespace(WhiteSpace::Replace, " a\tb\r\nc "),
            " a b  c "
        );
        assert!(matches!(
            apply_whitespace(WhiteSpace::Collapse, "a b"),
            Cow::Borrowed("a b")
        ));
        assert_eq!(
            apply_whitespace(WhiteSpace::Collapse, "  a \t b\n\nc  "),
            "a b c"
        );
        assert_eq!(apply_whitespace(WhiteSpace::Collapse, " \n "), "");
        assert_eq!(apply_whitespace(WhiteSpace::Collapse, ""), "");
    }

    #[test]
    fn lexical_and_typed_values_encode_identically() {
        use DatatypeKind as D;
        let dt = DateTime::from_lexical(DateTimeKind::Date, "2026-04-15").unwrap_or_default();
        let cases: Vec<(D, Value<'static>, &str)> = vec![
            (D::Boolean, Value::Boolean(true), " true "),
            (D::Boolean, Value::Boolean(false), "0"),
            (D::BooleanPattern, Value::Boolean(true), "\ttrue\n"),
            (D::Integer, Value::Integer(Int::Small(-913)), " -913 "),
            (D::Integer, Value::Unsigned(Uint::Small(40)), "+40"),
            (D::UnsignedInteger, Value::Unsigned(Uint::Small(421)), "421"),
            (D::UnsignedInteger, Value::Integer(Int::Small(421)), " 421"),
            (
                D::NBit { min: -200, bits: 8 },
                Value::Integer(Int::Small(-12)),
                "-12",
            ),
            (
                D::Decimal,
                Value::Decimal(Decimal::new(false, 5.into(), 100.into())),
                "5.001",
            ),
            (D::Decimal, Value::Integer(Int::Small(-7)), "-7.0"),
            (
                D::Float,
                Value::Float(Float {
                    mantissa: 7325,
                    exponent: -2,
                }),
                " 73.25 ",
            ),
            (
                D::DateTime(DateTimeKind::Date),
                Value::DateTime(DateTimeKind::Date, dt),
                "2026-04-15",
            ),
            (D::HexBinary, Value::Binary(&[0x0F, 0xB7]), " 0fb7 "),
            (D::Base64Binary, Value::Binary(&[0x0F, 0xB7]), "D7c=\n"),
            (
                D::Enumeration { set: 0, item: 1 },
                Value::Enum(1),
                "  green ",
            ),
            (D::Enumeration { set: 1, item: 0 }, Value::Enum(3), "+10"),
            (D::Enumeration { set: 2, item: 3 }, Value::Enum(1), "02.50"),
            (
                D::Enumeration { set: 2, item: 3 },
                Value::Decimal(Decimal::new(false, 1.into(), 0.into())),
                "1",
            ),
        ];
        for (kind, typed, lexical) in &cases {
            for aligned in [false, true] {
                let a = enc(aligned, *kind, typed);
                let b = enc(aligned, *kind, &Value::Lexical(lexical));
                assert!(a.is_ok(), "{kind:?} typed: {a:?}");
                assert_eq!(a, b, "{kind:?} {lexical:?}");
                let decoded = dec(aligned, *kind, &a.clone().unwrap_or_default());
                assert!(decoded.is_ok(), "{kind:?}: {decoded:?}");
                let decoded = decoded.unwrap_or(OwnedValue::Enum(u32::MAX));
                let expected = format_value(*kind, typed, &Mock);
                assert_eq!(
                    format_value(*kind, &decoded.as_value(), &Mock),
                    expected,
                    "{kind:?}"
                );
            }
        }
    }

    #[test]
    fn exact_bytes() {
        use DatatypeKind as D;
        assert_eq!(
            enc(false, D::Integer, &Value::Lexical("-913")),
            Ok(vec![0xC8, 0x03, 0x80])
        );
        assert_eq!(
            enc(true, D::Integer, &Value::Lexical("-913")),
            Ok(vec![0x01, 0x90, 0x07])
        );
        assert_eq!(
            enc(false, D::Decimal, &Value::Lexical("5.001")).map(|b| b[..2].to_vec()),
            Ok(vec![0x02, 0xB2])
        );
        assert_eq!(
            enc(false, D::Float, &Value::Lexical("500")).map(|b| b[..2].to_vec()),
            Ok(vec![0x02, 0x80])
        );
        assert_eq!(
            enc(
                false,
                D::Enumeration { set: 0, item: 1 },
                &Value::Lexical("blue")
            ),
            Ok(vec![0b1000_0000])
        );
        assert_eq!(
            enc(
                true,
                D::Enumeration { set: 1, item: 0 },
                &Value::Lexical("3")
            ),
            Ok(vec![0x02])
        );
        assert_eq!(
            enc(false, D::BooleanPattern, &Value::Lexical("1")),
            Ok(vec![0b1100_0000])
        );
        assert_eq!(
            enc(false, D::BooleanPattern, &Value::Lexical("0")),
            Ok(vec![0b0100_0000])
        );
        assert_eq!(
            enc(false, D::Base64Binary, &Value::Lexical("ZHM=")),
            Ok(vec![0x02, b'd', b's'])
        );
    }

    #[test]
    fn type_checks_and_errors() {
        use DatatypeKind as D;
        let value_err = |r: ExiResult<Vec<u8>>| matches!(r, Err(ExiError::Value { .. }));
        assert!(value_err(enc(
            false,
            D::UnsignedInteger,
            &Value::Integer(Int::Small(-1))
        )));
        assert!(value_err(enc(
            false,
            D::UnsignedInteger,
            &Value::Lexical("-1")
        )));
        assert!(value_err(enc(
            false,
            D::NBit { min: 2, bits: 4 },
            &Value::Lexical("18")
        )));
        assert!(value_err(enc(
            false,
            D::NBit { min: 2, bits: 4 },
            &Value::Integer(Int::Small(1))
        )));
        assert!(value_err(enc(
            false,
            D::Float,
            &Value::Integer(Int::Small(1))
        )));
        assert!(value_err(enc(false, D::Float, &Value::Lexical("1.2.3"))));
        assert!(value_err(enc(false, D::Boolean, &Value::Lexical("yes"))));
        assert!(value_err(enc(
            false,
            D::Boolean,
            &Value::Integer(Int::Small(1))
        )));
        assert!(value_err(enc(false, D::Decimal, &Value::Lexical("abc"))));
        assert!(value_err(enc(false, D::Decimal, &Value::Boolean(true))));
        assert!(value_err(enc(
            false,
            D::HexBinary,
            &Value::Lexical("0F B7")
        )));
        assert!(value_err(enc(
            false,
            D::Base64Binary,
            &Value::Lexical("0FB7 x")
        )));
        assert!(value_err(enc(
            false,
            D::Enumeration { set: 0, item: 1 },
            &Value::Lexical("purple")
        )));
        assert!(value_err(enc(
            false,
            D::Enumeration { set: 0, item: 1 },
            &Value::Enum(3)
        )));
        assert!(value_err(enc(
            false,
            D::Enumeration { set: 1, item: 0 },
            &Value::Lexical("4")
        )));
        assert!(value_err(enc(
            false,
            D::DateTime(DateTimeKind::Date),
            &Value::Lexical("2026")
        )));
        let dt = DateTime::from_lexical(DateTimeKind::GYear, "2026").unwrap_or_default();
        assert!(value_err(enc(
            false,
            D::DateTime(DateTimeKind::Date),
            &Value::DateTime(DateTimeKind::GYear, dt)
        )));
        // Non-scalar kinds are the codec's job.
        for kind in [
            D::String {
                whitespace: WhiteSpace::Preserve,
            },
            D::Untyped,
            D::QName,
            D::List { item: 0 },
        ] {
            assert!(
                matches!(
                    enc(false, kind, &Value::Lexical("x")),
                    Err(ExiError::Internal(_))
                ),
                "{kind:?}"
            );
            assert!(
                matches!(dec(false, kind, &[0]), Err(ExiError::Internal(_))),
                "{kind:?}"
            );
        }
        // Decoding an enumeration ordinal past the end.
        assert!(matches!(
            dec(false, D::Enumeration { set: 0, item: 1 }, &[0b1100_0000]),
            Err(ExiError::Value { .. })
        ));
    }

    #[test]
    fn parse_and_format() {
        use DatatypeKind as D;
        let list = parse_lexical(D::List { item: 0 }, " 1  -2\n3 ", &Mock);
        assert_eq!(
            list,
            Ok(OwnedValue::List(vec![
                OwnedValue::Integer(Int::Small(1)),
                OwnedValue::Integer(Int::Small(-2)),
                OwnedValue::Integer(Int::Small(3)),
            ]))
        );
        let list = list.unwrap_or(OwnedValue::List(Vec::new()));
        assert_eq!(
            format_value(D::List { item: 0 }, &list.as_value(), &Mock),
            Ok("1 -2 3".to_string())
        );
        assert_eq!(
            format_value(D::List { item: 0 }, &Value::Lexical(" 1  -2\n3 "), &Mock),
            Ok("1 -2 3".to_string())
        );
        assert_eq!(
            parse_lexical(D::List { item: 0 }, "", &Mock),
            Ok(OwnedValue::List(Vec::new()))
        );
        assert!(parse_lexical(D::List { item: 0 }, "1 x", &Mock).is_err());
        assert_eq!(
            parse_lexical(
                D::String {
                    whitespace: WhiteSpace::Collapse
                },
                " a  b ",
                &Mock
            ),
            Ok(OwnedValue::String("a b".into()))
        );
        assert_eq!(
            parse_lexical(D::Untyped, " a  b ", &Mock),
            Ok(OwnedValue::String(" a  b ".into()))
        );
        assert_eq!(
            parse_lexical(D::Enumeration { set: 0, item: 1 }, "blue", &Mock),
            Ok(OwnedValue::Enum(2))
        );
        assert_eq!(
            parse_lexical(D::Enumeration { set: 2, item: 3 }, "+1.00", &Mock),
            Ok(OwnedValue::Enum(0))
        );
        assert_eq!(
            parse_lexical(D::BooleanPattern, "1", &Mock),
            Ok(OwnedValue::Boolean(true))
        );
        assert_eq!(
            parse_lexical(D::Float, " 1.0E3 ", &Mock),
            Ok(OwnedValue::Float(Float {
                mantissa: 1,
                exponent: 3
            }))
        );
        assert!(parse_lexical(D::QName, "a:b", &Mock).is_err());
        assert_eq!(
            format_value(D::Enumeration { set: 0, item: 1 }, &Value::Enum(1), &Mock),
            Ok("green".into())
        );
        assert!(format_value(D::Enumeration { set: 0, item: 1 }, &Value::Enum(9), &Mock).is_err());
        assert_eq!(
            format_value(D::Float, &Value::Lexical("12.0"), &Mock),
            Ok("12E0".into())
        );
        assert_eq!(
            format_value(D::Decimal, &Value::Lexical("-.50"), &Mock),
            Ok("-0.5".into())
        );
        assert_eq!(
            format_value(D::Decimal, &Value::Integer(Int::Small(-7)), &Mock),
            Ok("-7.0".into())
        );
        assert_eq!(
            format_value(D::Integer, &Value::Integer(Int::Small(-7)), &Mock),
            Ok("-7".into())
        );
        assert_eq!(
            format_value(D::HexBinary, &Value::Binary(&[0xAB]), &Mock),
            Ok("AB".into())
        );
        assert_eq!(
            format_value(D::Base64Binary, &Value::Binary(b"foo"), &Mock),
            Ok("Zm9v".into())
        );
        assert!(format_value(D::Integer, &Value::Binary(b"foo"), &Mock).is_err());
        assert_eq!(
            format_value(D::Boolean, &Value::Lexical("1"), &Mock),
            Ok("true".into())
        );
        assert_eq!(
            format_value(
                D::DateTime(DateTimeKind::Time),
                &Value::Lexical("12:00:00+00:00"),
                &Mock
            ),
            Ok("12:00:00Z".into())
        );
    }

    #[test]
    fn read_scalar_all_kinds() {
        use DatatypeKind as D;
        let cases: Vec<(D, Value<'static>)> = vec![
            (D::Boolean, Value::Boolean(true)),
            (D::BooleanPattern, Value::Boolean(false)),
            (D::Integer, Value::Integer(Int::Small(-41))),
            (D::UnsignedInteger, Value::Unsigned(Uint::Small(12372))),
            (
                D::NBit { min: 0, bits: 12 },
                Value::Integer(Int::Small(4095)),
            ),
            (
                D::Decimal,
                Value::Decimal(Decimal::new(true, 1.into(), 32.into())),
            ),
            (D::Float, Value::Float(Float::NEG_INFINITY)),
            (D::HexBinary, Value::Binary(&[1, 2, 3])),
            (D::Enumeration { set: 1, item: 0 }, Value::Enum(2)),
        ];
        for (kind, value) in &cases {
            for aligned in [false, true] {
                let bytes = enc(aligned, *kind, value).unwrap_or_default();
                let decoded = dec(aligned, *kind, &bytes).unwrap_or(OwnedValue::Enum(u32::MAX));
                assert_eq!(decoded.as_value(), *value, "{kind:?}");
            }
        }
        let dt = DateTime::from_lexical(DateTimeKind::DateTime, "2026-04-15T17:45:00.5+02:00")
            .unwrap_or_default();
        let bytes = enc(
            false,
            D::DateTime(DateTimeKind::DateTime),
            &Value::DateTime(DateTimeKind::DateTime, dt),
        )
        .unwrap_or_default();
        assert_eq!(
            dec(false, D::DateTime(DateTimeKind::DateTime), &bytes),
            Ok(OwnedValue::DateTime(DateTimeKind::DateTime, dt))
        );
    }
}
