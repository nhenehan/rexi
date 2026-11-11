# Getting started

In this chapter you'll add rexi to your project and run the examples.

## Add rexi to your project

rexi is a set of small crates. Most programs need three of them:

| Crate | Use it for |
|---|---|
| `rexi-core` | The encoder, the decoder, options and values |
| `rexi-schema` | Turning an `.xsd` file into [grammar tables](glossary.md#grammar-grammar-tables) |
| `rexi-xml` | Going straight between XML text and EXI |

Add `rexi-serde` to map your own types, and `rexi-schema-gen` to build tables
ahead of time. The `rexi` crate gathers the first three, with `rexi-serde`
behind its `serde` feature:

```toml
[dependencies]
rexi = "0.3"
```

Or depend on them one by one:

```toml
[dependencies]
rexi-core = "0.3"
rexi-schema = "0.3"
rexi-xml = "0.3"
```

## Run the examples

The examples live in `examples/basics` and build on their own:

```bash
cd examples/basics
cargo run --bin roundtrip
./run-all.sh          # every example, in the order of this guide
```

## The sample order

Every example uses the same two files in `examples/basics/data`. The
[schema](glossary.md#schema) says an `Order` has an `orderId`, a `Customer`, one
or more `Item`s and maybe some `Notes`:

```xml
{{#include ../../examples/basics/data/order.xml}}
```

<details>
<summary>The schema, <code>order.xsd</code></summary>

```xml
{{#include ../../examples/basics/data/order.xsd}}
```

</details>
