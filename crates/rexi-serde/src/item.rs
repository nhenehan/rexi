//! The in-between tree that a Rust value is serialized into before it is written
//! out against the grammar. Holding a whole element in memory lets the writer send
//! attributes in grammar order, whatever the order of the struct's fields.

use crate::prelude::*;
/// A serialized value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    Scalar(Scalar),
    /// `()` or a unit struct: an element with no content.
    Unit,
    /// `Option::None`: the element or attribute is left out.
    None,
    /// A sequence: repeated elements, or the items of an `xs:list` value.
    Seq(Vec<Item>),
    /// A struct or map: attributes and child elements, by field name, in field
    /// order. `name` is the struct's name (the variant's name for a struct
    /// variant, and empty for a map).
    Struct {
        name: String,
        fields: Vec<(String, Item)>,
    },
    /// A newtype, tuple or struct enum variant: an `xs:choice` branch, written as
    /// an element named after the variant
    /// ([EXI 8.5.4.1.8.2](https://www.w3.org/TR/exi/#choiceGroupTerms)).
    Variant {
        name: String,
        inner: Box<Item>,
    },
}

/// A single value, as serde handed it over. Integer types smaller than 64 bits are
/// widened to 64 bits; 128-bit integers keep their own variants.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Scalar {
    Bool(bool),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    F32(f32),
    F64(f64),
    /// Strings, chars and unit enum variants (enumeration values).
    Str(String),
    Bytes(Vec<u8>),
}

impl Scalar {
    /// The XML Schema lexical form, that is, the value written as text. Bytes
    /// become uppercase hex
    /// ([hexBinary](https://www.w3.org/TR/xmlschema-2/#hexBinary)).
    pub(crate) fn lexical(&self) -> String {
        match self {
            Scalar::Bool(b) => b.to_string(),
            Scalar::I64(v) => v.to_string(),
            Scalar::U64(v) => v.to_string(),
            Scalar::I128(v) => v.to_string(),
            Scalar::U128(v) => v.to_string(),
            Scalar::F32(v) => float_lexical(f64::from(*v), v.to_string()),
            Scalar::F64(v) => float_lexical(*v, v.to_string()),
            Scalar::Str(s) => s.clone(),
            Scalar::Bytes(b) => b.iter().map(|x| format!("{x:02X}")).collect(),
        }
    }
}

/// Rust's `Display` form for a float, which is the shortest text that reads back
/// as the same value. NaN and the infinities use the XML Schema spellings `NaN`,
/// `INF` and `-INF` ([XML Schema double](https://www.w3.org/TR/xmlschema-2/#double)).
fn float_lexical(v: f64, display: String) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v == f64::INFINITY {
        "INF".into()
    } else if v == f64::NEG_INFINITY {
        "-INF".into()
    } else {
        display
    }
}
