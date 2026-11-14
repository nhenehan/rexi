//! EXI (Efficient XML Interchange) for Rust: small, fast binary XML, driven by your
//! XML Schema.
//!
//! EXI is a [W3C standard](https://www.w3.org/TR/exi/) for sending XML as compact
//! binary messages. Both ends compile the same XML Schema, so a message leaves out
//! everything the schema already says: element names become a few bits, and numbers
//! are sent as numbers. This crate is the one dependency for all of rexi.
//!
//! The [user guide](https://nhenehan.github.io/rexi/) walks through one idea per
//! chapter, from a first round trip to small devices and C.
//!
//! # Example
//!
//! ```
//! # #[cfg(feature = "xml")]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use rexi::{ExiOptions, HeaderConfig};
//!
//! let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
//!   <xs:element name="temp" type="xs:int"/>
//! </xs:schema>"#;
//! // Compile the schema once, then reuse the tables for every message.
//! let tables = rexi::schema::compile_str(xsd, None)?;
//! let xml = b"<temp>21</temp>";
//! let options = ExiOptions::strict();
//! let exi = rexi::xml::encode_xml(xml, &tables, options.clone(), HeaderConfig::default())?;
//! // The options are not in the stream, so the decoder is told them.
//! let back = rexi::xml::decode_to_xml(&exi[..], &tables, Some(options))?;
//! assert!(exi.len() < xml.len());
//! assert!(back.contains("<temp>21</temp>"));
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "xml"))]
//! # fn main() {}
//! ```
//!
//! # Ways to use it
//!
//! - **XML text in and out:** `xml::encode_xml` and `xml::decode_to_xml`
//!   (feature `xml`).
//! - **Your own Rust types:** `serde::to_exi` and `serde::from_exi` (feature
//!   `serde`). The schema decides the XML shape, not serde's defaults.
//! - **Event by event:** [`Encoder`] and [`Decoder`], for streaming or for a front
//!   end of your own.
//! - **Tables built ahead of time:** the `rexi-schema-gen` tool writes a schema's
//!   grammar tables as Rust source, so a small device needs no schema compiler and
//!   reading the tables never allocates.
//! - **Input you do not trust:** the decoder stops early on hostile streams; see
//!   [`DecodeLimits`].
//! - **Fragments:** several root elements in one message, with
//!   [`ExiOptions::with_fragment`].
//!
//! # Feature flags
//!
//! - `std` (default): every `std::io::Read` and `Write` works as a byte source and
//!   sink.
//! - `schema` (default): [`rexi::schema`](mod@schema), compiling an XML Schema
//!   (`.xsd`) into grammar tables at run time. Needs `std`.
//! - `xml` (default): [`rexi::xml`](mod@xml), XML text to EXI and back. Needs
//!   `schema`.
//! - `serde`: `rexi::serde`, your own Rust types to EXI and back. Works without
//!   `std`.
//!
//! On a small device without `std`, turn the default features off. That leaves
//! rexi-core (and rexi-serde, if you ask for it), which need only `alloc`:
//!
//! ```toml
//! rexi = { version = "0.3", default-features = false, features = ["serde"] }
//! ```
//!
//! # Crates
//!
//! This crate only re-exports rexi's other crates:
//!
//! | Path | Crate | Feature |
//! |---|---|---|
//! | `rexi::*` | [`rexi_core`]: encoder, decoder, options, values | always |
//! | [`rexi::schema`](mod@schema) | `rexi-schema`: `.xsd` to grammar tables | `schema` (default) |
//! | [`rexi::xml`](mod@xml) | `rexi-xml`: XML text to EXI and back | `xml` (default) |
//! | `rexi::serde` | `rexi-serde`: your Rust types to EXI and back | `serde` |
//!
//! The `rexi` command line tool lives in `rexi-xml`: `cargo install rexi-xml`.
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub use rexi_core::*;

#[cfg(feature = "schema")]
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
#[doc(inline)]
pub use rexi_schema as schema;

#[cfg(feature = "xml")]
#[cfg_attr(docsrs, doc(cfg(feature = "xml")))]
#[doc(inline)]
pub use rexi_xml as xml;

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
#[doc(inline)]
pub use rexi_serde as serde;
