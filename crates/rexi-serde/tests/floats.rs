//! Float fields: whole numbers past 2^63, which Rust writes out in full
//! (`1e19` is `10000000000000000000`), must encode to the same bytes as the XML
//! text, and read back to the same value.

use rexi_core::{ExiOptions, HeaderConfig};
use serde::{Deserialize, Serialize};

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="F"><xs:complexType><xs:sequence>
    <xs:element name="f" type="xs:double"/><xs:element name="d" type="xs:double"/>
  </xs:sequence></xs:complexType></xs:element></xs:schema>"#;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct F<T> {
    f: T,
    d: T,
}

fn check<T>(x: T, text: &str)
where
    T: Copy + PartialEq + std::fmt::Debug + Serialize + for<'de> Deserialize<'de>,
{
    let tables = rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"));
    let opts = ExiOptions::strict();
    let value = F { f: x, d: x };
    let exi = rexi_serde::to_exi_root(&value, "F", &tables, &opts, HeaderConfig::default())
        .unwrap_or_else(|e| panic!("{x:?}: {e}"));
    let xml = format!("<F><f>{text}</f><d>{text}</d></F>");
    let from_xml = rexi_xml::encode_xml(xml.as_bytes(), &tables, &opts, HeaderConfig::default())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(exi, from_xml, "{x:?}");
    let back: F<T> =
        rexi_serde::from_exi(&exi, &tables, Some(opts)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back, value);
}

#[test]
fn whole_floats_past_2_to_63() {
    check(1e19_f64, "1E19");
    check(-1e300_f64, "-1E300");
    check(f64::MAX, "17976931348623157E292");
    check(f32::MAX, "34028235E31");
}

#[test]
fn f32_fields_write_their_own_shortest_text() {
    check(0.1_f32, "0.1");
    check(1.0e-5_f32, "0.00001");
}
