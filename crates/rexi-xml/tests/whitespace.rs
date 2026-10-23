//! Whitespace-only text, as EXIficient 1.0.7 encodes it (`checkPendingCharacters`):
//! where the character data is untyped (mixed content) it is dropped between tags
//! (start/start, end/start, end/end) and kept directly between a start tag and its
//! end tag, or when `xml:space="preserve"` is in scope (inherited, and reset by
//! `xml:space="default"`). rexi used to keep it everywhere in mixed content.
//! The schema imports a minimal `xml.xsd` for `xml:space`.

use std::path::Path;

use rexi_core::{ExiOptions, HeaderConfig};

/// (document, EXIficient 1.0.7 `encode-xml --strict` bytes)
const CASES: &[(&str, &str)] = &[
    // " " (s+s) and " " (e+e) dropped, " t " kept
    (r#"<m xmlns="urn:w"> <e/> t <e/> </m>"#, "80340a40e84048"),
    // simple data (s+e) kept
    (r#"<m xmlns="urn:w"> </m>"#, "80606408"),
    // xml:space="preserve" keeps both
    (
        r#"<m xmlns="urn:w" xml:space="preserve"> <e/> </m>"#,
        "80180c80a004",
    ),
    // inherited by the first e; the second e resets it with xml:space="default"
    (
        r#"<m xmlns="urn:w" xml:space="preserve"><e> <e/> </e><e xml:space="default"> <e/> </e></m>"#,
        "8013032028010128",
    ),
    // pretty-printed: the indentation goes, "\n  " inside an otherwise empty e stays
    (
        "<m xmlns=\"urn:w\">\n  <e>\n  </e>\n  <e>x</e>\n</m>",
        "803828510102606f0a",
    ),
];

#[test]
fn whitespace_only_text_as_exificient() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/whitespace");
    let ws = dir.join("ws.xsd");
    let xml = dir.join("xml.xsd");
    let tables = rexi_schema::compile_files(&[ws.as_path(), xml.as_path()])
        .unwrap_or_else(|e| panic!("{e}"));
    let mut failures = Vec::new();
    for (doc, expected) in CASES {
        match rexi_xml::encode_xml(
            doc.as_bytes(),
            &tables,
            &ExiOptions::strict(),
            HeaderConfig::default(),
        ) {
            Ok(bytes) => {
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                if hex != *expected {
                    failures.push(format!("{doc}: {hex}, EXIficient {expected}"));
                }
            }
            Err(e) => failures.push(format!("{doc}: {e}")),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// List items are split only on the four XML space characters. A no-break space
/// or an ideographic space (common in CJK text) is part of an item. rexi used to
/// split on every Unicode space (found by fuzzing).
#[test]
fn list_items_split_on_xml_spaces_only() {
    const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
        targetNamespace="urn:l" xmlns="urn:l" elementFormDefault="qualified">
      <xs:element name="l"><xs:simpleType><xs:list itemType="xs:string"/></xs:simpleType></xs:element>
    </xs:schema>"#;
    // Two items, "a\u{3000}b\u{a0}c" and "d": EXIficient 1.0.7 `encode-xml --strict`.
    const XML: &str = "<l xmlns=\"urn:l\">a\u{3000}b\u{a0}c d</l>";
    let tables = rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"));
    let bytes = rexi_xml::encode_xml(
        XML.as_bytes(),
        &tables,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, "800103b0c030315000b181b200");
}

/// A literal CR LF in an attribute value is one line end, so it becomes one
/// space (XML 1.0 sections 2.11 and 3.3.3). rexi used to turn each of the two
/// characters into a space (found by jexi, the Julia port, which follows the
/// spec). Bytes: EXIficient 1.0.7 `encode-xml --strict`.
#[test]
fn attribute_cr_lf_is_one_space_as_exificient() {
    const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
        targetNamespace="urn:a" xmlns="urn:a" elementFormDefault="qualified">
      <xs:element name="r"><xs:complexType><xs:attribute name="a" type="xs:string"/></xs:complexType></xs:element>
    </xs:schema>"#;
    let tables = rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"));
    for (xml, expected) in [
        ("<r xmlns=\"urn:a\" a=\"x\r\ny\"/>", "80015e081e40"),
        ("<r xmlns=\"urn:a\" a=\"x\ry\"/>", "80015e081e40"),
        ("<r xmlns=\"urn:a\" a=\"x\n\ty\"/>", "80019e08081e40"),
    ] {
        let bytes = rexi_xml::encode_xml(
            xml.as_bytes(),
            &tables,
            &ExiOptions::strict(),
            HeaderConfig::default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, expected, "{xml:?}");
    }
}
