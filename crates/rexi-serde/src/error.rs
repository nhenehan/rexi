//! The error type shared by serialization and deserialization.

use crate::prelude::*;
use core::fmt;

use rexi_core::ExiError;

/// What went wrong while turning Rust values into EXI or back.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// The codec rejected an event or a value.
    Exi(ExiError),
    /// The Rust value does not fit the schema (no matching element, attribute or
    /// type), or a serde implementation reported an error.
    Message(String),
}

/// The result type of this crate's functions.
pub type Result<T> = core::result::Result<T, Error>;

impl Error {
    pub(crate) fn msg(m: impl Into<String>) -> Self {
        Error::Message(m.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Exi(e) => write!(f, "{e}"),
            Error::Message(m) => f.write_str(m),
        }
    }
}

impl core::error::Error for Error {}

impl From<ExiError> for Error {
    fn from(e: ExiError) -> Self {
        Error::Exi(e)
    }
}

impl serde::ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::Message(msg.to_string())
    }
}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::Message(msg.to_string())
    }
}
