# rexi

rexi turns XML into small, fast binary messages and back, in Rust and C. The format is EXI (Efficient XML Interchange), a W3C standard. You give rexi your XML Schema. Both ends use it to agree on the structure and types of the data, so element names and other information already defined by the schema don't need to be repeated in every message. In the guide's example, 538 bytes of XML become 129 bytes of EXI.

**Supported profile:** EXI 1.0 (Second Edition), schema-informed, strict, bit-packed or byte-aligned, with or without the `$EXI` cookie, and options in the header or out of band (the smallest form has a one-byte header). Documents and fragments.

## Quick example

```rust
use rexi::{ExiOptions, HeaderConfig};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile the schema once, then reuse the tables for every message.
    let tables = rexi::schema::compile_files([Path::new("order.xsd")])?;
    let xml = std::fs::read("order.xml")?;
    let options = ExiOptions::strict();
    let exi = rexi::xml::encode_xml(
        &xml, &tables, options.clone(), HeaderConfig::default()
    )?;

    // The options are not in the stream, so the decoder is told them.
    let back = rexi::xml::decode_to_xml(&exi[..], &tables, Some(options))?;

    println!("{} bytes of XML, {} bytes of EXI\n{back}", xml.len(), exi.len());
    Ok(())
}
```

See the full example in [`examples/basics/src/bin/roundtrip.rs`](https://github.com/nhenehan/rexi/blob/main/examples/basics/src/bin/roundtrip.rs).

## Add to your project

```toml
[dependencies]
rexi = "0.3"
```

The `rexi` crate brings the other crates together. `rexi::schema` (rexi-schema) and `rexi::xml` (rexi-xml) are enabled by default. The `serde` feature adds `rexi::serde` (rexi-serde), which maps your own Rust types.

On a small device without `std`, use:

```toml
rexi = { version = "0.3", default-features = false }
```

You can also depend on the individual crates (`rexi-core = "0.3"` and so on). `rexi-schema-gen` builds grammar tables ahead of time, and `cargo install rexi-xml` installs the `rexi` command-line tool.

rexi needs Rust 1.88 or newer.

## Learn more

- **[The user guide](https://nhenehan.github.io/rexi/)** works through one small program per idea, from a first round trip to `no_std` and C. To read it offline, build it with `mdbook-mermaid install guide && mdbook serve guide` (you need `mdbook` and `mdbook-mermaid`).
- **[API documentation](https://docs.rs/rexi)** on docs.rs.
- **[`examples/basics`](https://github.com/nhenehan/rexi/blob/main/examples/basics/README.md)** contains the programs from the guide. Each has a Julia twin in EXIFormat that prints the same output.
- **[`examples/mqtt-todo`](https://github.com/nhenehan/rexi/blob/main/examples/mqtt-todo/README.md)** is a todo list over MQTT, with a Rust server and Rust, C (rexi C API), Java (EXIficient) and C# (Nagasena for .NET) clients.
- **The C API** is defined in one header, [`crates/rexi-capi/include/rexi.h`](https://github.com/nhenehan/rexi/blob/main/crates/rexi-capi/include/rexi.h), for `librexi.a` or `librexi.so`. It has exip-style encoder calls and a callback decoder.

## EXI 1.0 implementation

What the codec can encode and decode.

| EXI feature | Status | Notes |
|---|---|---|
| Schema-informed grammars | Supported | Compiled from XML Schema |
| Strict mode | Supported | Documents must match the schema |
| Bit-packed encoding | Supported | |
| Byte-aligned encoding | Supported | |
| EXI header | Supported | Optional `$EXI` cookie; options document in the header |
| Out-of-band options | Supported | |
| Fragments | Supported | Any number of root elements; a name declared twice with different types is refused at the top level |
| Restricted character sets | Partial | When the `xs:pattern` alphabet can be read |
| Schema-less / built-in grammars | Not supported | |
| Non-strict mode | Not supported | |
| Compression | Not supported | |
| Pre-compression | Not supported | |
| Wildcards | Not supported | `xs:any`, `xs:anyAttribute` |
| `Preserve.lexicalValues` | Not supported | |
| `datatypeRepresentationMap` | Not supported | Uses standard datatype representations |
| Preserve comments, PIs, DTD, prefixes | Not allowed in strict mode | EXI forbids them together with `strict` ([EXI 5.4](https://www.w3.org/TR/exi/#options)) |
| `selfContained` | Not allowed in strict mode | Same rule |

Every "Not supported" or "Not allowed" row fails with `ExiError::Unsupported` and names the feature. rexi never quietly writes something "close enough". A schema can still compile even if it contains features that aren't supported; only streams or values that need those features fail.

For restricted character sets, EXI has a compact representation when an `xs:pattern` restricts a string to fewer than 256 characters ([EXI 1.0, 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)). rexi uses that representation when it can determine the alphabet. If it cannot parse the pattern, the value is refused on both encode and decode rather than producing bytes that another implementation might misread.

A fragment may start with any element the schema declares, global or local ([EXI 8.5.2](https://www.w3.org/TR/exi/#informedFragGrammars)). When two local declarations share a name but differ in type, EXI reads that name at the top level with a looser "relaxed" grammar, which rexi does not build; that one name is refused there, and everything else works.

## Conformance and interoperability

Test results from rexi's CI (the `exificient` job, with the exip judges required). The fixtures are 6 documents in `crates/rexi-tests/fixtures/interop`.

| Test | Profiles | Result |
|---|---|---|
| Encode, compared with EXIficient 1.0.7's bytes | strict bit-packed; strict bit-packed with options in the header; strict byte-aligned | Same bytes |
| Decode EXIficient's streams, compared with EXIficient's events | the same | Same events |
| Re-encode the decoded events | the same | Same bytes |
| exip's decoder (exipd) reads rexi's output | strict bit-packed; strict byte-aligned | Accepted |
| exip's own unit tests, cloned in `rexi-tests/tests/exip_clones.rs` | as in exip | Pass |
| W3C EXI test suite, strict group (219 cases): encode, decode and re-encode, compared with EXIficient 1.0.7 | strict bit-packed; strict byte-aligned | 161 of 219 the same. schemaValidDatatypes: 119 of 139; the others need schema-less streams (16), Preserve.prefixes (2) or non-strict mode (2). declaredProductions: 42 of 80; the others need wildcards (17), Preserve options (15), the element fragment grammar (4) or non-strict mode (2) |

From the W3C EXI test suite, rexi runs only the strict group (`scripts/w3c-strict.py`), so it makes no claim about the rest.

[rexi-interop](https://github.com/nhenehan/rexi-interop) is a separate matrix, run by hand, where EXIficient, Nagasena, Nagasena for .NET, exip, rexi and EXIFormat encode and decode each other's streams on 15 documents. Its `reports/latest.md` has the results and timings.

## Rust library features

| Feature | Crate | Description |
|---|---|---|
| One dependency | `rexi` | Re-exports the crates below by feature |
| Core encoder / decoder | `rexi-core` | Event-by-event EXI encoding and decoding |
| Schema compiler | `rexi-schema` | Compile XSD at run time |
| Static grammar generation | `rexi-schema-gen` | Generate Rust grammar tables ahead of time |
| XML conversion | `rexi-xml` | XML text to EXI and back |
| Serde | `rexi-serde` | Encode and decode Rust types |
| C interface | `rexi-capi` | C ABI with static and shared libraries; built from source, not on crates.io |
| `no_std` | `rexi-core`, `rexi-serde` | Need only `alloc` |
| Fixed-buffer output | `rexi-core` | Write into a fixed buffer, without a growable output buffer |
| CLI | `rexi-xml` | Command-line encoding and decoding |

## Safe with untrusted data

A few hostile bytes can ask a decoder for a lot of work, such as a list of four billion items or endless nesting. rexi's decoder has four limits, enabled by default, that stop this early with `ExiError::LimitExceeded`.

Schema loading has limits too: `xs:include` and `xs:import` read only regular files under the main schema's folder, never a URL.

See the guide's [Untrusted input](https://nhenehan.github.io/rexi/untrusted-input.html) chapter to change either.

## Thanks

EXI is a W3C Recommendation, and it is a really good one: a careful design, written up clearly, with solid open libraries behind it. That work is what got this project started. rexi and EXIFormat are my effort to take EXI further, to more languages and more places.

- **Rumen Kyusakov** wrote [exip](https://exip.sourceforge.net/), a compact C implementation of EXI, and co-edited the Second Edition of the [EXI specification](https://www.w3.org/TR/exi/). exip's decoder is one of the checks rexi's output must pass.
- **Daniel Peintner** wrote [EXIficient](https://exificient.github.io/) with Richard Kuntschke at Siemens, co-edited the Second Edition, and edited the [EXI Primer](https://www.w3.org/TR/exi-primer/). rexi's bytes are checked against EXIficient's.
- **Takuki Kamiya** edited the First Edition, chaired the EXI Working Group, and wrote Nagasena ([OpenEXI](https://sourceforge.net/projects/openexi/)). **John Schneider** also edited the First Edition, and **Santiago Pericas-Geertsen** co-edited the Primer and wrote the W3C test framework.
- The **W3C EXI Working Group** as a whole made the specification that rexi follows closely.

Thank you.

## License

BSD-3-Clause, see [LICENSE](https://github.com/nhenehan/rexi/blob/main/LICENSE).

For build instructions, tests, generated tables and other development details, see [CONTRIBUTING.md](https://github.com/nhenehan/rexi/blob/main/CONTRIBUTING.md).
