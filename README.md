# rexi

rexi turns XML into small, fast binary messages and back, in Rust and C. The
format is EXI (Efficient XML Interchange), a W3C standard. You give rexi your
XML Schema. Both ends use it to agree on the format, so the messages can leave
out element names and send numbers as numbers. In the guide's example, 538
bytes of XML become 129 bytes of EXI.

**Supported profile:** EXI 1.0 (Second Edition), schema-informed, strict,
bit-packed or byte-aligned, with or without the `$EXI` cookie, and options in
the header or out of band (the smallest form has a one-byte header).

## Quick example

```rust
use rexi_core::{ExiOptions, HeaderConfig};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile the schema once, then reuse the tables for every message.
    let tables = rexi_schema::compile_files(&[Path::new("order.xsd")])?;
    let xml = std::fs::read("order.xml")?;
    let options = ExiOptions::strict();
    let exi = rexi_xml::encode_xml(&xml, &tables, &options, HeaderConfig::default())?;
    // The options are not in the stream, so the decoder is told them.
    let back = rexi_xml::decode_to_xml(&exi[..], &tables, Some(options))?;
    println!("{} bytes of XML, {} bytes of EXI\n{back}", xml.len(), exi.len());
    Ok(())
}
```

The full version is [`examples/basics/src/bin/roundtrip.rs`](examples/basics/src/bin/roundtrip.rs).

## Add to your project

The crates are not on crates.io yet. Add them as git dependencies, pinned to the
release tag:

```toml
[dependencies]
rexi-core = { git = "https://github.com/nhenehan/rexi", tag = "v0.1.0" }   # encoder, decoder, options, values
rexi-schema = { git = "https://github.com/nhenehan/rexi", tag = "v0.1.0" } # .xsd to grammar tables
rexi-xml = { git = "https://github.com/nhenehan/rexi", tag = "v0.1.0" }    # XML text to EXI and back
```

Add `rexi-serde` to map your own Rust types, and `rexi-schema-gen` to build the
tables ahead of time. rexi needs Rust 1.88 or newer.

## Learn more

- **The user guide** walks through one small program per idea, from a first
  round trip to `no_std` and C. Build it with
  `mdbook-mermaid install guide && mdbook serve guide` (you need `mdbook` and
  `mdbook-mermaid`), or read the chapters in [`guide/src`](guide/src/introduction.md).
- **[`examples/basics`](examples/basics/README.md)** has those programs. Each
  has a Julia twin in JExi that prints the same output.
- **[`examples/mqtt-todo`](examples/mqtt-todo/README.md)** is a todo list over
  MQTT, with a Rust server and Rust, C (rexi C API), Java (EXIficient) and C#
  (Nagasena for .NET) clients.
- **The C API** is one header, [`crates/rexi-capi/include/rexi.h`](crates/rexi-capi/include/rexi.h),
  for `librexi.a` or `librexi.so`. It has exip-style encoder calls and a
  callback decoder.

## What it does

- Encodes and decodes the profile above. Everything else in EXI is either done
  or refused with `ExiError::Unsupported`. rexi never quietly writes something
  "close enough".
- Reads and writes XML text, a stream of events, or your own types through serde.
- Compiles a schema at run time, or ahead of time into a `static` table that
  never allocates.
- Runs without `std`: `rexi-core` and `rexi-serde` need only `alloc`, and can
  write into a fixed buffer.
- Checks its output against other tools. The test fixtures encode to the same
  bytes as EXIficient 1.0.7 (bit-packed and byte-aligned), decode to the same
  events, encode again to the same bytes from those events, and are accepted by
  exip's decoder. The W3C test suite's
  declaredProductions group passes too.

## What it doesn't do yet

Each of these is refused with a clear error, such as `Unsupported(Wildcard)`.
The schema still compiles; only the streams or values that need the feature
fail. The rest of EXI 1.0 is planned for later releases.

- **Compression and pre-compression.** They help most with large documents; the
  first release is built for small messages.
- **Schema-less streams** (built-in grammars). rexi always needs the schema.
- **Non-strict mode.** Documents must match the schema exactly.
- **Preserve options.** Comments, processing instructions, DTDs and prefixes
  can't be kept in strict mode. Keeping the exact text of values
  (`lexicalValues`) is not built yet.
- **Wildcards** (`xs:any`, `xs:anyAttribute`). The stream must then carry the
  name of the element or attribute, and that path is not built yet. Documents
  that don't use the wildcard work.
- **Restricted character sets.** A string type whose `xs:pattern` allows fewer
  than 256 characters, such as `[A-Z]{2}[0-9]{4}`, has its own compact form in
  EXI ([7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)). Writing it as a
  plain string would give bytes that EXIficient misreads with no error, so rexi
  refuses those values on encode and decode. Patterns with a large alphabet
  (`\d+`, `\p{L}+`, `.*`, `[^x]`) work as usual.
- **Fragments** (more than one root element in a stream). rexi handles whole
  documents.
- **selfContained.** EXI doesn't allow it in strict mode.
- **datatypeRepresentationMap** (custom binary forms for types). rexi uses the
  standard forms.

## Safe with untrusted data

A few hostile bytes can ask a decoder for a lot of work, such as a list of four
billion items or endless nesting. rexi's decoder has four limits, on by default,
that stop this early with `ExiError::LimitExceeded`. Schema loading has rules
too: `xs:include` and `xs:import` read only regular files under the main
schema's folder, never a URL. See the guide's
[Untrusted input](guide/src/untrusted-input.md) chapter to change either.

## Thanks

EXI is a W3C Recommendation, and it is a really good one: a careful design, written up clearly,
with solid open libraries behind it. That work is what got this project started. rexi and JExi are
my effort to take EXI further, to more languages and more places.

- **Rumen Kyusakov** wrote [exip](https://exip.sourceforge.net/), a compact C implementation of EXI,
  and co-edited the Second Edition of the [EXI specification](https://www.w3.org/TR/exi/). exip's
  decoder is one of the checks rexi's output must pass.
- **Daniel Peintner** wrote [EXIficient](https://exificient.github.io/) with Richard Kuntschke at
  Siemens, co-edited the Second Edition, and edited the [EXI Primer](https://www.w3.org/TR/exi-primer/).
  rexi's bytes are checked against EXIficient's.
- **Takuki Kamiya** edited the First Edition, chaired the EXI Working Group, and wrote Nagasena
  ([OpenEXI](https://sourceforge.net/projects/openexi/)). **John Schneider** also edited the First
  Edition, and **Santiago Pericas-Geertsen** co-edited the Primer and wrote the W3C test framework.
- The **W3C EXI Working Group** as a whole made the specification that rexi follows closely.

Thank you.

## License

BSD-3-Clause, see [LICENSE](LICENSE).

## For maintainers

Version 0.1.0, the first release.

| Crate | Purpose |
|---|---|
| `rexi-core` | Codec core with no dependencies: bit I/O, datatypes, string tables, grammar runtime, `Encoder`/`Decoder`, header and options document. `no_std` + `alloc` |
| `rexi-schema` | XSD to `GrammarSetBuf` tables (roxmltree) |
| `rexi-schema-gen` | XSD to a `static GrammarSet<'static>` (Rust source) |
| `rexi-xml` | XML to events and back (quick-xml), and the `rexi` CLI |
| `rexi-serde` | Serde `to_exi`/`from_exi`; the grammar decides attributes, namespaces, choices and value types. `no_std` + `alloc` |
| `rexi-capi` | C ABI, `librexi.a`/`.so`, header regenerated by cbindgen on every build (commit it with the source change) |
| `rexi-tests` | Conformance tests against EXIficient reference bytes, with exipd as judge |
| `rexi-nostd-check` | Uses core and serde from a `#![no_std]` crate, built for `thumbv7em-none-eabihf`, tested on the host |

- **`std` feature.** The default `std` feature of rexi-core and rexi-serde only
  makes every `std::io::Read`/`Write` an `io::ByteSource`/`io::ByteSink`.
  Without it, `&[u8]`, `Vec<u8>` and `io::SliceSink` are the sources and sinks;
  depend with `default-features = false`.
- **Tables.** Grammar tables are flat slices indexed by `u32`. Reading static
  tables never allocates; the codec allocates its string table when an
  `Encoder`/`Decoder` is built.
- **Examples** and `fuzz/` are separate workspaces, built only on request.
- **Untrusted input, in short** (the guide has the rest).
  Decode limits: `max_depth` 256, `max_list_items` 1,000,000,
  `max_integer_bits` 4096, `max_string_bytes` 256 MiB. Change them with
  `DecodeLimits` and `Decoder::with_limits` (or the `*_with_limits` functions of
  rexi-xml and rexi-serde), `RexiLimits` and `rexi_decoder_new_with_limits` in
  C, or `--max-depth N` and friends (`--no-limits`) in the CLI. Schema rules:
  includes and imports read only regular files under the main schema's folder,
  at most 16 MiB each, never a URL, pipe or device. A location outside is
  refused before any file is touched, so errors never tell which files exist.
  Sets like UBL (`../common/...`) set a root with `SchemaLoadOptions` and
  `compile_files_with`, `rexi_tables_from_xsd_with_root`, or `--schema-root DIR`;
  breaking a rule gives `SchemaError::Refused`. One type's content may expand to
  at most 100,000 grammar rules (`max_grammar_size`, else
  `SchemaError::TooLarge`), and circular groups are refused.

### Gate and commands

`scripts/gate.sh` runs everything a commit must pass: the first eight lines
below, plus the generated-table and EXIficient fixture checks. `--quick` skips
the fixtures, which need Java.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --no-deps
cargo test --workspace
cargo clippy -p rexi-core -p rexi-serde -p rexi-nostd-check --all-targets --no-deps --no-default-features
cargo test -p rexi-core -p rexi-serde -p rexi-nostd-check --no-default-features
cargo build -p rexi-nostd-check --target thumbv7em-none-eabihf   # rustup target add thumbv7em-none-eabihf
cargo +1.88 test --workspace   # oldest supported Rust (rust-version); rustup toolchain install 1.88
cargo run -p rexi-xml --bin rexi -- encode --xsd a.xsd in.xml out.exi
cargo run -p rexi-xml --bin rexi -- decode --xsd a.xsd out.exi
cargo run -p rexi-xml --bin rexi -- decode --xsd a.xsd --max-depth 1024 out.exi   # or --no-limits
cargo run -p rexi-schema-gen -- a.xsd [b.xsd ...] --out tables.rs [--name GRAMMARS] [--check]
fuzz/run.sh [seconds]   # fuzz targets, 60 s each by default (nightly, cargo install cargo-fuzz)
```

`cargo test -p rexi-capi` builds and runs the C smoke test in
`crates/rexi-capi/tests/c` with CMake, ASan and UBSan.

### Generated tables

Generated tables are a module (`mod tables;`), not an `include!`, because they
start with inner attributes. Each declared element and attribute name gets a
`pub const LN_<NAME>: QNameId`. `scripts/gen-tables.sh` regenerates the test
statics in `crates/rexi-tests/src/generated` (`--check` verifies them), and
`fixtures_strict_static.rs` runs the strict fixture gate through them.

### Reference fixtures

`scripts/gen-fixtures.sh` regenerates the reference bytes in
`crates/rexi-tests/fixtures/interop` with EXIficient 1.0.7 (from Maven Central),
through the pinned exip's oracle `utils/xsd2exip`, a Maven project that
`scripts/fetch-third-party.sh` checks out. See
`cases.tsv` and `manifest.tsv` there; the manifest records a hash of the oracle
sources. `--check` verifies the committed files are unchanged. Reference bytes
change only on purpose (`--force`).

### Third parties

Nothing third-party lives in the working tree. `scripts/fetch-third-party.sh`
checks out the commits pinned in `third-party.lock` (exip, for the exipd judge
and its test-set, and the W3C exi-testsuite) into
`${REXI_THIRD_PARTY:-${XDG_CACHE_HOME:-~/.cache}/rexi/third_party}`:

```bash
scripts/fetch-third-party.sh --build         # fetch pins, build exipd and the oracle jar
scripts/fetch-third-party.sh --check         # every checkout present, clean, at its pin
scripts/fetch-third-party.sh --update exip   # on purpose: re-pin to remote HEAD
```

Tests that need exipd or exip's test data skip when the cache is empty; set
`REXI_REQUIRE_JUDGES=1` to make them required.
