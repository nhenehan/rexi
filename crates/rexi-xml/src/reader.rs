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
//!
//! The parser hands out borrowed events. Names point into the XML text, and
//! values point into scratch buffers that are reused from one element to the
//! next, so after the first few elements parsing allocates nothing.

use std::io::Write;
use std::ops::Range;

use quick_xml::escape::unescape;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::name::{PrefixDeclaration, QName};
use quick_xml::reader::Reader;
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
        events.extend(OwnedEvent::from_event(ev));
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
    parse_into(xml, |ev| enc.push(ev).map_err(XmlError::from))
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

/// The namespace bound to the `xml` prefix, which never needs declaring
/// ([Namespaces in XML 1.0 section 3](https://www.w3.org/TR/xml-names/#ns-decl)).
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// The namespace declarations in scope
/// ([Namespaces in XML 1.0 section 6](https://www.w3.org/TR/xml-names/#scoping)).
///
/// Declarations are kept as a stack: an element's own come last and are dropped
/// at its end tag, and a later one hides an earlier one for the same prefix. The
/// prefix and URI text sits in one buffer, so declaring allocates nothing once
/// the buffer has grown.
#[derive(Default)]
struct Namespaces {
    text: String,
    /// (prefix, URI) places in `text`. The default namespace has an empty prefix.
    bindings: Vec<(Range<usize>, Range<usize>)>,
    /// How many bindings there were when each open element started.
    scopes: Vec<usize>,
}

impl Namespaces {
    /// Starts a new element's scope. Its declarations follow with `bind`.
    fn open(&mut self) {
        self.scopes.push(self.bindings.len());
    }

    /// Declares `prefix` (empty for the default namespace) on the element whose
    /// scope is open. Declaring the same prefix twice on one element is an error.
    fn bind(&mut self, prefix: &str, uri: &str) -> Result<()> {
        let scope = self.scopes.last().copied().unwrap_or(0);
        if self.bindings[scope..]
            .iter()
            .any(|(p, _)| &self.text[p.clone()] == prefix)
        {
            return Err(XmlError::Unsupported(format!(
                "namespace prefix {prefix:?} declared twice on one element"
            )));
        }
        let prefix = push_range(&mut self.text, prefix);
        let uri = push_range(&mut self.text, uri);
        self.bindings.push((prefix, uri));
        Ok(())
    }

    /// Ends the innermost element's scope and drops its declarations.
    fn close(&mut self) {
        if let Some(n) = self.scopes.pop() {
            self.bindings.truncate(n);
            self.text
                .truncate(self.bindings.last().map_or(0, |(_, uri)| uri.end));
        }
    }

    /// The namespace URI for `prefix`. No prefix means the default namespace,
    /// which is `""` when none is declared (or `xmlns=""` undeclared it). An
    /// undeclared prefix is an error.
    fn uri(&self, prefix: Option<&str>) -> Result<&str> {
        let Some(prefix) = prefix else {
            // Most names have no prefix: find the default namespace without
            // comparing any text.
            let default = self.bindings.iter().rev().find(|(p, _)| p.is_empty());
            return Ok(default.map_or("", |(_, uri)| &self.text[uri.clone()]));
        };
        if prefix == "xml" {
            return Ok(XML_NS);
        }
        for (p, uri) in self.bindings.iter().rev() {
            if &self.text[p.clone()] == prefix {
                return Ok(&self.text[uri.clone()]);
            }
        }
        if prefix.is_empty() {
            Ok("")
        } else {
            Err(XmlError::Unsupported(format!(
                "undeclared namespace prefix {prefix}"
            )))
        }
    }

    /// The URI and local name of an element name, or of an `xsi:type` value: a
    /// name with no prefix takes the default namespace.
    fn element<'n>(&self, name: &'n str) -> Result<(&str, &'n str)> {
        let (local, prefix) = QName(name).decompose();
        Ok((
            self.uri(prefix.as_ref().map(|p| p.as_ref()))?,
            local.into_inner(),
        ))
    }
}

/// Appends `text` to `out` with line ends normalized
/// ([XML 1.0 section 2.11](https://www.w3.org/TR/xml/#sec-line-ends)): `\r\n`
/// and a lone `\r` become `\n`.
fn push_normalised(out: &mut String, text: &str) {
    let mut rest = text;
    while let Some(i) = rest.find('\r') {
        out.push_str(&rest[..i]);
        out.push('\n');
        rest = &rest[i + 1..];
        if let Some(after) = rest.strip_prefix('\n') {
            rest = after;
        }
    }
    out.push_str(rest);
}

/// Appends an attribute value to `out`, normalized for a CDATA-typed attribute
/// ([XML 1.0 section 3.3.3](https://www.w3.org/TR/xml/#AVNormalize)). Line ends
/// are normalized first ([section 2.11](https://www.w3.org/TR/xml/#sec-line-ends),
/// so a CR LF pair is one line end). Then each literal tab, newline and carriage
/// return becomes a space. So `x<CR><LF>y` becomes `x y`, with one space, as
/// EXIficient writes it. Character references (`&#10;`) are still escaped at this
/// point, so they are not touched. After that the value is unescaped, which turns
/// references into the characters they stand for.
fn push_attribute_value(out: &mut String, raw: &str) -> Result<()> {
    let start = out.len();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\t' | '\n' => out.push(' '),
            c => out.push(c),
        }
    }
    if out[start..].contains('&') {
        let unescaped = unescape(&out[start..])?.into_owned();
        out.truncate(start);
        out.push_str(&unescaped);
    }
    Ok(())
}

/// Appends the text an entity or character reference stands for. Only character
/// references and the five predefined entities (`&lt;`, `&gt;`, `&amp;`,
/// `&apos;`, `&quot;`) are known. Any other entity would need a DTD, which strict
/// EXI can not carry.
fn push_reference(out: &mut String, r: &BytesRef<'_>) -> Result<()> {
    if let Some(c) = r.resolve_char_ref()? {
        out.push(c);
        return Ok(());
    }
    match &**r {
        "lt" => out.push('<'),
        "gt" => out.push('>'),
        "amp" => out.push('&'),
        "apos" => out.push('\''),
        "quot" => out.push('"'),
        // Gives quick-xml's own error for the unknown entity.
        other => out.push_str(&unescape(&format!("&{other};"))?),
    }
    Ok(())
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

/// Where one attribute's parts sit in [`Scratch::attr_text`]. The prefix is
/// replaced by the URI once all of the start tag's declarations are known.
struct AttrSpan {
    local: Range<usize>,
    prefix: Option<Range<usize>>,
    uri: Range<usize>,
    value: Range<usize>,
}

/// Buffers kept from one element to the next, so that their memory is reused.
#[derive(Default)]
struct Scratch {
    /// The text collected since the last tag. Text is joined across entity
    /// references and CDATA sections.
    text: String,
    has_text: bool,
    /// The names, URIs and values of one start tag's attributes, back to back.
    attr_text: String,
    /// One entry per attribute that is not a namespace declaration.
    attrs: Vec<AttrSpan>,
}

/// Reads one start tag: opens its namespace scope with its declarations, and
/// keeps its other attributes, normalized, in `scratch`. Declarations come
/// first because they apply to the element's own name and attributes, wherever
/// they stand in the tag.
fn read_start_tag(
    start: &BytesStart<'_>,
    ns: &mut Namespaces,
    scratch: &mut Scratch,
) -> Result<()> {
    let Scratch {
        attr_text: buf,
        attrs,
        ..
    } = scratch;
    buf.clear();
    attrs.clear();
    ns.open();
    // quick-xml's duplicate check allocates for every tag; duplicates are
    // caught after sorting instead (see `push_attributes`).
    for attr in start.attributes().with_checks(false) {
        let attr = attr?;
        match attr.key.as_namespace_binding() {
            Some(PrefixDeclaration::Default) => ns.bind("", &attr.value)?,
            Some(PrefixDeclaration::Named(prefix)) => ns.bind(prefix, &attr.value)?,
            None => {
                let (local, prefix) = attr.key.decompose();
                let value_start = buf.len();
                push_attribute_value(buf, &attr.value)?;
                let value = value_start..buf.len();
                let local = push_range(buf, local.as_ref());
                let prefix = prefix.map(|p| push_range(buf, p.as_ref()));
                attrs.push(AttrSpan {
                    local,
                    prefix,
                    uri: 0..0,
                    value,
                });
            }
        }
    }
    Ok(())
}

/// Sends the attribute events of the start tag read by [`read_start_tag`], in
/// grammar order: `xsi:type`, then `xsi:nil`, then the rest sorted by local name
/// and then URI. Comparing Rust strings byte by byte gives Unicode code point
/// order, which is the order the spec asks for
/// ([EXI 1.2](https://www.w3.org/TR/exi/#conventions)).
///
/// An attribute with no prefix is in no namespace. The `xsi:type` value is a
/// qname, a name that may have a prefix. Its prefix is resolved the same way as
/// an element name's, so a name with no prefix takes the default namespace.
/// `xsi:nil` accepts `true`, `1`, `false` and `0`, with surrounding whitespace
/// allowed.
fn push_attributes(
    ns: &Namespaces,
    scratch: &mut Scratch,
    sink: &mut impl FnMut(&ExiEvent<'_>) -> Result<()>,
) -> Result<()> {
    let Scratch {
        attr_text: buf,
        attrs,
        ..
    } = scratch;
    let mut xsi_type = None;
    let mut xsi_nil = None;
    let mut i = 0;
    while i < attrs.len() {
        let a = &attrs[i];
        let uri = match &a.prefix {
            Some(p) => ns.uri(Some(&buf[p.clone()]))?,
            None => "",
        };
        if uri == XSI_NS {
            let local = &buf[a.local.clone()];
            if (local == "type" && xsi_type.is_some()) || (local == "nil" && xsi_nil.is_some()) {
                return Err(XmlError::Unsupported(format!(
                    "attribute xsi:{local} appears twice on one element"
                )));
            }
            match local {
                "type" => {
                    let value = &buf[a.value.clone()];
                    let qname = value.trim_matches([' ', '\t', '\n', '\r']);
                    let (uri, local) = ns.element(qname)?;
                    // The local name is the end of the qname, kept as a place in
                    // `buf` because `buf` still grows below.
                    let end = a.value.start + value.trim_end_matches([' ', '\t', '\n', '\r']).len();
                    xsi_type = Some((uri, end - local.len()..end));
                    attrs.remove(i);
                    continue;
                }
                "nil" => {
                    xsi_nil = Some(parse_nil(&buf[a.value.clone()])?);
                    attrs.remove(i);
                    continue;
                }
                _ => {}
            }
        }
        let uri = push_range(buf, uri);
        attrs[i].uri = uri;
        i += 1;
    }
    if let Some((uri, local)) = xsi_type {
        sink(&ExiEvent::XsiType(ExiQName::new(uri, &buf[local])))?;
    }
    if let Some(nil) = xsi_nil {
        sink(&ExiEvent::XsiNil(nil))?;
    }
    let text = buf.as_str();
    // Two attributes never share a name, so an unstable sort gives the same order.
    attrs.sort_unstable_by(|a, b| {
        (&text[a.local.clone()], &text[a.uri.clone()])
            .cmp(&(&text[b.local.clone()], &text[b.uri.clone()]))
    });
    // Sorted, so an attribute given twice sits next to itself.
    if let Some(pair) = attrs.windows(2).find(|w| {
        (&text[w[0].local.clone()], &text[w[0].uri.clone()])
            == (&text[w[1].local.clone()], &text[w[1].uri.clone()])
    }) {
        let name = ExiQName::new(&text[pair[0].uri.clone()], &text[pair[0].local.clone()]);
        return Err(XmlError::Unsupported(format!(
            "attribute {name} appears twice on one element"
        )));
    }
    for a in attrs.iter() {
        let name = ExiQName::new(&text[a.uri.clone()], &text[a.local.clone()]);
        sink(&ExiEvent::Attribute(
            name,
            Value::Lexical(&text[a.value.clone()]),
        ))?;
    }
    Ok(())
}

/// Appends `s` to `buf` and returns where it went.
fn push_range(buf: &mut String, s: &str) -> Range<usize> {
    let start = buf.len();
    buf.push_str(s);
    start..buf.len()
}

/// The streaming core: parses `xml` and hands every event to `sink` in order.
///
/// It checks that there is exactly one root element, no text outside it other than
/// whitespace, and no DOCTYPE. Comments, processing instructions and the XML
/// declaration are dropped. An empty-element tag (`<a/>`) gives an `SE` and an `EE`
/// right away.
fn parse_into(xml: &[u8], mut sink: impl FnMut(&ExiEvent<'_>) -> Result<()>) -> Result<()> {
    let mut reader = Reader::from_reader(xml);
    let mut depth = 0usize;
    let mut seen_root = false;
    let mut ns = Namespaces::default();
    let mut scratch = Scratch::default();

    sink(&ExiEvent::StartDocument)?;
    loop {
        let event = reader.read_event()?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(start) | Event::Empty(start) => {
                flush_text(&mut scratch, depth, &mut sink)?;
                if depth == 0 {
                    if seen_root {
                        return Err(XmlError::Unsupported("more than one root element".into()));
                    }
                    seen_root = true;
                }
                read_start_tag(&start, &mut ns, &mut scratch)?;
                let (uri, local) = ns.element(start.name().into_inner())?;
                sink(&ExiEvent::StartElement(ExiQName::new(uri, local)))?;
                push_attributes(&ns, &mut scratch, &mut sink)?;
                if empty {
                    ns.close();
                    sink(&ExiEvent::EndElement)?;
                } else {
                    depth += 1;
                }
            }
            Event::End(_) => {
                flush_text(&mut scratch, depth, &mut sink)?;
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| XmlError::Unsupported("unexpected end tag".into()))?;
                ns.close();
                sink(&ExiEvent::EndElement)?;
            }
            // Text never holds references: they arrive as `GeneralRef` events and
            // are joined in here. Line endings are normalized only in the literal
            // text, so `&#13;` still comes out as a carriage return.
            Event::Text(t) => {
                push_normalised(&mut scratch.text, &t);
                scratch.has_text = true;
            }
            Event::CData(c) => {
                push_normalised(&mut scratch.text, &c);
                scratch.has_text = true;
            }
            Event::GeneralRef(r) => {
                push_reference(&mut scratch.text, &r)?;
                scratch.has_text = true;
            }
            Event::DocType(_) => {
                return Err(XmlError::Unsupported("DOCTYPE declarations".into()));
            }
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
            Event::Eof => break,
        }
    }
    flush_text(&mut scratch, depth, &mut sink)?;
    if depth != 0 {
        return Err(XmlError::Unsupported(
            "document ends inside an element".into(),
        ));
    }
    if !seen_root {
        return Err(XmlError::Unsupported("document has no root element".into()));
    }
    sink(&ExiEvent::EndDocument)
}

/// Sends the joined text collected so far as one `CH` event. Outside the root
/// element only whitespace is allowed, and it is dropped.
fn flush_text(
    scratch: &mut Scratch,
    depth: usize,
    sink: &mut impl FnMut(&ExiEvent<'_>) -> Result<()>,
) -> Result<()> {
    if !scratch.has_text {
        return Ok(());
    }
    scratch.has_text = false;
    if depth == 0 {
        let blank = scratch
            .text
            .chars()
            .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'));
        scratch.text.clear();
        if blank {
            return Ok(());
        }
        return Err(XmlError::Unsupported(
            "character data outside the root element".into(),
        ));
    }
    let result = sink(&ExiEvent::Characters(Value::Lexical(&scratch.text)));
    scratch.text.clear();
    result
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

    #[test]
    fn namespace_declarations_are_scoped() {
        let xml = br#"<r xmlns="urn:a" xmlns:p="urn:p" p:x="1" y="2">
            <c xmlns="urn:b"><d/></c><e xmlns=""/><p:f xmlns:p="urn:q" p:z="3"/><p:g/><xml:h/></r>"#;
        let events: Vec<_> = parse_events(xml)
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .filter(|e| !matches!(e, OwnedEvent::Characters(_)))
            .collect();
        assert_eq!(
            events,
            vec![
                OwnedEvent::StartDocument,
                se("urn:a", "r"),
                at("urn:p", "x", "1"),
                at("", "y", "2"),
                se("urn:b", "c"),
                se("urn:b", "d"),
                OwnedEvent::EndElement,
                OwnedEvent::EndElement,
                se("", "e"),
                OwnedEvent::EndElement,
                se("urn:q", "f"),
                at("urn:q", "z", "3"),
                OwnedEvent::EndElement,
                se("urn:p", "g"),
                OwnedEvent::EndElement,
                se("http://www.w3.org/XML/1998/namespace", "h"),
                OwnedEvent::EndElement,
                OwnedEvent::EndElement,
                OwnedEvent::EndDocument,
            ]
        );
        // A prefix declared on an element is gone after its end tag.
        assert!(parse_events(br#"<r><a xmlns:p="urn:p"/><p:b/></r>"#).is_err());
    }

    #[test]
    fn duplicates_are_rejected() {
        let bad: [&[u8]; 5] = [
            br#"<r a="1" a="2"/>"#,
            br#"<r xmlns:p="urn:x" xmlns:q="urn:x" p:a="1" q:a="2"/>"#,
            br#"<r xmlns:p="urn:x" xmlns:p="urn:y"/>"#,
            br#"<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:nil="true" xsi:nil="false"/>"#,
            br#"<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:x="http://www.w3.org/2001/XMLSchema-instance" xsi:type="t" x:type="u"/>"#,
        ];
        for xml in bad {
            assert!(
                matches!(parse_events(xml), Err(XmlError::Unsupported(_))),
                "{}",
                String::from_utf8_lossy(xml)
            );
        }
    }
}
