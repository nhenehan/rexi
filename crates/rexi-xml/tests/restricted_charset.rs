//! String types whose patterns give a restricted character set
//! ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)). rexi does not
//! write that form yet, so a value of such a type is refused on encode and on
//! decode, with `Unsupported(RestrictedCharacterSet)`. The schema still compiles.
//! Patterns with a large alphabet keep the plain string form, byte for byte.
//!
//! The patterns come from the W3C test suite
//! (`exi-testsuite/data/interop/datatypes/rcs`).

use rexi_core::{DatatypeKind, ExiError, ExiOptions, Feature, GrammarSetBuf, HeaderConfig};
use rexi_xml::XmlError;

/// A schema with one element `v` whose type is `xs:string` (or `base`) restricted
/// by `facets`.
fn schema(base: &str, facets: &str) -> String {
    format!(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="v"><xs:simpleType>
            <xs:restriction base="{base}">{facets}</xs:restriction>
          </xs:simpleType></xs:element>
        </xs:schema>"#
    )
}

fn pattern(p: &str) -> String {
    format!(r#"<xs:pattern value="{p}"/>"#)
}

fn compile(xsd: &str) -> GrammarSetBuf {
    rexi_schema::compile_str(xsd, None).unwrap_or_else(|e| panic!("{e}\n{xsd}"))
}

fn kinds(t: &GrammarSetBuf) -> Vec<DatatypeKind> {
    use rexi_core::GrammarTables;
    t.types().iter().filter_map(|d| d.datatype).collect()
}

fn has_restricted(t: &GrammarSetBuf) -> bool {
    kinds(t)
        .iter()
        .any(|k| matches!(k, DatatypeKind::RestrictedString { .. }))
}

fn encode(t: &GrammarSetBuf, xml: &str) -> rexi_xml::Result<Vec<u8>> {
    rexi_xml::encode_xml(
        xml.as_bytes(),
        t,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    )
}

fn is_refusal(e: &XmlError) -> bool {
    matches!(
        e,
        XmlError::Exi(ExiError::Unsupported(Feature::RestrictedCharacterSet))
    )
}

/// Decodes `bytes` and checks it ends in the refusal.
fn assert_decode_refused(t: &GrammarSetBuf, bytes: &[u8], what: &str) {
    match rexi_xml::decode_to_xml(bytes, t, Some(ExiOptions::strict())) {
        Err(e) if is_refusal(&e) => {}
        other => panic!("{what}: decoding {bytes:02x?} gave {other:?}"),
    }
}

/// Header `80`, then all zero bits: `SE(v)` (code 0), `CH` (code 0), and then
/// the value, which is where the decoder must stop. For a list the count is 1
/// (`00000001` after the one-bit `SE` code), so one item is read.
const SCALAR_STREAM: &[u8] = &[0x80, 0x00, 0x00, 0x00];
const LIST_STREAM: &[u8] = &[0x80, 0x00, 0x80, 0x00, 0x00];

#[test]
fn restricted_patterns_are_refused_on_encode_and_decode() {
    let cases = [
        ("[abc&#x3095;&#x00A9;]", "a"),
        ("[0-9]{3}-[0-9]{2}-[0-9]{4}", "123-45-6789"),
        ("[A-X]", "B"),
        ("[&#x03D7;-&#x03DA;]", "\u{03D8}"),
        ("[a-z]{3}", "abc"),
        ("[a-z-[aeiou]]+", "xyz"),
        ("", ""),
        // A pattern rexi cannot read is refused too, rather than guessed.
        ("[z-a]", "x"),
    ];
    for (p, value) in cases {
        let xsd = schema("xs:string", &pattern(p));
        let t = compile(&xsd);
        assert!(has_restricted(&t), "{p}: {:?}", kinds(&t));
        match encode(&t, &format!("<v>{value}</v>")) {
            Err(e) if is_refusal(&e) => {}
            other => panic!("{p}: encode gave {other:?}"),
        }
        assert_decode_refused(&t, SCALAR_STREAM, p);
    }
}

#[test]
fn restricted_list_item_is_refused() {
    let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
      <xs:element name="v"><xs:simpleType><xs:list><xs:simpleType>
        <xs:restriction base="xs:string"><xs:pattern value="[a-z]+"/></xs:restriction>
      </xs:simpleType></xs:list></xs:simpleType></xs:element>
    </xs:schema>"#;
    let t = compile(xsd);
    match encode(&t, "<v>ab cd</v>") {
        Err(e) if is_refusal(&e) => {}
        other => panic!("encode gave {other:?}"),
    }
    assert_decode_refused(&t, LIST_STREAM, "list");
}

/// A type below `xs:token` still derives from `xs:string`, and the most-derived
/// type with patterns is the one that counts.
#[test]
fn derived_types_use_the_nearest_patterns() {
    let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
      <xs:simpleType name="T"><xs:restriction base="xs:token">
        <xs:pattern value="[A-Z]{2}"/></xs:restriction></xs:simpleType>
      <xs:simpleType name="U"><xs:restriction base="T">
        <xs:maxLength value="2"/></xs:restriction></xs:simpleType>
      <xs:simpleType name="W"><xs:restriction base="T">
        <xs:pattern value=".*"/></xs:restriction></xs:simpleType>
      <xs:element name="u" type="U"/>
      <xs:element name="w" type="W"/>
    </xs:schema>"#;
    let t = compile(xsd);
    match encode(&t, "<u>AB</u>") {
        Err(e) if is_refusal(&e) => {}
        other => panic!("U: encode gave {other:?}"),
    }
    // W has its own pattern, `.*`, so it is a plain string again.
    encode(&t, "<w>AB</w>").unwrap_or_else(|e| panic!("W: {e}"));
}

/// Large alphabets keep the plain string form: the bytes are the same as for the
/// type with no pattern at all.
#[test]
fn large_alphabets_encode_as_before() {
    let cases = [
        ("xs:string", "[ABC&#x10FFF;]", "A"),
        ("xs:string", r"\p{Lm}", "\u{02B0}"),
        ("xs:string", r"\d+", "123"),
        ("xs:string", ".*", "anything"),
        ("xs:string", "[^:]", "x"),
        ("xs:string", "[&#x20;-&#x11F;]", "x"),
        ("xs:duration", "P[0-9]Y", "P1Y"),
        ("xs:anyURI", "[a-z:]+", "urn:x"),
    ];
    for (base, p, value) in cases {
        let doc = format!("<v>{value}</v>");
        let with = compile(&schema(base, &pattern(p)));
        let without = compile(&schema(base, ""));
        assert!(!has_restricted(&with), "{p}: {:?}", kinds(&with));
        let a = encode(&with, &doc).unwrap_or_else(|e| panic!("{p}: {e}"));
        let b = encode(&without, &doc).unwrap_or_else(|e| panic!("{p}: {e}"));
        assert_eq!(a, b, "{p}");
        let xml = rexi_xml::decode_to_xml(&a[..], &with, Some(ExiOptions::strict()))
            .unwrap_or_else(|e| panic!("{p}: {e}"));
        assert!(xml.contains(value), "{p}: {xml}");
    }
}

#[test]
fn boolean_pattern_stays_boolean_pattern() {
    let t = compile(&schema("xs:boolean", &pattern("true|false")));
    let k = kinds(&t);
    assert!(k.contains(&DatatypeKind::BooleanPattern), "{k:?}");
    assert!(!has_restricted(&t));
    encode(&t, "<v>true</v>").unwrap_or_else(|e| panic!("{e}"));
}

/// The refusal only comes when a value of the type is used. The rest of the
/// schema works.
#[test]
fn schema_with_restricted_type_still_works_without_it() {
    let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
      <xs:simpleType name="Code"><xs:restriction base="xs:string">
        <xs:pattern value="[A-Z]{2}[0-9]{4}"/></xs:restriction></xs:simpleType>
      <xs:element name="r"><xs:complexType><xs:sequence>
        <xs:element name="c" type="Code" minOccurs="0"/>
        <xs:element name="s" type="xs:string"/>
      </xs:sequence></xs:complexType></xs:element>
    </xs:schema>"#;
    let t = compile(xsd);
    assert!(has_restricted(&t));
    let doc = "<r><s>hello</s></r>";
    let bytes = encode(&t, doc).unwrap_or_else(|e| panic!("{e}"));
    let xml = rexi_xml::decode_to_xml(&bytes[..], &t, Some(ExiOptions::strict()))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(xml, doc);
    match encode(&t, "<r><c>AB1234</c><s>hello</s></r>") {
        Err(e) if is_refusal(&e) => {}
        other => panic!("encode gave {other:?}"),
    }
}

#[test]
fn refusal_message_names_the_feature() {
    let e = ExiError::Unsupported(Feature::RestrictedCharacterSet);
    assert!(e.to_string().contains("restricted character sets"), "{e}");
}
