//! Error type for schema loading and grammar compilation.

use std::fmt;
use std::path::PathBuf;

/// Everything that can go wrong while compiling XML Schema documents to grammars.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemaError {
    /// A schema document could not be read.
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// What the operating system said.
        message: String,
    },
    /// A schema document is not well-formed XML.
    Xml {
        /// The file name or other label of the document.
        source: String,
        /// What the XML parser said.
        message: String,
    },
    /// A schema construct this compiler does not implement (`xs:redefine`, ...).
    Unsupported(String),
    /// A schema that is badly formed, or breaks a rule this compiler depends on.
    Invalid(String),
    /// A schema file that the load rules do not allow to be read: outside the root
    /// folder, a URL, not a regular file, or too big (see
    /// [`SchemaLoadOptions`](crate::SchemaLoadOptions)).
    Refused {
        /// The `schemaLocation` or path that was asked for.
        location: String,
        /// Which rule refused it.
        reason: String,
    },
    /// One type's content model grows past `max_grammar_size` once group
    /// references and `maxOccurs` copies are expanded (see
    /// [`SchemaLoadOptions`](crate::SchemaLoadOptions)).
    TooLarge {
        /// The limit that was passed.
        max: usize,
    },
    /// A reference to a component that is not declared anywhere.
    Unresolved {
        /// What kind of component was looked for ("type", "element", ...).
        kind: &'static str,
        /// The name that was not found, as `{ns}local`.
        qname: String,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::Io { path, message } => {
                write!(f, "cannot read schema {}: {message}", path.display())
            }
            SchemaError::Xml { source, message } => {
                write!(f, "schema {source} is not well-formed: {message}")
            }
            SchemaError::Unsupported(what) => write!(f, "unsupported schema construct: {what}"),
            SchemaError::Invalid(what) => write!(f, "invalid schema: {what}"),
            SchemaError::Refused { location, reason } => {
                write!(f, "schema location '{location}' refused: {reason}")
            }
            SchemaError::TooLarge { max } => write!(
                f,
                "schema too large: one type's content expands past {max} (raise max_grammar_size to allow it)"
            ),
            SchemaError::Unresolved { kind, qname } => write!(f, "unresolved {kind} {qname}"),
        }
    }
}

impl std::error::Error for SchemaError {}

/// The result type of this crate's functions.
pub type Result<T> = std::result::Result<T, SchemaError>;
