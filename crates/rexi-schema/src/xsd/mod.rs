//! XML Schema handling, in four parts:
//!
//! - `loader` reads schema documents and follows their imports and includes;
//! - `raw` is the syntax tree (AST) of each document, as written;
//! - `resolve` looks up every name reference and links the parts together;
//! - `model` is the result: the resolved schema that grammars are built from.
//!
//! See [XML Schema 1.0 Part 1: Structures](https://www.w3.org/TR/xmlschema-1/).

pub mod loader;
pub mod model;
pub mod raw;
pub mod resolve;

pub use loader::Loader;
pub use resolve::resolve_with;
