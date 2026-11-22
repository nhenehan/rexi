# rexi and EXIFormat side by side

This page shows how each rexi call looks in EXIFormat, so you can move between
Rust and Julia.

[EXIFormat](https://github.com/nhenehan/EXIFormat.jl) is the Julia version of
rexi. Most examples in `rexi/examples/basics` have a twin in
`EXIFormat.jl/examples/basics` that prints exactly the same output.
`examples/basics/compare.sh path/to/EXIFormat.jl` runs both and checks.

| Task | rexi (Rust) | EXIFormat (Julia) |
|---|---|---|
| Compile a schema | `rexi_schema::compile_files([path])` | `EXIFormat.compile_files(path)` |
| Strict options | `ExiOptions::strict()` | `EXIFormat.strict_options()` |
| Change an option | `.with_alignment(Alignment::ByteAligned)` | `strict_options(; alignment = EXIFormat.ALIGN_BYTE_ALIGNED)` |
| Header settings | `HeaderConfig::new().with_cookie(true).with_include_options(true)` | `HeaderConfig(; cookie, include_options)` |
| XML to EXI | `rexi_xml::encode_xml(xml, tables, opts, header)` | `EXIFormat.encode_xml(xml, tables; options, header)` |
| EXI to XML | `rexi_xml::decode_to_xml(bytes, tables, Some(opts))` | `EXIFormat.decode_to_xml(bytes, tables; options)` |
| Read events | `while let Some(ev) = dec.next_event()?` | `for ev in dec` |
| Write events | `enc.start_element(ns, name)?` | `EXIFormat.start_element!(enc, ns, name)` |
| Text to parse | `Value::Lexical("42.95")` | `EXIFormat.Lexical("42.95")` |
| Structs | `rexi_serde::to_exi` / `from_exi` with serde | `EXIFormat.to_exi` / `from_exi` with `exi_names` |
| Prebuilt tables | `rexi-schema-gen` command | `EXIFormat.SchemaGen.generate` |
| Errors | `Result` with `ExiError` | Exceptions such as `UnsupportedError` |

Only in rexi: [`no_std`](glossary.md#no_std) and fixed buffers, streaming from
any `Read`/`Write`, and the C API.
