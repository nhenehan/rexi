//! Turns XSD (XML Schema) files into EXI schema-informed grammars.
//!
//! A grammar is a set of rules that says which events (start tag, attribute, text,
//! end tag, ...) may come next. A schema-informed grammar is built from a schema, so
//! it only allows what the schema allows
//! ([EXI 8.5](https://www.w3.org/TR/exi/#informedGrammars)).
//!
//! The public API is small: [`compile_files`] and [`compile_str`] (and their
//! `_with` forms, which take [`SchemaLoadOptions`]), [`dump`] and
//! [`canonical_dump`] for listings, and [`SchemaError`]. Inside, the steps are:
//!
//! 1. Load the XML Schema documents, following `xs:include` and `xs:import`
//!    under the rules of [`SchemaLoadOptions`].
//! 2. Map simple types to EXI datatypes.
//! 3. Build the proto-grammars (
//!    [EXI 8.5.4.1](https://www.w3.org/TR/exi/#protoGrammars)). A proto-grammar is a
//!    first draft of a grammar that may still have rules with no event and rules that
//!    start with the same event.
//! 4. Normalize them ([EXI 8.5.4.2](https://www.w3.org/TR/exi/#normalizedGrammars)),
//!    which removes both of those problems.
//! 5. Assign event codes ([EXI 8.5.4.3](https://www.w3.org/TR/exi/#eventCodeAssignment)).
//!    An event code is the number written in the stream to say which rule was used.
//! 6. Write out the grammar tables of `rexi-core` as a [`GrammarSetBuf`].
//!
//! Only strict grammars are built. The extra rules for content the schema does not
//! declare ("undeclared productions",
//! [EXI 8.5.4.4](https://www.w3.org/TR/exi/#undeclaredProductions)) are left to the
//! runtime.
//!
//! ## Where this crate fits
//!
//! `rexi-core` encodes and decodes, but it needs grammar tables to know what a
//! document may hold. This crate makes those tables from XSD files while the
//! program runs. `rexi-schema-gen` uses this crate at build time instead and writes
//! the tables out as Rust source, so a program (or a `no_std` device) can carry
//! them as a `static` and skip this crate. `rexi-xml`, `rexi-serde` and `rexi-capi`
//! all take the tables from either path.
//!
//! ```
//! let xsd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
//!                <xs:element name="note" type="xs:string"/>
//!              </xs:schema>"#;
//! let grammars = rexi_schema::compile_str(xsd, None)?;
//! # let _ = grammars;
//! # Ok::<(), rexi_schema::SchemaError>(())
//! ```

#![warn(missing_docs)]

mod builtin;
mod charset;
mod datatype_map;
mod dump;
mod error;
mod load_options;
mod proto;
mod xsd;

use std::path::Path;

pub use dump::{canonical_dump, dump};
pub use error::SchemaError;
pub use load_options::{SchemaLoadOptions, SchemaRoot};
pub use rexi_core::grammar::GrammarSetBuf;

use xsd::model::Model;

/// Compiles schema files. The first file is the main schema. The others are extra
/// documents, used for namespaces that are imported without a `schemaLocation`.
///
/// Includes and imports may only read files under the main files' folders; see
/// [`compile_files_with`] to change that.
pub fn compile_files(paths: &[&Path]) -> Result<GrammarSetBuf, SchemaError> {
    compile_files_with(paths, &SchemaLoadOptions::default())
}

/// [`compile_files`] with the given rules for reading schema files.
pub fn compile_files_with(
    paths: &[&Path],
    options: &SchemaLoadOptions,
) -> Result<GrammarSetBuf, SchemaError> {
    let model = load_files_with(paths, options)?;
    proto::emit::emit_with(&model, options.max_grammar_size)
}

/// Compiles a schema held in memory. `base_dir` is used to find imports and includes
/// given as relative paths. They may only read files under `base_dir`; with no
/// `base_dir`, the schema may not include or import files at all. See
/// [`compile_str_with`] to change that.
pub fn compile_str(xsd: &str, base_dir: Option<&Path>) -> Result<GrammarSetBuf, SchemaError> {
    compile_str_with(xsd, base_dir, &SchemaLoadOptions::default())
}

/// [`compile_str`] with the given rules for reading schema files.
pub fn compile_str_with(
    xsd: &str,
    base_dir: Option<&Path>,
    options: &SchemaLoadOptions,
) -> Result<GrammarSetBuf, SchemaError> {
    let model = load_str_with(xsd, base_dir, options)?;
    proto::emit::emit_with(&model, options.max_grammar_size)
}

/// Loads schema files and resolves them into a model: every name that points at
/// another schema part is looked up and linked to it.
fn load_files_with(paths: &[&Path], options: &SchemaLoadOptions) -> Result<Model, SchemaError> {
    let mut loader = xsd::Loader::with_options(options.clone())?;
    for p in paths {
        // A bare file name has an empty parent, which means the current folder.
        let dir = p
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        loader.add_main_folder(dir)?;
    }
    for p in paths {
        loader.load_main_file(p)?;
    }
    xsd::resolve_with(&loader.docs, options.max_grammar_size)
}

/// Loads a schema held in memory and resolves it into a model.
fn load_str_with(
    xsd: &str,
    base_dir: Option<&Path>,
    options: &SchemaLoadOptions,
) -> Result<Model, SchemaError> {
    let mut loader = xsd::Loader::with_options(options.clone())?;
    loader.load_str(xsd, base_dir)?;
    xsd::resolve_with(&loader.docs, options.max_grammar_size)
}
