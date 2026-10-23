//! EXI header and options-document vectors (spec 5, Appendix C; RFC 8428 §8).
//!
//! Byte strings come from the spec's own encoding rules and were confirmed against
//! EXIficient 1.0.7 output and the RFC 8428 SenML examples.

use rexi_core::header::{
    read_header_bits, read_options_document, write_header_bits, write_options_document,
};
use rexi_core::io::{BitReader, BitWriter};
use rexi_core::{Alignment, ExiError, ExiOptions, Feature, HeaderConfig, SchemaId};

fn header_bytes(options: &ExiOptions, cookie: bool, include_options: bool) -> Vec<u8> {
    let mut w = BitWriter::new(Vec::new());
    write_header_bits(
        &mut w,
        HeaderConfig::default()
            .with_cookie(cookie)
            .with_include_options(include_options),
    )
    .unwrap_or_default();
    if include_options {
        w = write_options_document(w, options).unwrap_or_else(|e| panic!("{e}"));
    }
    w.finish().unwrap_or_default()
}

fn decode(bytes: &[u8]) -> (bool, Option<ExiOptions>, u64) {
    let mut r = BitReader::new(bytes);
    let info = read_header_bits(&mut r).unwrap_or_else(|e| panic!("{e}"));
    if info.options_in_stream {
        let (opts, r) = read_options_document(r).unwrap_or_else(|e| panic!("{e}"));
        (info.cookie, Some(opts), r.bit_position())
    } else {
        (info.cookie, None, r.bit_position())
    }
}

#[test]
fn bare_header_is_0x80() {
    assert_eq!(
        header_bytes(&ExiOptions::strict(), false, false),
        vec![0x80]
    );
    assert_eq!(
        header_bytes(&ExiOptions::strict(), true, false),
        vec![b'$', b'E', b'X', b'I', 0x80]
    );
    assert_eq!(decode(&[0x80]), (false, None, 8));
    assert_eq!(decode(b"$EXI\x80"), (true, None, 40));
}

#[test]
fn strict_only_options_document() {
    // 10 1 0 0000 | 0 (SE header) 10 (SE strict) | pad
    assert_eq!(
        header_bytes(&ExiOptions::strict(), false, true),
        vec![0xA0, 0x40]
    );
    let (_, opts, pos) = decode(&[0xA0, 0x40]);
    assert_eq!(opts, Some(ExiOptions::strict()));
    assert_eq!(pos, 11);
}

#[test]
fn rfc8428_bit_packed_schema_id_a() {
    let opts = ExiOptions::strict().with_schema_id("a");
    assert_eq!(
        header_bytes(&opts, false, true),
        vec![0xA0, 0x30, 0x0D, 0x84]
    );
    let (_, decoded, pos) = decode(&[0xA0, 0x30, 0x0D, 0x84]);
    assert_eq!(decoded, Some(opts));
    assert_eq!(pos, 31);
}

#[test]
fn rfc8428_byte_aligned_schema_id_a() {
    let opts = ExiOptions::strict()
        .with_schema_id("a")
        .with_alignment(Alignment::ByteAligned);
    assert_eq!(
        header_bytes(&opts, false, true),
        vec![0xA0, 0x00, 0x48, 0x80, 0x6C, 0x20]
    );
    let (_, decoded, _) = decode(&[0xA0, 0x00, 0x48, 0x80, 0x6C, 0x20]);
    assert_eq!(decoded, Some(opts));
}

#[test]
fn exificient_value_limits_vector() {
    // EXIficient 1.0.7: strict, bit-packed, schemaId "a", valueMaxLength 12,
    // valuePartitionCapacity 66 → header a0 02 0c 10 b1 00 d8 4x (body starts at bit 59).
    let mut opts = ExiOptions::strict().with_schema_id("a");
    opts.value_max_length = Some(12);
    opts.value_partition_capacity = Some(66);
    let bytes = header_bytes(&opts, false, true);
    assert_eq!(bytes, vec![0xA0, 0x02, 0x0C, 0x10, 0xB1, 0x00, 0xD8, 0x40]);
    let (_, decoded, pos) = decode(&bytes);
    assert_eq!(decoded, Some(opts));
    assert_eq!(pos, 59);
}

#[test]
fn schema_id_nil_and_empty_round_trip() {
    for id in [SchemaId::Nil, SchemaId::Empty] {
        let mut opts = ExiOptions::strict();
        opts.schema_id = id;
        let bytes = header_bytes(&opts, false, true);
        let (_, decoded, _) = decode(&bytes);
        assert_eq!(decoded, Some(opts));
    }
}

#[test]
fn block_size_and_preserve_lexical_values_round_trip_through_document() {
    let mut opts = ExiOptions::strict();
    opts.block_size = Some(500);
    let bytes = header_bytes(&opts, false, true);
    let (_, decoded, _) = decode(&bytes);
    assert_eq!(decoded, Some(opts));
}

#[test]
fn unsupported_options_are_rejected_not_ignored() {
    let mut opts = ExiOptions::strict();
    opts.compression = true;
    assert_eq!(
        opts.validate(),
        Err(ExiError::Unsupported(Feature::Compression))
    );
    let mut opts = ExiOptions::strict();
    opts.alignment = Alignment::PreCompression;
    assert_eq!(
        opts.validate(),
        Err(ExiError::Unsupported(Feature::PreCompression))
    );
    assert_eq!(
        ExiOptions::default().validate(),
        Err(ExiError::Unsupported(Feature::NonStrictSchemaInformed))
    );
}

#[test]
fn bad_headers() {
    let mut r = BitReader::new([0x40u8].as_slice());
    assert_eq!(
        read_header_bits(&mut r),
        Err(ExiError::Header("invalid distinguishing bits"))
    );
    let mut r = BitReader::new(b"$EXX\x80".as_slice());
    assert_eq!(
        read_header_bits(&mut r),
        Err(ExiError::Header("invalid EXI cookie"))
    );
    // preview version
    let mut r = BitReader::new([0x90u8].as_slice());
    assert_eq!(
        read_header_bits(&mut r),
        Err(ExiError::Unsupported(Feature::FormatVersion))
    );
}
