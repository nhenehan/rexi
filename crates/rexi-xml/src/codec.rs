//! Helpers that do a whole job in one call, built on the reader and the codec.

use rexi_core::{Encoder, ExiOptions, GrammarTables, HeaderConfig};

use crate::Result;
use crate::reader::push_xml;

/// Encodes an XML document to EXI bytes with the given grammars, options and
/// header settings. The XML is parsed and pushed into the encoder event by event,
/// then the encoder is finished and its bytes returned.
///
/// # Errors
///
/// [`XmlError::Syntax`](crate::XmlError::Syntax) when the XML is not
/// well-formed, [`XmlError::Unsupported`](crate::XmlError::Unsupported) for XML
/// this front end does not handle (a DOCTYPE, an undeclared prefix, ...), and
/// [`XmlError::Exi`](crate::XmlError::Exi) when the document does not match the
/// schema or a value does not fit its datatype.
pub fn encode_xml<G: GrammarTables>(
    xml: &[u8],
    tables: G,
    options: ExiOptions,
    header: HeaderConfig,
) -> Result<Vec<u8>> {
    // EXI is nearly always smaller than its XML, so this is room enough.
    let mut enc = Encoder::new(Vec::with_capacity(xml.len()), tables, options, header)?;
    push_xml(xml, &mut enc)?;
    Ok(enc.finish()?)
}
