//! Serde support built on the strict schema-informed EXI codec of `rexi-core`.
//! ("Schema-informed" means the grammars come from an XML Schema
//! ([EXI 8.5](https://www.w3.org/TR/exi/#informedGrammars)); "strict" means the
//! stream may hold only what the schema allows.)
//!
//! ```ignore
//! let bytes = rexi_serde::to_exi(&order, &GRAMMARS, &ExiOptions::strict(), HeaderConfig::default())?;
//! let back: Order = rexi_serde::from_exi(&bytes, &GRAMMARS, Some(ExiOptions::strict()))?;
//! ```
//!
//! The schema drives the mapping, so plain `#[derive(Serialize, Deserialize)]`
//! types need few annotations. (The schema arrives as grammar tables, compiled at
//! run time by `rexi-schema` or generated ahead of time by `rexi-schema-gen`.)
//!
//! - **Root**: the top-level struct's name (or its `#[serde(rename)]`) is matched
//!   against the schema's global elements. [`to_exi_root`] names it explicitly.
//! - **Attributes vs elements**: a field is an attribute when the grammar allows
//!   `AT(name)` at that point; otherwise it is a child element. Attributes are
//!   written in schema order, whatever the field order. Child elements are written
//!   in field order. `@name` is also accepted as the name of an attribute field.
//! - **Namespaces** come from the grammar's productions (a production is one
//!   allowed event plus the rule to go to next,
//!   [EXI 8.1](https://www.w3.org/TR/exi/#grammarNotation)). A `{uri}local` rename
//!   picks one when a name is declared in several namespaces.
//! - **Repetition and optional fields**: `Vec<T>` is `maxOccurs > 1`, and
//!   `Option<T>` is `minOccurs = 0`. `None` is left out, and a decoded
//!   `xsi:nil="true"` becomes `None`.
//! - **Enums**: unit variants are enumeration values
//!   ([EXI 7.2](https://www.w3.org/TR/exi/#encodingEnumerations)); the variant
//!   name, after any rename, is the value's text. Variants with data are
//!   `xs:choice` branches, written as an element named after the variant. When
//!   deserializing, name the choice field `$value`.
//! - **Simple content**: a `$text` field (or a `$value` field holding a single
//!   value) is the element's character content, for example a string that also
//!   has attributes.
//! - **`xsi:type`**: `#[serde(tag = "@xsi:type")]` on an enum switches the element
//!   to the type named by the variant. A field `@xsi:nil: bool` sets `xsi:nil`.
//!   (Both are explained in
//!   [EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict).)
//! - **Values** are passed as typed values where the conversion is exact
//!   (booleans, integers, binary, and floats and decimals from `f32`/`f64`). All
//!   others are passed as text and parsed with the schema datatype, exactly as XML
//!   input would be. The datatype encodings are in
//!   [EXI 7.1](https://www.w3.org/TR/exi/#encodingDatatypes).
//!
//! Schema-less EXI, which uses the built-in grammars
//! ([EXI 8.4](https://www.w3.org/TR/exi/#builtinGrammars)), is not supported.
//! Tables without global elements are rejected with `ExiError::Unsupported`.

#![warn(missing_docs)]
#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

/// The `alloc` names that the `std` prelude would give us. This crate is
/// `no_std`, so they must be imported by hand.
mod prelude {
    pub(crate) use alloc::boxed::Box;
    pub(crate) use alloc::format;
    pub(crate) use alloc::string::{String, ToString};
    pub(crate) use alloc::vec;
    pub(crate) use alloc::vec::Vec;
}

mod de;
mod error;
mod item;
mod node;
mod ser;
mod write;

use crate::prelude::*;
use rexi_core::io::{ByteSink, ByteSource};

use rexi_core::{DecodeLimits, Decoder, Encoder, ExiOptions, GrammarTables, HeaderConfig};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub use error::{Error, Result};

/// Serializes `value` as an EXI stream. The root element is picked from the
/// value's type name.
pub fn to_exi<T: Serialize + ?Sized, G: GrammarTables>(
    value: &T,
    tables: G,
    options: &ExiOptions,
    header: HeaderConfig,
) -> Result<Vec<u8>> {
    to_writer(Vec::new(), value, tables, options, header, None)
}

/// Like [`to_exi`], with the root element named explicitly (`local` or
/// `{uri}local`) instead of taken from the value's type name.
pub fn to_exi_root<T: Serialize + ?Sized, G: GrammarTables>(
    value: &T,
    root: &str,
    tables: G,
    options: &ExiOptions,
    header: HeaderConfig,
) -> Result<Vec<u8>> {
    to_writer(Vec::new(), value, tables, options, header, Some(root))
}

/// Serializes `value` into `writer`, and returns the writer once the stream is
/// finished. `root` names the root element, as for [`to_exi_root`]; `None` picks
/// it from the value's type name.
///
/// The value is first turned into an in-memory tree, so that attributes can be
/// put in grammar order before anything is written.
pub fn to_writer<W: ByteSink, T: Serialize + ?Sized, G: GrammarTables>(
    writer: W,
    value: &T,
    tables: G,
    options: &ExiOptions,
    header: HeaderConfig,
    root: Option<&str>,
) -> Result<W> {
    let item = ser::to_item(value)?;
    let mut enc = Encoder::new(writer, tables, options.clone(), header)?;
    write::write_document(&mut enc, &item, root)?;
    Ok(enc.finish()?)
}

/// Deserializes a `T` from an EXI stream. `out_of_band` gives the options when
/// the header does not carry them. Uses the default [`DecodeLimits`].
pub fn from_exi<T: DeserializeOwned, G: GrammarTables>(
    bytes: &[u8],
    tables: G,
    out_of_band: Option<ExiOptions>,
) -> Result<T> {
    from_reader(bytes, tables, out_of_band)
}

/// Like [`from_exi`], with the given limits on what the stream may ask for.
pub fn from_exi_with_limits<T: DeserializeOwned, G: GrammarTables>(
    bytes: &[u8],
    tables: G,
    out_of_band: Option<ExiOptions>,
    limits: DecodeLimits,
) -> Result<T> {
    from_reader_with_limits(bytes, tables, out_of_band, limits)
}

/// Deserializes a `T` from an EXI stream read from `reader`. The whole document is
/// decoded into an element tree first, then `T` is built from the tree. Uses the
/// default [`DecodeLimits`].
pub fn from_reader<R: ByteSource, T: DeserializeOwned, G: GrammarTables>(
    reader: R,
    tables: G,
    out_of_band: Option<ExiOptions>,
) -> Result<T> {
    from_reader_with_limits(reader, tables, out_of_band, DecodeLimits::default())
}

/// Like [`from_reader`], with the given limits on what the stream may ask for.
///
/// The default depth limit (256) also keeps the deserializer, which walks the
/// tree by recursion, well inside the stack. With a much higher limit, a very
/// deep document can need a bigger stack.
pub fn from_reader_with_limits<R: ByteSource, T: DeserializeOwned, G: GrammarTables>(
    reader: R,
    tables: G,
    out_of_band: Option<ExiOptions>,
    limits: DecodeLimits,
) -> Result<T> {
    let dec = Decoder::with_limits(reader, tables, out_of_band, limits)?;
    let root = node::read_document(dec)?;
    T::deserialize(de::NodeDe(&root))
}
