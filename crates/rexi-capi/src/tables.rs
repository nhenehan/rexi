//! Grammar table handles. A handle holds either tables compiled from XSD at load
//! time, or a static `GrammarSet` that `rexi-schema-gen` wrote ahead of time. The
//! tables hold the schema-informed grammars
//! ([EXI 8.5](https://www.w3.org/TR/exi/#informedGrammars)) and the starting
//! string table entries for one schema.

use std::ffi::c_char;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use rexi_core::grammar::options_grammar::OPTIONS_GRAMMARS;
use rexi_core::grammar::{
    ElementDecl, GlobalAttribute, Grammar, GrammarSet, GrammarSetBuf, GrammarTables, Production,
    Rule, Span, UriEntry,
};
use rexi_core::ids::{ElemIdx, StrIdx};

use crate::status::{Fail, RexiStatus, c_str, guard};

enum Tables {
    Owned(Box<GrammarSetBuf>),
    Static(&'static GrammarSet<'static>),
}

/// Shared ownership of tables (a reference-counted `Arc`), so encoders and
/// decoders stay valid if the C caller frees the `RexiTables` handle first.
#[derive(Clone)]
pub(crate) struct SharedTables(Arc<Tables>);

// Implements `GrammarTables` for `SharedTables` by forwarding each listed method
// to the owned or static tables inside.
macro_rules! delegate {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        impl GrammarTables for SharedTables {
            $(fn $name(&self $(, $arg: $ty)*) -> $ret {
                match &*self.0 {
                    Tables::Owned(t) => t.$name($($arg),*),
                    Tables::Static(t) => t.$name($($arg),*),
                }
            })*
        }
    };
}

delegate! {
    string(i: StrIdx) -> &str;
    uris() -> &[UriEntry];
    prefixes() -> &[StrIdx];
    local_names() -> &[StrIdx];
    productions() -> &[Production];
    rules() -> &[Rule];
    grammars() -> &[Grammar];
    elements() -> &[ElementDecl];
    global_elements() -> &[ElemIdx];
    global_attributes() -> &[GlobalAttribute];
    types() -> &[rexi_core::grammar::TypeDecl];
    enum_sets() -> &[Span];
    enum_values() -> &[StrIdx];
    schema_id() -> Option<&str>;
}

/// Compiled grammar tables for one schema. C sees only an opaque pointer.
///
/// Each encoder and decoder holds its own reference, so a handle may be freed
/// while they are still in use. A handle is not changed after it is built, so
/// many encoders and decoders may share one.
pub struct RexiTables {
    pub(crate) tables: SharedTables,
}

impl RexiTables {
    /// Wraps tables that `rexi-schema-gen` wrote. This is for Rust crates that
    /// build a generated schema into their code and hand it to C. Pass the result
    /// through `Box::into_raw`; C frees it with `rexi_tables_free`.
    pub fn from_static(set: &'static GrammarSet<'static>) -> Box<RexiTables> {
        Box::new(RexiTables {
            tables: SharedTables(Arc::new(Tables::Static(set))),
        })
    }

    /// Wraps tables compiled at runtime.
    pub fn from_owned(buf: GrammarSetBuf) -> Box<RexiTables> {
        Box::new(RexiTables {
            tables: SharedTables(Arc::new(Tables::Owned(Box::new(buf)))),
        })
    }
}

static EXI_OPTIONS: LazyLock<RexiTables> = LazyLock::new(|| RexiTables {
    tables: SharedTables(Arc::new(Tables::Static(&OPTIONS_GRAMMARS))),
});

/// Borrows a tables handle. Returns `ErrNullArgument` for NULL.
///
/// # Safety
/// `p` is NULL or a live handle from this library.
pub(crate) unsafe fn tables_ref<'a>(p: *const RexiTables) -> Result<&'a RexiTables, Fail> {
    // SAFETY: per the caller's contract.
    unsafe { p.as_ref() }.ok_or_else(|| Fail::null("tables"))
}

fn store(out: *mut *mut RexiTables, tables: Box<RexiTables>) -> Result<RexiStatus, Fail> {
    if out.is_null() {
        return Err(Fail::null("out"));
    }
    // SAFETY: `out` is non-null and points to writable storage per the API contract.
    unsafe { *out = Box::into_raw(tables) };
    Ok(RexiStatus::Ok)
}

fn schema_fail(e: rexi_schema::SchemaError) -> Fail {
    Fail::new(RexiStatus::ErrSchema, e.to_string())
}

/// Loads and compiles XML Schema files. `paths[0]` is the main schema. The rest are
/// extra schema documents, for namespaces that are imported without a
/// `schemaLocation`.
///
/// On success `*out` gets a new handle that the caller owns and must release with
/// `rexi_tables_free`. On failure `*out` is not written. Schema errors return
/// `ErrSchema`.
///
/// # Safety
/// - `paths` points to an array of `count` readable pointers. `count` is the
///   number of paths, not bytes. A NULL `paths` returns `ErrNullArgument`, and a
///   `count` of 0 returns `ErrInvalidArgument`.
/// - Each entry is a NUL-terminated UTF-8 file path. A NULL entry returns
///   `ErrNullArgument`.
/// - `out` points to writable storage for one pointer. NULL returns
///   `ErrNullArgument` (the schema is still compiled first).
/// - rexi only reads the paths during the call; the caller keeps ownership.
///
/// Includes and imports may only read files under the main schema's folder; see
/// `rexi_tables_from_xsd_with_root`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_tables_from_xsd(
    paths: *const *const c_char,
    count: usize,
    out: *mut *mut RexiTables,
) -> RexiStatus {
    // SAFETY: the same contract, with no root.
    unsafe { rexi_tables_from_xsd_with_root(paths, count, std::ptr::null(), out) }
}

/// `rexi_tables_from_xsd`, where `xs:include` and `xs:import` may read files under
/// `root` instead of the main schema's folder. NULL `root` means the default; `"/"`
/// allows any file (only for schemas you trust). A file outside the root, a URL, a
/// file that is not a regular file, or a file over 16 MiB gives `ErrSchema`.
///
/// # Safety
/// The same as `rexi_tables_from_xsd`, and `root` is NULL or a NUL-terminated UTF-8
/// path, only read during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_tables_from_xsd_with_root(
    paths: *const *const c_char,
    count: usize,
    root: *const c_char,
    out: *mut *mut RexiTables,
) -> RexiStatus {
    guard(|| {
        if paths.is_null() {
            return Err(Fail::null("paths"));
        }
        if count == 0 {
            return Err(Fail::invalid("no schema paths given"));
        }
        let mut owned = Vec::with_capacity(count);
        for i in 0..count {
            // SAFETY: `paths` has `count` entries per the contract.
            let p = unsafe { *paths.add(i) };
            // SAFETY: each entry is a NUL-terminated string per the contract.
            owned.push(PathBuf::from(unsafe { c_str(p, "paths[i]") }?));
        }
        let refs: Vec<&Path> = owned.iter().map(PathBuf::as_path).collect();
        // SAFETY: per the contract.
        let options = unsafe { load_options(root) }?;
        let buf = rexi_schema::compile_files_with(&refs, &options).map_err(schema_fail)?;
        store(out, RexiTables::from_owned(buf))
    })
}

/// The load rules for a C `root` argument: NULL for the default (under the main
/// schema's folder), otherwise under that folder. `"/"` allows any file.
///
/// # Safety
/// `root` is NULL or a NUL-terminated UTF-8 path.
unsafe fn load_options(root: *const c_char) -> Result<rexi_schema::SchemaLoadOptions, Fail> {
    let options = rexi_schema::SchemaLoadOptions::default();
    if root.is_null() {
        return Ok(options);
    }
    // SAFETY: per the caller's contract.
    let dir = unsafe { c_str(root, "root") }?;
    Ok(options.with_root(rexi_schema::SchemaRoot::Folder(PathBuf::from(dir))))
}

/// Compiles a schema held in memory. `base_dir` is the folder used to find
/// relative `xs:import` and `xs:include` locations. It may be NULL.
///
/// On success `*out` gets a new handle that the caller owns and must release with
/// `rexi_tables_free`. On failure `*out` is not written. Schema errors return
/// `ErrSchema`.
///
/// # Safety
/// - `xsd` is the NUL-terminated UTF-8 schema text. NULL returns
///   `ErrNullArgument`.
/// - `base_dir` is NULL or a NUL-terminated UTF-8 path.
/// - `out` points to writable storage for one pointer. NULL returns
///   `ErrNullArgument` (the schema is still compiled first).
/// - rexi only reads the strings during the call; the caller keeps ownership.
///
/// Includes and imports may only read files under `base_dir`; with a NULL
/// `base_dir` they are refused. See `rexi_tables_from_xsd_string_with_root`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_tables_from_xsd_string(
    xsd: *const c_char,
    base_dir: *const c_char,
    out: *mut *mut RexiTables,
) -> RexiStatus {
    // SAFETY: the same contract, with no root.
    unsafe { rexi_tables_from_xsd_string_with_root(xsd, base_dir, std::ptr::null(), out) }
}

/// `rexi_tables_from_xsd_string`, where `xs:include` and `xs:import` may read
/// files under `root` (NULL: the default, under `base_dir`; `"/"`: any file).
///
/// # Safety
/// The same as `rexi_tables_from_xsd_string`, and `root` is NULL or a
/// NUL-terminated UTF-8 path, only read during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_tables_from_xsd_string_with_root(
    xsd: *const c_char,
    base_dir: *const c_char,
    root: *const c_char,
    out: *mut *mut RexiTables,
) -> RexiStatus {
    guard(|| {
        // SAFETY: per the contract.
        let xsd = unsafe { c_str(xsd, "xsd") }?;
        let base = if base_dir.is_null() {
            None
        } else {
            // SAFETY: per the contract.
            Some(Path::new(unsafe { c_str(base_dir, "base_dir") }?))
        };
        // SAFETY: per the contract.
        let options = unsafe { load_options(root) }?;
        let buf = rexi_schema::compile_str_with(xsd, base, &options).map_err(schema_fail)?;
        store(out, RexiTables::from_owned(buf))
    })
}

/// The grammar of the EXI Options document, built from the schema in
/// [EXI Appendix C](https://www.w3.org/TR/exi/#optionsSchema), as a static table.
/// Never NULL. The handle lives for the whole program. Passing it to
/// `rexi_tables_free` does nothing.
#[unsafe(no_mangle)]
pub extern "C" fn rexi_tables_exi_options() -> *const RexiTables {
    &*EXI_OPTIONS
}

/// Releases a tables handle. NULL is ignored, and so is the handle from
/// `rexi_tables_exi_options`. Encoders and decoders made from it stay valid,
/// because each holds its own reference.
///
/// # Safety
/// `tables` is NULL, or a handle from this library that has not been freed yet.
/// Do not use the handle after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_tables_free(tables: *mut RexiTables) {
    if tables.is_null() || std::ptr::eq(tables, &*EXI_OPTIONS) {
        return;
    }
    // SAFETY: allocated by `Box::into_raw` in this library, freed once.
    drop(unsafe { Box::from_raw(tables) });
}
