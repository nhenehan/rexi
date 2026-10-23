# rexi basics

Small programs, one idea each, in the order the [user guide](../../guide/src/introduction.md)
uses. Most have a Julia twin in `examples/basics` of
[JExi](https://github.com/nhenehan/jexi) that prints exactly the same output, so
you can compare the two side by side.

| # | Rust | Julia | Shows |
|---|---|---|---|
| 1 | [`roundtrip.rs`](src/bin/roundtrip.rs) | `roundtrip.jl` | XML to EXI and back |
| 2 | [`options.rs`](src/bin/options.rs) | `options.jl` | Alignment, cookie, options in the header |
| 3 | [`read_events.rs`](src/bin/read_events.rs) | `read_events.jl` | The pull decoder and typed values |
| 4 | [`write_events.rs`](src/bin/write_events.rs) | `write_events.jl` | Writing a document event by event |
| 5 | [`structs.rs`](src/bin/structs.rs) | `structs.jl` | Your own types to EXI and back |
| 6 | [`prebuilt_tables.rs`](src/bin/prebuilt_tables.rs) | `prebuilt_tables.jl` | Tables built ahead of time |
| 7 | [`fixed_buffer.rs`](src/bin/fixed_buffer.rs) | Rust only | Encoding into a fixed buffer (`no_std`) |
| 8 | [`c/order.c`](c/order.c) | C only | The C API |

All of them use [`data/order.xml`](data/order.xml) and [`data/order.xsd`](data/order.xsd).

```bash
cargo run --bin roundtrip     # one example
./run-all.sh [--c]            # all of them; --c adds the C example (needs cmake)
./compare.sh path/to/jexi     # run each Rust/Julia pair and diff the output
```

`compare.sh` needs Julia and a JExi checkout. Pass its path, or set `JEXI`:

```bash
git clone https://github.com/nhenehan/jexi ~/jexi
./compare.sh ~/jexi        # or: JEXI=~/jexi ./compare.sh
```

`src/order_tables.rs` is made by `rexi-schema-gen`. See `prebuilt_tables.rs` for
how to make it again.
