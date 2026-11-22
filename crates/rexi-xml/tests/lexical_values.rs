//! `Preserve.lexicalValues` ([EXI 6.3](https://www.w3.org/TR/exi/#fidelityOptions)):
//! every value is sent as its text, with the restricted character set of its
//! built-in type ([EXI 7](https://www.w3.org/TR/exi/#encodingDatatypes), Table 7-2).
//!
//! Without the `lexical-values` feature the option is refused. With it, the bytes
//! are checked against EXIficient by running the whole W3C strict group with the
//! option on (`scripts/w3c-strict.py --lexical-values`); these tests pin what
//! that run does not: one stream worked out by hand, exact text coming back, and
//! the refusals.

use rexi_core::{ExiError, ExiOptions, Feature, HeaderConfig};
use rexi_xml::encode_xml;

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="d" type="xs:decimal"/>
  <xs:element name="r"><xs:complexType><xs:sequence>
    <xs:element name="i" type="xs:int" maxOccurs="unbounded"/>
    <xs:element name="l" type="ints"/>
    <xs:element name="n" type="xs:boolean" nillable="true"/>
  </xs:sequence><xs:attribute name="b" type="xs:boolean"/></xs:complexType></xs:element>
  <xs:simpleType name="ints"><xs:list itemType="xs:int"/></xs:simpleType>
  <xs:complexType name="t"><xs:sequence>
    <xs:element name="v" type="xs:int"/>
  </xs:sequence></xs:complexType>
  <xs:complexType name="t2"><xs:complexContent><xs:extension base="t"><xs:sequence>
    <xs:element name="w" type="xs:int"/>
  </xs:sequence></xs:extension></xs:complexContent></xs:complexType>
  <xs:element name="e" type="t"/>
  <xs:element name="m"><xs:complexType mixed="true"><xs:sequence>
    <xs:element name="v" type="xs:int"/>
  </xs:sequence></xs:complexType></xs:element>
</xs:schema>"#;

fn lexical() -> ExiOptions {
    let mut options = ExiOptions::strict();
    options.preserve.lexical_values = true;
    options
}

fn feature_of(err: rexi_xml::XmlError) -> Option<Feature> {
    match err {
        rexi_xml::XmlError::Exi(ExiError::Unsupported(f)) => Some(f),
        _ => None,
    }
}

#[cfg(not(feature = "lexical-values"))]
#[test]
fn the_option_is_refused_without_the_feature() {
    let tables = rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"));
    let err = encode_xml(b"<d>1.50</d>", &tables, lexical(), HeaderConfig::default())
        .err()
        .unwrap_or_else(|| panic!("encoded"));
    assert_eq!(feature_of(err), Some(Feature::PreserveLexicalValues));
}

#[cfg(feature = "lexical-values")]
mod with_the_feature {
    use super::*;
    use rexi_core::Alignment;
    use rexi_xml::{decode_events, decode_to_xml, parse_events};

    fn tables() -> rexi_core::GrammarSetBuf {
        rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"))
    }

    fn encode(xml: &str, options: ExiOptions) -> Result<Vec<u8>, rexi_xml::XmlError> {
        encode_xml(xml.as_bytes(), tables(), options, HeaderConfig::default())
    }

    /// Encodes and decodes `xml` on both alignments; the events must be the
    /// XML's own, text for text.
    fn round_trip(xml: &str) {
        let t = tables();
        for options in [lexical(), lexical().with_alignment(Alignment::ByteAligned)] {
            let exi = encode(xml, options.clone()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
            let back = decode_events(&exi[..], &t, Some(options.clone()))
                .unwrap_or_else(|e| panic!("{e}"));
            let want = parse_events(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, want, "{xml}");
            let text = decode_to_xml(&exi[..], &t, Some(options)).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(text, xml);
        }
    }

    #[test]
    fn bits_follow_the_spec() {
        // Header 1000 0000 = 0x80. Then:
        //   SE(d): four global elements (d, e, m, r) plus SE(*), so 3 bits; d
        //       is first: 000
        //   CH: d's first rule has CH and the xsi:type escape (xs:decimal has
        //       subtypes): 1 bit, 0
        //   "1.50": a string table miss, length 4 + 2 = 6 as an Unsigned
        //       Integer: 0000 0110; then each character as a 5-bit index into
        //       the decimal set "\t\n\r +-.0123456789" (17 characters, so
        //       ceil_log2(18) = 5 bits): 1 = 8, . = 6, 5 = 12, 0 = 7
        //   EE, ED: 0 bits
        // 000 0 00000110 01000 00110 01100 00111, 32 bits:
        // 0000 0000 0110 0100 0001 1001 1000 0111 = 00 64 19 87.
        assert_eq!(
            encode("<d>1.50</d>", lexical()).unwrap_or_else(|e| panic!("{e}")),
            [0x80, 0x00, 0x64, 0x19, 0x87]
        );
    }

    #[test]
    fn values_come_back_as_written() {
        // Without the option these become 1.5, 5, the list "1 2", and false.
        round_trip("<d>1.50</d>");
        round_trip("<d> +01.0 </d>");
        round_trip(r#"<r b="0"><i>+05</i><i>7</i><l> 1  2 </l><n>0</n></r>"#);
        // A character outside the integer set is sent escaped, not refused:
        // EXI leaves checking the text to the schema, not to the codec.
        round_trip("<r><i>x</i><l>3</l><n>true</n></r>");
    }

    #[test]
    fn whitespace_between_elements_is_kept() {
        // EXIficient keeps every text a CH can take when the option is on.
        // Without it, the whitespace-only text in `m`'s mixed content is dropped.
        round_trip("<m>\n  <v>1</v>\n</m>");
    }

    #[test]
    fn xsi_nil_is_text() {
        round_trip(
            r#"<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><i>1</i><l>1</l><n xsi:nil="true"/></r>"#,
        );
    }

    #[test]
    fn xsi_type_with_a_namespace_needs_preserve_prefixes() {
        // The value is text, and a prefix in it means nothing without
        // Preserve.prefixes, which strict mode forbids. EXIficient refuses it too.
        let err = encode(
            r#"<e xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" xsi:type="xs:int">5</e>"#,
            lexical(),
        )
        .err()
        .unwrap_or_else(|| panic!("encoded"));
        assert_eq!(feature_of(err), Some(Feature::PreservePrefixes));
        // A type in no namespace is its local name.
        round_trip(
            r#"<e xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="t2"><v>05</v><w>6</w></e>"#,
        );
    }

    #[test]
    fn options_in_the_header() {
        let t = tables();
        let exi = encode_xml(
            b"<d>1.50</d>",
            &t,
            lexical(),
            HeaderConfig::default().with_include_options(true),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let text = decode_to_xml(&exi[..], &t, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(text, "<d>1.50</d>");
    }
}
