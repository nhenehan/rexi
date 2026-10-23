//! The EXI header ([EXI 5](https://www.w3.org/TR/exi/#header)): cookie,
//! distinguishing bits, presence bit, format version, and the EXI Options document
//! ([EXI 5.4](https://www.w3.org/TR/exi/#options),
//! [Appendix C](https://www.w3.org/TR/exi/#optionsSchema)).
//!
//! Bit layout, in stream order:
//!
//! ```text
//! [ "$EXI" ]   4 bytes, optional cookie (5.1)
//! 10           2 bits, distinguishing bits, always present (5.2)
//! p            1 bit, presence bit: 1 = an options document follows
//! 0            1 bit, 0 = final version, 1 = preview version (5.3)
//! 0000         4-bit chunks of the version number; 15 means "add 15 and read
//!              another chunk", 0-14 ends it; version = sum + 1, so 0000 = version 1
//! [ options ]  optional EXI Options document (5.4)
//! ```
//!
//! The spec also puts padding bits at the end of the header, up to a byte boundary,
//! when compression is on or the alignment is byte-alignment or pre-compression.
//!
//! The options document is an EXI body with no header of its own. It is encoded
//! with the grammar in [`crate::grammar::options_grammar`] under the options the
//! spec fixes for it (`<header><strict/></header>`): strict, bit-packed, and a fresh
//! string table. It goes through the normal [`Encoder`] / [`Decoder`].

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;

use crate::decoder::Decoder;
use crate::encoder::Encoder;
use crate::error::{ExiError, ExiResult, Feature};
use crate::event::Event;
use crate::grammar::options_grammar as og;
use crate::grammar::options_grammar::{EXI_URI, OPTIONS_GRAMMARS};
use crate::io::{BitReader, BitWriter};
use crate::options::{Alignment, ExiOptions, HeaderConfig, SchemaId};
use crate::value::{Int, Uint, Value};

/// The optional 4-byte EXI cookie, `$EXI`
/// ([EXI 5.1](https://www.w3.org/TR/exi/#EXICookie)).
pub const EXI_COOKIE: [u8; 4] = *b"$EXI";
const EXI_NS: &str = "http://www.w3.org/2009/exi";

/// What the header of a decoded stream said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeaderInfo {
    /// The stream starts with the `$EXI` cookie.
    pub cookie: bool,
    /// The header holds an options document.
    pub options_in_stream: bool,
    /// The version is a preview version (rexi refuses these).
    pub preview_version: bool,
    /// The format version number (1 for EXI 1.0).
    pub format_version: u32,
}

/// Writes `[$EXI] 10 presence 0 0000`: optional cookie, distinguishing bits,
/// presence bit, then the version field for final version 1
/// ([EXI 5](https://www.w3.org/TR/exi/#header)).
pub fn write_header_bits<W: ByteSink>(w: &mut BitWriter<W>, header: HeaderConfig) -> ExiResult<()> {
    if header.cookie {
        w.write_bytes(&EXI_COOKIE)?;
    }
    w.write_raw_bits(0b10, 2)?;
    w.write_raw_bits(u64::from(header.include_options), 1)?;
    // Final version 1: preview bit 0, then the single 4-bit chunk 0000 (0 + 1 = 1).
    w.write_raw_bits(0, 1)?;
    w.write_raw_bits(0, 4)?;
    Ok(())
}

/// Reads the header bits, with or without the cookie.
///
/// The first two bits tell the cases apart: `10` is the distinguishing bits (no
/// cookie), and `00` is the start of `$` (0x24), so a cookie must follow. Any other
/// pair is an error ([EXI 5.1](https://www.w3.org/TR/exi/#EXICookie),
/// [EXI 5.2](https://www.w3.org/TR/exi/#DistinguishingBits)). The version is read as
/// in [EXI 5.3](https://www.w3.org/TR/exi/#version), and anything but final version 1
/// is refused.
pub fn read_header_bits<R: ByteSource>(r: &mut BitReader<R>) -> ExiResult<HeaderInfo> {
    let mut info = HeaderInfo::default();
    let first = r.read_raw_bits(2)?;
    match first {
        0b10 => {}
        0b00 => {
            // '$' is 0x24 = 00 100100: the other six bits must match.
            let rest = r.read_raw_bits(6)?;
            if rest != u64::from(EXI_COOKIE[0] & 0x3F) {
                return Err(ExiError::Header("invalid EXI cookie"));
            }
            for expected in &EXI_COOKIE[1..] {
                if r.read_byte()? != *expected {
                    return Err(ExiError::Header("invalid EXI cookie"));
                }
            }
            info.cookie = true;
            if r.read_raw_bits(2)? != 0b10 {
                return Err(ExiError::Header("invalid distinguishing bits after cookie"));
            }
        }
        _ => return Err(ExiError::Header("invalid distinguishing bits")),
    }
    info.options_in_stream = r.read_raw_bits(1)? == 1;
    info.preview_version = r.read_raw_bits(1)? == 1;
    let mut version = 0u32;
    loop {
        let chunk = r.read_raw_bits(4)? as u32;
        version += chunk;
        if chunk != 15 {
            break;
        }
    }
    info.format_version = version + 1;
    if info.preview_version || info.format_version != 1 {
        return Err(ExiError::Unsupported(Feature::FormatVersion));
    }
    Ok(info)
}

fn exi(local: &str) -> crate::event::QName<'_> {
    crate::event::QName::new(EXI_NS, local)
}

/// Encodes `options` as an EXI Options document into `writer` (bit-packed, strict).
///
/// The Appendix C schema groups options as
/// `header( lesscommon( uncommon(...), preserve(...), blockSize ), common(...),
/// strict )`, and each group is optional. So a group's element is written only when
/// at least one option inside it is not the default. A true boolean option is an
/// empty element, and a false one is left out
/// ([EXI 5.4](https://www.w3.org/TR/exi/#options),
/// [Appendix C](https://www.w3.org/TR/exi/#optionsSchema)).
pub fn write_options_document<W: ByteSink>(
    writer: BitWriter<W>,
    options: &ExiOptions,
) -> ExiResult<BitWriter<W>> {
    let mut enc = Encoder::for_body(writer, &OPTIONS_GRAMMARS, ExiOptions::strict())?;
    let mut push = |ev: Event<'_>| enc.push(&ev);
    push(Event::StartDocument)?;
    push(Event::StartElement(exi("header")))?;

    let uncommon = options.alignment != Alignment::BitPacked
        || options.self_contained
        || options.value_max_length.is_some()
        || options.value_partition_capacity.is_some()
        || options.datatype_representation_map;
    let p = options.preserve;
    let preserve = p.dtd || p.prefixes || p.lexical_values || p.comments || p.pis;
    let lesscommon = uncommon || preserve || options.block_size.is_some();

    if lesscommon {
        push(Event::StartElement(exi("lesscommon")))?;
        if uncommon {
            push(Event::StartElement(exi("uncommon")))?;
            match options.alignment {
                Alignment::BitPacked => {}
                Alignment::ByteAligned | Alignment::PreCompression => {
                    push(Event::StartElement(exi("alignment")))?;
                    let which = if options.alignment == Alignment::ByteAligned {
                        "byte"
                    } else {
                        "pre-compress"
                    };
                    push(Event::StartElement(exi(which)))?;
                    push(Event::EndElement)?;
                    push(Event::EndElement)?;
                }
            }
            if options.self_contained {
                push(Event::StartElement(exi("selfContained")))?;
                push(Event::EndElement)?;
            }
            if let Some(n) = options.value_max_length {
                push(Event::StartElement(exi("valueMaxLength")))?;
                push(Event::Characters(Value::Unsigned(Uint::Small(u64::from(
                    n,
                )))))?;
                push(Event::EndElement)?;
            }
            if let Some(n) = options.value_partition_capacity {
                push(Event::StartElement(exi("valuePartitionCapacity")))?;
                push(Event::Characters(Value::Unsigned(Uint::Small(u64::from(
                    n,
                )))))?;
                push(Event::EndElement)?;
            }
            if options.datatype_representation_map {
                return Err(ExiError::Unsupported(Feature::DatatypeRepresentationMap));
            }
            push(Event::EndElement)?;
        }
        if preserve {
            push(Event::StartElement(exi("preserve")))?;
            for (flag, name) in [
                (p.dtd, "dtd"),
                (p.prefixes, "prefixes"),
                (p.lexical_values, "lexicalValues"),
                (p.comments, "comments"),
                (p.pis, "pis"),
            ] {
                if flag {
                    push(Event::StartElement(exi(name)))?;
                    push(Event::EndElement)?;
                }
            }
            push(Event::EndElement)?;
        }
        if let Some(n) = options.block_size {
            push(Event::StartElement(exi("blockSize")))?;
            push(Event::Characters(Value::Unsigned(Uint::Small(u64::from(
                n,
            )))))?;
            push(Event::EndElement)?;
        }
        push(Event::EndElement)?;
    }

    let common = options.compression || options.fragment || options.schema_id != SchemaId::Absent;
    if common {
        push(Event::StartElement(exi("common")))?;
        if options.compression {
            push(Event::StartElement(exi("compression")))?;
            push(Event::EndElement)?;
        }
        if options.fragment {
            push(Event::StartElement(exi("fragment")))?;
            push(Event::EndElement)?;
        }
        match &options.schema_id {
            SchemaId::Absent => {}
            SchemaId::Nil => {
                push(Event::StartElement(exi("schemaId")))?;
                push(Event::XsiNil(true))?;
                push(Event::EndElement)?;
            }
            SchemaId::Empty => {
                push(Event::StartElement(exi("schemaId")))?;
                push(Event::Characters(Value::String("")))?;
                push(Event::EndElement)?;
            }
            SchemaId::Id(id) => {
                push(Event::StartElement(exi("schemaId")))?;
                push(Event::Characters(Value::String(id)))?;
                push(Event::EndElement)?;
            }
        }
        push(Event::EndElement)?;
    }
    if options.strict {
        push(Event::StartElement(exi("strict")))?;
        push(Event::EndElement)?;
    }
    push(Event::EndElement)?;
    push(Event::EndDocument)?;
    Ok(enc.into_writer())
}

/// Decodes an EXI Options document from `reader`.
///
/// Options start at their spec defaults. Each element that turns an option on sets
/// it when its start tag arrives. Number and string options are set when their
/// character data arrives, using the stack of open elements to know which option
/// the text belongs to. `schemaId` with `xsi:nil="true"` means "no schema"
/// ([EXI 5.4](https://www.w3.org/TR/exi/#options)).
pub fn read_options_document<R: ByteSource>(
    reader: BitReader<R>,
) -> ExiResult<(ExiOptions, BitReader<R>)> {
    let mut dec = Decoder::for_body(reader, &OPTIONS_GRAMMARS, ExiOptions::strict())?;
    let mut options = ExiOptions::default();
    // The open elements, as local-name ids. Values are read when a CH arrives.
    let mut path: Vec<u32> = Vec::new();
    while let Some(ev) = dec.next_event()? {
        match ev {
            Event::StartDocument | Event::EndDocument => {}
            Event::StartElement(q) => {
                let Some(id) = OPTIONS_GRAMMARS_local(q.local) else {
                    return Err(ExiError::Header("unknown element in options document"));
                };
                match id {
                    og::LN_BYTE => options.alignment = Alignment::ByteAligned,
                    og::LN_PRE_COMPRESS => options.alignment = Alignment::PreCompression,
                    og::LN_SELFCONTAINED => options.self_contained = true,
                    og::LN_DATATYPEREPRESENTATIONMAP => options.datatype_representation_map = true,
                    og::LN_DTD => options.preserve.dtd = true,
                    og::LN_PREFIXES => options.preserve.prefixes = true,
                    og::LN_LEXICALVALUES => options.preserve.lexical_values = true,
                    og::LN_COMMENTS => options.preserve.comments = true,
                    og::LN_PIS => options.preserve.pis = true,
                    og::LN_COMPRESSION => options.compression = true,
                    og::LN_FRAGMENT => options.fragment = true,
                    og::LN_STRICT => options.strict = true,
                    og::LN_SCHEMAID => options.schema_id = SchemaId::Empty,
                    _ => {}
                }
                path.push(id);
            }
            Event::EndElement => {
                path.pop();
            }
            Event::XsiNil(true) if path.last() == Some(&og::LN_SCHEMAID) => {
                options.schema_id = SchemaId::Nil
            }
            Event::XsiNil(_) => {}
            Event::Characters(v) => {
                let Some(&current) = path.last() else {
                    return Err(ExiError::Header("character data outside an element"));
                };
                match current {
                    og::LN_SCHEMAID => {
                        let s = match &v {
                            Value::String(s) | Value::Lexical(s) => (*s).to_string(),
                            _ => return Err(ExiError::Header("schemaId is not a string")),
                        };
                        options.schema_id = if s.is_empty() {
                            SchemaId::Empty
                        } else {
                            SchemaId::Id(s)
                        };
                    }
                    og::LN_VALUEMAXLENGTH | og::LN_VALUEPARTITIONCAPACITY | og::LN_BLOCKSIZE => {
                        let n = match &v {
                            Value::Unsigned(u) => u.to_u64(),
                            Value::Integer(Int::Small(i)) if *i >= 0 => Some(*i as u64),
                            _ => None,
                        };
                        let Some(n) = n.and_then(|n| u32::try_from(n).ok()) else {
                            return Err(ExiError::Header("option value out of range"));
                        };
                        match current {
                            og::LN_VALUEMAXLENGTH => options.value_max_length = Some(n),
                            og::LN_VALUEPARTITIONCAPACITY => {
                                options.value_partition_capacity = Some(n)
                            }
                            _ => options.block_size = Some(n),
                        }
                    }
                    _ => {
                        return Err(ExiError::Header(
                            "unexpected character data in options document",
                        ));
                    }
                }
            }
            Event::XsiType(_) => return Err(ExiError::Header("xsi:type in options document")),
            _ => return Err(ExiError::Header("unexpected event in options document")),
        }
    }
    Ok((options, dec.into_reader()))
}

/// The local-name id of `local` in the exi namespace partition of the options
/// grammar, or `None` if it is not one of the names declared there.
#[allow(non_snake_case)]
fn OPTIONS_GRAMMARS_local(local: &str) -> Option<u32> {
    use crate::grammar::ir::GrammarTables;
    OPTIONS_GRAMMARS
        .find_qname(EXI_NS, local)
        .filter(|q| q.uri == EXI_URI)
        .map(|q| q.local)
}
