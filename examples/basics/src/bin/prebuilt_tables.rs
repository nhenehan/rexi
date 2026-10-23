//! Uses grammar tables that were turned into Rust code ahead of time, so the
//! program never reads or compiles the XSD when it runs.
//!
//! `src/order_tables.rs` was written by `rexi-schema-gen`. To write it again
//! after the schema changes, run this from the rexi folder:
//!
//! ```text
//! cargo run -p rexi-schema-gen -- examples/basics/data/order.xsd \
//!     --out examples/basics/src/order_tables.rs
//! ```
//!
//! The tables are a `static`, so they need no heap and no start-up work, which
//! suits small devices. The file also has a `QNameId` constant for every name,
//! such as `LN_ORDER`, for `Encoder::start_element_id`.
//!
//! Julia version: `jexi/examples/basics/prebuilt_tables.jl`.

use std::time::Instant;

use rexi_basics::{data, order_tables};
use rexi_core::{ExiOptions, HeaderConfig};

// ANCHOR: main
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xml = std::fs::read(data("order.xml"))?;
    let options = ExiOptions::strict();

    let start = Instant::now();
    let compiled = rexi_schema::compile_files(&[&data("order.xsd")])?;
    // Timings vary from run to run, so they go to stderr; stdout stays the same
    // on every run.
    eprintln!("compiling the XSD took {:?}", start.elapsed());

    let prebuilt = &order_tables::GRAMMARS;
    let a = rexi_xml::encode_xml(&xml, &compiled, &options, HeaderConfig::default())?;
    let b = rexi_xml::encode_xml(&xml, prebuilt, &options, HeaderConfig::default())?;
    println!("EXI: {} bytes", b.len());
    println!("same bytes with compiled and prebuilt tables: {}", a == b);

    let back = rexi_xml::decode_to_xml(&b[..], prebuilt, Some(options))?;
    let first_tag = back.split('>').next().unwrap_or_default();
    println!("decoded with prebuilt tables: {first_tag}>...");
    Ok(())
}
// ANCHOR_END: main
