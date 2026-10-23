//! `rexi`: encode XML to EXI, decode EXI to XML, and look at events and grammars.
//!
//! Errors go to stderr as `rexi: <message>` with exit code 1. `--help` and a
//! wrong command line also print the usage text that way.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rexi_core::grammar::runtime::describe_terminal;
use rexi_core::{
    Alignment, DecodeLimits, ExiOptions, GrammarSetBuf, GrammarTables, HeaderConfig, SchemaId,
};
use rexi_xml::{
    XmlError, decode_events_with_limits, decode_to_xml_with_limits, encode_xml, events_dump,
    parse_events,
};

const USAGE: &str = "\
usage:
  rexi encode  --xsd <main.xsd>[,<more.xsd>...] [options] <in.xml> <out.exi>
  rexi decode  --xsd <main.xsd>[,...] [options] <in.exi>        EXI to XML on stdout
  rexi dump-events --xsd <main.xsd>[,...] [options] <in.exi>    EXI to .events on stdout
  rexi dump-grammar --xsd <main.xsd>[,...]                      list the compiled grammars
  rexi xml-events <in.xml>                                      XML to .events on stdout
  rexi bench encode --xsd ... [options] [bench] <in.xml>        time encoding, JSON on stdout
  rexi bench decode --xsd ... [options] [bench] <in.exi>        time decoding, JSON on stdout

options (encode: header contents; decode/dump-events: out-of-band options used when
the stream carries none):
  --strict | --no-strict            strict schema-informed mode (default: strict)
  --alignment bit-packed|byte-aligned
  --include-options                 write the EXI options document into the header
  --cookie                          write the $EXI cookie
  --schema-id ID | --schema-id-nil  schemaId option (default: absent)
  --value-max-length N
  --value-partition-capacity N

decode limits (decode, dump-events, bench decode; 0 = no limit):
  --max-depth N                     elements open at once (default 256)
  --max-list-items N                items in one list value (default 1000000)
  --max-integer-bits N              bits in one integer value (default 4096)
  --max-string-bytes N              bytes of decoded string values (default 268435456)
  --no-limits                       turn every limit off

schema loading (every command with --xsd):
  --schema-root DIR                 xs:include and xs:import read only files under DIR
                                    (default: the main schema's folder; / allows any)

bench options (schemas are compiled once, outside the timed region):
  --warmup N                        untimed iterations first (default 20)
  --warmup-ms MS                    and keep warming up until MS milliseconds
                                    have passed (default 0); JIT runtimes need time,
                                    not a count, to reach optimized code
  --iterations N                    timed iterations (default 200)
  --from xml|events                 encode: time XML parsing + encoding (default), or
                                    encoding only, from events parsed once beforehand
  --out FILE                        encode: write the EXI produced
  --events-out FILE                 decode: write the decoded .events dump
";

#[derive(Debug, Default)]
struct Args {
    xsd: Vec<PathBuf>,
    /// `--schema-root`: where includes and imports may read from.
    schema_root: Option<PathBuf>,
    strict: Option<bool>,
    alignment: Option<Alignment>,
    include_options: bool,
    cookie: bool,
    schema_id: Option<SchemaId>,
    value_max_length: Option<u32>,
    value_partition_capacity: Option<u32>,
    /// The decoder limits: the defaults, changed flag by flag in command-line
    /// order.
    limits: DecodeLimits,
    warmup: Option<u32>,
    warmup_ms: Option<u32>,
    iterations: Option<u32>,
    from_events: bool,
    out: Option<PathBuf>,
    events_out: Option<PathBuf>,
    positional: Vec<String>,
}

fn parse_u32(flag: &str, value: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("{flag} expects an unsigned integer, got {value:?}"))
}

fn parse_u64(flag: &str, value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("{flag} expects an unsigned integer, got {value:?}"))
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value_of = |flag: &str| -> Result<&String, String> {
            it.next().ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "--xsd" => {
                let list = value_of("--xsd")?;
                out.xsd
                    .extend(list.split(',').filter(|s| !s.is_empty()).map(PathBuf::from));
            }
            "--schema-root" => {
                out.schema_root = Some(PathBuf::from(value_of("--schema-root")?));
            }
            "--strict" => out.strict = Some(true),
            "--no-strict" => out.strict = Some(false),
            "--alignment" => {
                out.alignment = Some(match value_of("--alignment")?.as_str() {
                    "bit-packed" => Alignment::BitPacked,
                    "byte-aligned" => Alignment::ByteAligned,
                    other => return Err(format!("unknown alignment {other:?}")),
                });
            }
            "--include-options" => out.include_options = true,
            "--cookie" => out.cookie = true,
            "--schema-id" => out.schema_id = Some(SchemaId::Id(value_of("--schema-id")?.clone())),
            "--schema-id-nil" => out.schema_id = Some(SchemaId::Nil),
            "--value-max-length" => {
                out.value_max_length = Some(parse_u32(
                    "--value-max-length",
                    value_of("--value-max-length")?,
                )?);
            }
            "--value-partition-capacity" => {
                out.value_partition_capacity = Some(parse_u32(
                    "--value-partition-capacity",
                    value_of("--value-partition-capacity")?,
                )?);
            }
            // Each flag changes the limits built so far, so a later flag wins
            // over an earlier `--no-limits`, and the other way round.
            "--no-limits" => out.limits = DecodeLimits::unlimited(),
            "--max-depth" => {
                let n = parse_u32("--max-depth", value_of("--max-depth")?)?;
                out.limits = out.limits.with_max_depth((n != 0).then_some(n));
            }
            "--max-list-items" => {
                let n = parse_u64("--max-list-items", value_of("--max-list-items")?)?;
                out.limits = out.limits.with_max_list_items((n != 0).then_some(n));
            }
            "--max-integer-bits" => {
                let n = parse_u32("--max-integer-bits", value_of("--max-integer-bits")?)?;
                out.limits = out.limits.with_max_integer_bits((n != 0).then_some(n));
            }
            "--max-string-bytes" => {
                let n = parse_u64("--max-string-bytes", value_of("--max-string-bytes")?)?;
                out.limits = out.limits.with_max_string_bytes((n != 0).then_some(n));
            }
            "--warmup" => out.warmup = Some(parse_u32("--warmup", value_of("--warmup")?)?),
            "--warmup-ms" => {
                out.warmup_ms = Some(parse_u32("--warmup-ms", value_of("--warmup-ms")?)?)
            }
            "--iterations" => {
                out.iterations = Some(parse_u32("--iterations", value_of("--iterations")?)?);
            }
            "--from" => {
                out.from_events = match value_of("--from")?.as_str() {
                    "xml" => false,
                    "events" => true,
                    other => return Err(format!("--from expects xml or events, got {other:?}")),
                };
            }
            "--out" => out.out = Some(PathBuf::from(value_of("--out")?)),
            "--events-out" => out.events_out = Some(PathBuf::from(value_of("--events-out")?)),
            "-h" | "--help" => return Err(USAGE.to_string()),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => out.positional.push(other.to_string()),
        }
    }
    Ok(out)
}

fn options_of(args: &Args) -> Result<ExiOptions, String> {
    if args.strict == Some(false) {
        return Err("--no-strict: non-strict (schema-informed with deviations) encoding is not supported yet".into());
    }
    let mut options = ExiOptions::strict();
    options.alignment = args.alignment.unwrap_or_default();
    options.value_max_length = args.value_max_length;
    options.value_partition_capacity = args.value_partition_capacity;
    options.schema_id = args.schema_id.clone().unwrap_or_default();
    Ok(options)
}

/// The one place schemas are compiled. The first path is the main schema.
fn compile_schemas(paths: &[PathBuf], root: Option<&Path>) -> Result<GrammarSetBuf, XmlError> {
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let mut options = rexi_schema::SchemaLoadOptions::default();
    if let Some(dir) = root {
        options = options.with_root(rexi_schema::SchemaRoot::Folder(dir.to_path_buf()));
    }
    Ok(rexi_schema::compile_files_with(&refs, &options)?)
}

fn tables_of(args: &Args) -> Result<GrammarSetBuf, String> {
    if args.xsd.is_empty() {
        return Err("--xsd is required".into());
    }
    compile_schemas(&args.xsd, args.schema_root.as_deref()).map_err(|e| e.to_string())
}

fn read_file(path: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))
}

fn positional<'a>(args: &'a Args, count: usize, names: &str) -> Result<&'a [String], String> {
    if args.positional.len() != count {
        return Err(format!("expected {names}\n{USAGE}"));
    }
    Ok(&args.positional)
}

fn qname_str(tables: &impl GrammarTables, q: rexi_core::QNameId) -> String {
    let uri = tables.uri_str(q.uri).unwrap_or("?");
    let local = tables.local_name_str(q).unwrap_or("?");
    if uri.is_empty() {
        local.to_string()
    } else {
        format!("{{{uri}}}{local}")
    }
}

/// A plain listing of the grammar tables, for a person to check what the compiler
/// produced. Each grammar is shown as its rules. Each rule lists its productions
/// in event-code order (`code: terminal -> next rule`). A production is one
/// allowed event plus the rule to go to next
/// ([EXI 8.1](https://www.w3.org/TR/exi/#grammarNotation)).
fn grammar_listing(tables: &impl GrammarTables) -> String {
    let mut out = String::new();
    let p = |out: &mut String, line: String| {
        out.push_str(&line);
        out.push('\n');
    };
    p(&mut out, format!("schemaId: {:?}", tables.schema_id()));
    p(&mut out, "uris:".into());
    for (i, entry) in tables.uris().iter().enumerate() {
        let locals: Vec<&str> = tables
            .local_names()
            .get(entry.local_names.range())
            .unwrap_or(&[])
            .iter()
            .map(|&s| tables.string(s))
            .collect();
        p(
            &mut out,
            format!(
                "  [{i}] {:?} local-names {:?}",
                tables.string(entry.uri),
                locals
            ),
        );
    }
    p(&mut out, "global elements:".into());
    for &e in tables.global_elements() {
        if let Some(d) = tables.element(e) {
            p(
                &mut out,
                format!(
                    "  element {e}: {} type {}",
                    qname_str(tables, d.qname),
                    d.type_idx
                ),
            );
        }
    }
    p(&mut out, "elements:".into());
    for (i, d) in tables.elements().iter().enumerate() {
        p(
            &mut out,
            format!(
                "  [{i}] {} type {} nillable {} global {}",
                qname_str(tables, d.qname),
                d.type_idx,
                d.nillable,
                d.global
            ),
        );
    }
    p(&mut out, "types:".into());
    for (i, t) in tables.types().iter().enumerate() {
        let name = if t.is_anonymous() {
            "(anonymous)".to_string()
        } else {
            qname_str(tables, t.qname)
        };
        p(
            &mut out,
            format!(
                "  [{i}] {name} grammar {} empty-grammar {} datatype {:?} typecastable {}",
                t.grammar, t.empty_grammar, t.datatype, t.typecastable
            ),
        );
    }
    p(&mut out, "grammars:".into());
    for (gi, g) in tables.grammars().iter().enumerate() {
        p(
            &mut out,
            format!(
                "  grammar {gi}: rules {}..{}",
                g.rules.start,
                g.rules.start + g.rules.len
            ),
        );
        for rule in g.rules.range() {
            let prods: Vec<String> = tables
                .rule_productions(rule as u32)
                .iter()
                .enumerate()
                .map(|(code, prod)| {
                    format!(
                        "{code}: {} -> {}",
                        describe_terminal(tables, &prod.terminal),
                        prod.next
                    )
                })
                .collect();
            p(&mut out, format!("    rule {rule}: [{}]", prods.join(", ")));
        }
    }
    p(&mut out, "enumerations:".into());
    for (i, set) in tables.enum_sets().iter().enumerate() {
        let values: Vec<&str> = tables
            .enum_values()
            .get(set.range())
            .unwrap_or(&[])
            .iter()
            .map(|&s| tables.string(s))
            .collect();
        p(&mut out, format!("  [{i}] {values:?}"));
    }
    out
}

fn run(argv: &[String]) -> Result<(), String> {
    let Some((command, rest)) = argv.split_first() else {
        return Err(USAGE.to_string());
    };
    let args = parse_args(rest)?;
    match command.as_str() {
        "encode" => {
            let [input, output] = positional(&args, 2, "<in.xml> <out.exi>")? else {
                return Err(USAGE.to_string());
            };
            let options = options_of(&args)?;
            let tables = tables_of(&args)?;
            let xml = read_file(input)?;
            let header = HeaderConfig::default()
                .with_cookie(args.cookie)
                .with_include_options(args.include_options);
            let bytes = encode_xml(&xml, &tables, &options, header).map_err(|e| e.to_string())?;
            fs::write(output, &bytes).map_err(|e| format!("cannot write {output}: {e}"))?;
            Ok(())
        }
        "decode" | "dump-events" => {
            let [input] = positional(&args, 1, "<in.exi>")? else {
                return Err(USAGE.to_string());
            };
            let options = options_of(&args)?;
            let tables = tables_of(&args)?;
            let bytes = read_file(input)?;
            let limits = args.limits;
            let text = if command == "decode" {
                decode_to_xml_with_limits(bytes.as_slice(), &tables, Some(options), limits)
                    .map_err(|e| e.to_string())?
            } else {
                let events =
                    decode_events_with_limits(bytes.as_slice(), &tables, Some(options), limits)
                        .map_err(|e| e.to_string())?;
                events_dump(&events)
            };
            print_out(&text)
        }
        "dump-grammar" => {
            positional(&args, 0, "no positional arguments")?;
            let tables = tables_of(&args)?;
            print_out(&grammar_listing(&tables))
        }
        "xml-events" => {
            let [input] = positional(&args, 1, "<in.xml>")? else {
                return Err(USAGE.to_string());
            };
            let xml = read_file(input)?;
            let events = parse_events(&xml).map_err(|e| e.to_string())?;
            print_out(&events_dump(&events))
        }
        "bench" => bench(&args),
        "help" | "-h" | "--help" => Err(USAGE.to_string()),
        other => Err(format!("unknown command {other:?}\n{USAGE}")),
    }
}

/// Runs `op` without timing it at least `warmup.0` times and for at least
/// `warmup.1` milliseconds, then `iterations` times with timing. Returns how
/// many nanoseconds each timed run took.
fn time_runs(
    warmup: (u32, u32),
    iterations: u32,
    mut op: impl FnMut() -> Result<(), String>,
) -> Result<Vec<u128>, String> {
    let (count, ms) = warmup;
    let start = std::time::Instant::now();
    let mut done = 0u32;
    while done < count || start.elapsed().as_millis() < u128::from(ms) {
        op()?;
        done = done.saturating_add(1);
    }
    let mut ns = Vec::with_capacity(iterations as usize);
    for _ in 0..iterations {
        let start = std::time::Instant::now();
        op()?;
        ns.push(start.elapsed().as_nanos());
    }
    Ok(ns)
}

/// `rexi bench encode|decode`: timing inside this process, for the interop
/// benchmark. Prints one JSON object: the operation, sizes, setup time (schema
/// compiling) and the nanoseconds of every timed run. For `encode`, `output` is
/// the EXI size in bytes; for `decode`, it is the number of decoded events.
fn bench(args: &Args) -> Result<(), String> {
    let [op, input] = positional(args, 2, "encode|decode <input>")? else {
        return Err(USAGE.to_string());
    };
    let options = options_of(args)?;
    let setup = std::time::Instant::now();
    let tables = tables_of(args)?;
    let setup_ns = setup.elapsed().as_nanos();
    let data = read_file(input)?;
    let (warmup, iterations) = (args.warmup.unwrap_or(20), args.iterations.unwrap_or(200));
    let warmup_ms = args.warmup_ms.unwrap_or(0);
    let header = HeaderConfig::default()
        .with_cookie(args.cookie)
        .with_include_options(args.include_options);
    let (input_kind, output_bytes, ns) = match op.as_str() {
        "encode" => {
            let encoded =
                encode_xml(&data, &tables, &options, header).map_err(|e| e.to_string())?;
            if let Some(path) = &args.out {
                fs::write(path, &encoded)
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            }
            let ns = if args.from_events {
                let events = parse_events(&data).map_err(|e| e.to_string())?;
                time_runs((warmup, warmup_ms), iterations, || {
                    let mut enc = rexi_core::Encoder::new(
                        Vec::with_capacity(encoded.len()),
                        &tables,
                        options.clone(),
                        header,
                    )
                    .map_err(|e| e.to_string())?;
                    for ev in &events {
                        enc.push(&rexi_xml::as_lexical_event(ev))
                            .map_err(|e| e.to_string())?;
                    }
                    enc.finish().map(drop).map_err(|e| e.to_string())
                })?
            } else {
                time_runs((warmup, warmup_ms), iterations, || {
                    encode_xml(&data, &tables, &options, header)
                        .map(drop)
                        .map_err(|e| e.to_string())
                })?
            };
            let kind = if args.from_events { "events" } else { "xml" };
            (kind, encoded.len(), ns)
        }
        "decode" => {
            let limits = args.limits;
            let events =
                decode_events_with_limits(data.as_slice(), &tables, Some(options.clone()), limits)
                    .map_err(|e| e.to_string())?;
            if let Some(path) = &args.events_out {
                fs::write(path, events_dump(&events))
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            }
            // Only the decoder is timed. Events are counted and thrown away;
            // `black_box` keeps the compiler from skipping the work.
            let ns = time_runs((warmup, warmup_ms), iterations, || {
                let mut dec = rexi_core::Decoder::with_limits(
                    data.as_slice(),
                    &tables,
                    Some(options.clone()),
                    limits,
                )
                .map_err(|e| e.to_string())?;
                let mut count = 0usize;
                while dec.next_event().map_err(|e| e.to_string())?.is_some() {
                    count += 1;
                }
                std::hint::black_box(count);
                Ok(())
            })?;
            ("exi", events.len(), ns)
        }
        other => {
            return Err(format!(
                "bench: unknown operation {other:?} (encode|decode)"
            ));
        }
    };
    let ns_list: Vec<String> = ns.iter().map(u128::to_string).collect();
    print_out(&format!(
        "{{\"impl\":\"rexi\",\"version\":\"{}\",\"op\":\"{op}\",\"input\":\"{input_kind}\",\
         \"input_bytes\":{},\"output\":{output_bytes},\"setup_ns\":{setup_ns},\
         \"warmup\":{warmup},\"iterations\":{iterations},\"ns\":[{}]}}\n",
        env!("CARGO_PKG_VERSION"),
        data.len(),
        ns_list.join(",")
    ))
}

fn print_out(text: &str) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|e| format!("cannot write stdout: {e}"))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("rexi: {msg}");
            ExitCode::from(1)
        }
    }
}
