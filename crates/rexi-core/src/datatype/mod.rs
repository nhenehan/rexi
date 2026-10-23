//! EXI datatype representations
//! ([EXI 7](https://www.w3.org/TR/exi/#encodingValues),
//! [EXI 7.1](https://www.w3.org/TR/exi/#encodingDatatypes)).
//!
//! A datatype representation is the bit layout EXI uses for one kind of typed value.
//! [`DatatypeKind`] says which layout a value uses. Three kinds are not written in
//! this module:
//!
//! - strings go through the value string table (the table of strings already seen,
//!   so a repeat can be sent as a short number);
//! - QNames (qualified names: a namespace URI plus a local name) go through the URI
//!   and local-name partitions of the string table;
//! - lists are walked by the encoder, which sends each item to the right encoder.
//!
//! Everything else is a single value (a scalar) handled by [`write_scalar`] /
//! [`read_scalar`].

pub(crate) mod biguint;
pub(crate) mod binary;
pub(crate) mod boolean;
pub(crate) mod datetime;
pub(crate) mod decimal;
pub(crate) mod float;
pub(crate) mod integer;
pub(crate) mod lexical;
pub(crate) mod string;
pub(crate) mod unsigned;

use crate::io::{ByteSink, ByteSource};

use crate::error::ExiResult;
use crate::ids::{EnumSetIdx, TypeIdx};
use crate::io::{BitReader, BitWriter};
use crate::value::{DateTimeKind, OwnedValue, Value};

/// XSD `whiteSpace` facet, applied when parsing lexical values (the text form a value
/// has in XML). See [XML Schema 1.0 Part 2,
/// whiteSpace](https://www.w3.org/TR/xmlschema-2/#rf-whiteSpace).
///
/// - `Preserve`: keep the text as it is.
/// - `Replace`: turn each tab, line feed and carriage return into a space.
/// - `Collapse`: do `Replace`, then join runs of spaces into one space and cut spaces
///   from both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhiteSpace {
    /// Keep the text as it is.
    Preserve,
    /// Turn tabs, line feeds and carriage returns into spaces.
    Replace,
    /// Do `Replace`, then join runs of spaces and cut spaces from both ends.
    Collapse,
}

/// How a value is represented in the stream
/// ([EXI 7.1](https://www.w3.org/TR/exi/#encodingDatatypes) and its Table 7-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DatatypeKind {
    /// String, encoded through the value string table.
    String {
        /// How whitespace in the text is handled before encoding.
        whitespace: WhiteSpace,
    },
    /// A string type whose `xsd:pattern` facets allow only a small set of
    /// characters: fewer than 256, all in the Basic Multilingual Plane (so each fits
    /// in one 16-bit unit). EXI calls this a restricted character set
    /// ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)).
    ///
    /// For such a type, EXI writes each character as a small index into the set
    /// instead of a full code point, so the bytes differ from a plain string. This
    /// crate does not implement that form yet. Writing the value as a plain string
    /// would give bytes that EXIficient and other readers that follow the spec turn
    /// into garbage, with no error. So the encoder and the decoder refuse a value of
    /// this kind with
    /// [`Feature::RestrictedCharacterSet`](crate::error::Feature::RestrictedCharacterSet).
    ///
    /// Only the value is refused. Grammars and event codes do not depend on the
    /// character set, so every document that does not hold a value of this type
    /// still works.
    RestrictedString {
        /// How whitespace in the text is handled before encoding.
        whitespace: WhiteSpace,
    },
    /// One bit ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)).
    Boolean,
    /// Two bits, used when the boolean type has pattern facets
    /// ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)).
    BooleanPattern,
    /// Signed integer: sign bit then magnitude (the value without its sign)
    /// ([EXI 7.1.5](https://www.w3.org/TR/exi/#encodingInteger)).
    Integer,
    /// Unsigned integer varint
    /// ([EXI 7.1.6](https://www.w3.org/TR/exi/#encodingUnsignedInteger)), for types
    /// whose lower bound is >= 0.
    UnsignedInteger,
    /// Bounded integer with at most 4096 possible values, sent as a `bits`-wide offset
    /// from `min` ([EXI 7.1.5](https://www.w3.org/TR/exi/#encodingInteger),
    /// [EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)).
    NBit {
        /// Smallest allowed value: the offset 0 stands for it.
        min: i64,
        /// Width of the offset in bits.
        bits: u8,
    },
    /// Decimal ([EXI 7.1.3](https://www.w3.org/TR/exi/#encodingDecimal)).
    Decimal,
    /// Float and double ([EXI 7.1.4](https://www.w3.org/TR/exi/#encodingFloat)).
    Float,
    /// Date and time types
    /// ([EXI 7.1.8](https://www.w3.org/TR/exi/#encodingDateTime)); the kind says which
    /// fields are present.
    DateTime(DateTimeKind),
    /// `xsd:base64Binary` ([EXI 7.1.1](https://www.w3.org/TR/exi/#encodingBinary)).
    Base64Binary,
    /// `xsd:hexBinary` ([EXI 7.1.1](https://www.w3.org/TR/exi/#encodingBinary)).
    HexBinary,
    /// QName ([EXI 7.1.7](https://www.w3.org/TR/exi/#encodingQName)). Only used for
    /// `xsi:type` values (the attribute that names a value's schema type); the codec
    /// writes these itself.
    QName,
    /// Enumeration ([EXI 7.2](https://www.w3.org/TR/exi/#encodingEnumerations)): the
    /// item's position in the list of allowed values, `ceil(log2(count))` bits wide.
    /// `item` is the base type. It is used to put text values into their one standard
    /// form before they are matched against the allowed values.
    Enumeration {
        /// The list of allowed values.
        set: EnumSetIdx,
        /// The base type of the enumeration.
        item: TypeIdx,
    },
    /// List ([EXI 7.1.11](https://www.w3.org/TR/exi/#encodingList)): item count, then
    /// each item encoded as `item`.
    List {
        /// The type of each item.
        item: TypeIdx,
    },
    /// Untyped character content (mixed content); a string.
    Untyped,
}

impl DatatypeKind {
    /// Whether values of this kind travel through the value string table.
    pub fn is_string_like(&self) -> bool {
        matches!(self, DatatypeKind::String { .. } | DatatypeKind::Untyped)
    }

    /// Short name for error messages.
    pub fn name(&self) -> &'static str {
        match self {
            DatatypeKind::String { .. } | DatatypeKind::RestrictedString { .. } => "String",
            DatatypeKind::Boolean | DatatypeKind::BooleanPattern => "Boolean",
            DatatypeKind::Integer => "Integer",
            DatatypeKind::UnsignedInteger => "Unsigned Integer",
            DatatypeKind::NBit { .. } => "n-bit Unsigned Integer",
            DatatypeKind::Decimal => "Decimal",
            DatatypeKind::Float => "Float",
            DatatypeKind::DateTime(_) => "Date-Time",
            DatatypeKind::Base64Binary | DatatypeKind::HexBinary => "Binary",
            DatatypeKind::QName => "QName",
            DatatypeKind::Enumeration { .. } => "Enumeration",
            DatatypeKind::List { .. } => "List",
            DatatypeKind::Untyped => "untyped",
        }
    }
}

/// Resolves type and enumeration indices, implemented by grammar tables.
pub trait TypeResolver {
    /// The datatype of type `t`.
    fn datatype(&self, t: TypeIdx) -> DatatypeKind;
    /// Number of values in enumeration `set`.
    fn enum_count(&self, set: EnumSetIdx) -> usize;
    /// The text of value `ordinal` of enumeration `set`, or `None` if out of range.
    fn enum_value(&self, set: EnumSetIdx, ordinal: usize) -> Option<&str>;
}

/// Encodes a scalar value of `kind`. `value` may be [`Value::Lexical`], in which case
/// it is parsed first. Returns an error for string-like, QName and list kinds.
pub fn write_scalar<W: ByteSink>(
    w: &mut BitWriter<W>,
    kind: DatatypeKind,
    value: &Value<'_>,
    types: &dyn TypeResolver,
) -> ExiResult<()> {
    lexical::write_scalar(w, kind, value, types)
}

/// Decodes a scalar value of `kind`. Returns an error for string-like, QName and list kinds.
pub fn read_scalar<R: ByteSource>(
    r: &mut BitReader<R>,
    kind: DatatypeKind,
    types: &dyn TypeResolver,
) -> ExiResult<OwnedValue> {
    lexical::read_scalar(r, kind, types)
}
