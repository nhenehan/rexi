//! Encodes an XML file to EXI with the help of its XML Schema, then decodes it back.
//!
//! Both ends build the same grammars from the schema. Because of that, the stream
//! can leave out names and types that both sides already know
//! ([EXI 8.5](https://www.w3.org/TR/exi/#informedGrammars)).
//!
//! Julia version: `jexi/examples/basics/roundtrip.jl`.

use rexi_basics::{data, hex_lines};
use rexi_core::{ExiOptions, HeaderConfig};

// ANCHOR: main
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Turn the schema into grammar tables. This is the slow step, so do it once
    // and reuse the tables for every document.
    let tables = rexi_schema::compile_files(&[&data("order.xsd")])?;

    let xml = std::fs::read(data("order.xml"))?;
    // Strict and bit-packed, with the options agreed outside the stream: the
    // smallest output, with a header of a single byte.
    let options = ExiOptions::strict();
    let exi = rexi_xml::encode_xml(&xml, &tables, &options, HeaderConfig::default())?;

    println!("XML: {} bytes", xml.len());
    println!("EXI: {} bytes", exi.len());
    print!("{}", hex_lines(&exi));

    // The stream does not say which options it used, so the decoder is told.
    // Values come back in their standard text form, so "31.50" turns into "31.5".
    let back = rexi_xml::decode_to_xml(&exi[..], &tables, Some(options))?;
    println!("decoded XML:");
    println!("{back}");
    Ok(())
}
// ANCHOR_END: main
