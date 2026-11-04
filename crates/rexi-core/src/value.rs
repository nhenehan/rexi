//! Typed values passed in and out through the SAX-like (event by event) API. Each
//! kind matches one of the built-in EXI datatype encodings
//! ([EXI 7.1](https://www.w3.org/TR/exi/#encodingDatatypes)).

use crate::error::{ExiError, ExiResult};
use crate::prelude::*;
use core::fmt;

pub use crate::datatype::biguint::BigUint;
pub use crate::datatype::unsigned::Uint;

/// A signed integer of any size
/// ([EXI 7.1.5](https://www.w3.org/TR/exi/#encodingInteger)).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Int {
    /// A value that fits in `i64`.
    Small(i64),
    /// Sign and size (magnitude) for values that do not fit in `i64`.
    Big {
        /// Whether the value is below zero.
        negative: bool,
        /// The size of the value, ignoring the sign.
        magnitude: BigUint,
    },
}

impl Int {
    /// Whether the value is below zero.
    pub fn is_negative(&self) -> bool {
        match self {
            Int::Small(v) => *v < 0,
            Int::Big { negative, .. } => *negative,
        }
    }

    /// The value as an `i64`, or `None` for `Big` values.
    pub fn to_i64(&self) -> Option<i64> {
        match self {
            Int::Small(v) => Some(*v),
            Int::Big { .. } => None,
        }
    }

    /// Parses base-10 digits with an optional `+` or `-` sign; `None` when the text
    /// is not a plain integer.
    pub fn from_decimal_str(s: &str) -> Option<Self> {
        let (negative, digits) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        if let Ok(v) = s.parse::<i64>() {
            return Some(Int::Small(v));
        }
        let magnitude = BigUint::from_decimal_str(digits)?;
        Some(Int::Big {
            negative,
            magnitude,
        })
    }
}

impl fmt::Display for Int {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Int::Small(v) => write!(f, "{v}"),
            Int::Big {
                negative,
                magnitude,
            } => {
                if *negative {
                    f.write_str("-")?;
                }
                write!(f, "{magnitude}")
            }
        }
    }
}

impl From<i64> for Int {
    fn from(v: i64) -> Self {
        Int::Small(v)
    }
}

/// A decimal ([EXI 7.1.3](https://www.w3.org/TR/exi/#encodingDecimal)): sign, the
/// whole-number part, and the digits after the point in reverse order.
///
/// `fraction_reversed` holds the digits after the point in reverse order, as an
/// integer. So `.05` is stored as 50 and `.50` as 5 (trailing zeros cannot be
/// stored).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Decimal {
    /// Whether the value is below zero.
    pub negative: bool,
    /// The digits before the point.
    pub integral: Uint,
    /// The digits after the point, in reverse order (see above).
    pub fraction_reversed: Uint,
}

/// A float ([EXI 7.1.4](https://www.w3.org/TR/exi/#encodingFloat)):
/// `mantissa * 10^exponent`.
///
/// An exponent of `-16384` marks the special values: mantissa 1 is +INF,
/// -1 is -INF, and anything else is NaN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Float {
    /// The digits, as a whole number.
    pub mantissa: i64,
    /// The power of ten to multiply by, or [`Float::SPECIAL_EXPONENT`].
    pub exponent: i16,
}

impl Float {
    /// The exponent that marks INF, -INF and NaN.
    pub const SPECIAL_EXPONENT: i16 = -16384;
    /// Positive infinity (INF).
    pub const INFINITY: Float = Float {
        mantissa: 1,
        exponent: Self::SPECIAL_EXPONENT,
    };
    /// Negative infinity (-INF).
    pub const NEG_INFINITY: Float = Float {
        mantissa: -1,
        exponent: Self::SPECIAL_EXPONENT,
    };
    /// Not a number (NaN).
    pub const NAN: Float = Float {
        mantissa: 0,
        exponent: Self::SPECIAL_EXPONENT,
    };
}

/// Which XML Schema date/time type a [`DateTime`] value belongs to
/// ([EXI 7.1.8](https://www.w3.org/TR/exi/#encodingDateTime), Table 7-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateTimeKind {
    /// `xsd:dateTime`: date and time.
    DateTime,
    /// `xsd:date`: a date.
    Date,
    /// `xsd:time`: a time of day.
    Time,
    /// `xsd:gYear`: a year.
    GYear,
    /// `xsd:gYearMonth`: a month of a year.
    GYearMonth,
    /// `xsd:gMonth`: a month of any year.
    GMonth,
    /// `xsd:gMonthDay`: a day of a month of any year.
    GMonthDay,
    /// `xsd:gDay`: a day of any month.
    GDay,
}

/// A date/time value ([EXI 7.1.8](https://www.w3.org/TR/exi/#encodingDateTime)).
/// Which fields mean anything depends on the [`DateTimeKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DateTime {
    /// Year (may be negative or above 9999).
    pub year: i64,
    /// Month, 1 to 12.
    pub month: u8,
    /// Day of the month, 1 to 31.
    pub day: u8,
    /// Hour, 0 to 23 (24 is changed to 0 of the next day).
    pub hour: u8,
    /// Minute, 0 to 59.
    pub minute: u8,
    /// Second, 0 to 60 (60 allows a leap second).
    pub second: u8,
    /// Fractions of a second as reversed digits (`.5` is 5, `.05` is 50), `None` if
    /// there are none.
    pub fractional_secs: Option<u64>,
    /// Time zone offset in minutes from UTC, `None` if there is none.
    pub timezone: Option<i16>,
}

/// A value passed to the encoder or produced by the decoder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Value<'a> {
    /// A value in its XML text form (XML Schema calls this the "lexical" form). The
    /// grammar's datatype decides how it is parsed.
    Lexical(&'a str),
    /// Text for a String datatype (or untyped content).
    String(&'a str),
    /// A Boolean.
    Boolean(bool),
    /// A signed Integer.
    Integer(Int),
    /// An Unsigned Integer.
    Unsigned(Uint),
    /// A Decimal.
    Decimal(Decimal),
    /// A Float (`xsd:float` or `xsd:double`).
    Float(Float),
    /// A date or time of the given kind.
    DateTime(DateTimeKind, DateTime),
    /// The bytes of a Binary value (`xsd:base64Binary` or `xsd:hexBinary`).
    Binary(&'a [u8]),
    /// Position of an enumeration value, in the order the schema lists them
    /// ([EXI 7.2](https://www.w3.org/TR/exi/#encodingEnumerations)).
    Enum(u32),
    /// The items of a List value
    /// ([EXI 7.1.11](https://www.w3.org/TR/exi/#encodingList)).
    List(Vec<Value<'a>>),
}

/// A value that owns its data, made by the decoder and by parsing text forms.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OwnedValue {
    /// See [`Value::String`].
    String(String),
    /// See [`Value::Boolean`].
    Boolean(bool),
    /// See [`Value::Integer`].
    Integer(Int),
    /// See [`Value::Unsigned`].
    Unsigned(Uint),
    /// See [`Value::Decimal`].
    Decimal(Decimal),
    /// See [`Value::Float`].
    Float(Float),
    /// See [`Value::DateTime`].
    DateTime(DateTimeKind, DateTime),
    /// See [`Value::Binary`].
    Binary(Vec<u8>),
    /// See [`Value::Enum`].
    Enum(u32),
    /// See [`Value::List`].
    List(Vec<OwnedValue>),
}

impl OwnedValue {
    /// This value as a [`Value`]. Strings and bytes are borrowed; a list and
    /// the digits of a big integer are copied, so this may allocate.
    pub fn to_value(&self) -> Value<'_> {
        match self {
            OwnedValue::String(s) => Value::String(s),
            OwnedValue::Boolean(b) => Value::Boolean(*b),
            OwnedValue::Integer(i) => Value::Integer(i.clone()),
            OwnedValue::Unsigned(u) => Value::Unsigned(u.clone()),
            OwnedValue::Decimal(d) => Value::Decimal(d.clone()),
            OwnedValue::Float(f) => Value::Float(*f),
            OwnedValue::DateTime(k, d) => Value::DateTime(*k, *d),
            OwnedValue::Binary(b) => Value::Binary(b),
            OwnedValue::Enum(o) => Value::Enum(*o),
            OwnedValue::List(items) => {
                Value::List(items.iter().map(OwnedValue::to_value).collect())
            }
        }
    }
}

impl Value<'_> {
    /// Copies this value into an [`OwnedValue`]. Both `Lexical` and `String` become
    /// `OwnedValue::String`.
    pub fn to_owned_value(&self) -> OwnedValue {
        match self {
            Value::Lexical(s) | Value::String(s) => OwnedValue::String((*s).to_string()),
            Value::Boolean(b) => OwnedValue::Boolean(*b),
            Value::Integer(i) => OwnedValue::Integer(i.clone()),
            Value::Unsigned(u) => OwnedValue::Unsigned(u.clone()),
            Value::Decimal(d) => OwnedValue::Decimal(d.clone()),
            Value::Float(f) => OwnedValue::Float(*f),
            Value::DateTime(k, d) => OwnedValue::DateTime(*k, *d),
            Value::Binary(b) => OwnedValue::Binary(b.to_vec()),
            Value::Enum(o) => OwnedValue::Enum(*o),
            Value::List(items) => {
                OwnedValue::List(items.iter().map(Value::to_owned_value).collect())
            }
        }
    }
}

/// The error for text that is not a number of the expected type.
fn not_a(datatype: &'static str, s: &str) -> ExiError {
    ExiError::value(
        datatype,
        format!("{s:?} is not a valid {datatype} text form"),
    )
}

/// Parses the XML Schema text form, as [`Int::from_decimal_str`] does.
impl core::str::FromStr for Int {
    type Err = ExiError;
    fn from_str(s: &str) -> ExiResult<Self> {
        Int::from_decimal_str(s).ok_or_else(|| not_a("Integer", s))
    }
}

/// Parses the XML Schema text form, as [`Uint::from_decimal_str`] does.
impl core::str::FromStr for Uint {
    type Err = ExiError;
    fn from_str(s: &str) -> ExiResult<Self> {
        Uint::from_decimal_str(s).ok_or_else(|| not_a("Unsigned Integer", s))
    }
}

/// Parses decimal digits, as [`BigUint::from_decimal_str`] does.
impl core::str::FromStr for BigUint {
    type Err = ExiError;
    fn from_str(s: &str) -> ExiResult<Self> {
        BigUint::from_decimal_str(s).ok_or_else(|| not_a("Unsigned Integer", s))
    }
}

/// Parses the XML Schema text form, as [`Decimal::from_lexical`] does.
impl core::str::FromStr for Decimal {
    type Err = ExiError;
    fn from_str(s: &str) -> ExiResult<Self> {
        Decimal::from_lexical(s).ok_or_else(|| not_a("Decimal", s))
    }
}

/// Parses the XML Schema text form, as [`Float::from_lexical`] does.
impl core::str::FromStr for Float {
    type Err = ExiError;
    fn from_str(s: &str) -> ExiResult<Self> {
        Float::from_lexical(s).ok_or_else(|| not_a("Float", s))
    }
}

impl From<u128> for BigUint {
    fn from(v: u128) -> Self {
        BigUint::from_u128(v)
    }
}

impl From<Int> for Decimal {
    fn from(v: Int) -> Self {
        Decimal::from_int(&v)
    }
}

impl From<Uint> for Decimal {
    fn from(v: Uint) -> Self {
        Decimal::from_uint(&v)
    }
}

impl From<&Value<'_>> for OwnedValue {
    fn from(v: &Value<'_>) -> Self {
        v.to_owned_value()
    }
}

/// Fails when the value does not fit in an `i64`.
impl TryFrom<&Int> for i64 {
    type Error = ExiError;
    fn try_from(v: &Int) -> ExiResult<Self> {
        v.to_i64()
            .ok_or_else(|| ExiError::value("Integer", "value does not fit in 64 bits"))
    }
}

/// Fails when the value does not fit in a `u64`.
impl TryFrom<&Uint> for u64 {
    type Error = ExiError;
    fn try_from(v: &Uint) -> ExiResult<Self> {
        v.to_u64()
            .ok_or_else(|| ExiError::value("Unsigned Integer", "value does not fit in 64 bits"))
    }
}

#[cfg(test)]
mod conversion_tests {
    use super::*;

    #[test]
    fn parse_and_convert() {
        assert_eq!("-12".parse::<Int>(), Ok(Int::Small(-12)));
        assert_eq!("12".parse::<Uint>(), Ok(Uint::Small(12)));
        assert!("x".parse::<Int>().is_err());
        assert!("1.5e".parse::<Float>().is_err());
        assert_eq!(BigUint::from(7u128), BigUint::from_u64(7));
        assert_eq!(i64::try_from(&Int::Small(3)), Ok(3));
        assert_eq!(u64::try_from(&Uint::Small(3)), Ok(3));
        let d: Decimal = "2.50".parse().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(d, Decimal::from_lexical("2.5").unwrap_or_else(|| panic!()));
        assert_eq!(
            OwnedValue::from(&Value::Boolean(true)),
            OwnedValue::Boolean(true)
        );
    }
}
