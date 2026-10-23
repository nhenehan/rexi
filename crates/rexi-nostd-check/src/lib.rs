//! The codec used from a `#![no_std]` crate.
//!
//! The gate builds this crate for `thumbv7em-none-eabihf`, a bare-metal Arm target
//! with no operating system. That proves rexi-core and rexi-serde need nothing
//! beyond `core` + `alloc`. The gate also runs this crate's tests on the host. There
//! the same code transcodes (decodes and re-encodes) the order-complex fixture, and
//! the output must match the input byte for byte.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use rexi_core::io::{ByteSink, ByteSource, SliceSink};
use rexi_core::{Alignment, Decoder, Encoder, ExiOptions, ExiResult, HeaderConfig};
use serde::de::DeserializeOwned;

/// The order-complex fixture's tables, as rexi-schema-gen renders them.
#[rustfmt::skip]
#[path = "../../rexi-tests/src/generated/order_complex.rs"]
pub mod order_complex;

/// Strict options with the given alignment. They are passed out of band, that is,
/// given to the decoder directly instead of being read from the stream header.
pub fn strict(alignment: Alignment) -> ExiOptions {
    ExiOptions::strict().with_alignment(alignment)
}

/// Decodes `input` and re-encodes every event into `out`, with the same options on
/// both sides. The header is written with `HeaderConfig::default()`.
pub fn transcode<R: ByteSource, W: ByteSink>(
    input: R,
    out: W,
    options: &ExiOptions,
) -> ExiResult<W> {
    let tables = &order_complex::GRAMMARS;
    let mut dec = Decoder::new(input, tables, Some(options.clone()))?;
    let mut enc = Encoder::new(out, tables, options.clone(), HeaderConfig::default())?;
    while let Some(ev) = dec.next_event()? {
        enc.push(&ev)?;
    }
    enc.finish()
}

/// [`transcode`] into a growable buffer.
pub fn transcode_to_vec(input: &[u8], options: &ExiOptions) -> ExiResult<Vec<u8>> {
    transcode(input, Vec::new(), options)
}

/// [`transcode`] into a fixed buffer from the caller, with no allocation for the
/// output. Returns the number of bytes written.
pub fn transcode_to_slice(input: &[u8], buf: &mut [u8], options: &ExiOptions) -> ExiResult<usize> {
    transcode(input, SliceSink::new(buf), options).map(|sink| sink.len())
}

/// rexi-serde without `std`: reads a value of `T` from an order-complex stream.
pub fn from_exi<T: DeserializeOwned>(input: &[u8], options: &ExiOptions) -> rexi_serde::Result<T> {
    rexi_serde::from_exi(input, order_complex::GRAMMARS, Some(options.clone()))
}
