//! `xsi:type` with a name that is not a type in the schema. The name is written
//! as a QName all the same ([EXI 7.1.7](https://www.w3.org/TR/exi/#encodingQName)),
//! and the element keeps the grammar it has, since there is no type grammar to
//! switch to ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).
//! A prefix with no namespace in scope gives uri `""` and the whole value as the
//! local name (EXI 7.1.7). EXIficient does both of these the same way.
//!
//! The W3C test suite's declaredProductions cases complexUrType-04 and -05 check
//! the same thing against EXIficient (scripts/w3c-strict.py).

use rexi_core::{Alignment, ExiOptions, GrammarSetBuf, HeaderConfig, OwnedEvent};
use rexi_xml::{decode_events, decode_to_xml, encode_xml, parse_events};

/// `a` has type `T`, and `U` extends `T`, so `a` takes `xsi:type`. No target
/// namespace: the schema's local names in uri `""` are `T`, `U`, `a`, `n`.
const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:complexType name="T">
    <xs:sequence><xs:element name="n" type="xs:boolean"/></xs:sequence>
  </xs:complexType>
  <xs:complexType name="U">
    <xs:complexContent><xs:extension base="T"/></xs:complexContent>
  </xs:complexType>
  <xs:element name="a" type="T"/>
</xs:schema>"#;

const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

fn tables() -> GrammarSetBuf {
    rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"))
}

fn encode(tables: &GrammarSetBuf, xml: &str, options: ExiOptions) -> Vec<u8> {
    encode_xml(xml.as_bytes(), tables, options, HeaderConfig::default())
        .unwrap_or_else(|e| panic!("{e}\n{xml}"))
}

/// Encodes `xml`, decodes it again, and checks that the events match the XML's.
fn round_trip(tables: &GrammarSetBuf, xml: &str, options: ExiOptions) -> Vec<u8> {
    let exi = encode(tables, xml, options.clone());
    let back = decode_events(&exi[..], tables, Some(options)).unwrap_or_else(|e| panic!("{e}"));
    let want = parse_events(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back, want, "{xml}");
    exi
}

#[test]
fn bits_follow_the_spec() {
    let tables = tables();
    let xml = format!(r#"<a xmlns:xsi="{XSI}" xsi:type="Nope"><n>true</n></a>"#);
    let exi = encode(&tables, &xml, ExiOptions::strict());
    // Header: distinguishing bits 10, no options, version 0 0000 = 0x80. Then:
    //   SE(a)          1 bit, code 0 (SE(*) is 1, and strict mode keeps it in
    //                  DocContent, EXI 8.5.1): 0
    //   escape         1 bit, code 1 (SE(n) is 0): 1
    //   AT(xsi:type)   0 bits (the only second-level production)
    //   uri ""         3 bits (4 uris: "", xml, xsi, xs), hit 0 is 1: 001
    //   local "Nope"   a miss: length + 1 = 5 as an unsigned integer, 0000 0101,
    //                  then each code point: N 0100 1110, o 0110 1111,
    //                  p 0111 0000, e 0110 0101
    //   SE(n)          T's grammar is kept, with its escape: 1 bit, code 0: 0
    //   CH             0 bits, the boolean true: 1
    //   EE, EE, ED     0 bits each
    // 0100 1000 0010 1010 0111 0011 0111 1011 1000 0011 0010 101, padded with zeros.
    // EXIficient 1.0.7 writes the same bytes.
    assert_eq!(exi, [0x80, 0x48, 0x2A, 0x73, 0x7B, 0x83, 0x2A]);
}

#[test]
fn unknown_type_round_trips() {
    let tables = tables();
    let xml = format!(r#"<a xmlns:xsi="{XSI}" xsi:type="Nope"><n>true</n></a>"#);
    for options in [
        ExiOptions::strict(),
        ExiOptions::strict().with_alignment(Alignment::ByteAligned),
    ] {
        round_trip(&tables, &xml, options);
    }
}

#[test]
fn undeclared_prefix_is_part_of_the_local_name() {
    let xml = format!(r#"<a xmlns:xsi="{XSI}" xsi:type="none:any"><n>true</n></a>"#);
    let events = parse_events(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        events.contains(&OwnedEvent::XsiType {
            uri: String::new(),
            local: "none:any".into(),
        }),
        "{events:?}"
    );

    let tables = tables();
    let exi = round_trip(&tables, &xml, ExiOptions::strict());
    // Written back, the value is the same text, and still has no namespace.
    let back = decode_to_xml(&exi[..], &tables, Some(ExiOptions::strict()))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(back.contains(r#"xsi:type="none:any""#), "{back}");
}

#[test]
fn a_known_type_still_switches_grammar() {
    // `U` adds nothing to `T`, so this only checks that the lookup still works.
    let tables = tables();
    let xml = format!(r#"<a xmlns:xsi="{XSI}" xsi:type="U"><n>false</n></a>"#);
    round_trip(&tables, &xml, ExiOptions::strict());
}
