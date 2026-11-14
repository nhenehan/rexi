//! Schema-informed fragments ([EXI 8.5.2](https://www.w3.org/TR/exi/#informedFragGrammars)):
//! a body that holds any number of root elements, each one any element the schema
//! declares, global or local. The top level is
//! `FragmentContent : SE(F_0) ... SE(F_n-1) SE(*) ED`, with the names sorted by local
//! name and then by URI, so its event codes are `ceil_log2(n + 2)` bits wide.
//!
//! The byte check against EXIficient for a larger schema lives in rexi-interop (case
//! F1). These tests pin the bits worked out by hand from the spec.

use rexi_core::{Alignment, ExiError, ExiOptions, Feature, GrammarSetBuf, HeaderConfig};
use rexi_xml::{XmlError, decode_events, decode_to_xml, encode_xml, parse_fragment_events};

/// Global `a` and `b`, and `c`, which is declared only inside `b`. Sorted, the
/// fragment's names are `a`, `b`, `c`: codes 0, 1, 2, then `SE(*)` = 3 and `ED` = 4,
/// three bits each.
const ABC: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="b">
    <xs:complexType><xs:sequence>
      <xs:element name="c" type="xs:boolean"/>
    </xs:sequence></xs:complexType>
  </xs:element>
  <xs:element name="a" type="xs:boolean"/>
</xs:schema>"#;

fn compile(xsd: &str) -> GrammarSetBuf {
    rexi_schema::compile_str(xsd, None).unwrap_or_else(|e| panic!("{e}\n{xsd}"))
}

fn fragment() -> ExiOptions {
    ExiOptions::strict().with_fragment(true)
}

fn encode(tables: &GrammarSetBuf, xml: &str, options: ExiOptions) -> Result<Vec<u8>, XmlError> {
    encode_xml(xml.as_bytes(), tables, options, HeaderConfig::default())
}

/// Encodes `xml` as a fragment, decodes it again, and checks that the events match
/// the XML's.
fn round_trip(tables: &GrammarSetBuf, xml: &str, options: ExiOptions) -> Vec<u8> {
    let exi = encode(tables, xml, options.clone()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    let back = decode_events(&exi[..], tables, Some(options)).unwrap_or_else(|e| panic!("{e}"));
    let want = parse_fragment_events(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back, want, "{xml}");
    exi
}

#[test]
fn bits_follow_the_spec() {
    let tables = compile(ABC);
    let exi =
        encode(&tables, "<c>true</c><a>false</a>", fragment()).unwrap_or_else(|e| panic!("{e}"));
    // Header: distinguishing bits 10, no options, version 0 0000 = 0x80. Then:
    //   SE(c)  code 2  010
    //   CH     0 bits, the boolean true: 1
    //   EE     0 bits (the only production)
    //   SE(a)  code 0  000
    //   CH     0 bits, the boolean false: 0
    //   EE     0 bits
    //   ED     code 4  100
    // 0101 0000 100, padded with zeros: 0x50 0x80.
    assert_eq!(exi, [0x80, 0x50, 0x80]);
}

#[test]
fn round_trips() {
    let tables = compile(ABC);
    for xml in [
        "",
        "<a>true</a>",
        "<b><c>false</c></b><a>true</a><c>true</c><b><c>true</c></b>",
    ] {
        round_trip(&tables, xml, fragment());
        round_trip(
            &tables,
            xml,
            fragment().with_alignment(Alignment::ByteAligned),
        );
    }
}

#[test]
fn empty_fragment_is_only_ed() {
    let tables = compile(ABC);
    // ED is code 4 in three bits: 100, padded.
    assert_eq!(round_trip(&tables, "", fragment()), [0x80, 0x80]);
}

#[test]
fn options_in_the_header() {
    let tables = compile(ABC);
    let xml = "<a>true</a><b><c>false</c></b>";
    let header = HeaderConfig::default().with_include_options(true);
    let exi =
        encode_xml(xml.as_bytes(), &tables, fragment(), header).unwrap_or_else(|e| panic!("{e}"));
    // No out-of-band options: the decoder finds `fragment` in the header.
    let back = decode_events(&exi[..], &tables, None).unwrap_or_else(|e| panic!("{e}"));
    let want = parse_fragment_events(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back, want);
}

#[test]
fn decodes_to_xml_with_namespaces_on_each_root() {
    let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
                   targetNamespace="urn:t" xmlns="urn:t" elementFormDefault="qualified">
      <xs:element name="p" type="xs:int"/>
      <xs:element name="q" type="xs:string"/>
    </xs:schema>"#;
    let tables = compile(xsd);
    let xml = r#"<p xmlns="urn:t">7</p><q xmlns="urn:t">hi</q>"#;
    let exi = round_trip(&tables, xml, fragment());
    let text = decode_to_xml(&exi[..], &tables, Some(fragment())).unwrap_or_else(|e| panic!("{e}"));
    // Each root declares its namespace, so the output reads back as the same
    // fragment.
    let again = parse_fragment_events(text.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let want = parse_fragment_events(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(again, want, "{text}");
}

#[test]
fn several_roots_need_the_fragment_option() {
    let tables = compile(ABC);
    let err = encode(&tables, "<a>true</a><a>false</a>", ExiOptions::strict());
    assert!(matches!(err, Err(XmlError::Unsupported(_))), "{err:?}");
    // A local element cannot be the root of a document either.
    let err = encode(&tables, "<c>true</c>", ExiOptions::strict());
    assert!(
        matches!(
            err,
            Err(XmlError::Exi(ExiError::Unsupported(
                Feature::BuiltInElementGrammar
            )))
        ),
        "{err:?}"
    );
}

#[test]
fn undeclared_root_needs_a_built_in_grammar() {
    let tables = compile(ABC);
    let err = encode(&tables, "<a>true</a><z/>", fragment());
    assert!(
        matches!(
            err,
            Err(XmlError::Exi(ExiError::Unsupported(
                Feature::BuiltInElementGrammar
            )))
        ),
        "{err:?}"
    );
    // And on decode: code 3 is SE(*). 011 after the header.
    let err = decode_events(&[0x80, 0x60][..], &tables, Some(fragment()));
    assert!(
        matches!(
            err,
            Err(XmlError::Exi(ExiError::Unsupported(
                Feature::BuiltInElementGrammar
            )))
        ),
        "{err:?}"
    );
}

/// Two local `x` declarations with different types: EXI reads a fragment-level `x`
/// with the relaxed element grammar, which rexi refuses. Inside its parent, each
/// `x` still has its own grammar.
#[test]
fn shared_name_with_different_types_is_refused_at_top_level_only() {
    let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
      <xs:element name="m"><xs:complexType><xs:sequence>
        <xs:element name="x" type="xs:int"/>
      </xs:sequence></xs:complexType></xs:element>
      <xs:element name="n"><xs:complexType><xs:sequence>
        <xs:element name="x" type="xs:string"/>
      </xs:sequence></xs:complexType></xs:element>
    </xs:schema>"#;
    let tables = compile(xsd);
    round_trip(&tables, "<m><x>5</x></m><n><x>five</x></n>", fragment());
    let err = encode(&tables, "<x>5</x>", fragment());
    assert!(
        matches!(
            err,
            Err(XmlError::Exi(ExiError::Unsupported(
                Feature::RelaxedElementGrammar
            )))
        ),
        "{err:?}"
    );
    // Names m, n, x, then SE(*) and ED: five codes, three bits. x is 010.
    let err = decode_events(&[0x80, 0x40][..], &tables, Some(fragment()));
    assert!(
        matches!(
            err,
            Err(XmlError::Exi(ExiError::Unsupported(
                Feature::RelaxedElementGrammar
            )))
        ),
        "{err:?}"
    );
}

/// The same name declared twice with the same type keeps that type's grammar.
#[test]
fn shared_name_with_the_same_type_is_one_entry() {
    let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
      <xs:element name="m"><xs:complexType><xs:sequence>
        <xs:element name="x" type="xs:int"/>
      </xs:sequence></xs:complexType></xs:element>
      <xs:element name="x" type="xs:int"/>
    </xs:schema>"#;
    let tables = compile(xsd);
    // Names m, x: two entries, so codes are ceil_log2(4) = 2 bits: x is 01. x's
    // first rule has CH and the xsi:type escape (xs:int has built-in subtypes,
    // EXI 8.5.4.4.2), so CH takes one bit: 0. Then the int 5: a sign bit 0 and one
    // byte 0000 0101 (EXI 7.1.5, 7.1.6). Then ED is 11.
    // 01 0 0 00000101 11 -> 0100 0000 0101 11(00) -> 0x40 0x5C.
    let exi = round_trip(&tables, "<x>5</x>", fragment());
    assert_eq!(exi, [0x80, 0x40, 0x5C]);
}
