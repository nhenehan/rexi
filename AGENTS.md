# AGENTS.md

## About this project

rexi is an implementation of W3C Efficient XML Interchange (EXI) 1.0 in Rust, with a C API.

It encodes and decodes XML using schema-informed EXI. Both communicating ends must use compatible schema grammars and EXI options.

The project is organized as a set of crates. Use the crate that provides the functionality you need rather than introducing another encoding, schema or XML layer.

## Crate selection

| If you need to... | Use |
|---|---|
| Encode or decode EXI streams | `rexi-core` |
| Compile an XSD at runtime | `rexi-schema` |
| Generate grammar tables ahead of time | `rexi-schema-gen` |
| Convert XML text to and from EXI | `rexi-xml` |
| Encode or decode Rust types | `rexi-serde` |
| Call rexi from C | `rexi-capi` (built from source; not on crates.io) |
| Use the combined Rust interface | `rexi` |

The `rexi` crate is the facade. Its default features provide schema compilation and XML conversion. Enable `serde` when working with Rust types.

## Choosing an interface

### XML input and output

Use `rexi-xml` when the application works with XML documents.

The normal workflow is:

1. Load and compile the XML Schema.
2. Keep the resulting grammar tables.
3. Configure the EXI options.
4. Encode XML using the grammar tables.
5. Decode using the same compatible grammar tables and options.

Compile the schema once and reuse the resulting tables for subsequent messages.

See `examples/basics` and the [user guide](https://nhenehan.github.io/rexi/).

### Rust types

Use `rexi-serde` when the application already works with Rust structures.

The schema grammar determines the XML structure, including element names, attributes, namespaces, choices and value types.

Do not assume that ordinary Serde serialization rules alone determine the EXI representation. The schema is part of the encoding contract.

See the Serde examples and the user guide's "Rust structs" chapter.

### Low-level encoding

Use `rexi-core` when working directly with EXI events, encoders, decoders, grammar tables or byte sources and sinks.

Prefer the existing codec interfaces over implementing another EXI encoder or decoder.

### Static grammar tables

Use `rexi-schema-gen` when schemas are known at build time.

Generated tables can be included in a Rust module and used without runtime schema compilation. Reading static grammar tables does not allocate.

This is particularly useful for embedded and `no_std` applications.

### C applications

Use `rexi-capi` for applications that need a C ABI.

Use the public header in `crates/rexi-capi/include/rexi.h`. Do not duplicate the Rust implementation in C.

Every build regenerates `rexi.h` with cbindgen. Commit the regenerated header together with the source change.

## EXI rules

The implemented profile is EXI 1.0 (Second Edition), schema-informed and strict, for documents and fragments.

Supported encoding modes:

- Bit-packed.
- Byte-aligned.
- Optional `$EXI` cookie.
- Options in the header or supplied out of band.

Important constraints:

- Both ends must agree on the schema and EXI options.
- Documents must match the schema in strict mode.
- Unsupported EXI features must return an appropriate error.
- Do not silently substitute a different encoding for an unsupported feature.
- Do not assume schema compilation guarantees that every possible document using that schema can be encoded.

Check the README's feature tables and the user guide before relying on a particular feature.

The W3C specification, implementation support and conformance test results are separate things. Do not claim full EXI conformance based on individual passing tests. Report only what tests show; of the W3C EXI test suite, rexi runs only the declaredProductions group (`scripts/w3c-declared.py`, expected results in `scripts/w3c-declared.expected`).

## no_std and memory usage

`rexi-core` and `rexi-serde` support `no_std` with `alloc`.

When working on constrained targets:

- Avoid introducing unnecessary dependencies on `std`.
- Prefer fixed-buffer output where appropriate.
- Consider runtime schema compilation versus generated static tables.
- Remember that the codec still allocates its string table when an encoder or decoder is constructed.
- Check feature flags before adding dependencies.

The `rexi-nostd-check` crate is used to verify the `no_std` configuration.

## Error handling and unsupported features

Unsupported EXI functionality must fail explicitly.

Use the existing error types and error paths. Do not:

- Ignore an unsupported feature.
- Encode a different representation without telling the caller.
- Accept invalid data merely because another implementation happens to decode it.
- Weaken validation to make an interoperability test pass.

Restricted character sets deserve particular care. If the pattern alphabet cannot be determined, do not encode the value as an ordinary string. Another EXI implementation may interpret those bytes differently.

## Security

Treat EXI streams and XML Schemas as untrusted input.

Preserve the decoder limits and schema-loading restrictions.

Do not remove or weaken:

- Decode depth limits.
- List item limits.
- Integer size limits.
- String size limits.
- Schema file size limits.
- Schema root restrictions.
- The grammar size limit (100,000 rules per type, `max_grammar_size`).
- Circular group detection.

Changes to these limits or their enforcement require corresponding tests.

## Interoperability

Interoperability is a core requirement.

When changing encoding or decoding behavior:

- Preserve existing reference-byte fixtures.
- Check bit-packed and byte-aligned encoding.
- Check decoding against reference events.
- Check that decoding and re-encoding preserves the expected bytes.
- Check acceptance by the external exip decoder where applicable.
- Do not regenerate reference fixtures merely to make a failing test pass. Reference bytes change only with `scripts/gen-fixtures.sh --force`, on purpose.

EXIficient is the reference encoder used by the interoperability fixtures. exip provides an independent decoder check.

The test suite and third-party tooling are described in `CONTRIBUTING.md`.

## Development conventions

Before changing an interface or implementation:

1. Read the relevant crate and its public API.
2. Check the user guide and existing examples.
3. Follow the existing module and crate boundaries.
4. Prefer extending an existing abstraction over adding a parallel one.
5. Keep the C ABI and Rust API consistent.
6. Add tests for both supported behavior and relevant failure cases.

Do not invent API methods, feature flags or error variants. Check the source before using them.

Do not move functionality between crates without considering the facade and downstream users.

Write comments in plain, everyday words, and link the EXI specification section (`https://www.w3.org/TR/exi/#...`) where code follows the spec.

## Testing

Read `CONTRIBUTING.md` for the full test procedure.

Run `scripts/gate.sh` before every commit. `scripts/gate.sh --quick` skips the EXIficient fixture checks, which need Java. Set `REXI_REQUIRE_JUDGES=1` to make the exip checks required instead of skipped.

Changes to EXI encoding or decoding require interoperability tests, not just Rust unit tests.

Changes to schema handling require tests for both accepted and refused schemas.

Changes to the C API require the C smoke tests: `cargo test -p rexi-capi` (CMake, ASan and UBSan).

Changes to `no_std` functionality must keep the `rexi-nostd-check` target working: `cargo build -p rexi-nostd-check --target thumbv7em-none-eabihf`.

## Documentation

Keep the README focused on what rexi does and how to use it.

Keep development, testing and maintenance instructions in `CONTRIBUTING.md`.

Update the user guide when a public API or user-facing behavior changes.

Keep examples compilable and consistent with the actual API.

Prefer direct technical explanations over marketing language.
