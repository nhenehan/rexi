//! The strict fixture gate, generic over where the grammar tables come from:
//! compiled from XSD at runtime (`fixtures_strict.rs`) or generated ahead of time
//! by `rexi-schema-gen` (`fixtures_strict_static.rs`).

#![allow(dead_code)]

use rexi_core::{Alignment, Encoder, ExiOptions, GrammarTables, HeaderConfig};
use rexi_tests::{Case, cases, describe_diff, first_bit_difference, judges_required, run_exipd};

pub struct Profile {
    pub name: &'static str,
    pub options: fn(&Case) -> ExiOptions,
    pub header: HeaderConfig,
    pub exipd_ops: Option<&'static str>,
}

pub const PROFILES: &[Profile] = &[
    Profile {
        name: "strict-bp",
        options: |_| ExiOptions::strict(),
        header: HeaderConfig::new(),
        exipd_ops: Some(rexi_tests::EXIPD_OPS_STRICT_BITPACKED),
    },
    Profile {
        name: "strict-bp-opts",
        options: |c| ExiOptions::strict().with_schema_id(c.name.clone()),
        header: HeaderConfig::new().with_include_options(true),
        exipd_ops: None,
    },
    Profile {
        name: "strict-ba",
        options: |_| ExiOptions::strict().with_alignment(Alignment::ByteAligned),
        header: HeaderConfig::new(),
        exipd_ops: Some(rexi_tests::EXIPD_OPS_STRICT_BYTEALIGNED),
    },
];

/// Compiles a case's schemas at runtime.
pub fn compile(case: &Case) -> rexi_core::GrammarSetBuf {
    let paths = case.xsd_paths();
    let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
    rexi_schema::compile_files(&refs)
        .unwrap_or_else(|e| panic!("{}: schema compile failed: {e}", case.name))
}

/// The EXIficient reference bytes for a case and profile.
pub fn reference(case: &Case, profile: &Profile) -> Vec<u8> {
    std::fs::read(case.reference(profile.name))
        .unwrap_or_else(|e| panic!("{}: missing reference {}: {e}", case.name, profile.name))
}

/// (a) encode(xml, xsd) == EXIficient reference bytes.
pub fn encode_matches_exificient<G: GrammarTables>(tables_for: impl Fn(&Case) -> G) {
    let mut failures = Vec::new();
    for case in cases() {
        let tables = tables_for(&case);
        let xml = std::fs::read(&case.xml).unwrap_or_default();
        for profile in PROFILES {
            let expected = reference(&case, profile);
            match rexi_xml::encode_xml(&xml, &tables, &(profile.options)(&case), profile.header) {
                Ok(actual) => {
                    if first_bit_difference(&actual, &expected).is_some() {
                        failures.push(describe_diff(
                            &format!("{} {}", case.name, profile.name),
                            &actual,
                            &expected,
                        ));
                    }
                }
                Err(e) => failures.push(format!(
                    "{} {}: encode failed: {e}",
                    case.name, profile.name
                )),
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// (b) decode(reference) yields exactly the oracle's event stream.
pub fn decode_matches_oracle_events<G: GrammarTables>(tables_for: impl Fn(&Case) -> G) {
    let mut failures = Vec::new();
    for case in cases() {
        let tables = tables_for(&case);
        for profile in PROFILES {
            let bytes = reference(&case, profile);
            let oob = if profile.header.include_options {
                None
            } else {
                Some((profile.options)(&case))
            };
            let expected =
                std::fs::read_to_string(case.events_file(profile.name)).unwrap_or_default();
            match rexi_xml::decode_events(bytes.as_slice(), &tables, oob) {
                Ok(events) => {
                    let actual = rexi_xml::events_dump(&events);
                    if actual.trim_end() != expected.trim_end() {
                        let first = actual
                            .lines()
                            .zip(expected.lines())
                            .position(|(a, b)| a != b)
                            .map(|i| {
                                format!(
                                    "line {}: got `{}` want `{}`",
                                    i + 1,
                                    actual.lines().nth(i).unwrap_or_default(),
                                    expected.lines().nth(i).unwrap_or_default()
                                )
                            })
                            .unwrap_or_else(|| "lengths differ".to_string());
                        failures.push(format!(
                            "{} {}: events differ: {first}",
                            case.name, profile.name
                        ));
                    }
                }
                Err(e) => failures.push(format!(
                    "{} {}: decode failed: {e}",
                    case.name, profile.name
                )),
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// (c) re-encoding the decoded (typed) events reproduces the reference bytes.
pub fn reencode_decoded_events_round_trips<G: GrammarTables>(tables_for: impl Fn(&Case) -> G) {
    let mut failures = Vec::new();
    for case in cases() {
        let tables = tables_for(&case);
        for profile in PROFILES {
            let bytes = reference(&case, profile);
            let oob = if profile.header.include_options {
                None
            } else {
                Some((profile.options)(&case))
            };
            let events = match rexi_xml::decode_events_typed(bytes.as_slice(), &tables, oob) {
                Ok(ev) => ev,
                Err(e) => {
                    failures.push(format!(
                        "{} {}: decode failed: {e}",
                        case.name, profile.name
                    ));
                    continue;
                }
            };
            let result = (|| -> Result<Vec<u8>, rexi_core::ExiError> {
                let mut enc = Encoder::new(
                    Vec::new(),
                    &tables,
                    (profile.options)(&case),
                    profile.header,
                )?;
                for ev in &events {
                    enc.push(&ev.as_event())?;
                }
                enc.finish()
            })();
            match result {
                Ok(actual) => {
                    if first_bit_difference(&actual, &bytes).is_some() {
                        failures.push(describe_diff(
                            &format!("{} {} (re-encode)", case.name, profile.name),
                            &actual,
                            &bytes,
                        ));
                    }
                }
                Err(e) => failures.push(format!(
                    "{} {}: re-encode failed: {e}",
                    case.name, profile.name
                )),
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// (e) exipd, the C reference implementation, decodes rexi's output.
pub fn exipd_accepts_rexi_output<G: GrammarTables>(tables_for: impl Fn(&Case) -> G) {
    if rexi_tests::exipd_binary().is_none() {
        assert!(!judges_required(), "exipd judge required but not built");
        eprintln!("exipd not built; skipping judge test");
        return;
    }
    let mut failures = Vec::new();
    for case in cases() {
        let tables = tables_for(&case);
        let xml = std::fs::read(&case.xml).unwrap_or_default();
        for profile in PROFILES {
            let Ok(bytes) =
                rexi_xml::encode_xml(&xml, &tables, &(profile.options)(&case), profile.header)
            else {
                continue; // reported by encode_matches_exificient
            };
            if let Err(e) = run_exipd(profile.exipd_ops, &case.schema_exis(), &bytes) {
                failures.push(format!("{} {}: {e}", case.name, profile.name));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
