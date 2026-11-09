# Changelog

## Unreleased

- Standards are linked, not committed: the copy of the EXI 1.0 Appendix C
  options schema is gone. `scripts/fetch-standards.sh` downloads the dated W3C
  Recommendation into `~/.cache/rexi/standards` and takes the schema out of it,
  both checked by sha256 against `standards.lock`. The tests and
  `gen-tables.sh` read it there (the generated tables are unchanged); the gate
  and CI fetch or check it.

## 0.2.0

- The API follows the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).
  These changes break code written for 0.1.0:
  - `OwnedValue::as_value`, `OwnedEvent::as_event` and `rexi_xml::as_lexical_event`
    are now `to_value`, `to_event` and `to_lexical_event`, because they may
    allocate.
  - `rexi_xml::encode_xml` and `rexi_serde::{to_exi, to_exi_root, to_writer}`
    take `ExiOptions` by value.
  - `XmlError::Xml(quick_xml::Error)` is now `XmlError::Syntax(XmlSyntaxError)`,
    with the parser's message and the byte offset; quick-xml is no longer part
    of the API. `XmlError` prints the inner error's text and returns no
    `source()`, so error reporters do not print it twice.
  - `DateTime::check` returns `ExiResult` instead of `Result<(), String>`.
  - No longer public: `Decoder::header`, `Decoder::into_reader`, `Uint::write`,
    `datatype::{read_scalar, write_scalar}`, the `BigUint` helpers for the codec
    and `DateTime::normalize_hour24`; `BigUint::limbs` and
    `BigUint::reverse_decimal_digits` are gone. `Encoder::into_writer` is
    hidden. `rexi_schema_gen::screaming_snake` is private.
- Other API changes, which keep 0.1.0 code working:
  - `rexi_xml::push_xml` takes any `ByteSink` and the `decode_*` functions any
    `ByteSource`, not only `std::io` types.
  - `rexi_schema::compile_files` takes any list of paths
    (`impl IntoIterator<Item = impl AsRef<Path>>`).
  - New impls: `FromStr` for `Int`, `Uint`, `BigUint`, `Decimal` and `Float`;
    `From<u128>` for `BigUint`; `From<Int>` and `From<Uint>` for `Decimal`;
    `From<&Value>` for `OwnedValue`; `TryFrom<&Int>` for `i64` and
    `TryFrom<&Uint>` for `u64`; `Eq` and `Hash` for `Value`, `OwnedValue`,
    `Event` and `OwnedEvent`; `Debug` for `Encoder` and `Decoder`; `Default` for
    `GrammarSet` and `WhiteSpace`; and more `Eq`, `Hash` and `Copy` derives.
  - The struct definitions of `Encoder`, `Decoder`, `BitReader` and `BitWriter`
    have no trait bounds; the bounds are on the impls.
  - Every public function that can fail says how in an `# Errors` section, and
    the crates have runnable examples.
- Packaging: each crate has its own keywords and categories, carries the
  LICENSE file, and docs.rs shows the `trace` feature's items.
- Encoding from XML is about 40 % faster. The XML reader hands borrowed events
  straight to the encoder, reuses its buffers and resolves namespaces itself,
  so it no longer allocates for each element (a new test holds this). The
  encoder reuses its text buffer too. Duplicate attributes and duplicate
  namespace declarations are still refused.
- `ExiError::SchemaIdMismatch` boxes its fields, so the error type is back to
  48 bytes; the larger error had made decoding up to 20 % slower on small
  documents.
- The decoder checks the stream's `schemaId` option against the tables'
  ([EXI 5.4](https://www.w3.org/TR/exi/#options)) and fails with
  `ExiError::SchemaIdMismatch` (C: `REXI_STATUS_ERR_SCHEMA_ID`) when they
  differ. An absent `schemaId` is not checked. `xsi:nil` (no schema) is always
  refused, since rexi's tables are schema-informed. `""` and any other id must
  equal the tables' id exactly, and tables with no id refuse a stream that names
  one. Tables compiled from XSD have no id: give them one with
  `GrammarSetBuf::with_schema_id`, `rexi_tables_with_schema_id`, or the CLI's
  `--schema-id`. JExi applies the same rule.
- Restricted character sets
  ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)) are encoded
  and decoded instead of refused. `DatatypeKind::RestrictedString` gains an
  `alphabet: Option<StrIdx>`, the set's characters in code point order in the
  string pool, and each character of a string-table miss is an n-bit index into
  it, or the escape index and the code point. The bytes equal EXIficient
  1.0.7's and JExi's, bit-packed and byte-aligned, on new test vectors. A
  character outside the BMP is one escaped code point, as the spec counts
  characters; EXIficient writes it as two UTF-16 units.
- A restricted string whose pattern does not parse has `alphabet: None`, and its
  values are still refused with `Unsupported(RestrictedCharacterSet)`.
- `dump-grammar` prints the alphabet's string index, and stays equal to JExi's
  `grammar_listing`. Tables made by `rexi-schema-gen` before this change have no
  alphabet field; regenerate them. Matching `RestrictedString { whitespace }`
  without `..` no longer compiles.

## 0.1.0

The first release of rexi.

### Codec and supported EXI profile

- EXI 1.0 (Second Edition), schema-informed and strict, bit-packed or
  byte-aligned.
- The header may carry the `$EXI` cookie and the options document, or the
  options can be agreed out of band (the smallest header is one byte).
- Every EXI datatype that strict mode uses, the string tables, and the
  `valueMaxLength` and `valuePartitionCapacity` options.
- The test fixtures encode to the same bytes as EXIficient 1.0.7, decode to the
  same events, and are accepted by exip's decoder. The W3C test suite's
  declaredProductions group passes.
- Anything outside this profile is refused with `ExiError::Unsupported` and a
  named feature, never written "close enough". Not supported yet: compression
  and pre-compression, schema-less streams, non-strict mode, preserve options,
  wildcards (`xs:any`, `xs:anyAttribute`), restricted character sets, fragments,
  `selfContained` and `datatypeRepresentationMap`.

### Schemas

- `rexi-schema` compiles XSD files or strings to grammar tables at run time.
- `xs:include` and `xs:import` read only regular files under the main schema's
  folder (or a root you choose with `SchemaLoadOptions`), at most 16 MiB each,
  never a URL, pipe or device. A location outside the root is refused with
  `SchemaError::Refused` before any file is opened.
- A string type whose `xs:pattern` allows fewer than 256 characters is a
  restricted character set in EXI. The schema still compiles, but values of that
  type are refused, so rexi never writes bytes another tool would misread.
- Size limits: one type's content may expand to at most 100,000 grammar rules
  (`SchemaError::TooLarge`), and circular groups are an invalid-schema error.

### Safety

- The decoder has four limits, on by default: nesting depth 256, 1,000,000
  items in one list, 4096 bits in one integer and 256 MiB of string data. Going
  past one gives `ExiError::LimitExceeded`.
- Change them with `DecodeLimits` and `Decoder::with_limits`, the
  `*_with_limits` functions of `rexi-xml` and `rexi-serde`, `RexiLimits` in C,
  or `--max-depth` and friends in the `rexi` tool.
- The decoder refuses date-time fields out of range (hour 31, month 15), as the
  encoder does.

### Bindings

- `rexi-core`: the event `Encoder` and pull `Decoder`, options, header and
  values. No dependencies; `no_std` + `alloc`, and it can write into a fixed
  buffer.
- `rexi-xml`: XML text to EXI and back.
- `rexi-serde`: `to_exi` and `from_exi` for your own types through serde.
  `no_std` + `alloc`.
- `rexi-capi`: a C API (`rexi.h`, `librexi.a` or `librexi.so`) with
  exip-style encoder calls, a callback decoder and schema compiling.

### Tools

- `rexi`: a command line tool to encode, decode, dump events and grammars, and
  time encoding and decoding.
- `rexi-schema-gen`: turns XSD into static Rust grammar tables ahead of time,
  so reading them never allocates.

### Docs and examples

- A user guide (mdBook) with one small program per idea, from a first round
  trip to `no_std`, C and untrusted input.
- `examples/basics`: those programs, each with a Julia twin in JExi.
- `examples/mqtt-todo`: a todo list over MQTT with a Rust server and Rust, C,
  Java (EXIficient) and C# (Nagasena for .NET) clients.
