//! Helpers that do a whole job in one call, built on the reader and the codec.

use rexi_core::{Encoder, ExiOptions, GrammarTables, HeaderConfig};

use crate::Result;
use crate::reader::push_xml;

/// Encodes an XML document to EXI bytes with the given grammars, options and
/// header settings. The XML is parsed and pushed into the encoder event by event,
/// then the encoder is finished and its bytes returned.
pub fn encode_xml<G: GrammarTables>(
    xml: &[u8],
    tables: G,
    options: &ExiOptions,
    header: HeaderConfig,
) -> Result<Vec<u8>> {
    // EXI is nearly always smaller than its XML, so this is room enough.
    let mut enc = Encoder::new(
        Vec::with_capacity(xml.len()),
        tables,
        options.clone(),
        header,
    )?;
    push_xml(xml, &mut enc)?;
    Ok(enc.finish()?)
}
