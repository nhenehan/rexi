//! Initial string table entries
//! ([EXI Appendix D](https://www.w3.org/TR/exi/#initialStringValues)): the URIs, prefixes
//! and local names every string table starts with, before any of the stream is read.

use super::StringTable;
use crate::grammar::GrammarSet;

/// Compact identifier of the empty URI (Table D-1,
/// [EXI D.1](https://www.w3.org/TR/exi/#initialUriValues)).
pub const EMPTY_URI_ID: u32 = 0;
/// Compact identifier of the XML namespace (Table D-1).
pub const XML_NS_URI_ID: u32 = 1;
/// Compact identifier of the XML Schema instance namespace (Table D-1).
pub const XSI_NS_URI_ID: u32 = 2;
/// Compact identifier of the XML Schema namespace, present only when schemas are
/// used (Table D-2).
pub const XSD_NS_URI_ID: u32 = 3;

/// The XML namespace URI, bound to the `xml` prefix.
pub const XML_NS_URI: &str = "http://www.w3.org/XML/1998/namespace";
/// The XML Schema instance namespace URI (`xsi:type`, `xsi:nil`).
pub const XSI_NS_URI: &str = "http://www.w3.org/2001/XMLSchema-instance";
/// The XML Schema namespace URI.
pub const XSD_NS_URI: &str = "http://www.w3.org/2001/XMLSchema";

/// Table D-4 ([EXI D.3](https://www.w3.org/TR/exi/#initialLocalNames)): local-names in
/// the XML namespace, in compact-identifier order.
pub const XML_LOCAL_NAMES: [&str; 4] = ["base", "id", "lang", "space"];
/// Table D-4: local-names in the XML Schema instance namespace.
pub const XSI_LOCAL_NAMES: [&str; 2] = ["nil", "type"];
/// Table D-5: local-names in the XML Schema namespace for schema-informed streams.
pub const XSD_LOCAL_NAMES: [&str; 46] = [
    "ENTITIES",
    "ENTITY",
    "ID",
    "IDREF",
    "IDREFS",
    "NCName",
    "NMTOKEN",
    "NMTOKENS",
    "NOTATION",
    "Name",
    "QName",
    "anySimpleType",
    "anyType",
    "anyURI",
    "base64Binary",
    "boolean",
    "byte",
    "date",
    "dateTime",
    "decimal",
    "double",
    "duration",
    "float",
    "gDay",
    "gMonth",
    "gMonthDay",
    "gYear",
    "gYearMonth",
    "hexBinary",
    "int",
    "integer",
    "language",
    "long",
    "negativeInteger",
    "nonNegativeInteger",
    "nonPositiveInteger",
    "normalizedString",
    "positiveInteger",
    "short",
    "string",
    "time",
    "token",
    "unsignedByte",
    "unsignedInt",
    "unsignedLong",
    "unsignedShort",
];

impl StringTable {
    /// Adds the Appendix D entries to an empty table: the three (schema-less) or four
    /// (schema-informed) URIs, their prefixes
    /// ([EXI D.2](https://www.w3.org/TR/exi/#initialPrefixValues)) and their local-names.
    /// The XML Schema URI gets no prefix.
    ///
    /// Schema-declared namespaces and names are the caller's job
    /// ([EXI 7.3.1](https://www.w3.org/TR/exi/#stringTablePartitions)): append the target
    /// namespaces in dictionary order with [`StringTable::add_uri`], the declared names
    /// with [`StringTable::add_local_name`], then call [`StringTable::sort_local_names`].
    pub fn add_initial_entries(&mut self, schema_informed: bool) {
        const NO_TABLES: &GrammarSet<'static> = &GrammarSet::EMPTY;
        let empty = self.add_uri(NO_TABLES, "");
        self.add_prefix(NO_TABLES, empty, "");
        let xml = self.add_uri(NO_TABLES, XML_NS_URI);
        self.add_prefix(NO_TABLES, xml, "xml");
        for name in XML_LOCAL_NAMES {
            self.add_local_name(NO_TABLES, xml, name);
        }
        let xsi = self.add_uri(NO_TABLES, XSI_NS_URI);
        self.add_prefix(NO_TABLES, xsi, "xsi");
        for name in XSI_LOCAL_NAMES {
            self.add_local_name(NO_TABLES, xsi, name);
        }
        if schema_informed {
            let xsd = self.add_uri(NO_TABLES, XSD_NS_URI);
            for name in XSD_LOCAL_NAMES {
                self.add_local_name(NO_TABLES, xsd, name);
            }
        }
    }

    /// A table holding exactly the Appendix D entries.
    pub fn with_initial_entries(schema_informed: bool) -> Self {
        let mut table = Self::new();
        table.add_initial_entries(schema_informed);
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::GrammarSet;

    const NO: &GrammarSet<'static> = &GrammarSet::EMPTY;
    use crate::prelude::*;

    #[test]
    fn schema_less_matches_exificient_initial_entries_test() {
        // InitialEntriesStringTablePartitionsTestCase.testSchemaLess
        let t = StringTable::with_initial_entries(false);
        assert_eq!(t.uri_count(), 3);
        assert_eq!(t.uri(NO, 0), Some(""));
        assert_eq!(t.uri(NO, 1), Some(XML_NS_URI));
        assert_eq!(t.uri(NO, 2), Some(XSI_NS_URI));
        assert_eq!(t.prefix_count(0), 1);
        assert_eq!(t.prefix(NO, 0, 0), Some(""));
        assert_eq!(t.prefix_count(1), 1);
        assert_eq!(t.prefix(NO, 1, 0), Some("xml"));
        assert_eq!(t.prefix_count(2), 1);
        assert_eq!(t.prefix(NO, 2, 0), Some("xsi"));
        assert_eq!(t.local_name_count(0), 0);
        assert_eq!(t.local_name_count(1), 4);
        assert_eq!(t.local_name(NO, 1, 0), Some("base"));
        assert_eq!(t.local_name(NO, 1, 1), Some("id"));
        assert_eq!(t.local_name(NO, 1, 2), Some("lang"));
        assert_eq!(t.local_name(NO, 1, 3), Some("space"));
        assert_eq!(t.local_name_count(2), 2);
        assert_eq!(t.local_name(NO, 2, 0), Some("nil"));
        assert_eq!(t.local_name(NO, 2, 1), Some("type"));
        assert_eq!(t.uri(NO, 3), None);
    }

    #[test]
    fn schema_informed_matches_exificient_xsd_types_only_test() {
        // InitialEntriesStringTablePartitionsTestCase.testXsdTypesOnly
        let t = StringTable::with_initial_entries(true);
        assert_eq!(t.uri_count(), 4);
        assert_eq!(t.uri(NO, 3), Some(XSD_NS_URI));
        assert_eq!(t.prefix_count(3), 0);
        assert_eq!(t.local_name_count(3), 46);
        assert_eq!(t.local_name(NO, 3, 0), Some("ENTITIES"));
        assert_eq!(t.local_name(NO, 3, 45), Some("unsignedShort"));
        assert_eq!(t.find_local_name(NO, 3, "dateTime"), Some(18));
        assert_eq!(
            t.qname_id(NO, XSI_NS_URI, "type"),
            Some(crate::ids::QNameId::new(2, 1))
        );
    }

    #[test]
    fn appendix_d_local_names_are_already_sorted() {
        // Spec D.3: each partition is sorted in dictionary order. The tables are
        // listed in that order, so sorting must change nothing.
        let mut t = StringTable::with_initial_entries(true);
        let before: Vec<Option<String>> = (0..46)
            .map(|i| t.local_name(NO, 3, i).map(str::to_owned))
            .collect();
        t.sort_local_names();
        let after: Vec<Option<String>> = (0..46)
            .map(|i| t.local_name(NO, 3, i).map(str::to_owned))
            .collect();
        assert_eq!(before, after);
        assert!(XML_LOCAL_NAMES.windows(2).all(|w| w[0] < w[1]));
        assert!(XSI_LOCAL_NAMES.windows(2).all(|w| w[0] < w[1]));
    }
}
