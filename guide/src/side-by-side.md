# rexi and JExi side by side

This page shows how each rexi call looks in JExi, so you can move between Rust
and Julia.

[JExi](https://github.com/nhenehan/jexi) is the Julia version of rexi. Most
examples in `rexi/examples/basics` have a twin in `jexi/examples/basics` that
prints exactly the same output. `examples/basics/compare.sh path/to/jexi` runs
both and checks.

| Task | rexi (Rust) | JExi (Julia) |
|---|---|---|
| Compile a schema | `rexi_schema::compile_files(&[path])` | `JExi.compile_files(path)` |
| Strict options | `ExiOptions::strict()` | `JExi.strict_options()` |
| Change an option | `.with_alignment(Alignment::ByteAligned)` | `strict_options(; alignment = JExi.ALIGN_BYTE_ALIGNED)` |
| Header settings | `HeaderConfig::new().with_cookie(true).with_include_options(true)` | `HeaderConfig(; cookie, include_options)` |
| XML to EXI | `rexi_xml::encode_xml(xml, tables, &opts, header)` | `JExi.encode_xml(xml, tables; options, header)` |
| EXI to XML | `rexi_xml::decode_to_xml(bytes, tables, Some(opts))` | `JExi.decode_to_xml(bytes, tables; options)` |
| Read events | `while let Some(ev) = dec.next_event()?` | `for ev in dec` |
| Write events | `enc.start_element(ns, name)?` | `JExi.start_element!(enc, ns, name)` |
| Text to parse | `Value::Lexical("42.95")` | `JExi.Lexical("42.95")` |
| Structs | `rexi_serde::to_exi` / `from_exi` with serde | `JExi.to_exi` / `from_exi` with `exi_names` |
| Prebuilt tables | `rexi-schema-gen` command | `JExi.SchemaGen.generate` |
| Errors | `Result` with `ExiError` | Exceptions such as `UnsupportedError` |

Only in rexi: [`no_std`](glossary.md#no_std) and fixed buffers, streaming from
any `Read`/`Write`, and the C API.
