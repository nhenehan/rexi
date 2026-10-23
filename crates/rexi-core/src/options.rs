//! EXI options ([EXI 5.4](https://www.w3.org/TR/exi/#options)) and the part of them
//! this codec supports.

use crate::error::{ExiError, ExiResult, Feature};
use crate::prelude::*;

/// The `alignment` option: how event codes and values are laid out in the body
/// ([EXI 5.4](https://www.w3.org/TR/exi/#options)). Bit-packed puts them back to
/// back with no padding. Byte-aligned starts each one on a byte boundary.
/// Pre-compression does every compression step except the final DEFLATE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Alignment {
    #[default]
    /// No padding between items: the most compact form.
    BitPacked,
    /// Every event code and value starts on a byte boundary.
    ByteAligned,
    /// Byte-aligned, with the compression steps but no DEFLATE
    /// ([EXI 9](https://www.w3.org/TR/exi/#compression)).
    PreCompression,
}

/// The fidelity options: which kinds of XML items the stream keeps. Each one that
/// is on keeps the matching events, or the text form of values for
/// `lexical_values` ([EXI 6.3](https://www.w3.org/TR/exi/#fidelityOptions)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct Preserve {
    /// Keep DOCTYPE and entity references.
    pub dtd: bool,
    /// Keep namespace prefixes and declarations.
    pub prefixes: bool,
    /// Keep values in their exact text form instead of typed form.
    pub lexical_values: bool,
    /// Keep comments.
    pub comments: bool,
    /// Keep processing instructions.
    pub pis: bool,
}

/// The `schemaId` option ([EXI 5.4](https://www.w3.org/TR/exi/#options)).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum SchemaId {
    /// Element absent: the header says nothing, so the schema must be agreed out of
    /// band (outside the stream).
    #[default]
    Absent,
    /// `xsi:nil="true"`: no schema information is used (built-in grammars).
    Nil,
    /// Empty string: only the built-in XSD types are used.
    Empty,
    /// Any other value, agreed between the parties.
    Id(String),
}

/// The full set of EXI options from the header's options document, or agreed out
/// of band ([EXI 5.4](https://www.w3.org/TR/exi/#options)).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ExiOptions {
    /// How items are packed into bytes.
    pub alignment: Alignment,
    /// EXI compression ([EXI 9](https://www.w3.org/TR/exi/#compression)).
    pub compression: bool,
    /// Only what the schema declares may appear
    /// ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).
    pub strict: bool,
    /// The body is a fragment (any number of root elements) instead of a document.
    pub fragment: bool,
    /// Which kinds of events and values are kept.
    pub preserve: Preserve,
    /// Self-contained elements may be used.
    pub self_contained: bool,
    /// Longest string value that is added to the value string table (`None`: no
    /// limit).
    pub value_max_length: Option<u32>,
    /// Most values the value string table holds at once (`None`: no limit).
    pub value_partition_capacity: Option<u32>,
    /// Number of values per compression block (`None`: the default, 1,000,000).
    pub block_size: Option<u32>,
    /// Which schema the stream uses, if the stream says.
    pub schema_id: SchemaId,
    /// Whether a `datatypeRepresentationMap` was present (it is never applied).
    pub datatype_representation_map: bool,
}

impl Default for ExiOptions {
    /// The spec defaults: bit-packed, non-strict, nothing preserved.
    fn default() -> Self {
        Self {
            alignment: Alignment::BitPacked,
            compression: false,
            strict: false,
            fragment: false,
            preserve: Preserve::default(),
            self_contained: false,
            value_max_length: None,
            value_partition_capacity: None,
            block_size: None,
            schema_id: SchemaId::Absent,
            datatype_representation_map: false,
        }
    }
}

impl ExiOptions {
    /// Strict, bit-packed, nothing preserved: the most compact schema-informed setup.
    ///
    /// Strict means the grammars allow only what the schema declares, and leave out
    /// the productions for namespace declarations, comments, processing instructions,
    /// entity references and self-contained elements
    /// ([EXI 5.4](https://www.w3.org/TR/exi/#options),
    /// [EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).
    pub fn strict() -> Self {
        Self {
            strict: true,
            ..Self::default()
        }
    }

    /// Sets the schema id that is written into the options document.
    #[must_use]
    pub fn with_schema_id(mut self, id: impl Into<String>) -> Self {
        self.schema_id = SchemaId::Id(id.into());
        self
    }

    /// Sets the alignment.
    #[must_use]
    pub fn with_alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Checks the spec's rules on which options may be combined
    /// ([EXI 5.4](https://www.w3.org/TR/exi/#options)) and this codec's limits.
    ///
    /// The spec rule checked here: `strict` may not be used with Preserve.dtd,
    /// prefixes, comments or pis (Preserve.lexicalValues is allowed). Every other
    /// refusal is a feature this codec does not support, including non-strict mode.
    pub fn validate(&self) -> ExiResult<()> {
        if self.compression {
            return Err(ExiError::Unsupported(Feature::Compression));
        }
        if self.alignment == Alignment::PreCompression {
            return Err(ExiError::Unsupported(Feature::PreCompression));
        }
        if self.self_contained {
            return Err(ExiError::Unsupported(Feature::SelfContained));
        }
        if self.datatype_representation_map {
            return Err(ExiError::Unsupported(Feature::DatatypeRepresentationMap));
        }
        if self.fragment {
            return Err(ExiError::Unsupported(Feature::Fragment));
        }
        if self.strict
            && (self.preserve.dtd
                || self.preserve.prefixes
                || self.preserve.comments
                || self.preserve.pis)
        {
            return Err(ExiError::Header(
                "strict cannot be combined with Preserve.dtd/prefixes/comments/pis",
            ));
        }
        if self.preserve.dtd {
            return Err(ExiError::Unsupported(Feature::PreserveDtd));
        }
        if self.preserve.prefixes {
            return Err(ExiError::Unsupported(Feature::PreservePrefixes));
        }
        if self.preserve.comments {
            return Err(ExiError::Unsupported(Feature::PreserveComments));
        }
        if self.preserve.pis {
            return Err(ExiError::Unsupported(Feature::PreservePis));
        }
        if self.preserve.lexical_values {
            return Err(ExiError::Unsupported(Feature::PreserveLexicalValues));
        }
        if !self.strict {
            return Err(ExiError::Unsupported(Feature::NonStrictSchemaInformed));
        }
        Ok(())
    }

    /// Whether the body is byte-aligned (true for both byte-alignment and
    /// pre-compression).
    pub fn byte_aligned(&self) -> bool {
        self.alignment != Alignment::BitPacked
    }
}

/// What to put in front of the body ([EXI 5](https://www.w3.org/TR/exi/#header)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct HeaderConfig {
    /// Write the `$EXI` cookie.
    pub cookie: bool,
    /// Write the EXI options document into the header.
    pub include_options: bool,
}

impl HeaderConfig {
    /// No cookie and no options document: a one-byte header. The same as
    /// `HeaderConfig::default()`, usable in a `const`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cookie: false,
            include_options: false,
        }
    }

    /// Sets whether the `$EXI` cookie is written.
    #[must_use]
    pub const fn with_cookie(mut self, cookie: bool) -> Self {
        self.cookie = cookie;
        self
    }

    /// Sets whether the options document is written into the header.
    #[must_use]
    pub const fn with_include_options(mut self, include: bool) -> Self {
        self.include_options = include;
        self
    }
}
