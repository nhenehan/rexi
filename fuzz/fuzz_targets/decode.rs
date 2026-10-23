#![no_main]
//! The decoder on any input. The first byte picks the schema and how the options
//! arrive; the rest is the stream. When a stream with out-of-band options decodes,
//! its XML must encode again and decode to the same events, compared the way XML
//! text holds them. Two things do not survive XML text, so the comparison ignores
//! them: where XML spaces sit in a value (a stream may hold a list item with a
//! space, tab or line break inside, but in XML text that is only a separator,
//! [XSD whiteSpace](https://www.w3.org/TR/xmlschema-2/#rf-whiteSpace)), and how
//! text is cut into events (an empty one vanishes and neighbors join into one).

use libfuzzer_sys::fuzz_target;
use rexi_core::value::OwnedValue;
use rexi_core::{HeaderConfig, OwnedEvent};
use rexi_xml::{decode_events, decode_to_xml, encode_xml};

fuzz_target!(|data: &[u8]| {
    let Some((&first, stream)) = data.split_first() else {
        return;
    };
    let grammars = rexi_fuzz::grammars();
    let tables = grammars[usize::from(first & 0x0f) % grammars.len()];
    let options = rexi_fuzz::options(first >> 4);
    let Ok(xml) = decode_to_xml(stream, tables, options.clone()) else {
        return;
    };
    let Some(options) = options else {
        return;
    };
    let header = HeaderConfig::default().with_include_options(false);
    let again = match encode_xml(xml.as_bytes(), tables, &options, header) {
        Ok(bytes) => bytes,
        // The grammar lets xsi:nil="false" come back to the same rule, so a stream
        // may repeat it (EXI 8.5.4.1.3) and still be conformant (EXI 10.1): the
        // decoder accepts it, by Nathan's choice to follow the spec. As XML that is
        // a duplicate attribute, which no XML parser accepts.
        Err(e) if format!("{e:?}").contains("Duplicated") => return,
        Err(e) => panic!("decoded XML does not encode: {e:?}\n{xml}"),
    };
    let first_events = decode_events(stream, tables, Some(options.clone()));
    let second_events = decode_events(&again[..], tables, Some(options));
    match (first_events, second_events) {
        (Ok(a), Ok(b)) => assert_eq!(
            as_xml_text(a),
            as_xml_text(b),
            "events differ after a round trip\n{xml}"
        ),
        (a, b) => panic!("round trip decode failed: {:?} / {:?}", a.err(), b.err()),
    }
});

/// The events as XML text keeps them: neighboring text events join, empty ones
/// go, and runs of XML spaces in string values collapse to one space.
fn as_xml_text(events: Vec<OwnedEvent>) -> Vec<OwnedEvent> {
    let space = |value: OwnedValue| match value {
        OwnedValue::String(s) => OwnedValue::String(
            s.split([' ', '\t', '\n', '\r'])
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        other => other,
    };
    let mut out: Vec<OwnedEvent> = Vec::with_capacity(events.len());
    for event in events {
        match event {
            OwnedEvent::Characters(OwnedValue::String(s)) if s.is_empty() => {}
            OwnedEvent::Characters(value) => {
                if let (
                    Some(OwnedEvent::Characters(OwnedValue::String(previous))),
                    OwnedValue::String(next),
                ) = (out.last_mut(), &value)
                {
                    previous.push_str(next);
                } else {
                    out.push(OwnedEvent::Characters(value));
                }
            }
            other => out.push(other),
        }
    }
    out.into_iter()
        .filter_map(|event| match event {
            OwnedEvent::Characters(value) => match space(value) {
                OwnedValue::String(s) if s.is_empty() => None,
                value => Some(OwnedEvent::Characters(value)),
            },
            OwnedEvent::Attribute { uri, local, value } => Some(OwnedEvent::Attribute {
                uri,
                local,
                value: space(value),
            }),
            other => Some(other),
        })
        .collect()
}
