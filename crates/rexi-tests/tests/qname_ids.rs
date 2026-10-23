//! `Encoder::start_element_id` / `attribute_id` give the same bytes as
//! the string forms, and `find_qname`'s binary search finds what a scan finds.

use rexi_core::{Encoder, Event, ExiOptions, GrammarTables, HeaderConfig};
use rexi_tests::{cases, generated};

/// Every strict bit-packed interop fixture, decoded and re-encoded with the
/// qnames of start elements and attributes resolved up front.
#[test]
fn id_entry_points_reproduce_the_fixtures() {
    for case in cases() {
        let tables = generated::by_case(&case.name).unwrap_or_else(|| panic!("{}", case.name));
        let path = case
            .xml
            .with_file_name(format!("{}.strict-bp.exi", case.name));
        let expected = std::fs::read(&path).unwrap_or_default();
        let events =
            rexi_xml::decode_events_typed(expected.as_slice(), tables, Some(ExiOptions::strict()))
                .unwrap_or_else(|e| panic!("{}: decode: {e}", case.name));
        let Ok(mut enc) = Encoder::new(
            Vec::new(),
            tables,
            ExiOptions::strict(),
            HeaderConfig::default(),
        ) else {
            panic!("{}: encoder", case.name);
        };
        for ev in &events {
            let ev = ev.as_event();
            let r = match &ev {
                Event::StartElement(q) => match tables.find_qname(q.uri, q.local) {
                    Some(id) => enc.start_element_id(id),
                    None => enc.push(&ev),
                },
                Event::Attribute(q, v) => match tables.find_qname(q.uri, q.local) {
                    Some(id) => enc.attribute_id(id, v),
                    None => enc.push(&ev),
                },
                other => enc.push(other),
            };
            r.unwrap_or_else(|e| panic!("{}: encode: {e}", case.name));
        }
        let out = enc
            .finish()
            .unwrap_or_else(|e| panic!("{}: {e}", case.name));
        assert_eq!(out, expected, "{}", case.name);
    }
}

/// The binary search agrees with a linear scan for every name of every
/// generated table, and for names that are absent.
#[test]
fn find_qname_matches_a_linear_scan() {
    for case in cases() {
        let t = generated::by_case(&case.name).unwrap_or_else(|| panic!("{}", case.name));
        for (u, entry) in t.uris().iter().enumerate() {
            let uri = t.string(entry.uri);
            let names = t
                .local_names()
                .get(entry.local_names.range())
                .unwrap_or_default();
            for (i, &s) in names.iter().enumerate() {
                let local = t.string(s);
                let scan = names
                    .iter()
                    .position(|&x| t.string(x) == local)
                    .unwrap_or(usize::MAX);
                assert_eq!(scan, i, "{}: duplicate name {local}", case.name);
                assert_eq!(
                    t.find_qname(uri, local),
                    Some(rexi_core::QNameId::new(u as u32, i as u32)),
                    "{}: {uri} {local}",
                    case.name
                );
            }
            assert_eq!(t.find_qname(uri, "~not-declared~"), None);
        }
    }
}

/// A hand-built table whose local names are not sorted still resolves.
#[test]
fn find_qname_falls_back_on_unsorted_tables() {
    use rexi_core::grammar::{GrammarSet, Span, UriEntry};
    static STRINGS: [&str; 4] = ["", "zeta", "alpha", "mid"];
    static URIS: [UriEntry; 1] = [UriEntry {
        uri: 0,
        prefixes: Span::new(0, 0),
        local_names: Span::new(0, 3),
    }];
    static LOCALS: [u32; 3] = [1, 2, 3];
    let t = GrammarSet {
        strings: &STRINGS,
        uris: &URIS,
        local_names: &LOCALS,
        ..GrammarSet::EMPTY
    };
    for (i, name) in ["zeta", "alpha", "mid"].iter().enumerate() {
        assert_eq!(
            t.find_qname("", name),
            Some(rexi_core::QNameId::new(0, i as u32)),
            "{name}"
        );
    }
    assert_eq!(t.find_qname("", "beta"), None);
}

/// The string table borrows unsorted initial partitions too. Ids stay
/// positions in the partition, lookups by text fall back to a linear scan, and
/// a name added later gets the next id.
#[test]
fn string_table_falls_back_on_unsorted_tables() {
    use rexi_core::grammar::{GrammarSet, Span, UriEntry};
    use rexi_core::string_table::StringTable;
    static STRINGS: [&str; 4] = ["", "zeta", "alpha", "mid"];
    static URIS: [UriEntry; 1] = [UriEntry {
        uri: 0,
        prefixes: Span::new(0, 0),
        local_names: Span::new(0, 3),
    }];
    static LOCALS: [u32; 3] = [1, 2, 3];
    let t = GrammarSet {
        strings: &STRINGS,
        uris: &URIS,
        local_names: &LOCALS,
        ..GrammarSet::EMPTY
    };
    let mut st = StringTable::from_grammar(&t, None, None);
    assert!(st.borrows_initial_entries());
    assert!(!st.initial_entries_sorted(&t));
    for (i, name) in ["zeta", "alpha", "mid"].iter().enumerate() {
        assert_eq!(st.find_local_name(&t, 0, name), Some(i as u32), "{name}");
        assert_eq!(st.local_name(&t, 0, i as u32), Some(*name));
    }
    assert_eq!(st.find_local_name(&t, 0, "beta"), None);
    assert_eq!(st.add_local_name(&t, 0, "beta"), 3);
    assert_eq!(st.add_local_name(&t, 0, "alpha"), 1);
    assert_eq!(st.local_name_count(0), 4);
}
