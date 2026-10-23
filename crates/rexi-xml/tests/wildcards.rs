//! Wildcards (`xs:any`, `xs:anyAttribute`) are refused as their own feature,
//! `Feature::Wildcard`. They are strict, schema-declared productions, so they
//! must not be reported as non-strict or as built-in grammars.

use rexi_core::{ExiError, ExiOptions, Feature, HeaderConfig};
use rexi_xml::{decode_events, encode_xml};

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:attribute name="g" type="xs:int"/>
  <xs:element name="root"><xs:complexType>
    <xs:sequence><xs:any processContents="strict" minOccurs="0"/></xs:sequence>
    <xs:anyAttribute processContents="strict"/>
  </xs:complexType></xs:element>
  <xs:element name="leaf" type="xs:int"/>
</xs:schema>"#;

fn feature_of(err: rexi_xml::XmlError) -> Option<Feature> {
    match err {
        rexi_xml::XmlError::Exi(ExiError::Unsupported(f)) => Some(f),
        _ => None,
    }
}

#[test]
fn encoding_a_wildcard_is_refused_as_a_wildcard() {
    let tables = rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"));
    for xml in [r#"<root g="5"/>"#, "<root><leaf>3</leaf></root>"] {
        let err = encode_xml(
            xml.as_bytes(),
            &tables,
            &ExiOptions::strict(),
            HeaderConfig::default(),
        )
        .err()
        .unwrap_or_else(|| panic!("{xml} encoded"));
        assert_eq!(feature_of(err), Some(Feature::Wildcard), "{xml}");
    }
    // A document that uses no wildcard still works.
    let ok = encode_xml(
        b"<root/>",
        &tables,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    );
    assert!(ok.is_ok());
}

#[test]
fn decoding_a_wildcard_is_refused_as_a_wildcard() {
    // After the header (0x80) and the root's start, try every choice of the next
    // event codes. Some choice is a wildcard; none may be reported as a
    // non-strict or built-in grammar.
    let tables = rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"));
    let mut wildcards = 0;
    for byte in 0..=255u8 {
        let stream = [0x80, byte, 0x00];
        if let Err(err) = decode_events(&stream[..], &tables, Some(ExiOptions::strict())) {
            match feature_of(err) {
                Some(Feature::Wildcard) => wildcards += 1,
                Some(f @ (Feature::NonStrictSchemaInformed | Feature::BuiltInElementGrammar)) => {
                    // The root itself may be an undeclared element (SE(*) at the
                    // document level): that one really is a built-in grammar.
                    assert_eq!(f, Feature::BuiltInElementGrammar, "byte {byte:#04x}");
                    assert!(byte & 0x80 != 0, "byte {byte:#04x}: {f}");
                }
                _ => {}
            }
        }
    }
    assert!(wildcards > 0);
}
