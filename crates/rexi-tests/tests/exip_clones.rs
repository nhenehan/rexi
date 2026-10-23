//! Clones of exip's C unit tests that exercise the codec end to end. exip's test
//! data lives in the pinned exip checkout in the third-party cache
//! (`scripts/fetch-third-party.sh`).

use rexi_core::{ExiOptions, OwnedEvent, OwnedValue};
use rexi_tests::third_party_dir;

fn exip_test_set() -> std::path::PathBuf {
    third_party_dir().join("exip/tests/test-set")
}

fn se(local: &str) -> OwnedEvent {
    OwnedEvent::StartElement {
        uri: "urn:foo".into(),
        local: local.into(),
    }
}

fn ch(text: &str) -> OwnedEvent {
    OwnedEvent::Characters(OwnedValue::String(text.into()))
}

/// check_strict_grammar.c :: test_acceptance_for_A_01: a strict stream produced by
/// exip (options in the header) decodes to the expected element sequence.
#[test]
fn exip_acceptance_a_01_decodes() {
    let dir = exip_test_set().join("testStates");
    if !dir.join("acceptance_a_01.exi").is_file() {
        assert!(
            !rexi_tests::judges_required(),
            "exip test data missing from the third-party cache"
        );
        eprintln!("exip not fetched (scripts/fetch-third-party.sh); skipping");
        return;
    }
    let xsd = dir.join("acceptance.xsd");
    let tables = rexi_schema::compile_files(&[xsd.as_path()]).unwrap_or_else(|e| panic!("{e}"));
    let bytes = std::fs::read(dir.join("acceptance_a_01.exi")).unwrap_or_default();
    let events =
        rexi_xml::decode_events_typed(bytes.as_slice(), &tables, Some(ExiOptions::strict()))
            .unwrap_or_else(|e| panic!("{e}"));
    let mut expected = vec![OwnedEvent::StartDocument, se("A")];
    for name in ["AB", "AC", "AC", "AD", "AE"] {
        expected.push(se(name));
        expected.push(ch(""));
        expected.push(OwnedEvent::EndElement);
    }
    expected.push(OwnedEvent::EndElement);
    expected.push(OwnedEvent::EndDocument);
    assert_eq!(events, expected);
}

/// check_contentIO.c :: test_decodeHeader: `80 ...` is a header without cookie and
/// without options; `$EXI 80 ...` the same with the cookie.
#[test]
fn exip_decode_header_vectors() {
    use rexi_core::header::read_header_bits;
    use rexi_core::io::BitReader;
    let mut r = BitReader::new([0x80u8, 0x60, 0x7C].as_slice());
    let info = read_header_bits(&mut r).unwrap_or_else(|e| panic!("{e}"));
    assert!(!info.cookie && !info.options_in_stream && info.format_version == 1);
    assert_eq!(r.bit_position(), 8);
    let mut r = BitReader::new(b"$EXI\x80\x60\x7C".as_slice());
    let info = read_header_bits(&mut r).unwrap_or_else(|e| panic!("{e}"));
    assert!(info.cookie && !info.options_in_stream);
    assert_eq!(r.bit_position(), 40);
}
