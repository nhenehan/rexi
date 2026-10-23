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
    let mut enc = Encoder::new(Vec::new(), tables, options.clone(), header)?;
    push_xml(xml, &mut enc)?;
    Ok(enc.finish()?)
}
