//! XML front end for the `rexi-core` codec: turns XML text into EXI events and
//! back. The events are those of [EXI 4](https://www.w3.org/TR/exi/#streams).
//!
//! * [`reader`] parses XML into the events that the strict codec accepts, with
//!   attributes already sorted into grammar order.
//! * [`writer`] writes events back out as XML and drives the
//!   [`rexi_core::Decoder`].
//! * [`dump`] prints events in the `.events` text format that the interop oracle
//!   (the reference test tool) uses.
//! * [`codec`] holds `encode_xml`, which encodes a document in one call.
//!
//! The `rexi` binary in `src/bin` is the command-line entry point. It is the only
//! place that compiles schemas (through `rexi-schema`):
//!
//! ```text
//! rexi encode --xsd order.xsd order.xml order.exi
//! rexi decode --xsd order.xsd order.exi
//! ```
//!
//! A round trip from XML to EXI and back:
//!
//! ```
//! use rexi_core::{ExiOptions, HeaderConfig};
//!
//! let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
//!                <xs:element name="note" type="xs:string"/>
//!              </xs:schema>"#;
//! let tables = rexi_schema::compile_str(xsd, None)?;
//!
//! let exi = rexi_xml::encode_xml(b"<note>Hello</note>", &tables, ExiOptions::strict(), HeaderConfig::default())?;
//! let xml = rexi_xml::decode_to_xml(&exi[..], &tables, Some(ExiOptions::strict()))?;
//! assert!(xml.contains("<note>Hello</note>"));
//! # Ok::<(), rexi_xml::XmlError>(())
//! ```
//!
//! The library itself takes grammar tables from either path: compiled at run time
//! by `rexi-schema`, or generated ahead of time by `rexi-schema-gen`. Use
//! `rexi-serde` instead to go straight from Rust types to EXI with no XML text in
//! between, and `rexi-capi` to call rexi from C.

#![warn(missing_docs)]

pub mod codec;
pub mod dump;
pub mod reader;
pub mod writer;

use std::fmt;

pub use codec::encode_xml;
pub use dump::{events_dump, events_dump_with, plain_format};
pub use reader::{parse_events, push_xml, to_lexical_event};
pub use rexi_core::{OwnedEvent, OwnedValue};
pub use writer::{
    decode_events, decode_events_typed, decode_events_typed_with_limits, decode_events_with_limits,
    decode_to_xml, decode_to_xml_with_limits, write_xml,
};

/// Everything this crate can fail with.
///
/// `Display` gives the whole message, with the inner error's text, so
/// `source()` returns nothing: an error reporter would print that text twice.
/// Match on the variant to get the inner error.
#[derive(Debug)]
#[non_exhaustive]
pub enum XmlError {
    /// The input is not well-formed XML (syntax, character encoding, escaping).
    Syntax(XmlSyntaxError),
    /// The codec rejected an event or a value.
    Exi(rexi_core::ExiError),
    /// Schema compilation failed.
    Schema(rexi_schema::SchemaError),
    /// Well-formed XML that this front end does not handle (DOCTYPE, unknown
    /// prefix, an attribute given twice, ...).
    Unsupported(String),
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XmlError::Syntax(e) => write!(f, "XML error: {e}"),
            XmlError::Exi(e) => write!(f, "EXI error: {e}"),
            XmlError::Schema(e) => write!(f, "schema error: {e}"),
            XmlError::Unsupported(what) => write!(f, "unsupported XML construct: {what}"),
        }
    }
}

impl std::error::Error for XmlError {}

/// XML text that the parser rejected. It keeps the parser's message and, when
/// known, the byte offset in the input where the problem was found. The parser
/// itself is not part of this crate's API, so its error type is not exposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlSyntaxError {
    message: String,
    position: Option<u64>,
}

impl XmlSyntaxError {
    /// Wraps a parser error, with the byte offset where it was found if known.
    pub(crate) fn new(error: impl fmt::Display, position: Option<u64>) -> Self {
        Self {
            message: error.to_string(),
            position,
        }
    }

    /// The parser's description of the problem.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The byte offset in the input where the problem was found, if known.
    pub fn position(&self) -> Option<u64> {
        self.position
    }
}

impl fmt::Display for XmlSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some(at) => write!(f, "{} (at byte {at})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for XmlSyntaxError {}

/// A parser error with no known position, as an [`XmlError`].
pub(crate) fn syntax(error: impl fmt::Display) -> XmlError {
    XmlError::Syntax(XmlSyntaxError::new(error, None))
}

impl From<rexi_core::ExiError> for XmlError {
    fn from(e: rexi_core::ExiError) -> Self {
        XmlError::Exi(e)
    }
}

impl From<rexi_schema::SchemaError> for XmlError {
    fn from(e: rexi_schema::SchemaError) -> Self {
        XmlError::Schema(e)
    }
}

/// The result type of this crate's functions.
pub type Result<T> = std::result::Result<T, XmlError>;
