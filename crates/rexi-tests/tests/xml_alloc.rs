//! Encoding from XML does not allocate per element.
//!
//! The XML front end hands the encoder borrowed events and reuses its buffers,
//! and the encoder reuses its text buffer, so a document with many elements
//! makes about as many allocations as one with few. Only growth of the reused
//! buffers and the codec's own setup allocate. This test holds that gain.
//!
//! One test per binary. The counter is kept per thread (see `static_alloc.rs`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Write;

use rexi_core::{ExiOptions, HeaderConfig};

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

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
    targetNamespace="urn:t" xmlns="urn:t" elementFormDefault="qualified">
  <xs:element name="r">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="v" maxOccurs="unbounded">
          <xs:complexType>
            <xs:simpleContent>
              <xs:extension base="xs:string">
                <xs:attribute name="n" type="xs:int"/>
                <xs:attribute name="b" type="xs:boolean"/>
              </xs:extension>
            </xs:simpleContent>
          </xs:complexType>
        </xs:element>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

/// A document with `n` `v` elements, each with two attributes (written out of
/// order, so they are sorted), text that needs no copy and text with an entity
/// reference and a line end. The text repeats, so the string table finds it
/// and adds nothing.
fn document(n: usize) -> String {
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<r xmlns=\"urn:t\">\n");
    for i in 0..n {
        let text = if i % 2 == 0 {
            "plain"
        } else {
            "a &amp; b\r\nc"
        };
        let _ = writeln!(xml, "  <v n=\"{i}\" b=\"true\">{text}</v>");
    }
    xml.push_str("</r>\n");
    xml
}

#[test]
fn encoding_from_xml_does_not_allocate_per_element() {
    let tables = rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"));
    let encode = |xml: &str| {
        let before = allocations();
        let exi = rexi_xml::encode_xml(
            xml.as_bytes(),
            &tables,
            ExiOptions::strict(),
            HeaderConfig::default(),
        );
        let n = allocations() - before;
        assert!(exi.is_ok(), "{exi:?}");
        n
    };
    let (small, large) = (document(10), document(1000));
    let at_10 = encode(&small);
    let at_1000 = encode(&large);
    eprintln!("encode_xml allocations: 10 elements {at_10}, 1000 elements {at_1000}");
    // Buffers grow by doubling, so 100 times the elements adds a handful of
    // allocations, not hundreds.
    assert!(
        at_1000 <= at_10 + 4,
        "1000 elements: {at_1000} allocations, 10 elements: {at_10}"
    );
}
