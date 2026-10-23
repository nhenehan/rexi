//! Rules for loading a schema: which files `xs:include` and `xs:import` may read,
//! and how big the grammars built from it may grow.
//!
//! A schema names other schema files with `schemaLocation`. Without a rule, a
//! hostile schema could name any file the program can read (`/etc/passwd`,
//! `../../secrets.xml`), or a file that never ends (`/dev/zero`) or never answers
//! (a named pipe). The error messages would then tell an attacker which files exist,
//! and the last two would use up memory or hang. So by default every included or
//! imported file must sit under a root folder: the folder of the main schema.
//!
//! Schema sets that reach outside that folder (UBL's `../common/...`, for example)
//! set the root to the top folder of the set with [`SchemaLoadOptions::with_root`].
//! Schemas you fully trust can use [`SchemaRoot::Anywhere`].

use std::path::PathBuf;

/// Where `xs:include` and `xs:import` may read schema files from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SchemaRoot {
    /// Under the folders of the main schema files (for a schema held in memory,
    /// under its `base_dir`; with no `base_dir`, no file may be read at all).
    #[default]
    MainSchemaFolders,
    /// Under this folder.
    Folder(PathBuf),
    /// Any local file. Only for schemas you trust.
    Anywhere,
}

/// How schema files are read: where includes and imports may come from, and how
/// big one file may be.
///
/// In every mode, only regular files are read (no pipes or devices), a
/// `schemaLocation` that is a URL is refused (nothing is fetched over a network),
/// and a file over `max_file_bytes` is refused. Going against a rule gives
/// [`SchemaError::Refused`](crate::SchemaError::Refused).
///
/// ```
/// use rexi_schema::{SchemaLoadOptions, SchemaRoot};
///
/// // A UBL-style set, where `maindoc/Order.xsd` includes `../common/...`.
/// let opts = SchemaLoadOptions::default().with_root(SchemaRoot::Folder("ubl".into()));
/// assert_eq!(opts.max_file_bytes, Some(SchemaLoadOptions::DEFAULT_MAX_FILE_BYTES));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SchemaLoadOptions {
    /// Where includes and imports may read from.
    pub root: SchemaRoot,
    /// Largest schema file read, in bytes (`None`: no limit).
    pub max_file_bytes: Option<u64>,
    /// How big one type's content model may grow once group references are
    /// replaced by their content and `maxOccurs` copies are made, counted in
    /// grammar rules (`None`: no limit). EXI builds one copy of a particle for each
    /// allowed occurrence ([EXI 8.5.4.1.5](https://www.w3.org/TR/exi/#particles)),
    /// so `maxOccurs="4000000000"`, or groups that each use the one before twice,
    /// would otherwise use up memory.
    pub max_grammar_size: Option<usize>,
}

impl SchemaLoadOptions {
    /// The default size limit for one schema file: 16 MiB, far above any real
    /// schema.
    pub const DEFAULT_MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

    /// The default limit for one type's grammar: 100,000 rules, far above real
    /// schemas (a `maxOccurs` of 10,000 on one element makes about 20,000).
    pub const DEFAULT_MAX_GRAMMAR_SIZE: usize = 100_000;

    /// Sets where includes and imports may read from.
    #[must_use]
    pub fn with_root(mut self, root: SchemaRoot) -> Self {
        self.root = root;
        self
    }

    /// Sets the size limit for one schema file (`None`: no limit).
    #[must_use]
    pub fn with_max_file_bytes(mut self, max: Option<u64>) -> Self {
        self.max_file_bytes = max;
        self
    }

    /// Sets the size limit for one type's grammar (`None`: no limit).
    #[must_use]
    pub fn with_max_grammar_size(mut self, max: Option<usize>) -> Self {
        self.max_grammar_size = max;
        self
    }
}

impl Default for SchemaLoadOptions {
    /// Under the main schema's folder, at most 16 MiB per file and 100,000 grammar
    /// rules per type.
    fn default() -> Self {
        Self {
            root: SchemaRoot::MainSchemaFolders,
            max_file_bytes: Some(Self::DEFAULT_MAX_FILE_BYTES),
            max_grammar_size: Some(Self::DEFAULT_MAX_GRAMMAR_SIZE),
        }
    }
}
