//! Error type shared by every layer of `rexi-core`.

use alloc::string::String;
use core::fmt;

use crate::limits::Limit;

/// An EXI feature this crate does not implement, on purpose.
///
/// When one of these shows up, it is always reported as an error. It is never
/// quietly ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Feature {
    /// EXI compression ([EXI 9](https://www.w3.org/TR/exi/#compression)).
    Compression,
    /// Pre-compression alignment
    /// ([EXI 5.4](https://www.w3.org/TR/exi/#options)).
    PreCompression,
    /// Self-contained elements (`SC`)
    /// ([EXI 8.5.4.4.1](https://www.w3.org/TR/exi/#addingProductions)).
    SelfContained,
    /// User-defined value forms for built-in types
    /// ([EXI 7.4](https://www.w3.org/TR/exi/#datatypeRepresentationMap)).
    DatatypeRepresentationMap,
    /// Restricted character sets from `xsd:pattern` facets
    /// ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)).
    ///
    /// A pattern like `[A-Z]{2}[0-9]{4}` has a small alphabet. EXI then writes
    /// each character as a small index into that alphabet instead of a full code
    /// point, so the bytes change. Writing such a value as a plain string gives
    /// bytes that EXIficient and other spec readers decode to garbage, without any
    /// error. Until that form is implemented, the encoder and decoder refuse these
    /// values instead of writing or reading wrong bytes.
    ///
    /// The refusal only happens when a value of that type is written or read. The
    /// schema still compiles, because grammars and event codes do not depend on the
    /// character set. Patterns with a large alphabet (`\d+`, `\p{L}+`, `.*`,
    /// `[^x]`, or anything outside the Basic Multilingual Plane) are not restricted
    /// and work as usual. See
    /// [`DatatypeKind::RestrictedString`](crate::datatype::DatatypeKind::RestrictedString).
    RestrictedCharacterSet,
    /// EXI fragments: several root elements in one body
    /// ([EXI 8.4.2](https://www.w3.org/TR/exi/#builtinFragGrammars),
    /// [EXI 8.5.2](https://www.w3.org/TR/exi/#informedFragGrammars)).
    Fragment,
    /// Built-in element grammars: the grammars used when there is no schema. They
    /// "learn", adding productions as new events show up
    /// ([EXI 8.4.3](https://www.w3.org/TR/exi/#builtinElemGrammars)).
    BuiltInElementGrammar,
    /// Undeclared productions in non-strict schema-informed grammars: the extra
    /// productions that let a stream hold items the schema does not declare
    /// ([EXI 8.5.4.4.1](https://www.w3.org/TR/exi/#addingProductions)).
    NonStrictSchemaInformed,
    /// Wildcards inside an element: `xs:any` (an `SE(*)` or `SE(uri:*)` event) and
    /// `xs:anyAttribute` (`AT(*)`, `AT(uri:*)`)
    /// ([EXI 8.5.4.1.6](https://www.w3.org/TR/exi/#elementTerms)).
    ///
    /// These are strict, schema-declared productions, so they are not a non-strict
    /// feature. They are not implemented yet: a matched name must be written after
    /// its event code, and an element it names is read with that element's global
    /// declaration. A root element that is not declared at all is a different
    /// case, reported as [`Feature::BuiltInElementGrammar`].
    Wildcard,
    /// `Preserve.dtd`: DOCTYPE and entity reference events.
    PreserveDtd,
    /// `Preserve.prefixes`: namespace prefixes and `NS` events.
    PreservePrefixes,
    /// `Preserve.comments`: comment events.
    PreserveComments,
    /// `Preserve.pis`: processing instruction events.
    PreservePis,
    /// `Preserve.lexicalValues`: values kept in their exact text form.
    PreserveLexicalValues,
    /// An entity reference (`ER`) event.
    EntityReference,
    /// EXI format versions other than final version 1
    /// ([EXI 5.3](https://www.w3.org/TR/exi/#version)).
    FormatVersion,
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Feature::Compression => "compression",
            Feature::PreCompression => "pre-compression alignment",
            Feature::SelfContained => "selfContained",
            Feature::DatatypeRepresentationMap => "datatypeRepresentationMap",
            Feature::RestrictedCharacterSet => "restricted character sets",
            Feature::Fragment => "fragment grammars",
            Feature::BuiltInElementGrammar => "built-in (schema-less) element grammars",
            Feature::Wildcard => "wildcards (xs:any, xs:anyAttribute)",
            Feature::NonStrictSchemaInformed => "non-strict schema-informed grammars",
            Feature::PreserveDtd => "Preserve.dtd",
            Feature::PreservePrefixes => "Preserve.prefixes",
            Feature::PreserveComments => "Preserve.comments",
            Feature::PreservePis => "Preserve.pis",
            Feature::PreserveLexicalValues => "Preserve.lexicalValues",
            Feature::EntityReference => "entity references",
            Feature::FormatVersion => "EXI format versions other than 1",
        };
        f.write_str(name)
    }
}

/// Why a [`ByteSource`](crate::io::ByteSource) or [`ByteSink`](crate::io::ByteSink)
/// failed. `rexi-core` builds without `std`, so this takes the place of
/// `std::io::ErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IoErrorKind {
    /// The input ended in the middle of a read.
    UnexpectedEof,
    /// A fixed-size output buffer is full.
    BufferFull,
    /// Any other failure of the underlying reader or writer.
    Other,
}

impl fmt::Display for IoErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IoErrorKind::UnexpectedEof => "unexpected end of input",
            IoErrorKind::BufferFull => "output buffer full",
            IoErrorKind::Other => "reader or writer failed",
        })
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for IoErrorKind {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::UnexpectedEof => IoErrorKind::UnexpectedEof,
            std::io::ErrorKind::WriteZero => IoErrorKind::BufferFull,
            _ => IoErrorKind::Other,
        }
    }
}

/// Every failure `rexi-core` can report.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExiError {
    /// The underlying reader or writer failed.
    Io(IoErrorKind),
    /// The stream ended before the decoder finished.
    Truncated,
    /// The EXI header is badly formed.
    Header(&'static str),
    /// A feature that is documented as not supported, on purpose, was asked for or
    /// found in the stream.
    Unsupported(Feature),
    /// The event does not match any production of the current grammar rule.
    Grammar {
        /// The events the rule allows here.
        expected: String,
        /// The event that was given or found.
        got: String,
    },
    /// A value cannot be written in the datatype the grammar calls for.
    Value {
        /// Name of the datatype, for example `"Integer"`.
        datatype: &'static str,
        /// Why the value does not fit.
        reason: String,
    },
    /// The string table is in a state that does not add up, or a compact identifier
    /// is out of range.
    StringTable(&'static str),
    /// The grammar tables themselves contradict each other.
    Schema(String),
    /// A bug: a rule that this crate counts on always holding was broken.
    Internal(&'static str),
    /// The stream asked for more than one of the decoder's
    /// [`DecodeLimits`](crate::DecodeLimits) allows. This is how a hostile or
    /// broken stream is stopped before it uses up time, memory or stack.
    LimitExceeded {
        /// Which limit was hit.
        limit: Limit,
        /// The limit's value.
        max: u64,
    },
}

impl fmt::Display for ExiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExiError::Io(kind) => write!(f, "I/O error: {kind}"),
            ExiError::Truncated => f.write_str("EXI stream ended prematurely"),
            ExiError::Header(msg) => write!(f, "invalid EXI header: {msg}"),
            ExiError::Unsupported(feature) => write!(f, "unsupported EXI feature: {feature}"),
            ExiError::Grammar { expected, got } => {
                write!(f, "grammar mismatch: expected {expected}, got {got}")
            }
            ExiError::Value { datatype, reason } => {
                write!(f, "value not representable as {datatype}: {reason}")
            }
            ExiError::StringTable(msg) => write!(f, "string table error: {msg}"),
            ExiError::Schema(msg) => write!(f, "schema error: {msg}"),
            ExiError::Internal(msg) => write!(f, "internal error: {msg}"),
            ExiError::LimitExceeded { limit, max } => {
                write!(f, "decoder limit exceeded: {limit} is over {max}")
            }
        }
    }
}

impl core::error::Error for ExiError {}

impl From<IoErrorKind> for ExiError {
    fn from(kind: IoErrorKind) -> Self {
        if kind == IoErrorKind::UnexpectedEof {
            ExiError::Truncated
        } else {
            ExiError::Io(kind)
        }
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for ExiError {
    fn from(err: std::io::Error) -> Self {
        IoErrorKind::from(err).into()
    }
}

/// Result type of every fallible `rexi-core` function.
pub type ExiResult<T> = Result<T, ExiError>;

impl ExiError {
    /// Shortcut for building an [`ExiError::Value`].
    pub fn value(datatype: &'static str, reason: impl Into<String>) -> Self {
        ExiError::Value {
            datatype,
            reason: reason.into(),
        }
    }
}
