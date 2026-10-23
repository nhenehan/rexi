//! `rexi-schema-gen` output: the generated statics are exactly the runtime-compiled
//! tables, regenerating them is a no-op, and the statics generated from the
//! Appendix C schema encode Options documents exactly like the hand-built
//! `OPTIONS_GRAMMARS`.

mod common;

use std::path::{Path, PathBuf};

use rexi_core::grammar::options_grammar::OPTIONS_GRAMMARS;
use rexi_core::header::write_options_document;
use rexi_core::io::BitWriter;
use rexi_core::{
    Alignment, Encoder, ExiOptions, GrammarSet, GrammarSetBuf, GrammarTables, SchemaId,
};
use rexi_schema_gen::{RenderOptions, render};
use rexi_tests::{cases, describe_diff, generated};

fn options_xsd() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/spec/exi-options.xsd")
}

/// (module, static name, schema files) for every generated module, mirroring
/// `scripts/gen-tables.sh`.
fn modules() -> Vec<(String, &'static str, Vec<PathBuf>)> {
    let mut v: Vec<_> = cases()
        .into_iter()
        .map(|c| (c.name.replace('-', "_"), "GRAMMARS", c.xsd_paths()))
        .collect();
    v.push(("exi_options".into(), "EXI_OPTIONS", vec![options_xsd()]));
    v
}

fn compile(paths: &[PathBuf]) -> GrammarSetBuf {
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    rexi_schema::compile_files(&refs).unwrap_or_else(|e| panic!("{paths:?}: {e}"))
}

fn static_for(module: &str) -> &'static GrammarSet<'static> {
    if module == "exi_options" {
        return &generated::exi_options::EXI_OPTIONS;
    }
    generated::by_case(&module.replace('_', "-")).unwrap_or_else(|| panic!("no module {module}"))
}

#[test]
fn statics_equal_runtime_compiled_tables() {
    for (module, _, xsds) in modules() {
        let compiled = compile(&xsds);
        let from_static = GrammarSetBuf::from_set(static_for(&module));
        assert!(
            from_static == compiled,
            "{module}: generated static differs from runtime-compiled tables; \
             run scripts/gen-tables.sh"
        );
    }
}

#[test]
fn regeneration_is_a_no_op() {
    for (module, name, xsds) in modules() {
        let source: Vec<String> = xsds
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let text = render(
            &compile(&xsds),
            &RenderOptions {
                name,
                core_path: "rexi_core",
                source: &source.join(", "),
            },
        );
        let committed = generated::source_of(&module).unwrap_or_default();
        assert!(
            text == committed,
            "{module}.rs is stale; run scripts/gen-tables.sh"
        );
    }
}

#[test]
fn generated_name_constants_resolve() {
    use generated::order_complex as oc;
    let g = &oc::GRAMMARS;
    assert_eq!(g.local_name_str(oc::LN_ORDER), Some("Order"));
    assert_eq!(g.local_name_str(oc::LN_ORDER_ID), Some("orderId"));
    assert_eq!(
        g.find_qname("urn:rexi:interop:order", "Order"),
        Some(oc::LN_ORDER)
    );
    assert!(g.find_global_element(oc::LN_ORDER).is_some());

    use generated::exi_options as eo;
    let o = &eo::EXI_OPTIONS;
    assert_eq!(o.local_name_str(eo::LN_PRE_COMPRESS), Some("pre-compress"));
    assert_eq!(o.local_name_str(eo::LN_SCHEMA_ID), Some("schemaId"));
}

fn option_vectors() -> Vec<ExiOptions> {
    let s = ExiOptions::strict;
    let mut nil = s();
    nil.schema_id = SchemaId::Nil;
    let mut empty = s();
    empty.schema_id = SchemaId::Empty;
    let mut lengths = s();
    lengths.value_max_length = Some(12);
    lengths.value_partition_capacity = Some(65_536);
    let mut everything = s()
        .with_alignment(Alignment::ByteAligned)
        .with_schema_id("urn:rexi:interop:order");
    everything.self_contained = true;
    everything.value_max_length = Some(0);
    everything.fragment = true;
    everything.preserve.dtd = true;
    everything.preserve.prefixes = true;
    everything.preserve.lexical_values = true;
    everything.preserve.comments = true;
    everything.preserve.pis = true;
    let mut compressed = s();
    compressed.compression = true;
    compressed.block_size = Some(1_000_000);
    compressed.alignment = Alignment::PreCompression;
    vec![
        s(),
        s().with_schema_id("a"),
        nil,
        empty,
        s().with_alignment(Alignment::ByteAligned),
        lengths,
        everything,
        compressed,
        ExiOptions::default(),
    ]
}

/// Encodes the options document body with `write_options_document` (the
/// hand-built table) as the byte oracle.
fn oracle(options: &ExiOptions) -> Vec<u8> {
    write_options_document(BitWriter::new(Vec::new()), options)
        .unwrap_or_else(|e| panic!("{options:?}: {e}"))
        .finish()
        .unwrap_or_default()
}

/// Decodes the oracle bytes with `decoding`, re-encodes the events with
/// `encoding`, both as an out-of-band strict body.
fn transcode<D: GrammarTables, E: GrammarTables>(
    bytes: &[u8],
    decoding: D,
    encoding: E,
) -> Vec<u8> {
    let mut stream = vec![0x80];
    stream.extend_from_slice(bytes);
    let events =
        rexi_xml::decode_events_typed(stream.as_slice(), decoding, Some(ExiOptions::strict()))
            .unwrap_or_else(|e| panic!("decode: {e}"));
    let mut enc = Encoder::new(
        Vec::new(),
        encoding,
        ExiOptions::strict(),
        rexi_core::HeaderConfig::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    for ev in &events {
        enc.push(&ev.as_event())
            .unwrap_or_else(|e| panic!("encode: {e}"));
    }
    let mut out = enc.finish().unwrap_or_else(|e| panic!("{e}"));
    out.remove(0);
    out
}

/// The Appendix C schema compiles to tables that are not
/// structurally the hand-built `OPTIONS_GRAMMARS` (which shares one type per
/// content model), so equivalence is checked on bytes and events instead.
#[test]
fn appendix_c_statics_match_hand_built_options_grammar() {
    let generated = &generated::exi_options::EXI_OPTIONS;
    let compiled = compile(&[options_xsd()]);
    for opts in option_vectors() {
        let expected = oracle(&opts);
        for (label, actual) in [
            (
                "hand→generated",
                transcode(&expected, OPTIONS_GRAMMARS, generated),
            ),
            (
                "generated→generated",
                transcode(&expected, generated, generated),
            ),
            (
                "generated→hand",
                transcode(&expected, generated, OPTIONS_GRAMMARS),
            ),
            (
                "compiled→compiled",
                transcode(&expected, &compiled, &compiled),
            ),
        ] {
            assert!(
                actual == expected,
                "{opts:?} {label}\n{}",
                describe_diff(label, &actual, &expected)
            );
        }
        let hand_events = rexi_xml::events_dump(
            &rexi_xml::decode_events(
                [&[0x80][..], &expected].concat().as_slice(),
                OPTIONS_GRAMMARS,
                Some(ExiOptions::strict()),
            )
            .unwrap_or_default(),
        );
        let gen_events = rexi_xml::events_dump(
            &rexi_xml::decode_events(
                [&[0x80][..], &expected].concat().as_slice(),
                generated,
                Some(ExiOptions::strict()),
            )
            .unwrap_or_default(),
        );
        assert_eq!(hand_events, gen_events, "{opts:?}");
        assert!(gen_events.contains("header"), "{opts:?}: {gen_events}");
    }
}
