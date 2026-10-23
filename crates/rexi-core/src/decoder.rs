//! The SAX-like EXI decoder: pull typed events out of a stream.
//!
//! Each call reads one event code for the current rule of the innermost open
//! element. The code's width comes from the rule (see [`crate::grammar::runtime`]).
//! The code picks a production; the decoder reads that event's content, moves to
//! the production's next rule, and returns the event
//! ([EXI 6](https://www.w3.org/TR/exi/#encodingEvents)). A code one past the last
//! production is the escape to `xsi:type` / `xsi:nil`
//! ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).

use crate::io::ByteSource;
use crate::prelude::*;

use crate::datatype::unsigned::read_unsigned;
use crate::datatype::{self, DatatypeKind, TypeResolver, binary, lexical};
use crate::error::{ExiError, ExiResult, Feature};
use crate::event::{Event, QName};
use crate::grammar::ir::{GrammarTables, Terminal};
use crate::grammar::runtime::{Cursor, Frame};
use crate::header::{self, HeaderInfo};
use crate::ids::{ElemIdx, NONE, QNameId, TypeIdx};
use crate::io::{BitReader, ceil_log2};
use crate::limits::{DecodeLimits, Limit};
use crate::options::ExiOptions;
use crate::string_table::StringTable;
use crate::value::OwnedValue;

/// What the last `step` produced. It is turned into an [`Event`] afterwards, so the
/// event can borrow strings from the decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Sd,
    Ed,
    Se(QNameId),
    Ee,
    At(QNameId),
    XsiType(QNameId),
    XsiNil(bool),
    Ch,
    Done,
}

/// Decodes an EXI stream from the byte source `R` into [`Event`]s, using the
/// grammar tables `G`.
pub struct Decoder<R: ByteSource, G: GrammarTables> {
    inp: BitReader<R>,
    tables: G,
    options: ExiOptions,
    header: HeaderInfo,
    strings: StringTable,
    frames: Vec<Frame>,
    cursor: Cursor,
    value: OwnedValue,
    /// The type of the last value read, as a type index (`NONE` for untyped
    /// text); `None` before the first value. The datatype itself is looked up only
    /// when asked for: keeping the index is cheaper on every event.
    last_type: Option<TypeIdx>,
    /// Spare buffers: while `value` holds something else, the last string's and
    /// the last binary value's storage wait here to be reused.
    text: String,
    bytes: Vec<u8>,
    limits: DecodeLimits,
    /// UTF-8 bytes of every string value decoded so far, for
    /// [`DecodeLimits::max_string_bytes`].
    string_bytes: u64,
}

impl<R: ByteSource, G: GrammarTables> Decoder<R, G> {
    /// Reads the EXI header ([EXI 5](https://www.w3.org/TR/exi/#header)). Options
    /// found in the stream win over `out_of_band`. If the stream has none,
    /// `out_of_band` must be given.
    ///
    /// Uses the safe [`DecodeLimits::default`]. Call [`Decoder::with_limits`] to
    /// pick others.
    pub fn new(input: R, tables: G, out_of_band: Option<ExiOptions>) -> ExiResult<Self> {
        Self::with_limits(input, tables, out_of_band, DecodeLimits::default())
    }

    /// Like [`Decoder::new`], with the given limits on what the stream may ask
    /// for. See [`DecodeLimits`].
    pub fn with_limits(
        input: R,
        tables: G,
        out_of_band: Option<ExiOptions>,
        limits: DecodeLimits,
    ) -> ExiResult<Self> {
        let mut reader = BitReader::new(input);
        reader.set_max_integer_bits(limits.max_integer_bits);
        let info = header::read_header_bits(&mut reader)?;
        let options = if info.options_in_stream {
            let (opts, r) = header::read_options_document(reader)?;
            reader = r;
            opts
        } else {
            out_of_band.ok_or(ExiError::Header(
                "stream carries no options and none were given out of band",
            ))?
        };
        options.validate()?;
        if options.byte_aligned() {
            reader.set_byte_aligned(true);
        }
        let mut dec = Self::for_body_with_limits(reader, tables, options, limits)?;
        dec.header = info;
        Ok(dec)
    }

    /// Decodes a bare body (no header) with the given options and the default
    /// limits.
    #[doc(hidden)]
    pub fn for_body(reader: BitReader<R>, tables: G, options: ExiOptions) -> ExiResult<Self> {
        Self::for_body_with_limits(reader, tables, options, DecodeLimits::default())
    }

    /// Decodes a bare body (no header) with the given options and limits.
    #[doc(hidden)]
    pub fn for_body_with_limits(
        mut reader: BitReader<R>,
        tables: G,
        options: ExiOptions,
        limits: DecodeLimits,
    ) -> ExiResult<Self> {
        options.validate()?;
        reader.set_max_integer_bits(limits.max_integer_bits);
        let strings = crate::encoder::init_string_table(&tables, &options);
        Ok(Self {
            inp: reader,
            tables,
            options,
            header: HeaderInfo::default(),
            strings,
            frames: Vec::new(),
            cursor: Cursor::Document,
            value: OwnedValue::Boolean(false),
            last_type: None,
            text: String::new(),
            bytes: Vec::new(),
            limits,
            string_bytes: 0,
        })
    }

    /// The limits this decoder checks.
    pub fn limits(&self) -> &DecodeLimits {
        &self.limits
    }

    /// The options the stream is decoded with (from the header, or the ones given
    /// out of band).
    pub fn options(&self) -> &ExiOptions {
        &self.options
    }

    /// What the header said.
    pub fn header(&self) -> &HeaderInfo {
        &self.header
    }

    /// Number of bits read so far, header included (if there was one).
    pub fn bit_position(&self) -> u64 {
        self.inp.bit_position()
    }

    /// Gives back the bit reader, for example to read what follows the EXI body.
    pub fn into_reader(self) -> BitReader<R> {
        self.inp
    }

    /// Returns the next event, or `None` after `EndDocument`.
    ///
    /// Inlined, so the event is built where the caller uses it instead of being
    /// copied out through memory.
    #[inline]
    pub fn next_event(&mut self) -> ExiResult<Option<Event<'_>>> {
        let step = self.step()?;
        Ok(self.materialise(step))
    }

    #[inline]
    fn materialise(&self, step: Step) -> Option<Event<'_>> {
        let qn = |q: QNameId| {
            let (uri, local) = self.strings.qname(&self.tables, q).unwrap_or(("", ""));
            QName::new(uri, local)
        };
        Some(match step {
            Step::Sd => Event::StartDocument,
            Step::Ed => Event::EndDocument,
            Step::Se(q) => Event::StartElement(qn(q)),
            Step::Ee => Event::EndElement,
            Step::At(q) => Event::Attribute(qn(q), self.value.as_value()),
            Step::XsiType(q) => Event::XsiType(qn(q)),
            Step::XsiNil(b) => Event::XsiNil(b),
            Step::Ch => Event::Characters(self.value.as_value()),
            Step::Done => return None,
        })
    }

    /// Reads one event. `SD` and `ED` take no bits (each is the only choice where
    /// it occurs); everything else starts with an event code.
    fn step(&mut self) -> ExiResult<Step> {
        match self.cursor {
            Cursor::Document => {
                self.cursor = Cursor::DocContent;
                return Ok(Step::Sd);
            }
            Cursor::DocEnd => {
                self.cursor = Cursor::Done;
                return Ok(Step::Ed);
            }
            Cursor::Done => return Ok(Step::Done),
            Cursor::DocContent => {}
        }
        if self.frames.is_empty() {
            // DocContent : SE(G_0) ... SE(G_n-1) SE(*)  (EXI 8.5.1)
            // A code of n or more is SE(*), which would need a built-in grammar.
            let globals = self.tables.global_elements();
            let width = ceil_log2(globals.len() + 1);
            let code = self.inp.read_nbit_unsigned(width)? as usize;
            if code >= globals.len() {
                return Err(ExiError::Unsupported(Feature::BuiltInElementGrammar));
            }
            let elem = globals[code];
            let qname = self.push_element(elem)?;
            return Ok(Step::Se(qname));
        }
        // Only the fields needed here are read from the top frame. Copying the
        // whole `Frame` out and reading parts of it back stalls the CPU.
        let (rule, context, escapes) = match self.frames.last() {
            Some(f) => (f.rule, f.context, f.escape_count() > 0),
            None => return Err(ExiError::Internal("empty grammar stack")),
        };
        let prods = self.tables.rule_productions(rule);
        let width = ceil_log2(prods.len() + usize::from(escapes));
        let code = self.inp.read_nbit_unsigned(width)? as usize;
        if code >= prods.len() {
            if code == prods.len() && escapes {
                let frame = self.top()?;
                return self.step_escape(frame);
            }
            return Err(ExiError::Grammar {
                expected: format!("event code < {}", prods.len()),
                got: format!("{code} at bit {}", self.inp.bit_position()),
            });
        }
        let prod = prods[code];
        match prod.terminal {
            Terminal::Se(e) => {
                self.top_mut()?.rule = prod.next;
                let qname = self.push_element(e)?;
                Ok(Step::Se(qname))
            }
            Terminal::At(q, type_idx) => {
                let kind = self.tables.datatype(type_idx);
                self.last_type = Some(type_idx);
                self.read_value_in_place(kind, q)?;
                self.top_mut()?.rule = prod.next;
                Ok(Step::At(q))
            }
            Terminal::Ch(type_idx) => {
                let kind = self.tables.datatype(type_idx);
                self.last_type = Some(type_idx);
                self.read_value_in_place(kind, context)?;
                self.top_mut()?.rule = prod.next;
                Ok(Step::Ch)
            }
            Terminal::ChUntyped => {
                self.last_type = Some(NONE);
                self.read_value_in_place(DatatypeKind::Untyped, context)?;
                self.top_mut()?.rule = prod.next;
                Ok(Step::Ch)
            }
            Terminal::Ee => {
                self.frames.pop();
                if self.frames.is_empty() {
                    self.cursor = Cursor::DocEnd;
                }
                Ok(Step::Ee)
            }
            Terminal::SeAny
            | Terminal::SeNs(_)
            | Terminal::AtAny
            | Terminal::AtNs(_)
            | Terminal::AtAnyUntyped => Err(ExiError::Unsupported(Feature::Wildcard)),
            other => Err(ExiError::Grammar {
                expected: "a strict schema-informed production".into(),
                got: format!("{other:?}"),
            }),
        }
    }

    /// Reads the second-level code after an escape, then the `xsi:type` QName
    /// ([EXI 7.1.7](https://www.w3.org/TR/exi/#encodingQName)) or the `xsi:nil`
    /// Boolean, and switches grammars the same way as the encoder does.
    fn step_escape(&mut self, frame: Frame) -> ExiResult<Step> {
        let code2 = self.inp.read_nbit_unsigned(frame.second_level_width())? as u32;
        if Some(code2) == frame.xsi_type_code() {
            let qname = self.strings.read_qname(&self.tables, &mut self.inp)?;
            let Some(type_idx) = self.tables.find_type(qname) else {
                let (uri, local) = self.strings.qname(&self.tables, qname).unwrap_or(("", ""));
                return Err(ExiError::Schema(format!(
                    "xsi:type names unknown type {}",
                    QName::new(uri, local)
                )));
            };
            let Some(decl) = self.tables.type_decl(type_idx) else {
                return Err(ExiError::Internal("dangling type index"));
            };
            let Some(start) = self.tables.grammar_start_rule(decl.grammar) else {
                return Err(ExiError::Internal("dangling grammar index"));
            };
            let top = self.top_mut()?;
            top.rule = start;
            top.start_rule = start;
            top.type_idx = type_idx;
            // No escapes after a cast: see `Encoder::xsi_type`.
            top.typecastable = false;
            top.nillable = false;
            Ok(Step::XsiType(qname))
        } else if Some(code2) == frame.xsi_nil_code() {
            let nil = self.inp.read_bool()?;
            if nil {
                let Some(decl) = self.tables.type_decl(frame.type_idx) else {
                    return Err(ExiError::Internal("dangling type index"));
                };
                let Some(start) = self.tables.grammar_start_rule(decl.empty_grammar) else {
                    return Err(ExiError::Internal("dangling empty grammar index"));
                };
                let top = self.top_mut()?;
                top.rule = start;
                top.nil = true;
            }
            Ok(Step::XsiNil(nil))
        } else {
            Err(ExiError::Grammar {
                expected: "xsi:type or xsi:nil escape".into(),
                got: format!("second-level code {code2}"),
            })
        }
    }

    /// Reads one value of `kind` into `self.value`. A string goes straight into
    /// the string already there, keeping its capacity, with no copy of
    /// the value afterwards.
    fn read_value_in_place(&mut self, kind: DatatypeKind, context: QNameId) -> ExiResult<()> {
        if kind.is_string_like() {
            if !matches!(self.value, OwnedValue::String(_)) {
                let s = OwnedValue::String(core::mem::take(&mut self.text));
                self.keep_buffer(s);
            }
            if let OwnedValue::String(buf) = &mut self.value {
                self.strings.read_value(&mut self.inp, context, buf)?;
                let n = buf.len();
                self.count_string_bytes(n)?;
            }
            return Ok(());
        }
        if matches!(kind, DatatypeKind::Base64Binary | DatatypeKind::HexBinary) {
            if !matches!(self.value, OwnedValue::Binary(_)) {
                let b = OwnedValue::Binary(core::mem::take(&mut self.bytes));
                self.keep_buffer(b);
            }
            if let OwnedValue::Binary(buf) = &mut self.value {
                binary::read_binary_into(&mut self.inp, buf)?;
            }
            return Ok(());
        }
        let v = self.read_value(kind, context)?;
        self.keep_buffer(v);
        Ok(())
    }

    /// Puts `v` in `self.value`, keeping the old value's string or byte storage
    /// as a spare for later values.
    fn keep_buffer(&mut self, v: OwnedValue) {
        match core::mem::replace(&mut self.value, v) {
            OwnedValue::String(s) => self.text = s,
            OwnedValue::Binary(b) => self.bytes = b,
            _ => {}
        }
    }

    /// Reads one value in the datatype `kind`; the reverse of `Encoder::write_value`.
    fn read_value(&mut self, kind: DatatypeKind, context: QNameId) -> ExiResult<OwnedValue> {
        match kind {
            // Refused before the string table is touched: see
            // `DatatypeKind::RestrictedString` (EXI 7.1.10.1).
            DatatypeKind::RestrictedString { .. } => {
                Err(ExiError::Unsupported(Feature::RestrictedCharacterSet))
            }
            k if k.is_string_like() => {
                // Read into the last string value's buffer, keeping its capacity,
                // instead of growing a new String each time.
                let mut buf = match core::mem::replace(&mut self.value, OwnedValue::Boolean(false))
                {
                    OwnedValue::String(s) => s,
                    _ => core::mem::take(&mut self.text),
                };
                self.strings.read_value(&mut self.inp, context, &mut buf)?;
                self.count_string_bytes(buf.len())?;
                Ok(OwnedValue::String(buf))
            }
            DatatypeKind::List { item } => {
                let item_kind = self.tables.datatype(item);
                // A list is its item count, then the items (EXI 7.1.11). The up-front
                // allocation is capped so a bad count cannot ask for a huge buffer.
                let count = read_unsigned(&mut self.inp)?;
                if let Some(max) = self.limits.max_list_items
                    && count > max
                {
                    return Err(ExiError::LimitExceeded {
                        limit: Limit::ListItems,
                        max,
                    });
                }
                let mut items = Vec::with_capacity(count.min(1024) as usize);
                for _ in 0..count {
                    items.push(self.read_value(item_kind, context)?);
                }
                Ok(OwnedValue::List(items))
            }
            DatatypeKind::QName => Err(ExiError::Internal("QName values are decoded by the codec")),
            _ => datatype::read_scalar(&mut self.inp, kind, &self.tables),
        }
    }

    /// Adds `n` bytes of a decoded string value to the running total, and fails
    /// once the total is over [`DecodeLimits::max_string_bytes`]. A string-table
    /// hit costs only a few bits in the stream but can stand for a long string,
    /// so this total is what stops memory from blowing up
    /// ([EXI 7.3.3](https://www.w3.org/TR/exi/#encodingOptimizedForMisses)).
    #[inline]
    fn count_string_bytes(&mut self, n: usize) -> ExiResult<()> {
        self.string_bytes = self.string_bytes.saturating_add(n as u64);
        match self.limits.max_string_bytes {
            Some(max) if self.string_bytes > max => Err(ExiError::LimitExceeded {
                limit: Limit::StringBytes,
                max,
            }),
            _ => Ok(()),
        }
    }

    /// Opens a frame for element `elem`, starting at the first rule of its type's
    /// grammar, and returns the element's qname. Fails when one more open element
    /// would be over [`DecodeLimits::max_depth`].
    fn push_element(&mut self, elem: ElemIdx) -> ExiResult<QNameId> {
        if let Some(max) = self.limits.max_depth
            && self.frames.len() >= max as usize
        {
            return Err(ExiError::LimitExceeded {
                limit: Limit::Depth,
                max: u64::from(max),
            });
        }
        let Some(decl) = self.tables.element(elem) else {
            return Err(ExiError::Internal("dangling element index"));
        };
        let Some(ty) = self.tables.type_decl(decl.type_idx) else {
            return Err(ExiError::Internal("dangling type index"));
        };
        let Some(start) = self.tables.grammar_start_rule(ty.grammar) else {
            return Err(ExiError::Internal("dangling grammar index"));
        };
        self.frames.push(Frame {
            rule: start,
            start_rule: start,
            context: decl.qname,
            elem,
            type_idx: decl.type_idx,
            nillable: decl.nillable,
            typecastable: ty.typecastable,
            nil: false,
            space_preserve: None,
        });
        Ok(decl.qname)
    }

    fn top(&self) -> ExiResult<Frame> {
        self.frames
            .last()
            .copied()
            .ok_or(ExiError::Internal("empty grammar stack"))
    }

    fn top_mut(&mut self) -> ExiResult<&mut Frame> {
        self.frames
            .last_mut()
            .ok_or(ExiError::Internal("empty grammar stack"))
    }

    /// The datatype of the value carried by the most recent `Attribute` or
    /// `Characters` event.
    pub fn last_datatype(&self) -> Option<DatatypeKind> {
        self.last_type.map(|t| self.tables.datatype(t))
    }

    /// Formats a value in the standard text form (the XML Schema "canonical lexical"
    /// form) of `kind`.
    pub fn format(&self, kind: DatatypeKind, value: &crate::value::Value<'_>) -> ExiResult<String> {
        lexical::format_value(kind, value, &self.tables)
    }

    /// Formats the value of the most recent `Attribute`/`Characters` event, using
    /// its datatype (untyped if there was none yet).
    pub fn format_last(&self, value: &crate::value::Value<'_>) -> ExiResult<String> {
        let kind = self.last_datatype().unwrap_or(DatatypeKind::Untyped);
        lexical::format_value(kind, value, &self.tables)
    }
}
