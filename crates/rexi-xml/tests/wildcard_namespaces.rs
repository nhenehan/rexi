//! Spec 7.3.1, 8.5.4.1.3.2 and 8.5.4.1.7: every uri listed in a wildcard's
//! namespace constraint (`xs:any` or `xs:anyAttribute`) joins the uri partition,
//! sorted with the target namespaces. rexi used to collect only `xs:any` lists,
//! so a namespace named only by an attribute wildcard made the schema fail to
//! compile (112 W3C interop cases, e.g. schemaInformedGrammar/acceptance.xsd).

use rexi_core::{ExiOptions, GrammarTables, HeaderConfig};

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
    targetNamespace="urn:t" xmlns="urn:t" elementFormDefault="qualified">
  <xs:complexType name="Base"><xs:sequence><xs:element name="n" type="xs:string"/></xs:sequence></xs:complexType>
  <xs:complexType name="Derived"><xs:complexContent><xs:extension base="Base">
    <xs:sequence><xs:element name="m" type="xs:int"/></xs:sequence></xs:extension></xs:complexContent></xs:complexType>
  <xs:element name="r">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="c" type="Base"/>
        <xs:any namespace="urn:only-in-any ##local" processContents="skip" minOccurs="0"/>
      </xs:sequence>
      <xs:attribute name="a" type="xs:int"/>
      <xs:anyAttribute namespace="urn:zz-only-in-anyAttribute urn:b-only-in-anyAttribute" processContents="skip"/>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

/// The xsi:type value is written with a uri compact identifier, so these bytes
/// depend on the partition's size and order. EXIficient 1.0.7
/// (`xsd2exip-cli.jar encode-xml --strict`) writes `80001ae00406f003c0`; without
/// the two attribute-wildcard uris it would be `80001f00101bc00f00`.
const XML: &str = r#"<r xmlns="urn:t" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" a="1"><c xsi:type="Derived"><n>x</n><m>7</m></c></r>"#;
const EXIFICIENT: &str = "80001ae00406f003c0";

#[test]
fn attribute_wildcard_uris_join_the_uri_partition() {
    let t = rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"));
    let uris: Vec<&str> = (0..t.uris().len() as u32)
        .filter_map(|u| t.uri_str(u))
        .collect();
    assert_eq!(
        &uris[4..],
        [
            "urn:b-only-in-anyAttribute",
            "urn:only-in-any",
            "urn:t",
            "urn:zz-only-in-anyAttribute"
        ],
        "same order as EXIficient's grammar dump"
    );
    let bytes = rexi_xml::encode_xml(
        XML.as_bytes(),
        &t,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, EXIFICIENT);
}
