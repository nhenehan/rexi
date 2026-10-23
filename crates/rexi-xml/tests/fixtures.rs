//! The interop fixtures through the XML reader, checked against the oracle's
//! `.events` files (produced by EXIficient from the same XML).
//!
//! The oracle prints typed values in canonical form (`2175E-2` for the double
//! `21.75`, `31.5` for the decimal `31.50`), which the untyped reader cannot know,
//! so numeric-looking values are compared structurally only. Whitespace-only text is
//! dropped on our side: the encoder, not the reader, decides whether it matters.

use std::path::PathBuf;

use rexi_core::{OwnedEvent, OwnedValue};
use rexi_xml::{events_dump, parse_events};

const FIXTURES: [&str; 5] = [
    "order-complex",
    "telemetry-complex",
    "polymorphic-choice",
    "inheritance-extension",
    "cross-schema-main",
];

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rexi-tests/fixtures/interop")
}

fn read(name: &str) -> Vec<u8> {
    let path = fixture_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn looks_numeric(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | 'E'))
}

/// Compares one of our lines with one oracle line, ignoring canonicalized numbers.
fn lines_match(ours: &str, oracle: &str) -> bool {
    let split = |line: &str| -> Option<(String, String)> {
        let (kind, rest) = line.split_once(' ')?;
        if kind == "CH" {
            return Some((kind.to_string(), rest.to_string()));
        }
        if kind == "AT" {
            let (name, value) = rest.split_once(' ')?;
            return Some((format!("{kind} {name}"), value.to_string()));
        }
        None
    };
    match (split(ours), split(oracle)) {
        (Some((head_a, value_a)), Some((head_b, value_b))) => {
            head_a == head_b && (value_a == value_b || looks_numeric(&value_b))
        }
        _ => ours == oracle,
    }
}

fn is_whitespace_ch(ev: &OwnedEvent) -> bool {
    matches!(ev, OwnedEvent::Characters(OwnedValue::String(s)) if s.chars().all(char::is_whitespace))
}

#[test]
fn fixtures_match_oracle_events() {
    for name in FIXTURES {
        let xml = read(&format!("{name}.xml"));
        let events = parse_events(&xml).unwrap_or_else(|e| panic!("{name}: {e}"));
        let significant: Vec<OwnedEvent> = events
            .into_iter()
            .filter(|e| !is_whitespace_ch(e))
            .collect();
        let ours = events_dump(&significant);
        let oracle =
            String::from_utf8(read(&format!("{name}.strict-bp.events"))).unwrap_or_default();
        let ours: Vec<&str> = ours.lines().collect();
        let oracle: Vec<&str> = oracle.lines().collect();
        assert_eq!(ours.len(), oracle.len(), "{name}: event count");
        for (i, (a, b)) in ours.iter().zip(&oracle).enumerate() {
            assert!(
                lines_match(a, b),
                "{name} line {}: ours {a:?} oracle {b:?}",
                i + 1
            );
        }
    }
}

#[test]
fn telemetry_root_attributes_are_sorted() {
    let events = parse_events(&read("telemetry-complex.xml")).unwrap_or_else(|e| panic!("{e}"));
    let attrs: Vec<&str> = events
        .iter()
        .skip(2)
        .take(2)
        .filter_map(|e| match e {
            OwnedEvent::Attribute { local, .. } => Some(local.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(attrs, ["generatedAt", "site"]);
}

#[test]
fn cross_schema_prefixes_resolve() {
    let events = parse_events(&read("cross-schema-main.xml")).unwrap_or_else(|e| panic!("{e}"));
    let common: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            OwnedEvent::StartElement { uri, local } if uri == "urn:rexi:interop:common" => {
                Some(local.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        common,
        ["Site", "Zone", "AuditInfo", "CreatedBy", "CreatedAt"]
    );
    assert_eq!(
        events[1],
        OwnedEvent::StartElement {
            uri: "urn:rexi:interop:main".into(),
            local: "IncidentReport".into()
        }
    );
}

#[test]
fn cdata_and_entities_unescape() {
    let xml = b"<r xmlns=\"urn:t\" a=\"1 &lt; 2 &amp;&#x41;\"><![CDATA[a < b & c]]>&gt;&#10;x</r>";
    let events = parse_events(xml).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        events,
        vec![
            OwnedEvent::StartDocument,
            OwnedEvent::StartElement {
                uri: "urn:t".into(),
                local: "r".into()
            },
            OwnedEvent::Attribute {
                uri: String::new(),
                local: "a".into(),
                value: OwnedValue::String("1 < 2 &A".into())
            },
            OwnedEvent::Characters(OwnedValue::String("a < b & c>\nx".into())),
            OwnedEvent::EndElement,
            OwnedEvent::EndDocument,
        ]
    );
    assert_eq!(
        events_dump(&events),
        "SD\nSE {urn:t}r\nAT {}a 1 < 2 &A\nCH a < b & c>\\nx\nEE\nED\n"
    );
}
