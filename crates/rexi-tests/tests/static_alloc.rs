//! With generated static tables, nothing is heap-allocated until
//! the codec is constructed. Getting the tables and every lookup the codec does on
//! them (name resolution, rule and production traversal, type and enumeration
//! queries) is allocation-free.
//!
//! Decision: the codec's string table is owned (`String` entries and `HashMap`
//! indices, pre-populated from the tables' initial partitions), so allocation is
//! accepted at `Encoder::new` / `Decoder::new`. A string table that borrows the
//! initial partitions from the static tables would remove it; that is a separate
//! change to `rexi-core`.
//!
//! One test per binary. The counter is kept per thread, so allocations made by
//! the test harness's other threads (output, timing) are not counted: with one
//! global counter they made this test fail now and then.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use rexi_core::{Encoder, ExiOptions, GrammarTables, HeaderConfig, TypeResolver};
use rexi_tests::generated::order_complex as oc;

struct Counting;

thread_local! {
    // `const` so that reading it never allocates.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// Counts one allocation on this thread. `try_with` fails only while the
/// thread is being torn down; that allocation is not ours to count.
fn count() {
    let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
}

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

// SAFETY: forwards to the system allocator unchanged; only counts calls.
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
        count();
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations_in(f: impl FnOnce()) -> usize {
    let before = allocations();
    f();
    allocations() - before
}

/// Walks the tables the way the codec does; returns a checksum so the work is not
/// optimized away.
fn exercise(g: &impl GrammarTables) -> usize {
    let mut sum = 0usize;
    let root = g.find_qname("urn:rexi:interop:order", "Order");
    assert_eq!(root, Some(oc::LN_ORDER));
    let elem = g.find_global_element(oc::LN_ORDER);
    assert!(elem.is_some());
    for e in g.elements() {
        sum += g.local_name_str(e.qname).map_or(0, str::len);
        sum += g.uri_str(e.qname.uri).map_or(0, str::len);
        let Some(t) = g.type_decl(e.type_idx) else {
            continue;
        };
        sum += usize::from(t.typecastable) + g.datatype(e.type_idx).name().len();
        let Some(start) = g.grammar_start_rule(t.grammar) else {
            continue;
        };
        for p in g.rule_productions(start) {
            sum += p.next as usize % 7;
        }
    }
    for set in 0..g.enum_sets().len() as u32 {
        for i in 0..g.enum_count(set) {
            sum += g.enum_value(set, i).map_or(0, str::len);
        }
    }
    sum
}

#[test]
fn static_tables_allocate_nothing_before_the_codec() {
    // Sanity: the counter sees allocations.
    assert!(allocations_in(|| drop(std::hint::black_box(vec![1u8; 64]))) > 0);

    let mut checksum = 0;
    let n = allocations_in(|| checksum = exercise(&&oc::GRAMMARS));
    assert_eq!(n, 0, "static table access allocated {n} times");
    assert!(checksum > 0);

    // Runtime compilation, by contrast, allocates; and the codec allocates once
    // constructed (accepted, see the module docs).
    let case = rexi_tests::cases()
        .into_iter()
        .find(|c| c.name == "order-complex")
        .unwrap_or_else(|| panic!("order-complex case missing"));
    let xsd = case.main_xsd.clone();
    assert!(allocations_in(|| drop(rexi_schema::compile_files(&[xsd.as_path()]))) > 0);

    let out = Vec::with_capacity(4096);
    let at_new = allocations_in(|| {
        let enc = Encoder::new(
            out,
            &oc::GRAMMARS,
            ExiOptions::strict(),
            HeaderConfig::default(),
        );
        assert!(enc.is_ok());
    });
    eprintln!("Encoder::new with static tables: {at_new} allocations (string table)");
}
