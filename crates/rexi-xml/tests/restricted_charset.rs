//! String types whose patterns give a restricted character set
//! ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)): each character
//! of a string-table miss is an n-bit index into the set, or the escape index and
//! the code point. The bytes are EXIficient 1.0.7's, and the same as JExi's
//! (`test/schema.jl` there has the same vectors). A pattern rexi cannot read has
//! no known set, so its values are refused with `Unsupported(RestrictedCharacterSet)`.
//! Patterns with a large alphabet keep the plain string form, byte for byte.
//!
//! The patterns come from the W3C test suite
//! (`exi-testsuite/data/interop/datatypes/rcs`).

use rexi_core::{
    Alignment, DatatypeKind, ExiError, ExiOptions, Feature, GrammarSetBuf, HeaderConfig,
};
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
        ExiOptions::strict(),
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
/// the value, which is where the decoder must stop.
const SCALAR_STREAM: &[u8] = &[0x80, 0x00, 0x00, 0x00];

fn encode_with(t: &GrammarSetBuf, xml: &str, o: &ExiOptions) -> rexi_xml::Result<Vec<u8>> {
    rexi_xml::encode_xml(xml.as_bytes(), t, o.clone(), HeaderConfig::default())
}

fn byte_aligned() -> ExiOptions {
    ExiOptions::strict().with_alignment(Alignment::ByteAligned)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or_else(|e| panic!("{e}")))
        .collect()
}

const R_ELEM: &str = r#"<xs:element name="r" type="R"/>"#;

fn r_type(p: &str) -> String {
    format!(
        r#"<xs:simpleType name="R"><xs:restriction base="xs:string"><xs:pattern value="{p}"/></xs:restriction></xs:simpleType>"#
    )
}

fn xs(body: &str) -> String {
    format!(r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">{body}</xs:schema>"#)
}

/// EXIficient 1.0.7's bytes, bit-packed and byte-aligned, and the document they
/// decode to (the input, unless whitespace changes it).
#[test]
fn restricted_strings_match_exificient() {
    let token = r#"<xs:simpleType name="R"><xs:restriction base="xs:token"><xs:pattern value="[a-c ]+"/></xs:restriction></xs:simpleType>"#;
    let two = format!(
        r#"{}<xs:simpleType name="S"><xs:restriction base="xs:string"><xs:pattern value="[x-z]+"/></xs:restriction></xs:simpleType><xs:element name="r"><xs:complexType><xs:sequence><xs:element name="p" type="R"/><xs:element name="q" type="S"/></xs:sequence></xs:complexType></xs:element>"#,
        r_type("[a-c]+")
    );
    let hits = format!(
        r#"{}<xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" type="R" maxOccurs="unbounded"/></xs:sequence><xs:attribute name="b" type="R"/></xs:complexType></xs:element>"#,
        r_type("[0-9]{3}-[0-9]{2}-[0-9]{4}")
    );
    let list = format!(
        r#"{}<xs:simpleType name="L"><xs:list itemType="R"/></xs:simpleType><xs:element name="r" type="L"/>"#,
        r_type("[A-X]+")
    );
    let cases: Vec<(String, &str, Option<&str>, &str, &str)> = vec![
        // The VIN of ISO 3779.
        (
            r_type("[A-HJ-NPR-Z0-9]{17}") + R_ELEM,
            "<r>1HGCM82633A004352</r>",
            None,
            "8009828a062a404306194000206284",
            "8000130111100c1508020603030a000004030502",
        ),
        // Characters outside the set: the escape index, then the code point.
        (
            r_type("[A-HJ-NPR-Z0-9]{17}") + R_ELEM,
            "<r>IOQ-\u{e9}\u{20ac}</r>",
            None,
            "800442930a7c2a3096c3d2030d6208",
            "8000082149214f2151212d21e90121ac41",
        ),
        // Indexes follow code point order, not pattern order.
        (
            r_type("[abc&#x3095;&#x00A9;]+") + R_ELEM,
            "<r>a\u{a9}\u{3095}cb</r>",
            None,
            "80038711",
            "8000070003040201",
        ),
        // An empty set: zero-bit indexes, every character escaped.
        (
            r_type("") + R_ELEM,
            "<r>xy</r>",
            None,
            "80023c3c80",
            "8000047879",
        ),
        // String-table hits, local and global, in an element and an attribute.
        (
            hits,
            r#"<r b="123-45-6789"><a>123-45-6789</a><a>123-45-6789</a><a>987-65-4321</a></r>"#,
            None,
            "800348d01581e26804020da98076054328",
            "8000000d020304000506000708090a010001000d0a0908000706000504030201",
        ),
        // The item type of a list.
        (
            list,
            "<r>AB CD AB Z</r>",
            None,
            "80020200208218000f0b40",
            "800004040001040203000003185a",
        ),
        // xs:token collapses before the characters are written.
        (
            token.to_string() + R_ELEM,
            "<r>  a  b   c </r>",
            Some("<r>a b c</r>"),
            "80039083",
            "8000070100020003",
        ),
        // Two types, two sets.
        (
            two,
            "<r><p>abc</p><q>zyx</q></r>",
            None,
            "80028c0b20",
            "80000500010205020100",
        ),
        // 95 characters: 7-bit indexes.
        (
            r_type("[ -~]*") + R_ELEM,
            "<r>Hello, World! ~</r>",
            None,
            "8008a88b3264f1801bcfa53220101780",
            "80001128454c4c4f0c00374f524c4401005e",
        ),
    ];
    for (body, doc, decoded, bp, ba) in cases {
        let t = compile(&xs(&body));
        assert!(has_restricted(&t), "{doc}: {:?}", kinds(&t));
        for (o, want) in [(ExiOptions::strict(), bp), (byte_aligned(), ba)] {
            let got = encode_with(&t, doc, &o).unwrap_or_else(|e| panic!("{doc}: {e}"));
            assert_eq!(hex(&got), want, "{doc} {:?}", o.alignment);
            let xml = rexi_xml::decode_to_xml(&unhex(want)[..], &t, Some(o.clone()))
                .unwrap_or_else(|e| panic!("{doc}: {e}"));
            assert_eq!(xml, decoded.unwrap_or(doc), "{:?}", o.alignment);
        }
    }
}

/// A character outside the BMP is one escaped code point, as EXI 7.1.10 counts
/// characters. EXIficient 1.0.7 counts UTF-16 units instead
/// (`AbstractTypeEncoder.writeRCSValue` loops over `String.length()` and writes
/// the low surrogate as a second character), so its bytes are not the reference.
#[test]
fn characters_outside_the_bmp_are_one_escape() {
    let t = compile(&xs(&(r_type("[A-Z]+") + R_ELEM)));
    let doc = "<r>A\u{1F600}B</r>";
    let bytes = encode(&t, doc).unwrap_or_else(|e| panic!("{e}"));
    let xml = rexi_xml::decode_to_xml(&bytes[..], &t, Some(ExiOptions::strict()))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(xml, doc);
    // One character more than "AĀB" (U+0100 is a three-octet escape too).
    let plain = encode(&t, "<r>A\u{100}B</r>").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(bytes.len(), plain.len() + 1);
}

/// An index past the escape is an error, not a character: for `[a-e]` the
/// indexes are 3 bits and 6, 7 are unused.
#[test]
fn index_past_the_escape_is_an_error() {
    let t = compile(&xs(&(r_type("[a-e]") + R_ELEM)));
    match rexi_xml::decode_to_xml(&[0x80, 0x03, 0xff][..], &t, Some(ExiOptions::strict())) {
        Err(XmlError::Exi(ExiError::Value { .. })) => {}
        other => panic!("{other:?}"),
    }
}

/// A pattern rexi cannot read gives no set, so the value is refused rather than
/// guessed, on encode and on decode.
#[test]
fn unreadable_patterns_are_refused() {
    let t = compile(&schema("xs:string", &pattern("[z-a]")));
    assert!(has_restricted(&t), "{:?}", kinds(&t));
    match encode(&t, "<v>x</v>") {
        Err(e) if is_refusal(&e) => {}
        other => panic!("encode gave {other:?}"),
    }
    assert_decode_refused(&t, SCALAR_STREAM, "[z-a]");
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
      <xs:element name="s" type="xs:string"/>
    </xs:schema>"#;
    let t = compile(xsd);
    let u = encode(&t, "<u>AB</u>").unwrap_or_else(|e| panic!("U: {e}"));
    let xml = rexi_xml::decode_to_xml(&u[..], &t, Some(ExiOptions::strict()))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(xml, "<u>AB</u>");
    // W has its own pattern, `.*`, so it is a plain string again: two 8-bit
    // code points, where U's are 5-bit indexes.
    let w = encode(&t, "<w>AB</w>").unwrap_or_else(|e| panic!("W: {e}"));
    assert!(w.len() > u.len(), "{u:02x?} {w:02x?}");
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

/// The refusal (an unreadable pattern) only comes when a value of the type is
/// used. The rest of the schema works.
#[test]
fn schema_with_unreadable_type_still_works_without_it() {
    let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
      <xs:simpleType name="Code"><xs:restriction base="xs:string">
        <xs:pattern value="[z-a]"/></xs:restriction></xs:simpleType>
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
    match encode(&t, "<r><c>x</c><s>hello</s></r>") {
        Err(e) if is_refusal(&e) => {}
        other => panic!("encode gave {other:?}"),
    }
}

#[test]
fn refusal_message_names_the_feature() {
    let e = ExiError::Unsupported(Feature::RestrictedCharacterSet);
    assert!(e.to_string().contains("restricted character sets"), "{e}");
}
