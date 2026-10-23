//! XML text to EXI events, in the order the strict codec wants them.
//!
//! Namespace declarations (`xmlns` and `xmlns:p` attributes) are used up by the
//! namespace resolver, which maps each prefix to its namespace URI
//! ([Namespaces in XML](https://www.w3.org/TR/xml-names/)). They never show up as
//! events, because strict mode has no `NS` event. `xsi:type` and `xsi:nil` become
//! their own events and come first
//! ([EXI 4](https://www.w3.org/TR/exi/#streams)). The other attributes are sorted
//! by local name, then by namespace URI
//! ([EXI 8.5.4.3](https://www.w3.org/TR/exi/#eventCodeAssignment)).
//!
//! Text is joined across entity references and CDATA sections. Line endings become
//! `\n` ([XML 1.0 section 2.11](https://www.w3.org/TR/xml/#sec-line-ends)), and
//! whitespace in attribute values is normalized
//! ([section 3.3.3](https://www.w3.org/TR/xml/#AVNormalize)). A conforming XML
//! parser does both before the application sees the data. Text that is only
//! whitespace is kept, because the encoder knows from the grammar whether it
//! matters.

use std::io::Write;

use quick_xml::escape::unescape;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;
use rexi_core::event::Event as ExiEvent;
use rexi_core::grammar::well_known::XSI_NS;
use rexi_core::value::Value;
use rexi_core::{Encoder, GrammarTables, OwnedEvent, OwnedValue, QName as ExiQName};

use crate::{Result, XmlError};

/// Parses a whole XML document into owned events: `SD`, the element tree, `ED`.
/// The whole list is held in memory.
pub fn parse_events(xml: &[u8]) -> Result<Vec<OwnedEvent>> {
    let mut events = Vec::new();
    parse_into(xml, |ev| {
        events.push(ev);
        Ok(())
    })?;
    Ok(events)
}

/// Streams the events of `xml` straight into `enc`, without holding the whole
/// document as events.
///
/// Only one element's attributes are held at a time, because they must be sorted
/// before they are sent.
pub fn push_xml<W: Write, G: GrammarTables>(xml: &[u8], enc: &mut Encoder<W, G>) -> Result<()> {
    parse_into(xml, |ev| {
        enc.push(&as_lexical_event(&ev)).map_err(XmlError::from)
    })
}

/// Borrows an event for the encoder, with text values turned into
/// [`Value::Lexical`]. That way the grammar's datatype decides how the text is
/// parsed. A `Value::String` would be rejected where the grammar wants a boolean
/// or an integer. Values that are not strings pass through as they are.
pub fn as_lexical_event(ev: &OwnedEvent) -> ExiEvent<'_> {
    fn lexical(v: &OwnedValue) -> Value<'_> {
        match v {
            OwnedValue::String(s) => Value::Lexical(s),
            other => other.as_value(),
        }
    }
    match ev {
        OwnedEvent::Attribute { uri, local, value } => {
            ExiEvent::Attribute(ExiQName::new(uri, local), lexical(value))
        }
        OwnedEvent::Characters(value) => ExiEvent::Characters(lexical(value)),
        other => other.as_event(),
    }
}

/// The namespace URI a prefix resolved to. A name with no prefix and no default
/// namespace gives `""`. An undeclared prefix is an error.
fn uri_of(resolution: ResolveResult<'_>) -> Result<String> {
    match resolution {
        ResolveResult::Unbound => Ok(String::new()),
        ResolveResult::Bound(ns) => Ok(ns.as_ref().to_string()),
        ResolveResult::Unknown(prefix) => Err(XmlError::Unsupported(format!(
            "undeclared namespace prefix {prefix}"
        ))),
    }
}

/// Attribute-value normalization for CDATA-typed attributes
/// ([XML 1.0 section 3.3.3](https://www.w3.org/TR/xml/#AVNormalize)): line ends are
/// normalized first ([section 2.11](https://www.w3.org/TR/xml/#sec-line-ends), so
/// a CR LF pair is one line end), then each literal tab, newline and carriage return
/// becomes a space. So `x<CR><LF>y` becomes `x y`, with one space, as EXIficient
/// writes it. Character references (`&#10;`) are still escaped at this point, so
/// they are not touched. After that the value is unescaped, which turns references
/// into the characters they stand for.
fn normalise_attribute(raw: &str) -> Result<String> {
    let replaced: String = normalise_newlines(raw)
        .chars()
        .map(|c| {
            if matches!(c, '\t' | '\n' | '\r') {
                ' '
            } else {
                c
            }
        })
        .collect();
    Ok(unescape(&replaced)?.into_owned())
}

/// [XML 1.0 section 2.11](https://www.w3.org/TR/xml/#sec-line-ends): `\r\n` and a
/// lone `\r` become `\n`.
fn normalise_newlines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_nil(value: &str) -> Result<bool> {
    match value.trim_matches([' ', '\t', '\n', '\r']) {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(XmlError::Unsupported(format!(
            "xsi:nil value {other:?} is not a boolean"
        ))),
    }
}

/// The attribute events of one start tag, in grammar order: `xsi:type`, then
/// `xsi:nil`, then the rest sorted by local name and then URI. Comparing Rust
/// strings byte by byte gives Unicode code point order, which is the order the
/// spec asks for ([EXI 1.2](https://www.w3.org/TR/exi/#conventions)).
///
/// The `xsi:type` value is a qname, a name that may have a prefix. Its prefix is
/// resolved the same way as an element name's, so a name with no prefix takes the
/// default namespace. `xsi:nil` accepts `true`, `1`, `false` and `0`, with
/// surrounding whitespace allowed.
fn attribute_events(reader: &NsReader<&[u8]>, start: &BytesStart<'_>) -> Result<Vec<OwnedEvent>> {
    let mut xsi_type = None;
    let mut xsi_nil = None;
    let mut plain: Vec<(String, String, String)> = Vec::new();
    for attr in start.attributes() {
        let attr: Attribute<'_> = attr?;
        if attr.key.as_namespace_binding().is_some() {
            continue;
        }
        let (resolution, local) = reader.resolver().resolve_attribute(attr.key);
        let uri = uri_of(resolution)?;
        let local = local.as_ref().to_string();
        let value = normalise_attribute(&attr.value)?;
        if uri == XSI_NS {
            match local.as_str() {
                "type" => {
                    let qname = value.trim_matches([' ', '\t', '\n', '\r']);
                    let (resolution, local) = reader.resolver().resolve_element(QName(qname));
                    let uri = uri_of(resolution)?;
                    let local = local.as_ref().to_string();
                    xsi_type = Some(OwnedEvent::XsiType { uri, local });
                    continue;
                }
                "nil" => {
                    xsi_nil = Some(OwnedEvent::XsiNil(parse_nil(&value)?));
                    continue;
                }
                _ => {}
            }
        }
        plain.push((local, uri, value));
    }
    plain.sort_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())));
    let mut events = Vec::with_capacity(plain.len() + 2);
    events.extend(xsi_type);
    events.extend(xsi_nil);
    events.extend(
        plain
            .into_iter()
            .map(|(local, uri, value)| OwnedEvent::Attribute {
                uri,
                local,
                value: OwnedValue::String(value),
            }),
    );
    Ok(events)
}

/// The streaming core: parses `xml` and hands every event to `sink` in order.
///
/// It checks that there is exactly one root element, no text outside it other than
/// whitespace, and no DOCTYPE. Comments, processing instructions and the XML
/// declaration are dropped. An empty-element tag (`<a/>`) gives an `SE` and an `EE`
/// right away.
fn parse_into(xml: &[u8], mut sink: impl FnMut(OwnedEvent) -> Result<()>) -> Result<()> {
    let mut reader = NsReader::from_reader(xml);
    let mut depth = 0usize;
    let mut seen_root = false;
    let mut text = String::new();
    let mut has_text = false;

    sink(OwnedEvent::StartDocument)?;
    loop {
        let (resolution, event) = reader.read_resolved_event()?;
        // Resolve now: the resolution borrows the reader, and the attribute step
        // needs the reader again.
        let uri = uri_of(resolution)?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(start) | Event::Empty(start) => {
                flush_text(&mut text, &mut has_text, depth, &mut sink)?;
                if depth == 0 {
                    if seen_root {
                        return Err(XmlError::Unsupported("more than one root element".into()));
                    }
                    seen_root = true;
                }
                let local = start.local_name().as_ref().to_string();
                sink(OwnedEvent::StartElement { uri, local })?;
                for attr in attribute_events(&reader, &start)? {
                    sink(attr)?;
                }
                if empty {
                    sink(OwnedEvent::EndElement)?;
                } else {
                    depth += 1;
                }
            }
            Event::End(_) => {
                flush_text(&mut text, &mut has_text, depth, &mut sink)?;
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| XmlError::Unsupported("unexpected end tag".into()))?;
                sink(OwnedEvent::EndElement)?;
            }
            // Text never holds references: they arrive as `GeneralRef` events and
            // are joined in here. Line endings are normalized only in the literal
            // text, so `&#13;` still comes out as a carriage return.
            Event::Text(t) => {
                text.push_str(&normalise_newlines(&t.into_inner()));
                has_text = true;
            }
            Event::CData(c) => {
                text.push_str(&normalise_newlines(&c.into_inner()));
                has_text = true;
            }
            Event::GeneralRef(r) => {
                // Character references and the five predefined entities (`&lt;`,
                // `&gt;`, `&amp;`, `&apos;`, `&quot;`). Any other entity would need a
                // DTD, which strict EXI can not carry.
                text.push_str(&unescape(&format!("&{};", &*r))?);
                has_text = true;
            }
            Event::DocType(_) => {
                return Err(XmlError::Unsupported("DOCTYPE declarations".into()));
            }
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
            Event::Eof => break,
        }
    }
    flush_text(&mut text, &mut has_text, depth, &mut sink)?;
    if depth != 0 {
        return Err(XmlError::Unsupported(
            "document ends inside an element".into(),
        ));
    }
    if !seen_root {
        return Err(XmlError::Unsupported("document has no root element".into()));
    }
    sink(OwnedEvent::EndDocument)
}

/// Sends the joined text collected so far as one `CH` event. Outside the root
/// element only whitespace is allowed, and it is dropped.
fn flush_text(
    text: &mut String,
    has_text: &mut bool,
    depth: usize,
    sink: &mut impl FnMut(OwnedEvent) -> Result<()>,
) -> Result<()> {
    if !*has_text {
        return Ok(());
    }
    *has_text = false;
    let value = std::mem::take(text);
    if depth == 0 {
        if value.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r')) {
            return Ok(());
        }
        return Err(XmlError::Unsupported(
            "character data outside the root element".into(),
        ));
    }
    sink(OwnedEvent::Characters(OwnedValue::String(value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn se(uri: &str, local: &str) -> OwnedEvent {
        OwnedEvent::StartElement {
            uri: uri.into(),
            local: local.into(),
        }
    }

    fn at(uri: &str, local: &str, value: &str) -> OwnedEvent {
        OwnedEvent::Attribute {
            uri: uri.into(),
            local: local.into(),
            value: OwnedValue::String(value.into()),
        }
    }

    fn ch(value: &str) -> OwnedEvent {
        OwnedEvent::Characters(OwnedValue::String(value.into()))
    }

    #[test]
    fn attributes_are_sorted_and_xsi_first() {
        let xml = br#"<r xmlns="u" xmlns:x="http://www.w3.org/2001/XMLSchema-instance" xmlns:o="v"
                        z="1" x:nil="false" o:a="3" a="2" x:type="o:T"/>"#;
        let events = parse_events(xml).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            events,
            vec![
                OwnedEvent::StartDocument,
                se("u", "r"),
                OwnedEvent::XsiType {
                    uri: "v".into(),
                    local: "T".into()
                },
                OwnedEvent::XsiNil(false),
                at("", "a", "2"),
                at("v", "a", "3"),
                at("", "z", "1"),
                OwnedEvent::EndElement,
                OwnedEvent::EndDocument,
            ]
        );
    }

    #[test]
    fn xsi_type_without_prefix_uses_default_namespace() {
        let xml =
            br#"<r xmlns="u" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="T"/>"#;
        let events = parse_events(xml).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            events[2],
            OwnedEvent::XsiType {
                uri: "u".into(),
                local: "T".into()
            }
        );
    }

    #[test]
    fn text_is_merged_and_unescaped() {
        let xml = b"<r a=\"x &amp; y&#10;z\ttab\">one&lt;<![CDATA[<two>]]>&#x26;three\r\nfour</r>";
        let events = parse_events(xml).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            events,
            vec![
                OwnedEvent::StartDocument,
                se("", "r"),
                at("", "a", "x & y\nz tab"),
                ch("one<<two>&three\nfour"),
                OwnedEvent::EndElement,
                OwnedEvent::EndDocument,
            ]
        );
    }

    #[test]
    fn attribute_line_ends_become_one_space() {
        // A literal CR LF in an attribute value is one line end, so one space
        // (XML 1.0 sections 2.11 and 3.3.3). A lone CR is one space too; LF then
        // tab is two. `&#13;&#10;` are references and stay as written.
        let xml = b"<r a=\"x\r\ny\" b=\"x\ry\" c=\"x\n\ty\" d=\"x&#13;&#10;y\"/>";
        let events = parse_events(xml).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            &events[2..6],
            [
                at("", "a", "x y"),
                at("", "b", "x y"),
                at("", "c", "x  y"),
                at("", "d", "x\r\ny"),
            ]
        );
    }

    #[test]
    fn character_references_skip_newline_normalisation() {
        // XML 1.0 section 2.11 applies only to literal line breaks: `&#13;` and
        // `&#xD;&#xA;` are data and must come through as written.
        let xml = b"<r>a&#13;b&#xD;&#xA;c\r\nd</r>";
        let events = parse_events(xml).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(events[2], ch("a\rb\r\nc\nd"));
        // Entities other than the predefined five need a DTD.
        assert!(parse_events(b"<r>&nbsp;</r>").is_err());
    }

    #[test]
    fn whitespace_text_is_kept_inside_and_dropped_outside() {
        let xml = b"<?xml version=\"1.0\"?>\n<!-- c -->\n<r>\n  <a/>\n</r>\n";
        let events = parse_events(xml).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            events,
            vec![
                OwnedEvent::StartDocument,
                se("", "r"),
                ch("\n  "),
                se("", "a"),
                OwnedEvent::EndElement,
                ch("\n"),
                OwnedEvent::EndElement,
                OwnedEvent::EndDocument,
            ]
        );
    }

    #[test]
    fn rejects_doctype_unknown_prefix_and_malformed() {
        assert!(matches!(
            parse_events(b"<!DOCTYPE r><r/>"),
            Err(XmlError::Unsupported(_))
        ));
        assert!(matches!(
            parse_events(b"<p:r/>"),
            Err(XmlError::Unsupported(_))
        ));
        assert!(parse_events(b"<r><a></r>").is_err());
        assert!(parse_events(b"<r/><r/>").is_err());
        assert!(parse_events(b"").is_err());
        assert!(parse_events(b"<r>x</r>y").is_err());
    }
}
