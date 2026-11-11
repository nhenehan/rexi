//! EXI (Efficient XML Interchange, a [W3C standard](https://www.w3.org/TR/exi/))
//! for Rust, in one crate. It turns XML into small binary messages and back, using
//! your XML Schema.
//!
//! This crate only re-exports rexi's other crates, so you can depend on one name:
//!
//! | Path | Crate | Feature |
//! |---|---|---|
//! | `rexi::*` | [`rexi_core`]: encoder, decoder, options, values | always |
//! | [`rexi::schema`](mod@schema) | `rexi-schema`: `.xsd` to grammar tables | `schema` (default) |
//! | [`rexi::xml`](mod@xml) | `rexi-xml`: XML text to EXI and back | `xml` (default) |
//! | `rexi::serde` | `rexi-serde`: your Rust types to EXI and back | `serde` |
//!
//! On a small device without `std`, turn the default features off. That leaves
//! rexi-core (and rexi-serde, if you ask for it), which need only `alloc`:
//!
//! ```toml
//! rexi = { version = "0.3", default-features = false, features = ["serde"] }
//! ```
//!
//! The `rexi` command line tool lives in `rexi-xml`: `cargo install rexi-xml`.
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
