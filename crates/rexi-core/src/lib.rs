//! The core of an EXI codec, with no outside dependencies. EXI is Efficient XML
//! Interchange 1.0, Second Edition
//! ([spec](https://www.w3.org/TR/exi/)), a compact binary form of XML.
//!
//! Layers, from the bottom up: [`io`] reads and writes bits, [`datatype`] encodes
//! typed values, [`string_table`] gives strings their compact identifiers (small
//! numbers that stand for strings already seen,
//! [EXI 7.3](https://www.w3.org/TR/exi/#stringTable)), [`grammar`] holds the grammar
//! tables and the code that walks them, and on top sit the SAX-like (event by event)
//! [`Encoder`] and [`Decoder`].
//!
//! The setup this crate aims at first is: schema-informed (the grammars come from an
//! XML Schema, [EXI 8.5](https://www.w3.org/TR/exi/#informedGrammars)), strict (only
//! what the schema declares may appear), bit-packed (no padding between items), with
//! no `$EXI` cookie, and with the options usually agreed outside the stream. Anything
//! else is either supported or refused with [`ExiError::Unsupported`]. Nothing is
//! quietly encoded in some "close enough" way. For example, a string type whose
//! pattern gives a restricted character set
//! ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)) is refused when
//! one of its values is written or read; see [`DatatypeKind::RestrictedString`].
//!
//! The decoder is meant to be safe on input you do not trust. [`DecodeLimits`]
//! caps nesting depth, list length, integer size and the total bytes of decoded
//! strings, with defaults far above what normal documents need. Going over one
//! stops decoding with [`ExiError::LimitExceeded`]. [`Decoder::new`] uses the
//! defaults; [`Decoder::with_limits`] takes others. See [`limits`].
//!
//! The crate is `no_std` and needs `alloc` (string tables and grammars allocate).
//! The default `std` feature adds `std::io` support: every `std::io::Read` /
//! `Write` is also an [`io::ByteSource`] / [`io::ByteSink`].
//!
//! # Example
//!
//! Grammar tables usually come from `rexi-schema` (at run time) or `rexi-schema-gen`
//! (ahead of time, as Rust code). This example uses the tables of the EXI Options
//! document that ship with the crate. It encodes `<header><strict/></header>` with
//! strict options agreed out of band, then decodes it again.
//!
//! ```
//! use rexi_core::grammar::options_grammar::OPTIONS_GRAMMARS;
//! use rexi_core::{Decoder, Encoder, Event, ExiOptions, HeaderConfig};
//!
//! # fn main() -> rexi_core::ExiResult<()> {
//! const EXI: &str = "http://www.w3.org/2009/exi";
//! let mut enc = Encoder::new(
//!     Vec::new(),
//!     &OPTIONS_GRAMMARS,
//!     ExiOptions::strict(),
//!     HeaderConfig::default(),
//! )?;
//! enc.start_document()?;
//! enc.start_element(EXI, "header")?;
//! enc.start_element(EXI, "strict")?;
//! enc.end_element()?;
//! enc.end_element()?;
//! enc.end_document()?;
//! let bytes = enc.finish()?;
//!
//! let mut dec = Decoder::new(&bytes[..], &OPTIONS_GRAMMARS, Some(ExiOptions::strict()))?;
//! let mut names = Vec::new();
//! while let Some(event) = dec.next_event()? {
//!     if let Event::StartElement(q) = event {
//!         assert_eq!(q.uri, EXI);
//!         names.push(q.local.to_string());
//!     }
//! }
//! assert_eq!(names, ["header", "strict"]);
//! # Ok(())
//! # }
//! ```

#![no_std]
#![warn(missing_docs)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

/// The `alloc` names that the `std` prelude would bring in on its own.
mod prelude {
    pub(crate) use alloc::borrow::ToOwned;
    pub(crate) use alloc::format;
    pub(crate) use alloc::string::{String, ToString};
    pub(crate) use alloc::vec;
    pub(crate) use alloc::vec::Vec;
}

pub mod datatype;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod event;
pub mod grammar;
// The header and string table code are used by rexi's own tests and tools, but
// are not part of the supported API: they may change in any release.
#[doc(hidden)]
pub mod header;
pub mod ids;
pub mod io;
pub mod limits;
pub mod options;
#[doc(hidden)]
pub mod string_table;
pub mod value;

pub use datatype::{DatatypeKind, TypeResolver, WhiteSpace};
pub use decoder::Decoder;
pub use encoder::Encoder;
pub use error::{ExiError, ExiResult, Feature, IoErrorKind};
pub use event::{Event, OwnedEvent, QName};
pub use grammar::{GrammarSet, GrammarSetBuf, GrammarTables};
pub use ids::{
    ElemIdx, EnumSetIdx, GrammarIdx, LocalId, NONE, PrefixId, QNameId, RuleIdx, StrIdx, TypeIdx,
    UriId,
};
pub use limits::{DecodeLimits, Limit};
pub use options::{Alignment, ExiOptions, HeaderConfig, Preserve, SchemaId};
pub use value::{BigUint, DateTime, DateTimeKind, Decimal, Float, Int, OwnedValue, Uint, Value};
