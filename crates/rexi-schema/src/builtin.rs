//! The 46 XML Schema built-in types
//! ([EXI Appendix D.3](https://www.w3.org/TR/exi/#initialLocalNames), Table D-5),
//! each with its base type and the EXI datatype it maps to
//! ([EXI 7.1](https://www.w3.org/TR/exi/#encodingDatatypes), Table 7-1).
//!
//! The table is in the order of [`well_known::XSD_LOCAL_NAMES`]. So the index of a
//! built-in is both its local-name id in the XSD partition of the string table and
//! its `TypeIdx`.

use rexi_core::grammar::well_known::XSD_LOCAL_NAMES;
use rexi_core::{DateTimeKind, WhiteSpace};

/// What a built-in type is, before facets are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// `xs:anyType`, the complex type at the root of all types (XML Schema calls it
    /// the "ur-type").
    AnyType,
    /// `xs:string` and the types derived from it, with their white-space rule.
    String(WhiteSpace),
    /// `xs:boolean`.
    Boolean,
    /// `xs:decimal`.
    Decimal,
    /// `xs:float` and `xs:double` (both are EXI Floats).
    Float,
    /// The date and time types (`xs:dateTime`, `xs:date`, `xs:gYear`, ...).
    DateTime(DateTimeKind),
    /// `xs:base64Binary`.
    Base64Binary,
    /// `xs:hexBinary`.
    HexBinary,
    /// `xs:integer` and every type derived from it. The bounds decide how values
    /// are encoded.
    Integer {
        /// The smallest allowed value; `None` means no lower bound.
        min: Option<i128>,
        /// The largest allowed value; `None` means no upper bound.
        max: Option<i128>,
    },
    /// A list type. The item is the index of another built-in.
    List(usize),
}

/// One row of the built-in table: a name, a base type and a category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Builtin {
    /// The local name in the XML Schema namespace, such as `"int"`.
    pub name: &'static str,
    /// Index of the base type in this table; `None` for `xs:anyType`.
    pub base: Option<usize>,
    /// What the type is, which decides its EXI datatype.
    pub category: Category,
}

/// Number of built-in types (rows in the table).
pub const COUNT: usize = 46;

/// Index of `xs:anyType`.
pub const ANY_TYPE: usize = 12;
/// Index of `xs:anySimpleType`.
pub const ANY_SIMPLE_TYPE: usize = 11;
/// Index of `xs:string`.
pub const STRING: usize = 39;
/// Index of `xs:integer`.
#[cfg(test)]
pub const INTEGER: usize = 30;
/// Index of `xs:QName`.
pub const QNAME: usize = 10;
/// Index of `xs:NOTATION`.
pub const NOTATION: usize = 8;

/// Index of a built-in by local name, worked out at compile time. Panics (so the
/// build fails) for an unknown name.
const fn idx(name: &str) -> usize {
    let mut i = 0;
    while i < XSD_LOCAL_NAMES.len() {
        if str_eq(XSD_LOCAL_NAMES[i], name) {
            return i;
        }
        i += 1;
    }
    panic!("unknown built-in")
}

/// String equality that can run at compile time (`==` on `str` cannot, in a
/// `const fn`).
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const I8: (Option<i128>, Option<i128>) = (Some(-128), Some(127));
const I16: (Option<i128>, Option<i128>) = (Some(-32768), Some(32767));
const I32: (Option<i128>, Option<i128>) = (Some(-2_147_483_648), Some(2_147_483_647));
const I64: (Option<i128>, Option<i128>) = (Some(i64::MIN as i128), Some(i64::MAX as i128));
const U8: (Option<i128>, Option<i128>) = (Some(0), Some(255));
const U16: (Option<i128>, Option<i128>) = (Some(0), Some(65535));
const U32: (Option<i128>, Option<i128>) = (Some(0), Some(4_294_967_295));
const U64: (Option<i128>, Option<i128>) = (Some(0), Some(u64::MAX as i128));
const UNBOUNDED: (Option<i128>, Option<i128>) = (None, None);

const fn int(bounds: (Option<i128>, Option<i128>)) -> Category {
    Category::Integer {
        min: bounds.0,
        max: bounds.1,
    }
}

// `b!(name, base, category)`: one table row whose base is given by name.
macro_rules! b {
    ($name:literal, $base:literal, $cat:expr) => {
        Builtin {
            name: $name,
            base: Some(idx($base)),
            category: $cat,
        }
    };
}

/// The built-in table, indexed like [`XSD_LOCAL_NAMES`].
pub static BUILTINS: [Builtin; COUNT] = [
    b!("ENTITIES", "anySimpleType", Category::List(idx("ENTITY"))),
    b!("ENTITY", "NCName", Category::String(WhiteSpace::Collapse)),
    b!("ID", "NCName", Category::String(WhiteSpace::Collapse)),
    b!("IDREF", "NCName", Category::String(WhiteSpace::Collapse)),
    b!("IDREFS", "anySimpleType", Category::List(idx("IDREF"))),
    b!("NCName", "Name", Category::String(WhiteSpace::Collapse)),
    b!("NMTOKEN", "token", Category::String(WhiteSpace::Collapse)),
    b!("NMTOKENS", "anySimpleType", Category::List(idx("NMTOKEN"))),
    b!(
        "NOTATION",
        "anySimpleType",
        Category::String(WhiteSpace::Collapse)
    ),
    b!("Name", "token", Category::String(WhiteSpace::Collapse)),
    b!(
        "QName",
        "anySimpleType",
        Category::String(WhiteSpace::Collapse)
    ),
    b!(
        "anySimpleType",
        "anyType",
        Category::String(WhiteSpace::Preserve)
    ),
    Builtin {
        name: "anyType",
        base: None,
        category: Category::AnyType,
    },
    b!(
        "anyURI",
        "anySimpleType",
        Category::String(WhiteSpace::Collapse)
    ),
    b!("base64Binary", "anySimpleType", Category::Base64Binary),
    b!("boolean", "anySimpleType", Category::Boolean),
    b!("byte", "short", int(I8)),
    b!(
        "date",
        "anySimpleType",
        Category::DateTime(DateTimeKind::Date)
    ),
    b!(
        "dateTime",
        "anySimpleType",
        Category::DateTime(DateTimeKind::DateTime)
    ),
    b!("decimal", "anySimpleType", Category::Decimal),
    b!("double", "anySimpleType", Category::Float),
    b!(
        "duration",
        "anySimpleType",
        Category::String(WhiteSpace::Collapse)
    ),
    b!("float", "anySimpleType", Category::Float),
    b!(
        "gDay",
        "anySimpleType",
        Category::DateTime(DateTimeKind::GDay)
    ),
    b!(
        "gMonth",
        "anySimpleType",
        Category::DateTime(DateTimeKind::GMonth)
    ),
    b!(
        "gMonthDay",
        "anySimpleType",
        Category::DateTime(DateTimeKind::GMonthDay)
    ),
    b!(
        "gYear",
        "anySimpleType",
        Category::DateTime(DateTimeKind::GYear)
    ),
    b!(
        "gYearMonth",
        "anySimpleType",
        Category::DateTime(DateTimeKind::GYearMonth)
    ),
    b!("hexBinary", "anySimpleType", Category::HexBinary),
    b!("int", "long", int(I32)),
    b!("integer", "decimal", int(UNBOUNDED)),
    b!("language", "token", Category::String(WhiteSpace::Collapse)),
    b!("long", "integer", int(I64)),
    b!(
        "negativeInteger",
        "nonPositiveInteger",
        int((None, Some(-1)))
    ),
    b!("nonNegativeInteger", "integer", int((Some(0), None))),
    b!("nonPositiveInteger", "integer", int((None, Some(0)))),
    b!(
        "normalizedString",
        "string",
        Category::String(WhiteSpace::Replace)
    ),
    b!(
        "positiveInteger",
        "nonNegativeInteger",
        int((Some(1), None))
    ),
    b!("short", "int", int(I16)),
    b!(
        "string",
        "anySimpleType",
        Category::String(WhiteSpace::Preserve)
    ),
    b!(
        "time",
        "anySimpleType",
        Category::DateTime(DateTimeKind::Time)
    ),
    b!(
        "token",
        "normalizedString",
        Category::String(WhiteSpace::Collapse)
    ),
    b!("unsignedByte", "unsignedShort", int(U8)),
    b!("unsignedInt", "unsignedLong", int(U32)),
    b!("unsignedLong", "nonNegativeInteger", int(U64)),
    b!("unsignedShort", "unsignedInt", int(U16)),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_well_known_order() {
        for (i, b) in BUILTINS.iter().enumerate() {
            assert_eq!(b.name, XSD_LOCAL_NAMES[i]);
        }
        assert_eq!(BUILTINS[ANY_TYPE].name, "anyType");
        assert_eq!(BUILTINS[ANY_SIMPLE_TYPE].name, "anySimpleType");
        assert_eq!(BUILTINS[STRING].name, "string");
        assert_eq!(BUILTINS[INTEGER].name, "integer");
        assert_eq!(BUILTINS[QNAME].name, "QName");
        assert_eq!(BUILTINS[NOTATION].name, "NOTATION");
    }

    #[test]
    fn chains_end_at_any_type() {
        for i in 0..COUNT {
            let mut cur = Some(i);
            let mut steps = 0;
            while let Some(c) = cur {
                cur = BUILTINS[c].base;
                steps += 1;
                assert!(steps < 20);
            }
        }
    }
}
