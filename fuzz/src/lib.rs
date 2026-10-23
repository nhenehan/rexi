//! Shared setup for the fuzz targets.

use rexi_core::{Alignment, ExiOptions, GrammarSet};
use rexi_tests::generated;

/// The schemas the decode target picks from: every generated fixture case and the
/// EXI Options document.
pub fn grammars() -> [&'static GrammarSet<'static>; 7] {
    [
        &generated::order_complex::GRAMMARS,
        &generated::telemetry_complex::GRAMMARS,
        &generated::polymorphic_choice::GRAMMARS,
        &generated::inheritance_extension::GRAMMARS,
        &generated::cross_schema_main::GRAMMARS,
        &generated::multilingual::GRAMMARS,
        &generated::exi_options::EXI_OPTIONS,
    ]
}

/// The options a selector byte stands for: in the header (`None`), or agreed out of
/// band as strict with either alignment.
pub fn options(selector: u8) -> Option<ExiOptions> {
    match selector % 3 {
        0 => None,
        1 => Some(ExiOptions::strict()),
        _ => Some(ExiOptions::strict().with_alignment(Alignment::ByteAligned)),
    }
}
