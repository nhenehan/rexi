//! XML front end for the `rexi-core` codec: turns XML text into EXI events and
//! back. The events are those of [EXI 4](https://www.w3.org/TR/exi/#streams).
//!
//! * [`reader`] parses XML (with `quick-xml`) into the [`OwnedEvent`] set that the
//!   strict codec accepts, with attributes already sorted into grammar order.
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
pub use reader::{as_lexical_event, parse_events, push_xml};
pub use rexi_core::{OwnedEvent, OwnedValue};
pub use writer::{
    decode_events, decode_events_typed, decode_events_typed_with_limits, decode_events_with_limits,
    decode_to_xml, decode_to_xml_with_limits, write_xml,
};

/// Everything this crate can fail with.
#[derive(Debug)]
#[non_exhaustive]
pub enum XmlError {
    /// `quick-xml` rejected the input (syntax, character encoding, escaping,
    /// namespaces).
    Xml(quick_xml::Error),
    /// The codec rejected an event or a value.
    Exi(rexi_core::ExiError),
    /// Schema compilation failed.
    Schema(rexi_schema::SchemaError),
    /// Well-formed XML that this front end does not handle (DOCTYPE, unknown
    /// prefix, ...).
    Unsupported(String),
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XmlError::Xml(e) => write!(f, "XML error: {e}"),
            XmlError::Exi(e) => write!(f, "EXI error: {e}"),
            XmlError::Schema(e) => write!(f, "schema error: {e}"),
            XmlError::Unsupported(what) => write!(f, "unsupported XML construct: {what}"),
        }
    }
}

impl std::error::Error for XmlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            XmlError::Xml(e) => Some(e),
            XmlError::Exi(e) => Some(e),
            XmlError::Schema(e) => Some(e),
            XmlError::Unsupported(_) => None,
        }
    }
}

impl From<quick_xml::Error> for XmlError {
    fn from(e: quick_xml::Error) -> Self {
        XmlError::Xml(e)
    }
}

impl From<quick_xml::events::attributes::AttrError> for XmlError {
    fn from(e: quick_xml::events::attributes::AttrError) -> Self {
        XmlError::Xml(quick_xml::Error::InvalidAttr(e))
    }
}

impl From<quick_xml::encoding::EncodingError> for XmlError {
    fn from(e: quick_xml::encoding::EncodingError) -> Self {
        XmlError::Xml(quick_xml::Error::Encoding(e))
    }
}

impl From<quick_xml::escape::EscapeError> for XmlError {
    fn from(e: quick_xml::escape::EscapeError) -> Self {
        XmlError::Xml(quick_xml::Error::Escape(e))
    }
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
