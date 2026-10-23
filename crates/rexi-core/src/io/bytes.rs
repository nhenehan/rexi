//! Byte-level input and output for the bit reader and writer.
//!
//! `rexi-core` builds without `std`, so it cannot require `std::io::Read` / `Write`
//! for its streams. [`ByteSource`] and [`ByteSink`] are the smallest stand-ins that
//! do the job. They are always implemented for `&[u8]` (source), `Vec<u8>`
//! (sink) and [`SliceSink`] (a fixed buffer); with the `std` feature, every
//! `std::io::Read` is a `ByteSource` and every `std::io::Write` a `ByteSink`, so
//! callers holding a `File`, `Cursor` or `&mut Vec<u8>` need no adapter.
//!
//! The method names are not the same as `std::io`'s, so a caller with both traits in
//! scope never gets an unclear (ambiguous) method call.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use crate::error::IoErrorKind;

/// Where the decoder reads its bytes from.
pub trait ByteSource {
    /// Reads up to `buf.len()` bytes into `buf`; `Ok(0)` means end of input.
    fn get_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoErrorKind>;

    /// Fills `buf` completely, or fails with [`IoErrorKind::UnexpectedEof`].
    fn get_bytes_exact(&mut self, mut buf: &mut [u8]) -> Result<(), IoErrorKind> {
        while !buf.is_empty() {
            match self.get_bytes(buf)? {
                0 => return Err(IoErrorKind::UnexpectedEof),
                n => buf = &mut buf[n..],
            }
        }
        Ok(())
    }
}

/// Where the encoder writes its bytes to.
pub trait ByteSink {
    /// Writes all of `bytes`, or fails.
    fn put_bytes(&mut self, bytes: &[u8]) -> Result<(), IoErrorKind>;

    /// Pushes buffered bytes to where they are going. Does nothing for in-memory
    /// sinks.
    fn flush_bytes(&mut self) -> Result<(), IoErrorKind> {
        Ok(())
    }
}

#[cfg(feature = "std")]
impl<R: std::io::Read + ?Sized> ByteSource for R {
    /// Retries when the read is interrupted (`ErrorKind::Interrupted`), as
    /// `std::io` callers are expected to.
    fn get_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoErrorKind> {
        loop {
            match std::io::Read::read(self, buf) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                other => return other.map_err(IoErrorKind::from),
            }
        }
    }

    fn get_bytes_exact(&mut self, buf: &mut [u8]) -> Result<(), IoErrorKind> {
        std::io::Read::read_exact(self, buf).map_err(IoErrorKind::from)
    }
}

#[cfg(feature = "std")]
impl<W: std::io::Write + ?Sized> ByteSink for W {
    fn put_bytes(&mut self, bytes: &[u8]) -> Result<(), IoErrorKind> {
        std::io::Write::write_all(self, bytes).map_err(IoErrorKind::from)
    }

    fn flush_bytes(&mut self) -> Result<(), IoErrorKind> {
        std::io::Write::flush(self).map_err(IoErrorKind::from)
    }
}

// Without `std`, the same conveniences by hand. (With `std` the blanket impls
// above already cover these types, and a second impl would conflict.)

#[cfg(not(feature = "std"))]
impl ByteSource for &[u8] {
    fn get_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoErrorKind> {
        let n = buf.len().min(self.len());
        let (head, tail) = self.split_at(n);
        buf[..n].copy_from_slice(head);
        *self = tail;
        Ok(n)
    }
}

#[cfg(not(feature = "std"))]
impl<R: ByteSource + ?Sized> ByteSource for &mut R {
    fn get_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoErrorKind> {
        (**self).get_bytes(buf)
    }

    fn get_bytes_exact(&mut self, buf: &mut [u8]) -> Result<(), IoErrorKind> {
        (**self).get_bytes_exact(buf)
    }
}

#[cfg(not(feature = "std"))]
impl ByteSink for Vec<u8> {
    fn put_bytes(&mut self, bytes: &[u8]) -> Result<(), IoErrorKind> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

#[cfg(not(feature = "std"))]
impl<W: ByteSink + ?Sized> ByteSink for &mut W {
    fn put_bytes(&mut self, bytes: &[u8]) -> Result<(), IoErrorKind> {
        (**self).put_bytes(bytes)
    }

    fn flush_bytes(&mut self) -> Result<(), IoErrorKind> {
        (**self).flush_bytes()
    }
}

/// A sink over a buffer the caller provides, for targets with no heap to spare.
/// Writing past its end fails with [`IoErrorKind::BufferFull`].
#[derive(Debug)]
pub struct SliceSink<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> SliceSink<'a> {
    /// A sink that writes into `buf`, starting at its first byte.
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0 }
    }

    /// Number of bytes written so far.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing has been written yet.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes written so far.
    pub fn written(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Gives back the written front part of the buffer, with the buffer's lifetime.
    pub fn into_written(self) -> &'a mut [u8] {
        &mut self.buf[..self.len]
    }
}

impl ByteSink for SliceSink<'_> {
    /// Writes all of `bytes` or none of them: if they do not fit, it fails with
    /// [`IoErrorKind::BufferFull`] and leaves the buffer as it was.
    fn put_bytes(&mut self, bytes: &[u8]) -> Result<(), IoErrorKind> {
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(IoErrorKind::BufferFull)?;
        let dst = self
            .buf
            .get_mut(self.len..end)
            .ok_or(IoErrorKind::BufferFull)?;
        dst.copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    #[test]
    fn slice_source_reads_then_ends() {
        let mut src: &[u8] = &[1, 2, 3];
        let mut buf = [0u8; 2];
        assert_eq!(src.get_bytes(&mut buf), Ok(2));
        assert_eq!(buf, [1, 2]);
        assert_eq!(
            src.get_bytes_exact(&mut buf),
            Err(IoErrorKind::UnexpectedEof)
        );
    }

    #[test]
    fn vec_sink_appends() {
        let mut out = Vec::new();
        out.put_bytes(&[1, 2])
            .and_then(|()| out.put_bytes(&[3]))
            .unwrap_or_default();
        assert_eq!(out, [1, 2, 3]);
    }

    #[test]
    fn slice_sink_fails_when_full() {
        let mut buf = [0u8; 3];
        let mut sink = SliceSink::new(&mut buf);
        assert_eq!(sink.put_bytes(&[1, 2]), Ok(()));
        assert_eq!(sink.put_bytes(&[3, 4]), Err(IoErrorKind::BufferFull));
        assert_eq!(sink.put_bytes(&[3]), Ok(()));
        assert_eq!(sink.written(), [1, 2, 3]);
    }
}
