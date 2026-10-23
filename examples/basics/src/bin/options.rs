//! Encodes the same document with different EXI options and header settings, and
//! shows how the size and the first bytes change.
//!
//! The header ([EXI 5](https://www.w3.org/TR/exi/#header)) can hold the `$EXI`
//! cookie and an options document. Without them, both sides must agree on the
//! options some other way ("out of band"). The options are listed in
//! [EXI 5.4](https://www.w3.org/TR/exi/#options).
//!
//! Julia version: `jexi/examples/basics/options.jl`.

use rexi_basics::{data, hex_start};
use rexi_core::{Alignment, ExiError, ExiOptions, HeaderConfig};

// ANCHOR: main
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tables = rexi_schema::compile_files(&[&data("order.xsd")])?;
    let xml = std::fs::read(data("order.xml"))?;

    let plain = HeaderConfig::default();
    let with_options = HeaderConfig::default().with_include_options(true);
    let with_cookie = with_options.with_cookie(true);
    let cases = [
        ("bit-packed", ExiOptions::strict(), plain),
        // Every event code and value starts on a new byte: bigger, but easier
        // to read in a hex dump.
        (
            "byte-aligned",
            ExiOptions::strict().with_alignment(Alignment::ByteAligned),
            plain,
        ),
        ("options in header", ExiOptions::strict(), with_options),
        (
            "cookie + options + schemaId",
            ExiOptions::strict().with_schema_id("urn:rexi:interop:order"),
            with_cookie,
        ),
    ];

    let mut streams = Vec::new();
    for (label, options, header) in cases {
        let exi = rexi_xml::encode_xml(&xml, &tables, &options, header)?;
        println!(
            "{label:<28} {:>4} bytes  starts {}",
            exi.len(),
            hex_start(&exi, 8)
        );
        streams.push((exi, options, header));
    }

    // A stream that carries its options can be decoded without being told them.
    // The others need them passed in.
    let mut decoded = Vec::new();
    for (exi, options, header) in &streams {
        let out_of_band = if header.include_options {
            None
        } else {
            Some(options.clone())
        };
        decoded.push(rexi_xml::decode_to_xml(&exi[..], &tables, out_of_band)?);
    }
    let all_same = decoded.iter().all(|x| *x == decoded[0]);
    println!("all four decode to the same XML: {all_same}");

    // Options this library does not support are refused, never guessed at.
    let mut compressed = ExiOptions::strict();
    compressed.compression = true;
    let not_strict = ExiOptions::default();
    for (label, options) in [("compression", compressed), ("non-strict", not_strict)] {
        match options.validate() {
            Err(ExiError::Unsupported(_)) => println!("{label}: refused as unsupported"),
            other => println!("{label}: {other:?}"),
        }
    }
    Ok(())
}
// ANCHOR_END: main
