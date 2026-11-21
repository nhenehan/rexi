//! Wildcards (`xs:any`, `xs:anyAttribute`): `SE(*)`, `SE(uri:*)`, `AT(*)` and
//! `AT(uri:*)` in strict mode ([EXI 8.5.4.1.6](https://www.w3.org/TR/exi/#elementTerms)).
//!
//! Without the `wildcard` feature they are refused as their own feature,
//! `Feature::Wildcard`. They are strict, schema-declared productions, so they
//! must not be reported as non-strict or as built-in grammars.
//!
//! With the feature they are encoded and decoded. The bytes are checked against
//! EXIficient by the W3C suite's wildcard cases (`scripts/w3c-strict.py`); these
//! tests pin what those cases do not: round trips, the value's type, the
//! namespace of `SE(uri:*)`, and the refusals.

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

#[cfg(not(feature = "wildcard"))]
#[test]
fn encoding_a_wildcard_is_refused_as_a_wildcard() {
    let tables = rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"));
    for xml in [r#"<root g="5"/>"#, "<root><leaf>3</leaf></root>"] {
        let err = encode_xml(
            xml.as_bytes(),
            &tables,
            ExiOptions::strict(),
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
        ExiOptions::strict(),
        HeaderConfig::default(),
    );
    assert!(ok.is_ok());
}

#[cfg(not(feature = "wildcard"))]
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

#[cfg(feature = "wildcard")]
mod with_the_feature {
    use super::*;
    use rexi_core::Alignment;
    use rexi_xml::parse_events;

    /// `root` takes any attribute and any element (`AT(*)`, `SE(*)`); `box` takes
    /// attributes and elements of `urn:w` only (`AT(urn:w:*)`, `SE(urn:w:*)`).
    /// `g` and `w:n` are global attributes with a type, `leaf` and `w:item` global
    /// elements.
    const NS: &str = r###"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:w="urn:w" targetNamespace="urn:w" elementFormDefault="qualified">
      <xs:attribute name="n" type="xs:int"/>
      <xs:element name="item" type="xs:boolean"/>
      <xs:element name="box"><xs:complexType>
        <xs:sequence><xs:any namespace="##targetNamespace" maxOccurs="unbounded"/></xs:sequence>
        <xs:anyAttribute namespace="##targetNamespace"/>
      </xs:complexType></xs:element>
    </xs:schema>"###;

    fn compile(xsd: &str) -> rexi_core::GrammarSetBuf {
        rexi_schema::compile_str(xsd, None).unwrap_or_else(|e| panic!("{e}"))
    }

    fn encode(
        tables: &rexi_core::GrammarSetBuf,
        xml: &str,
        options: ExiOptions,
    ) -> Result<Vec<u8>, rexi_xml::XmlError> {
        encode_xml(xml.as_bytes(), tables, options, HeaderConfig::default())
    }

    /// Encodes, decodes, and checks the events come back, both alignments.
    fn round_trip(tables: &rexi_core::GrammarSetBuf, xml: &str, want: &str) {
        for options in [
            ExiOptions::strict(),
            ExiOptions::strict().with_alignment(Alignment::ByteAligned),
        ] {
            let exi = encode(tables, xml, options.clone()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
            let back =
                decode_events(&exi[..], tables, Some(options)).unwrap_or_else(|e| panic!("{e}"));
            let want = parse_events(want.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, want, "{xml}");
        }
    }

    #[test]
    fn any_element_and_any_attribute_round_trip() {
        let tables = compile(XSD);
        round_trip(&tables, "<root/>", "<root/>");
        round_trip(&tables, r#"<root g="5"/>"#, r#"<root g="5"/>"#);
        round_trip(
            &tables,
            "<root><leaf>3</leaf></root>",
            "<root><leaf>3</leaf></root>",
        );
        // Names the schema does not know at all: the attribute is a string, and
        // the name goes into the string table.
        round_trip(
            &tables,
            r#"<root zz="hello" g="1"/>"#,
            r#"<root g="1" zz="hello"/>"#,
        );
    }

    #[test]
    fn a_global_attribute_keeps_its_type() {
        let tables = compile(XSD);
        // `g` is a global xs:int: "+05" comes back as the integer 5. `h` has no
        // declaration: it is a string, so its text comes back as it was.
        round_trip(
            &tables,
            r#"<root g="+05" h="+05"/>"#,
            r#"<root g="5" h="+05"/>"#,
        );
        let err = encode(&tables, r#"<root g="five"/>"#, ExiOptions::strict());
        assert!(
            matches!(err, Err(rexi_xml::XmlError::Exi(ExiError::Value { .. }))),
            "{err:?}"
        );
    }

    #[test]
    fn an_element_with_no_global_declaration_needs_a_built_in_grammar() {
        let tables = compile(XSD);
        let err = encode(&tables, "<root><other/></root>", ExiOptions::strict())
            .err()
            .unwrap_or_else(|| panic!("encoded"));
        assert_eq!(feature_of(err), Some(Feature::BuiltInElementGrammar));
    }

    #[test]
    fn a_namespace_wildcard_takes_only_its_namespace() {
        let tables = compile(NS);
        round_trip(
            &tables,
            r#"<w:box xmlns:w="urn:w" w:n="7"><w:item>true</w:item><w:item>0</w:item></w:box>"#,
            r#"<w:box xmlns:w="urn:w" w:n="7"><w:item>true</w:item><w:item>false</w:item></w:box>"#,
        );
        // An attribute or element in another namespace matches no production.
        for xml in [
            r#"<w:box xmlns:w="urn:w" xmlns:v="urn:v" v:n="7"/>"#,
            r#"<w:box xmlns:w="urn:w"><item>true</item></w:box>"#,
        ] {
            let err = encode(&tables, xml, ExiOptions::strict());
            assert!(
                matches!(err, Err(rexi_xml::XmlError::Exi(ExiError::Grammar { .. }))),
                "{xml}: {err:?}"
            );
        }
    }
}
