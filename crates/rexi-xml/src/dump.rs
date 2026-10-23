//! The `.events` text format: one event per line, the way the interop oracle prints
//! it. Names are printed as `{uri}local`, with `{}` for no namespace.
//!
//! ```text
//! SD
//! SE {urn:x}root
//! AT {}id 7
//! XSI_TYPE {urn:x}T
//! XSI_NIL true
//! CH text
//! EE
//! ED
//! ```
//!
//! In values, `\`, newline, carriage return and tab are escaped as `\\`, `\n`, `\r`
//! and `\t`, so each event stays on one line.

use std::fmt::Write as _;

use rexi_core::{OwnedEvent, OwnedValue};

/// Formats a value without knowing the grammar. Strings pass through unchanged.
/// Other single values use their standard text form ("lexical form" in XML
/// Schema). Enumerations print their position number (ordinal). Binary data prints
/// as uppercase hex. List items are joined with single spaces.
pub fn plain_format(value: &OwnedValue) -> String {
    match value {
        OwnedValue::String(s) => s.clone(),
        OwnedValue::Boolean(b) => b.to_string(),
        OwnedValue::Integer(i) => i.to_string(),
        OwnedValue::Unsigned(u) => u.to_string(),
        OwnedValue::Decimal(d) => d.to_lexical(),
        OwnedValue::Float(f) => f.to_lexical(),
        OwnedValue::DateTime(kind, dt) => dt.to_lexical(*kind),
        OwnedValue::Binary(bytes) => bytes.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02X}");
            s
        }),
        OwnedValue::Enum(ordinal) => ordinal.to_string(),
        OwnedValue::List(items) => items.iter().map(plain_format).collect::<Vec<_>>().join(" "),
        // A value kind added to rexi-core later.
        other => format!("{other:?}"),
    }
}

fn escape_value(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
}

/// Prints events in the `.events` format, using [`plain_format`] for values.
pub fn events_dump(events: &[OwnedEvent]) -> String {
    events_dump_with(events, plain_format)
}

/// Prints events in the `.events` format, formatting values with `fmt`. Each line,
/// including the last, ends with a newline.
pub fn events_dump_with(events: &[OwnedEvent], fmt: impl Fn(&OwnedValue) -> String) -> String {
    let mut out = String::new();
    for ev in events {
        match ev {
            OwnedEvent::StartDocument => out.push_str("SD"),
            OwnedEvent::EndDocument => out.push_str("ED"),
            OwnedEvent::StartElement { uri, local } => {
                let _ = write!(out, "SE {{{uri}}}{local}");
            }
            OwnedEvent::EndElement => out.push_str("EE"),
            OwnedEvent::Attribute { uri, local, value } => {
                let _ = write!(out, "AT {{{uri}}}{local} ");
                escape_value(&mut out, &fmt(value));
            }
            OwnedEvent::XsiType { uri, local } => {
                let _ = write!(out, "XSI_TYPE {{{uri}}}{local}");
            }
            OwnedEvent::XsiNil(nil) => {
                let _ = write!(out, "XSI_NIL {nil}");
            }
            OwnedEvent::Characters(value) => {
                out.push_str("CH ");
                escape_value(&mut out, &fmt(value));
            }
            // An event kind added to rexi-core later.
            other => {
                let _ = write!(out, "{other:?}");
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexi_core::value::Int;

    #[test]
    fn line_format_and_escapes() {
        let events = vec![
            OwnedEvent::StartDocument,
            OwnedEvent::StartElement {
                uri: "urn:x".into(),
                local: "r".into(),
            },
            OwnedEvent::XsiType {
                uri: "urn:x".into(),
                local: "T".into(),
            },
            OwnedEvent::XsiNil(false),
            OwnedEvent::Attribute {
                uri: String::new(),
                local: "a".into(),
                value: OwnedValue::String("x\ty\\z".into()),
            },
            OwnedEvent::Characters(OwnedValue::String("l1\nl2\r".into())),
            OwnedEvent::Characters(OwnedValue::Integer(Int::Small(-4))),
            OwnedEvent::EndElement,
            OwnedEvent::EndDocument,
        ];
        assert_eq!(
            events_dump(&events),
            "SD\nSE {urn:x}r\nXSI_TYPE {urn:x}T\nXSI_NIL false\nAT {}a x\\ty\\\\z\nCH l1\\nl2\\r\nCH -4\nEE\nED\n"
        );
        let upper = events_dump_with(&events, |v| plain_format(v).to_uppercase());
        assert!(upper.contains("CH L1\\nL2\\r\n"));
    }
}
