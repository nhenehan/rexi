//! Host run of the no_std code paths on the order-complex fixtures.

use rexi_core::{Alignment, ExiError, IoErrorKind};
use rexi_nostd_check::{strict, transcode_to_slice, transcode_to_vec};

const BIT_PACKED: &[u8] =
    include_bytes!("../../rexi-tests/fixtures/interop/order-complex.strict-bp.exi");
const BYTE_ALIGNED: &[u8] =
    include_bytes!("../../rexi-tests/fixtures/interop/order-complex.strict-ba.exi");

#[test]
fn vec_sink_round_trips_both_alignments() {
    for (input, alignment) in [
        (BIT_PACKED, Alignment::BitPacked),
        (BYTE_ALIGNED, Alignment::ByteAligned),
    ] {
        assert_eq!(
            transcode_to_vec(input, &strict(alignment)).as_deref(),
            Ok(input)
        );
    }
}

#[test]
fn slice_sink_round_trips_and_reports_a_full_buffer() {
    let options = strict(Alignment::BitPacked);
    let mut buf = [0u8; 512];
    let n = transcode_to_slice(BIT_PACKED, &mut buf, &options);
    assert_eq!(n, Ok(BIT_PACKED.len()));
    assert_eq!(&buf[..BIT_PACKED.len()], BIT_PACKED);

    let mut small = [0u8; 16];
    assert_eq!(
        transcode_to_slice(BIT_PACKED, &mut small, &options),
        Err(ExiError::Io(IoErrorKind::BufferFull))
    );
}

#[test]
fn truncated_input_is_reported() {
    let options = strict(Alignment::BitPacked);
    let cut = &BIT_PACKED[..BIT_PACKED.len() / 2];
    assert_eq!(transcode_to_vec(cut, &options), Err(ExiError::Truncated));
}
