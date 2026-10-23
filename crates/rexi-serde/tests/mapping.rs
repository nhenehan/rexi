//! Mapping rules beyond the interop fixtures: `xsi:type`, `xsi:nil`, lists,
//! binary, big and n-bit integers, dates, attribute ordering, and the errors for
//! values that do not fit the schema. Bytes are checked against the XML front-end
//! encoding the equivalent document.

use rexi_core::{ExiError, ExiOptions, Feature, GrammarSetBuf, HeaderConfig};
use rexi_serde::{Error, from_exi, to_exi, to_exi_root};
use serde::{Deserialize, Serialize};

const XSD: &str = r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:t"
           xmlns="urn:t" elementFormDefault="qualified">
  <xs:complexType name="Shape">
    <xs:sequence><xs:element name="Label" type="xs:string"/></xs:sequence>
  </xs:complexType>
  <xs:complexType name="Circle">
    <xs:complexContent>
      <xs:extension base="Shape">
        <xs:sequence><xs:element name="Radius" type="xs:double"/></xs:sequence>
      </xs:extension>
    </xs:complexContent>
  </xs:complexType>
  <xs:simpleType name="Ints"><xs:list itemType="xs:int"/></xs:simpleType>
  <xs:element name="Drawing">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="Shape" type="Shape" maxOccurs="unbounded"/>
        <xs:element name="Counts" type="Ints"/>
        <xs:element name="Blob" type="xs:hexBinary"/>
        <xs:element name="Note" type="xs:string" nillable="true"/>
        <xs:element name="When" type="xs:date"/>
        <xs:element name="Big" type="xs:integer"/>
        <xs:element name="Small" type="xs:unsignedByte"/>
      </xs:sequence>
      <xs:attribute name="b" type="xs:string"/>
      <xs:attribute name="a" type="xs:int" use="required"/>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

fn tables() -> GrammarSetBuf {
    rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"))
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Drawing {
    // Struct order b, a: written in schema order a, b.
    #[serde(rename = "b")]
    b: Option<String>,
    #[serde(rename = "a")]
    a: i32,
    shape: Vec<Shape>,
    counts: Vec<i32>,
    blob: Vec<u8>,
    note: Option<String>,
    when: String,
    big: i128,
    small: u8,
}

/// `xsi:type` polymorphism: the variant names the type.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "@xsi:type")]
enum Shape {
    Shape {
        #[serde(rename = "Label")]
        label: String,
    },
    Circle {
        #[serde(rename = "Label")]
        label: String,
        #[serde(rename = "Radius")]
        radius: f64,
    },
}

fn drawing() -> Drawing {
    Drawing {
        b: Some("x".into()),
        a: 7,
        shape: vec![
            Shape::Circle {
                label: "c".into(),
                radius: 1.5,
            },
            Shape::Shape { label: "s".into() },
        ],
        counts: vec![1, 2, 3],
        blob: vec![0xCA, 0xFE],
        note: Some("hi".into()),
        when: "2026-10-10".into(),
        big: 123_456_789_012_345_678_901_234_567_890,
        small: 200,
    }
}

const XML: &str = r#"<Drawing xmlns="urn:t" xmlns:t="urn:t"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" b="x" a="7">
  <Shape xsi:type="t:Circle"><Label>c</Label><Radius>1.5</Radius></Shape>
  <Shape xsi:type="t:Shape"><Label>s</Label></Shape>
  <Counts>1 2 3</Counts><Blob>CAFE</Blob><Note>hi</Note><When>2026-10-10</When>
  <Big>123456789012345678901234567890</Big><Small>200</Small>
</Drawing>"#;

fn opts() -> ExiOptions {
    ExiOptions::strict()
}

/// EXIficient 1.0.7 encodings of `XML` and of the nil document below (strict,
/// bit-packed, no options), from
/// `xsd2exip-cli.jar encode-xml --xsd t.xsd --xml d.xml --out d.exi --strict` with
/// `XSD` saved as t.xsd. They pin two behaviors no interop fixture covers: no
/// second-level escape after `xsi:type` (EXIficient's type grammars are neither
/// typecastable nor nillable) and an empty `CH` for empty simple content.
const EXIFICIENT_DRAWING: &str =
    "8001c06f1a004036307c01a012037381804040302cafe02343486a9434a57f3c79277e2e70fb6ff23b8c7200";
const EXIFICIENT_NIL: &str = "80403a0120373800070d08400000";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn round_trip_matches_xml_encoding() {
    let t = tables();
    let bytes =
        to_exi(&drawing(), &t, &opts(), HeaderConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let via_xml = rexi_xml::encode_xml(XML.as_bytes(), &t, &opts(), HeaderConfig::default())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(bytes, via_xml);
    assert_eq!(hex(&bytes), EXIFICIENT_DRAWING);
    let back: Drawing = from_exi(&bytes, &t, Some(opts())).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back, drawing());
}

#[derive(Serialize)]
#[serde(rename = "Drawing", rename_all = "PascalCase")]
struct NilNote {
    #[serde(rename = "a")]
    a: i32,
    shape: Vec<Shape>,
    counts: Vec<i32>,
    blob: Vec<u8>,
    note: Nil,
    when: String,
    big: i64,
    small: u8,
}

#[derive(Serialize)]
struct Nil {
    #[serde(rename = "@xsi:nil")]
    nil: bool,
}

#[test]
fn xsi_nil_writes_and_reads_as_none() {
    let t = tables();
    let value = NilNote {
        a: -1,
        shape: vec![Shape::Shape { label: "s".into() }],
        counts: vec![],
        blob: vec![],
        note: Nil { nil: true },
        when: "2026-01-01".into(),
        big: 0,
        small: 0,
    };
    let bytes =
        to_exi(&value, &t, &opts(), HeaderConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let xml = r#"<Drawing xmlns="urn:t" xmlns:t="urn:t"
      xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" a="-1">
      <Shape xsi:type="t:Shape"><Label>s</Label></Shape><Counts></Counts><Blob></Blob>
      <Note xsi:nil="true"/><When>2026-01-01</When><Big>0</Big><Small>0</Small></Drawing>"#;
    let via_xml = rexi_xml::encode_xml(xml.as_bytes(), &t, &opts(), HeaderConfig::default())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(bytes, via_xml);
    assert_eq!(hex(&bytes), EXIFICIENT_NIL);
    let back: Drawing = from_exi(&bytes, &t, Some(opts())).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.note, None);
    assert_eq!(back.b, None);
    assert_eq!(back.counts, Vec::<i32>::new());
}

#[test]
fn explicit_root_name() {
    #[derive(Serialize)]
    struct Anything {
        a: i32,
    }
    let t = tables();
    // The type name does not match a global element...
    let err = to_exi(&Anything { a: 1 }, &t, &opts(), HeaderConfig::default());
    assert!(
        matches!(&err, Err(Error::Message(m)) if m.contains("no global element named Anything")),
        "{err:?}"
    );
    // ...but naming the root works up to the first missing required content.
    let err = to_exi_root(
        &Anything { a: 1 },
        "{urn:t}Drawing",
        &t,
        &opts(),
        HeaderConfig::default(),
    );
    assert!(
        matches!(err, Err(Error::Exi(ExiError::Grammar { .. }))),
        "{err:?}"
    );
}

#[test]
fn values_that_do_not_fit_are_reported() {
    let t = tables();

    #[derive(Serialize)]
    #[serde(rename = "Drawing")]
    struct MissingRequiredAttribute {
        b: String,
    }
    let err = to_exi(
        &MissingRequiredAttribute { b: "x".into() },
        &t,
        &opts(),
        HeaderConfig::default(),
    );
    assert!(
        matches!(err, Err(Error::Message(ref m)) if m.contains("no element b")),
        "{err:?}"
    );

    #[derive(Serialize)]
    #[serde(rename = "Drawing")]
    struct UnknownChild {
        a: i32,
        #[serde(rename = "Bogus")]
        bogus: String,
    }
    let err = to_exi(
        &UnknownChild {
            a: 1,
            bogus: "x".into(),
        },
        &t,
        &opts(),
        HeaderConfig::default(),
    );
    assert!(
        matches!(&err, Err(Error::Message(m)) if m.contains("no element Bogus allowed here; expected attribute b, element Shape")),
        "{err:?}"
    );

    let bad_date = Drawing {
        when: "not-a-date".into(),
        ..drawing()
    };
    let err = to_exi(&bad_date, &t, &opts(), HeaderConfig::default());
    assert!(
        matches!(err, Err(Error::Exi(ExiError::Value { .. }))),
        "{err:?}"
    );
}

#[test]
fn schema_less_is_rejected() {
    let empty = GrammarSetBuf::default();
    let err = to_exi(&drawing(), &empty, &opts(), HeaderConfig::default());
    assert!(
        matches!(
            err,
            Err(Error::Exi(ExiError::Unsupported(
                Feature::BuiltInElementGrammar
            )))
        ),
        "{err:?}"
    );
}
