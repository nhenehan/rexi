//! Round trips through the codec using the EXI Options grammar (spec Appendix C) as
//! the schema: it is the one grammar `rexi-core` ships, and `write_options_document`
//! encodes the same documents, which gives an independent byte oracle.

use rexi_core::grammar::options_grammar::OPTIONS_GRAMMARS;
use rexi_core::header::{write_header_bits, write_options_document};
use rexi_core::io::BitWriter;
use rexi_core::{ExiOptions, HeaderConfig, OwnedEvent, OwnedValue};
use rexi_xml::{
    decode_events, decode_events_typed, decode_to_xml, encode_xml, events_dump, parse_events,
};

const EXI_NS: &str = "http://www.w3.org/2009/exi";

fn encode(xml: &str) -> Vec<u8> {
    encode_xml(
        xml.as_bytes(),
        OPTIONS_GRAMMARS,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// `80` header followed by `write_options_document`'s body bits.
fn oracle(options: &ExiOptions) -> Vec<u8> {
    let mut w = BitWriter::new(Vec::new());
    write_header_bits(&mut w, HeaderConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let w = write_options_document(w, options).unwrap_or_else(|e| panic!("{e}"));
    w.finish().unwrap_or_default()
}

fn decode(bytes: &[u8]) -> String {
    decode_to_xml(bytes, OPTIONS_GRAMMARS, Some(ExiOptions::strict()))
        .unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn schema_id_document_bytes_and_round_trip() {
    let xml = format!(
        "<header xmlns=\"{EXI_NS}\"><common><schemaId>a</schemaId></common><strict/></header>"
    );
    let bytes = encode(&xml);
    assert_eq!(bytes.len(), 4);
    assert_eq!(bytes[0], 0x80);
    assert_eq!(bytes, vec![0x80, 0x30, 0x0D, 0x84]);
    assert_eq!(bytes, oracle(&ExiOptions::strict().with_schema_id("a")));
    assert_eq!(decode(&bytes), xml);
}

#[test]
fn pretty_printed_input_encodes_identically() {
    let compact = format!(
        "<header xmlns=\"{EXI_NS}\"><common><schemaId>a</schemaId></common><strict/></header>"
    );
    let pretty = format!(
        "<?xml version=\"1.0\"?>\n<!-- options -->\n<header xmlns=\"{EXI_NS}\">\n  <common>\n    \
         <schemaId>a</schemaId>\n  </common>\n  <strict></strict>\n</header>\n"
    );
    assert_eq!(encode(&pretty), encode(&compact));
}

#[test]
fn xsi_nil_round_trip() {
    let xml = format!(
        "<header xmlns=\"{EXI_NS}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <common><schemaId xsi:nil=\"true\"/></common><strict/></header>"
    );
    let bytes = encode(&xml);
    let mut nil = ExiOptions::strict();
    nil.schema_id = rexi_core::SchemaId::Nil;
    assert_eq!(bytes, oracle(&nil));
    assert_eq!(decode(&bytes), xml);
    let events = decode_events(
        bytes.as_slice(),
        OPTIONS_GRAMMARS,
        Some(ExiOptions::strict()),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(events, parse_events(xml.as_bytes()).unwrap_or_default());
    assert!(
        events_dump(&events)
            .contains("SE {http://www.w3.org/2009/exi}schemaId\nXSI_NIL true\nEE\n")
    );
}

#[test]
fn typed_values_are_canonicalised() {
    // valueMaxLength is an unsigned integer: lexical "012" with surrounding space
    // must encode like the typed value and come back as "12".
    let xml = format!(
        "<header xmlns=\"{EXI_NS}\"><lesscommon><uncommon><valueMaxLength> 012 </valueMaxLength>\
         </uncommon></lesscommon><strict/></header>"
    );
    let bytes = encode(&xml);
    let mut opts = ExiOptions::strict();
    opts.value_max_length = Some(12);
    assert_eq!(bytes, oracle(&opts));
    let canonical = xml.replace(" 012 ", "12");
    assert_eq!(decode(&bytes), canonical);

    let typed = decode_events_typed(
        bytes.as_slice(),
        OPTIONS_GRAMMARS,
        Some(ExiOptions::strict()),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let value = typed.iter().find_map(|e| match e {
        OwnedEvent::Characters(v) => Some(v.clone()),
        _ => None,
    });
    assert_eq!(
        value,
        Some(OwnedValue::Unsigned(rexi_core::Uint::Small(12)))
    );
    let strings = decode_events(
        bytes.as_slice(),
        OPTIONS_GRAMMARS,
        Some(ExiOptions::strict()),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(strings.contains(&OwnedEvent::Characters(OwnedValue::String("12".into()))));
}

#[test]
fn grammar_violations_are_reported() {
    let xml = format!("<header xmlns=\"{EXI_NS}\"><bogus/></header>");
    let err = encode_xml(
        xml.as_bytes(),
        OPTIONS_GRAMMARS,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    );
    assert!(
        matches!(
            err,
            Err(rexi_xml::XmlError::Exi(rexi_core::ExiError::Grammar { .. }))
        ),
        "{err:?}"
    );
    let err = encode_xml(
        b"<header xmlns=\"urn:other\"/>",
        OPTIONS_GRAMMARS,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    );
    assert!(matches!(err, Err(rexi_xml::XmlError::Exi(_))), "{err:?}");
    let err = encode_xml(
        b"<header xmlns=",
        OPTIONS_GRAMMARS,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    );
    assert!(matches!(err, Err(rexi_xml::XmlError::Xml(_))), "{err:?}");
}
