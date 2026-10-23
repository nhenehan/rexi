//! EXI events to XML text, and the decoder driver that produces the events.
//!
//! Strict EXI streams carry no namespace prefixes: strict mode leaves them out
//! ([EXI 5.4](https://www.w3.org/TR/exi/#options), and the prefix fidelity option
//! in [EXI 6.3](https://www.w3.org/TR/exi/#fidelityOptions)). So the writer makes
//! up its own ([Namespaces in XML](https://www.w3.org/TR/xml-names/)).
//!
//! It declares the root element's namespace as the default namespace. Every other
//! namespace in use gets a prefix `ns1`, `ns2`, ... in the order first seen. `xsi`
//! is declared when `xsi:type` or `xsi:nil` appears. All of these are declared on
//! the root element, so the writer never has to track where a prefix is in scope.
//! Attributes never take the default namespace (that is a Namespaces in XML rule),
//! so an attribute in a namespace always gets a prefix, even in the root's
//! namespace.
//!
//! An `xsi:type` value is a QName, and one with no prefix means a type in the
//! default namespace in scope
//! ([XML Schema 1.0 Part 2, QName](https://www.w3.org/TR/xmlschema-2/#QName)). So:
//!
//! * a type in a namespace that is not the default in scope is written with a
//!   prefix, and gets one (even the root's namespace) when it needs it;
//! * a type in no namespace needs a scope with no default namespace. The element
//!   that carries it then declares `xmlns=""`, and is itself written with a prefix
//!   if it is in a namespace. On the root, the default namespace is simply not
//!   declared.

use std::fmt::Write as _;
use std::io::Read;

use rexi_core::grammar::well_known::XSI_NS;
use rexi_core::{DecodeLimits, Decoder, ExiOptions, GrammarTables, OwnedEvent, OwnedValue};

use crate::dump::plain_format;
use crate::{Result, XmlError};

const XSI_PREFIX: &str = "xsi";

/// Escapes text content. `\r` is written as `&#13;` so that the reader's
/// line-ending step does not turn it into `\n`.
fn escape_text(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
}

/// Escapes an attribute value so that it comes through attribute-value
/// normalization unchanged. Tab, newline and carriage return are written as
/// character references, because literal ones would be turned into spaces
/// ([XML 1.0 section 3.3.3](https://www.w3.org/TR/xml/#AVNormalize)).
fn escape_attribute(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
}

/// The namespaces one document uses, the prefix picked for each, and how every
/// start tag and `xsi:type` value is written. Built from all events before any XML
/// is written.
struct Namespaces {
    default: String,
    /// `(uri, prefix)` pairs, in the order they are declared.
    prefixed: Vec<(String, String)>,
    xsi: bool,
    layout: Layout,
}

/// How each start tag and each `xsi:type` value is written, in event order.
#[derive(Default)]
struct Layout {
    /// For each `StartElement`: the written name, and the default namespace the tag
    /// declares with `xmlns="..."`, if any. On the root, `Some("")` means the
    /// default namespace is left undeclared.
    tags: Vec<(String, Option<String>)>,
    /// For each `XsiType`: the written type name.
    types: Vec<String>,
}

impl Namespaces {
    fn collect(events: &[OwnedEvent]) -> Self {
        let default = events
            .iter()
            .find_map(|e| match e {
                OwnedEvent::StartElement { uri, .. } => Some(uri.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let mut ns = Self {
            default,
            prefixed: Vec::new(),
            xsi: false,
            layout: Layout::default(),
        };
        for ev in events {
            match ev {
                OwnedEvent::StartElement { uri, .. } if !uri.is_empty() && *uri != ns.default => {
                    ns.prefix_for(uri);
                }
                OwnedEvent::Attribute { uri, .. } if !uri.is_empty() => {
                    if uri == XSI_NS {
                        ns.xsi = true;
                    } else {
                        ns.prefix_for(uri);
                    }
                }
                OwnedEvent::XsiType { uri, .. } => {
                    ns.xsi = true;
                    if !uri.is_empty() && *uri != ns.default {
                        ns.prefix_for(uri);
                    }
                }
                OwnedEvent::XsiNil(_) => ns.xsi = true,
                _ => {}
            }
        }
        // Each missing prefix found by the layout is added and the layout is redone.
        // This ends, because a namespace gets a prefix at most once.
        loop {
            match ns.plan(events) {
                Ok(layout) => {
                    ns.layout = layout;
                    return ns;
                }
                Err(uri) => {
                    ns.prefix_for(&uri);
                }
            }
        }
    }

    /// Works out every start tag and `xsi:type` value, tracking the default
    /// namespace in scope the same way [`write_xml`] does. Fails with the namespace
    /// URI that needs a prefix, when one is missing.
    fn plan(&self, events: &[OwnedEvent]) -> std::result::Result<Layout, String> {
        let mut layout = Layout::default();
        // The default namespace in scope inside each open element, as a stack.
        let mut scopes: Vec<String> = Vec::new();
        for (i, ev) in events.iter().enumerate() {
            match ev {
                OwnedEvent::StartElement { uri, local } => {
                    let current = scopes.last().map_or(self.default.as_str(), String::as_str);
                    let (mut name, mut redeclare) = self.element_name(uri, local, current);
                    let scope_inside = redeclare.as_deref().unwrap_or(current);
                    // The events right after a start element are its xsi:type,
                    // xsi:nil and attributes.
                    let untyped_ns = events[i + 1..]
                        .iter()
                        .take_while(|e| {
                            matches!(
                                e,
                                OwnedEvent::XsiType { .. }
                                    | OwnedEvent::XsiNil(_)
                                    | OwnedEvent::Attribute { .. }
                            )
                        })
                        .any(|e| matches!(e, OwnedEvent::XsiType { uri, .. } if uri.is_empty()));
                    if untyped_ns && !scope_inside.is_empty() {
                        // The type has no namespace, so this element turns the default
                        // namespace off and can no longer use it for its own name.
                        if !uri.is_empty() {
                            let prefix = self.existing_prefix(uri).ok_or_else(|| uri.clone())?;
                            name = format!("{prefix}:{local}");
                        }
                        redeclare = Some(String::new());
                    }
                    let inner = redeclare.clone().unwrap_or_else(|| current.to_string());
                    layout.tags.push((name, redeclare));
                    scopes.push(inner);
                }
                OwnedEvent::EndElement => {
                    scopes.pop();
                }
                OwnedEvent::XsiType { uri, local } => {
                    let current = scopes.last().map_or(self.default.as_str(), String::as_str);
                    // A type in no namespace is always in a scope with no default
                    // namespace: its element took care of that above.
                    let name = if uri == current || uri.is_empty() {
                        local.clone()
                    } else {
                        let prefix = self.existing_prefix(uri).ok_or_else(|| uri.clone())?;
                        format!("{prefix}:{local}")
                    };
                    layout.types.push(name);
                }
                _ => {}
            }
        }
        Ok(layout)
    }

    fn prefix_for(&mut self, uri: &str) -> &str {
        if let Some(pos) = self.prefixed.iter().position(|(u, _)| u == uri) {
            return &self.prefixed[pos].1;
        }
        let prefix = format!("ns{}", self.prefixed.len() + 1);
        self.prefixed.push((uri.to_string(), prefix));
        &self.prefixed[self.prefixed.len() - 1].1
    }

    fn existing_prefix(&self, uri: &str) -> Option<&str> {
        if uri == XSI_NS {
            return Some(XSI_PREFIX);
        }
        self.prefixed
            .iter()
            .find(|(u, _)| u == uri)
            .map(|(_, p)| p.as_str())
    }

    /// The written name of an element, given the default namespace in scope. Also
    /// returns the new default namespace the element must declare, if any.
    /// Namespaces that have no prefix (no namespace at all, or the root's) are
    /// switched to with `xmlns="..."` on the element.
    fn element_name(
        &self,
        uri: &str,
        local: &str,
        current_default: &str,
    ) -> (String, Option<String>) {
        if uri == current_default {
            return (local.to_string(), None);
        }
        match self.existing_prefix(uri) {
            Some(p) => (format!("{p}:{local}"), None),
            None => (local.to_string(), Some(uri.to_string())),
        }
    }

    /// The written name of an attribute: always with a prefix when it is in a
    /// namespace.
    fn attribute_name(&self, uri: &str, local: &str) -> String {
        if uri.is_empty() {
            return local.to_string();
        }
        match self.existing_prefix(uri) {
            Some(p) => format!("{p}:{local}"),
            None => local.to_string(),
        }
    }

    /// The root element's declarations. `with_default` is false when the root
    /// leaves the default namespace undeclared (see [`Layout::tags`]).
    fn declarations(&self, out: &mut String, with_default: bool) {
        if with_default && !self.default.is_empty() {
            out.push_str(" xmlns=\"");
            escape_attribute(out, &self.default);
            out.push('"');
        }
        if self.xsi {
            let _ = write!(out, " xmlns:{XSI_PREFIX}=\"{XSI_NS}\"");
        }
        for (uri, prefix) in &self.prefixed {
            let _ = write!(out, " xmlns:{prefix}=\"");
            escape_attribute(out, uri);
            out.push('"');
        }
    }
}

/// Writes events as compact XML: no XML declaration and no indentation. An element
/// with no content is written as an empty-element tag (`<a/>`).
///
/// `formatter` turns attribute and character values into text. Values that are
/// already strings go through it too. Use [`plain_format`] when no grammar is at
/// hand.
pub fn write_xml(events: &[OwnedEvent], formatter: impl Fn(&OwnedValue) -> String) -> String {
    let ns = Namespaces::collect(events);
    let mut out = String::new();
    // The end-tag names of the open elements, as a stack.
    let mut open: Vec<String> = Vec::new();
    let mut tag_pending = false;
    let mut tags = ns.layout.tags.iter();
    let mut types = ns.layout.types.iter();

    let close_tag = |out: &mut String, tag_pending: &mut bool| {
        if *tag_pending {
            out.push('>');
            *tag_pending = false;
        }
    };

    for ev in events {
        match ev {
            OwnedEvent::StartDocument | OwnedEvent::EndDocument => {}
            OwnedEvent::StartElement { .. } => {
                close_tag(&mut out, &mut tag_pending);
                let is_root = open.is_empty();
                // `Namespaces::plan` made one entry per start element.
                let Some((name, redeclare)) = tags.next() else {
                    break;
                };
                out.push('<');
                out.push_str(name);
                if is_root {
                    ns.declarations(&mut out, redeclare.is_none());
                } else if let Some(default) = redeclare {
                    out.push_str(" xmlns=\"");
                    escape_attribute(&mut out, default);
                    out.push('"');
                }
                open.push(name.clone());
                tag_pending = true;
            }
            OwnedEvent::EndElement => {
                if tag_pending {
                    out.push_str("/>");
                    tag_pending = false;
                    open.pop();
                } else if let Some(name) = open.pop() {
                    out.push_str("</");
                    out.push_str(&name);
                    out.push('>');
                }
            }
            OwnedEvent::Attribute { uri, local, value } => {
                out.push(' ');
                out.push_str(&ns.attribute_name(uri, local));
                out.push_str("=\"");
                escape_attribute(&mut out, &formatter(value));
                out.push('"');
            }
            OwnedEvent::XsiType { .. } => {
                out.push_str(" xsi:type=\"");
                // `Namespaces::plan` made one entry per xsi:type.
                if let Some(name) = types.next() {
                    escape_attribute(&mut out, name);
                }
                out.push('"');
            }
            OwnedEvent::XsiNil(nil) => {
                let _ = write!(out, " xsi:nil=\"{nil}\"");
            }
            OwnedEvent::Characters(value) => {
                close_tag(&mut out, &mut tag_pending);
                escape_text(&mut out, &formatter(value));
            }
            // An event kind added to rexi-core later has no XML form here.
            _ => {}
        }
    }
    out
}

/// Pulls every event out of `input` and passes each value through `map`. `map` also
/// gets the decoder, so it can ask for the value's standard text form.
fn drain<R: Read, G: GrammarTables>(
    input: R,
    tables: G,
    out_of_band: Option<ExiOptions>,
    limits: DecodeLimits,
    mut map: impl FnMut(&Decoder<R, G>, OwnedValue) -> Result<OwnedValue>,
) -> Result<Vec<OwnedEvent>> {
    let mut dec = Decoder::with_limits(input, tables, out_of_band, limits)?;
    let mut events = Vec::new();
    loop {
        let owned = {
            let Some(ev) = dec.next_event()? else { break };
            match OwnedEvent::from_event(&ev) {
                Some(owned) => owned,
                None => {
                    return Err(XmlError::Unsupported(format!(
                        "decoder produced {} event",
                        ev.kind_name()
                    )));
                }
            }
        };
        let owned = match owned {
            OwnedEvent::Attribute { uri, local, value } => OwnedEvent::Attribute {
                uri,
                local,
                value: map(&dec, value)?,
            },
            OwnedEvent::Characters(value) => OwnedEvent::Characters(map(&dec, value)?),
            other => other,
        };
        events.push(owned);
    }
    Ok(events)
}

/// Decodes a stream into events whose values are strings in their standard text
/// form (`OwnedValue::String`). That is the same form [`crate::parse_events`] gives
/// for the matching XML, so the two can be compared. Uses the default
/// [`DecodeLimits`].
pub fn decode_events<R: Read, G: GrammarTables>(
    input: R,
    tables: G,
    out_of_band: Option<ExiOptions>,
) -> Result<Vec<OwnedEvent>> {
    decode_events_with_limits(input, tables, out_of_band, DecodeLimits::default())
}

/// Like [`decode_events`], with the given limits on what the stream may ask for.
pub fn decode_events_with_limits<R: Read, G: GrammarTables>(
    input: R,
    tables: G,
    out_of_band: Option<ExiOptions>,
    limits: DecodeLimits,
) -> Result<Vec<OwnedEvent>> {
    drain(input, tables, out_of_band, limits, |dec, value| {
        Ok(OwnedValue::String(dec.format_last(&value.as_value())?))
    })
}

/// Decodes a stream into events, keeping the decoder's typed values. Uses the
/// default [`DecodeLimits`].
pub fn decode_events_typed<R: Read, G: GrammarTables>(
    input: R,
    tables: G,
    out_of_band: Option<ExiOptions>,
) -> Result<Vec<OwnedEvent>> {
    decode_events_typed_with_limits(input, tables, out_of_band, DecodeLimits::default())
}

/// Like [`decode_events_typed`], with the given limits on what the stream may ask
/// for.
pub fn decode_events_typed_with_limits<R: Read, G: GrammarTables>(
    input: R,
    tables: G,
    out_of_band: Option<ExiOptions>,
    limits: DecodeLimits,
) -> Result<Vec<OwnedEvent>> {
    drain(input, tables, out_of_band, limits, |_, value| Ok(value))
}

/// Decodes a stream straight to XML text, with values in their standard text
/// form. Uses the default [`DecodeLimits`].
pub fn decode_to_xml<R: Read, G: GrammarTables>(
    input: R,
    tables: G,
    out_of_band: Option<ExiOptions>,
) -> Result<String> {
    decode_to_xml_with_limits(input, tables, out_of_band, DecodeLimits::default())
}

/// Like [`decode_to_xml`], with the given limits on what the stream may ask for.
pub fn decode_to_xml_with_limits<R: Read, G: GrammarTables>(
    input: R,
    tables: G,
    out_of_band: Option<ExiOptions>,
    limits: DecodeLimits,
) -> Result<String> {
    let events = decode_events_with_limits(input, tables, out_of_band, limits)?;
    Ok(write_xml(&events, plain_format))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_events;

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

    #[test]
    fn namespaces_prefixes_and_empty_elements() {
        let events = vec![
            OwnedEvent::StartDocument,
            se("urn:a", "root"),
            at("", "id", "1"),
            at("urn:a", "own", "2"),
            se("urn:b", "child"),
            OwnedEvent::XsiNil(true),
            OwnedEvent::EndElement,
            se("urn:c", "other"),
            OwnedEvent::XsiType {
                uri: "urn:a".into(),
                local: "T".into(),
            }
            .clone(),
            OwnedEvent::Characters(OwnedValue::String("a<b&c".into())),
            OwnedEvent::EndElement,
            se("", "plain"),
            OwnedEvent::EndElement,
            OwnedEvent::EndElement,
            OwnedEvent::EndDocument,
        ];
        let xml = write_xml(&events, plain_format);
        assert_eq!(
            xml,
            "<root xmlns=\"urn:a\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
             xmlns:ns1=\"urn:a\" xmlns:ns2=\"urn:b\" xmlns:ns3=\"urn:c\" id=\"1\" ns1:own=\"2\">\
             <ns2:child xsi:nil=\"true\"/><ns3:other xsi:type=\"T\">a&lt;b&amp;c</ns3:other>\
             <plain xmlns=\"\"/></root>"
        );
        // And parsing it again gives the same events.
        assert_eq!(parse_events(xml.as_bytes()).unwrap_or_default(), events);
    }

    fn xsi_type(uri: &str, local: &str) -> OwnedEvent {
        OwnedEvent::XsiType {
            uri: uri.into(),
            local: local.into(),
        }
    }

    /// An unprefixed `xsi:type` value means a type in the default namespace in scope,
    /// so a type in no namespace needs a scope with no default namespace.
    #[test]
    fn xsi_type_in_no_namespace_under_a_default_namespace() {
        let events = vec![
            OwnedEvent::StartDocument,
            se("urn:a", "root"),
            xsi_type("", "T"),
            se("urn:a", "child"),
            xsi_type("", "U"),
            se("urn:a", "leaf"),
            OwnedEvent::EndElement,
            OwnedEvent::EndElement,
            OwnedEvent::EndElement,
            OwnedEvent::EndDocument,
        ];
        let xml = write_xml(&events, plain_format);
        assert_eq!(
            xml,
            "<ns1:root xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
             xmlns:ns1=\"urn:a\" xsi:type=\"T\"><ns1:child xsi:type=\"U\">\
             <ns1:leaf/></ns1:child></ns1:root>"
        );
        assert_eq!(parse_events(xml.as_bytes()).unwrap_or_default(), events);
    }

    /// A type in the root's namespace, used where `xmlns=""` has turned the default
    /// namespace off, needs a prefix.
    #[test]
    fn xsi_type_in_the_root_namespace_under_no_default_namespace() {
        let events = vec![
            OwnedEvent::StartDocument,
            se("urn:a", "root"),
            se("", "plain"),
            xsi_type("urn:a", "T"),
            OwnedEvent::EndElement,
            se("urn:a", "back"),
            xsi_type("urn:a", "T"),
            OwnedEvent::EndElement,
            OwnedEvent::EndElement,
            OwnedEvent::EndDocument,
        ];
        let xml = write_xml(&events, plain_format);
        assert_eq!(
            xml,
            "<root xmlns=\"urn:a\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
             xmlns:ns1=\"urn:a\"><plain xmlns=\"\" xsi:type=\"ns1:T\"/>\
             <back xsi:type=\"T\"/></root>"
        );
        assert_eq!(parse_events(xml.as_bytes()).unwrap_or_default(), events);
    }

    #[test]
    fn attribute_escaping_survives_reparsing() {
        let events = vec![
            OwnedEvent::StartDocument,
            se("", "r"),
            at("", "a", "tab\tnl\ncr\rq\"lt<amp&"),
            OwnedEvent::Characters(OwnedValue::String("x\ny>z".into())),
            OwnedEvent::EndElement,
            OwnedEvent::EndDocument,
        ];
        let xml = write_xml(&events, plain_format);
        assert_eq!(
            xml,
            "<r a=\"tab&#9;nl&#10;cr&#13;q&quot;lt&lt;amp&amp;\">x\ny&gt;z</r>"
        );
        assert_eq!(parse_events(xml.as_bytes()).unwrap_or_default(), events);
    }
}
