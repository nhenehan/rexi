//! Bit-level reading and writing
//! ([EXI 6.2](https://www.w3.org/TR/exi/#encodingEventCodes) and
//! [EXI 7.1](https://www.w3.org/TR/exi/#encodingDatatypes)).
//!
//! Both alignment modes the codec supports live here. The `alignment` option in the
//! header picks one ([EXI 5.4](https://www.w3.org/TR/exi/#options)):
//!
//! * **bit-packed**: values are packed MSB-first (highest bit first) with no padding
//!   between them;
//! * **byte-aligned**: every n-bit value takes `ceil(n / 8)` whole octets, least
//!   significant octet first, and booleans take one octet
//!   ([EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)).
//!
//! The raw bit operations (`write_raw_bits`, `read_raw_bits`) ignore the alignment
//! mode. They exist for the EXI header, whose bits are always packed.

mod bit_reader;
mod bit_writer;
#[cfg(test)]
mod bit_writer_reference;
mod bits;
mod bytes;

// The bit reader and writer are used by rexi's own tests and tools, but are not
// part of the supported API.
#[doc(hidden)]
pub use bit_reader::BitReader;
#[doc(hidden)]
pub use bit_writer::BitWriter;
pub use bits::{bits_to_represent, ceil_log2};
pub use bytes::{ByteSink, ByteSource, SliceSink};

pub use crate::error::IoErrorKind;
