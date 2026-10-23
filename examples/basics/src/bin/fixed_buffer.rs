//! Rust only: encodes into a fixed buffer instead of a growing `Vec`.
//!
//! On a small device with little or no heap, the output can go into a byte array
//! the program owns. If the buffer is too small, encoding fails cleanly with
//! `BufferFull` instead of writing past the end. The `rexi-nostd-check` crate
//! builds this same code for a bare-metal Arm target (`thumbv7em-none-eabihf`),
//! with no operating system and no `std`.
//!
//! Here the events come from decoding an existing stream (a transcode), because
//! reading XML text needs `std`.

use rexi_basics::{data, order_tables};
use rexi_core::io::SliceSink;
use rexi_core::{Decoder, Encoder, ExiError, ExiOptions, ExiResult, HeaderConfig, IoErrorKind};

// ANCHOR: main
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xml = std::fs::read(data("order.xml"))?;
    let options = ExiOptions::strict();
    let tables = &order_tables::GRAMMARS;
    let exi = rexi_xml::encode_xml(&xml, tables, &options, HeaderConfig::default())?;

    let mut big = [0u8; 256];
    let used = transcode(&exi, &mut big, &options)?;
    println!("256-byte buffer: used {used} bytes");
    println!("same bytes as the Vec version: {}", big[..used] == exi[..]);

    let mut small = [0u8; 32];
    match transcode(&exi, &mut small, &options) {
        Err(ExiError::Io(IoErrorKind::BufferFull)) => {
            println!("32-byte buffer: refused, buffer full");
        }
        other => println!("32-byte buffer: {other:?}"),
    }
    Ok(())
}
// ANCHOR_END: main

/// Decodes `input` and writes every event again into `buf`. Returns how many
/// bytes were written.
fn transcode(input: &[u8], buf: &mut [u8], options: &ExiOptions) -> ExiResult<usize> {
    let tables = &order_tables::GRAMMARS;
    let mut dec = Decoder::new(input, tables, Some(options.clone()))?;
    let mut enc = Encoder::new(
        SliceSink::new(buf),
        tables,
        options.clone(),
        HeaderConfig::default(),
    )?;
    while let Some(event) = dec.next_event()? {
        enc.push(&event)?;
    }
    Ok(enc.finish()?.len())
}
