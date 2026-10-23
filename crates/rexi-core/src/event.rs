//! The SAX-like set of events (one call or result per XML item). The event types
//! are listed in [EXI 4](https://www.w3.org/TR/exi/#streams) (Table 4-1) and used as
//! grammar terminals in [EXI 8.1](https://www.w3.org/TR/exi/#grammarNotation).

use crate::prelude::*;
#[allow(unused_imports)]
use crate::value::OwnedValue;
use crate::value::Value;

/// A qualified name: namespace URI plus local name. Prefixes are not kept, because
/// strict mode does not allow Preserve.prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QName<'a> {
    /// Namespace URI; empty for no namespace.
    pub uri: &'a str,
    /// Local name.
    pub local: &'a str,
}

impl<'a> QName<'a> {
    /// The qname `{uri}local`.
    pub const fn new(uri: &'a str, local: &'a str) -> Self {
        Self { uri, local }
    }
}

impl core::fmt::Display for QName<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.uri.is_empty() {
            f.write_str(self.local)
        } else {
            write!(f, "{{{}}}{}", self.uri, self.local)
        }
    }
}

/// An event pushed into the [`crate::Encoder`] or pulled from the [`crate::Decoder`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event<'a> {
    /// Start of the document (`SD`).
    StartDocument,
    /// End of the document (`ED`).
    EndDocument,
    /// Start of an element (`SE`).
    StartElement(QName<'a>),
    /// End of the current element (`EE`).
    EndElement,
    /// An attribute and its value (`AT`).
    Attribute(QName<'a>, Value<'a>),
    /// The `xsi:type` attribute, which names the type to use for this element in
    /// place of its declared type. It must come right after `StartElement`.
    XsiType(QName<'a>),
    /// The `xsi:nil` attribute, which marks an element as having no content. It must
    /// come right after `StartElement` (after `XsiType`, if there is one).
    XsiNil(bool),
    /// Character content (`CH`): a typed value or text.
    Characters(Value<'a>),
    /// A namespace declaration (`NS`).
    ///
    /// Refused by the strict codec. It is listed (with the events below it) so that
    /// front-ends can still describe these items.
    NamespaceDecl {
        /// The prefix being declared (empty for the default namespace).
        prefix: &'a str,
        /// The namespace URI it stands for.
        uri: &'a str,
        /// Whether this declaration gives the prefix of the element it is on.
        local_element_ns: bool,
    },
    /// A comment (`CM`).
    Comment(&'a str),
    /// A processing instruction (`PI`).
    ProcessingInstruction {
        /// The PI target (the name right after `<?`).
        target: &'a str,
        /// The rest of the PI.
        data: &'a str,
    },
    /// A DOCTYPE declaration (`DT`).
    DocType {
        /// The root element name.
        name: &'a str,
        /// The public id, or empty.
        public_id: &'a str,
        /// The system id, or empty.
        system_id: &'a str,
        /// The internal subset text, or empty.
        text: &'a str,
    },
    /// An entity reference that was not expanded (`ER`), by name.
    EntityReference(&'a str),
    /// Start of a self-contained element (`SC`).
    SelfContained,
}

impl Event<'_> {
    /// Short name used in error messages: the spec's event abbreviation
    /// (`SE`, `AT`, `CH`, ...).
    pub fn kind_name(&self) -> &'static str {
        match self {
            Event::StartDocument => "SD",
            Event::EndDocument => "ED",
            Event::StartElement(_) => "SE",
            Event::EndElement => "EE",
            Event::Attribute(..) => "AT",
            Event::XsiType(_) => "AT(xsi:type)",
            Event::XsiNil(_) => "AT(xsi:nil)",
            Event::Characters(_) => "CH",
            Event::NamespaceDecl { .. } => "NS",
            Event::Comment(_) => "CM",
            Event::ProcessingInstruction { .. } => "PI",
            Event::DocType { .. } => "DT",
            Event::EntityReference(_) => "ER",
            Event::SelfContained => "SC",
        }
    }
}

/// An event that owns its strings, for buffering and tests. It covers only the
/// events the strict codec handles.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum OwnedEvent {
    /// See [`Event::StartDocument`].
    StartDocument,
    /// See [`Event::EndDocument`].
    EndDocument,
    /// See [`Event::StartElement`].
    StartElement {
        /// Namespace URI.
        uri: String,
        /// Local name.
        local: String,
    },
    /// See [`Event::EndElement`].
    EndElement,
    /// See [`Event::Attribute`].
    Attribute {
        /// Namespace URI.
        uri: String,
        /// Local name.
        local: String,
        /// The attribute value.
        value: crate::value::OwnedValue,
    },
    /// See [`Event::XsiType`].
    XsiType {
        /// Namespace URI of the type.
        uri: String,
        /// Local name of the type.
        local: String,
    },
    /// See [`Event::XsiNil`].
    XsiNil(bool),
    /// See [`Event::Characters`].
    Characters(crate::value::OwnedValue),
}

impl OwnedEvent {
    /// Copies `ev` into an owned event, or returns `None` for the kinds the strict
    /// codec does not handle (namespaces, comments, PIs, DOCTYPE, entity references,
    /// self-contained).
    pub fn from_event(ev: &Event<'_>) -> Option<Self> {
        Some(match ev {
            Event::StartDocument => OwnedEvent::StartDocument,
            Event::EndDocument => OwnedEvent::EndDocument,
            Event::StartElement(q) => OwnedEvent::StartElement {
                uri: q.uri.to_string(),
                local: q.local.to_string(),
            },
            Event::EndElement => OwnedEvent::EndElement,
            Event::Attribute(q, v) => OwnedEvent::Attribute {
                uri: q.uri.to_string(),
                local: q.local.to_string(),
                value: v.to_owned_value(),
            },
            Event::XsiType(q) => OwnedEvent::XsiType {
                uri: q.uri.to_string(),
                local: q.local.to_string(),
            },
            Event::XsiNil(b) => OwnedEvent::XsiNil(*b),
            Event::Characters(v) => OwnedEvent::Characters(v.to_owned_value()),
            _ => return None,
        })
    }

    /// Borrows this event as an [`Event`].
    pub fn as_event(&self) -> Event<'_> {
        match self {
            OwnedEvent::StartDocument => Event::StartDocument,
            OwnedEvent::EndDocument => Event::EndDocument,
            OwnedEvent::StartElement { uri, local } => Event::StartElement(QName::new(uri, local)),
            OwnedEvent::EndElement => Event::EndElement,
            OwnedEvent::Attribute { uri, local, value } => {
                Event::Attribute(QName::new(uri, local), value.as_value())
            }
            OwnedEvent::XsiType { uri, local } => Event::XsiType(QName::new(uri, local)),
            OwnedEvent::XsiNil(b) => Event::XsiNil(*b),
            OwnedEvent::Characters(v) => Event::Characters(v.as_value()),
        }
    }
}
