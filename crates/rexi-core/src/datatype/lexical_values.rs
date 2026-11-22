//! `Preserve.lexicalValues` ([EXI 6.3](https://www.w3.org/TR/exi/#fidelityOptions)):
//! every value is sent as its text, so the decoder gives back exactly what the
//! encoder was given (`"1.50"` stays `"1.50"`).
//!
//! The text is a String with the restricted character set of the value's
//! built-in EXI type ([EXI 7](https://www.w3.org/TR/exi/#encodingDatatypes),
//! Table 7-2), whatever pattern facets the schema type has. A list uses the set
//! of its item type, and an enumeration the set of its base type. Strings,
//! unions and QNames have no set. The text still goes through the value string
//! table, so a repeated value is a short number.

use super::{DatatypeKind, TypeResolver};
use crate::ids::{LocalId, QNameId, UriId};

// Each set in code point order, the order EXI numbers its characters in
// (https://www.w3.org/TR/exi/#restrictedCharSet).
const BASE64: &str = "\t\n\r +/0123456789=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const HEX: &str = "\t\n\r 0123456789ABCDEFabcdef";
const BOOLEAN: &str = "\t\n\r 01aeflrstu";
const DATE_TIME: &str = "\t\n\r +-.0123456789:TZ";
const DECIMAL: &str = "\t\n\r +-.0123456789";
const DOUBLE: &str = "\t\n\r +-.0123456789EFINae";
const INTEGER: &str = "\t\n\r +-0123456789";

/// The restricted character set for a value of `kind` (Table 7-2), or `None`
/// for a plain String.
pub(crate) fn alphabet(kind: DatatypeKind, types: &dyn TypeResolver) -> Option<&'static str> {
    match kind {
        DatatypeKind::Base64Binary => Some(BASE64),
        DatatypeKind::HexBinary => Some(HEX),
        DatatypeKind::Boolean | DatatypeKind::BooleanPattern => Some(BOOLEAN),
        DatatypeKind::DateTime(_) => Some(DATE_TIME),
        DatatypeKind::Decimal => Some(DECIMAL),
        DatatypeKind::Float => Some(DOUBLE),
        DatatypeKind::Integer | DatatypeKind::UnsignedInteger | DatatypeKind::NBit { .. } => {
            Some(INTEGER)
        }
        // A list of an enumeration takes the enumeration's base type's set, as
        // EXIficient does (each datatype reports its item's or base's ID).
        // Schema types cannot nest without end, so this stops.
        DatatypeKind::Enumeration { item, .. } | DatatypeKind::List { item } => {
            alphabet(types.datatype(item), types)
        }
        DatatypeKind::String { .. }
        | DatatypeKind::RestrictedString { .. }
        | DatatypeKind::QName
        | DatatypeKind::Untyped => None,
    }
}

/// The set for `xsi:nil` values: the Boolean one.
pub(crate) const NIL_SET: &str = BOOLEAN;

/// `xsi:type` and `xsi:nil` in the initial string table
/// ([EXI Appendix D.3](https://www.w3.org/TR/exi/#initialUriValues)): the names
/// their values are kept under in the value table.
pub(crate) const XSI_TYPE: QNameId = QNameId::new(UriId(2), LocalId(1));
/// See [`XSI_TYPE`].
pub(crate) const XSI_NIL: QNameId = QNameId::new(UriId(2), LocalId(0));

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    #[test]
    fn xsi_names_match_the_initial_table() {
        use crate::grammar::GrammarSet;
        use crate::string_table::StringTable;
        let no: &GrammarSet<'static> = &GrammarSet::EMPTY;
        let xsi = "http://www.w3.org/2001/XMLSchema-instance";
        let t = StringTable::with_initial_entries(true);
        assert_eq!(t.qname_id(no, xsi, "type"), Some(XSI_TYPE));
        assert_eq!(t.qname_id(no, xsi, "nil"), Some(XSI_NIL));
    }

    #[test]
    fn sets_are_in_code_point_order_with_no_repeats() {
        for set in [BASE64, HEX, BOOLEAN, DATE_TIME, DECIMAL, DOUBLE, INTEGER] {
            let chars: Vec<char> = set.chars().collect();
            assert!(chars.windows(2).all(|w| w[0] < w[1]), "{set:?}");
        }
        // Sizes from Table 7-2: 4 whitespace characters plus the rest.
        assert_eq!(BASE64.len(), 4 + 2 + 10 + 1 + 26 + 26);
        assert_eq!(HEX.len(), 4 + 10 + 6 + 6);
        assert_eq!(BOOLEAN.len(), 4 + 10);
        assert_eq!(DATE_TIME.len(), 4 + 3 + 10 + 3);
        assert_eq!(DECIMAL.len(), 4 + 3 + 10);
        assert_eq!(DOUBLE.len(), 4 + 3 + 10 + 6);
        assert_eq!(INTEGER.len(), 4 + 2 + 10);
    }
}
