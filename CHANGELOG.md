# Changelog

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
