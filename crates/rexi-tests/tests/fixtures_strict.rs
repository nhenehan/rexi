//! Milestone 1 gate: the five interop fixtures, byte-exact against EXIficient 1.0.7
//! in the strict profiles, decoded back to the oracle's event stream, re-encoded
//! from the decoded events, and accepted by exipd.

mod common;

use common::{PROFILES, compile};
use rexi_tests::cases;

#[test]
fn fixtures_present() {
    let cs = cases();
    assert_eq!(cs.len(), 6, "expected six cases in cases.tsv");
    for c in &cs {
        assert!(c.xml.is_file(), "{:?}", c.xml);
        for x in c.xsd_paths() {
            assert!(x.is_file(), "{x:?}");
        }
        for p in PROFILES {
            assert!(c.reference(p.name).is_file(), "{:?}", c.reference(p.name));
            assert!(
                c.events_file(p.name).is_file(),
                "{:?}",
                c.events_file(p.name)
            );
        }
    }
}

/// (a) encode(xml, xsd) == EXIficient reference bytes.
#[test]
fn encode_matches_exificient() {
    common::encode_matches_exificient(compile);
}

/// (b) decode(reference) yields exactly the oracle's event stream.
#[test]
fn decode_matches_oracle_events() {
    common::decode_matches_oracle_events(compile);
}

/// (c) re-encoding the decoded (typed) events reproduces the reference bytes.
#[test]
fn reencode_decoded_events_round_trips() {
    common::reencode_decoded_events_round_trips(compile);
}

/// (e) exipd, the C reference implementation, decodes rexi's output.
#[test]
fn exipd_accepts_rexi_output() {
    common::exipd_accepts_rexi_output(compile);
}

/// The manifest describes every reference file we rely on, all from EXIficient 1.0.7.
#[test]
fn manifest_is_consistent() {
    let rows = rexi_tests::manifest();
    assert!(!rows.is_empty());
    for row in rows.iter().filter(|r| r.file.ends_with(".exi")) {
        let path = rexi_tests::fixtures_dir().join(&row.file);
        let bytes = std::fs::read(&path).unwrap_or_default();
        assert_eq!(bytes.len() as u64, row.bytes, "{}", row.file);
        assert_eq!(row.exificient_version, "1.0.7", "{}", row.file);
    }
}
