//! Starting a stream costs a handful of allocations, whatever the
//! schema, because the string table references the grammar's initial entries
//! instead of copying them.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rexi_core::encoder::init_string_table;
use rexi_core::grammar::options_grammar::OPTIONS_GRAMMARS;
use rexi_core::{
    Decoder, Encoder, ExiOptions, GrammarSetBuf, GrammarTables, HeaderConfig, SchemaId,
};
use rexi_tests::{cases, generated};

struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Allocations of this thread only (tests run in parallel).
    static MINE: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

fn count() {
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    let _ = MINE.try_with(|c| c.set(c.get() + 1));
}

#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = (layout, new_size);
        count();
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Allocations made by `f` on this thread.
fn allocations_in<T>(f: impl FnOnce() -> T) -> (usize, T) {
    let before = MINE.with(core::cell::Cell::get);
    let out = f();
    (MINE.with(core::cell::Cell::get) - before, out)
}

/// Allocations to start a stream with options in the header, for any schema:
/// once 423, then 12, now 8 with the small value
/// partitions (two allocations for the options document's value table).
const BUDGET: usize = 8;

fn compile(paths: &[PathBuf]) -> GrammarSetBuf {
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    rexi_schema::compile_files(&refs).unwrap_or_else(|e| panic!("{paths:?}: {e}"))
}

fn strict_with_schema_id() -> ExiOptions {
    let mut options = ExiOptions::strict();
    options.schema_id = SchemaId::Id("s".into());
    options
}

/// Every grammar lends its initial entries, and their partitions are sorted
/// with no duplicates, so lookups by text use binary search.
#[test]
fn every_grammar_lends_its_initial_entries() {
    let options = ExiOptions::strict();
    let st = init_string_table(&OPTIONS_GRAMMARS, &options);
    assert!(st.borrows_initial_entries() && st.initial_entries_sorted(&OPTIONS_GRAMMARS));
    for case in cases() {
        let statics = generated::by_case(&case.name).unwrap_or_else(|| panic!("{}", case.name));
        let st = init_string_table(statics, &options);
        assert!(
            st.borrows_initial_entries() && st.initial_entries_sorted(statics),
            "{}",
            case.name
        );
        let runtime = compile(&case.xsd_paths());
        let st = init_string_table(&runtime, &options);
        assert!(
            st.borrows_initial_entries() && st.initial_entries_sorted(&runtime),
            "{}",
            case.name
        );
    }
}

fn check_setup(name: &str, tables: &impl GrammarTables) {
    let header = HeaderConfig::new().with_include_options(true);
    let (n, enc) =
        allocations_in(|| Encoder::new(Vec::new(), tables, strict_with_schema_id(), header));
    let Ok(enc) = enc else {
        panic!("{name}: encoder");
    };
    assert!(n <= BUDGET, "{name}: Encoder::new made {n} allocations");
    let enc_n = n;
    let stream = enc.into_writer().finish().unwrap_or_default();
    let (n, dec) = allocations_in(|| Decoder::new(&stream[..], tables, None));
    assert!(dec.is_ok(), "{name}: decoder");
    assert!(n <= BUDGET, "{name}: Decoder::new made {n} allocations");
    println!("{name}: encoder {enc_n}, decoder {n} allocations");
}

#[test]
fn starting_a_stream_stays_within_the_allocation_budget() {
    check_setup("options grammar", &OPTIONS_GRAMMARS);
    for case in cases() {
        let statics = generated::by_case(&case.name).unwrap_or_else(|| panic!("{}", case.name));
        check_setup(&case.name, statics);
        check_setup(
            &format!("{} (runtime)", case.name),
            &compile(&case.xsd_paths()),
        );
    }
}

/// After the header, decoding allocates only for string text (the
/// value buffer, new string-table entries, `xs:list` items), not for integers,
/// floats, enumerations or structure. Every strict bit-packed interop fixture, event by event.
#[test]
fn decoding_allocates_only_for_string_text() {
    use rexi_core::{Event, Value};
    let options = ExiOptions::strict();
    for case in cases() {
        let statics = generated::by_case(&case.name).unwrap_or_else(|| panic!("{}", case.name));
        let path = case
            .xml
            .with_file_name(format!("{}.strict-bp.exi", case.name));
        let stream = std::fs::read(&path).unwrap_or_default();
        let Ok(mut dec) = Decoder::new(&stream[..], statics, Some(options.clone())) else {
            panic!("{}: decoder", case.name);
        };
        let (mut strings, mut other_allocs, mut string_allocs) = (0usize, 0usize, 0usize);
        loop {
            // Classify inside the counted closure: nothing is copied out.
            let (n, kind) = allocations_in(|| match dec.next_event() {
                Ok(Some(
                    Event::Attribute(_, Value::String(_) | Value::List(_))
                    | Event::Characters(Value::String(_) | Value::List(_)),
                )) => Some(true),
                Ok(Some(_)) => Some(false),
                Ok(None) => None,
                Err(e) => panic!("{}: {e}", case.name),
            });
            match kind {
                Some(true) => {
                    strings += 1;
                    string_allocs += n;
                }
                Some(false) => other_allocs += n,
                None => break,
            }
        }
        println!(
            "{}: {strings} string values ({string_allocs} allocations), other events {other_allocs} allocations",
            case.name
        );
        // Structure, integers and floats: only the element stack's first growth.
        assert!(
            other_allocs <= 2,
            "{}: {other_allocs} allocations outside strings",
            case.name
        );
    }
}
