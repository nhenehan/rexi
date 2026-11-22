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
use crate::grammar::runtime::{Cursor, FragmentEntry, Frame, fragment_entries};
use crate::header::{self, HeaderInfo};
use crate::ids::{ElemIdx, QNameId, TypeIdx};
use crate::io::{BitReader, ceil_log2};
use crate::limits::{DecodeLimits, Limit};
use crate::options::ExiOptions;
use crate::string_table::StringTable;
use crate::value::OwnedValue;
#[cfg(feature = "lexical-values")]
use datatype::lexical_values::{XSI_NIL, XSI_TYPE};

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
pub struct Decoder<R, G> {
    inp: BitReader<R>,
    tables: G,
    options: ExiOptions,
    header: HeaderInfo,
    strings: StringTable,
    frames: Vec<Frame>,
    cursor: Cursor,
    /// The `SE(F_i)` list of `FragmentContent` when the body is a fragment
    /// ([EXI 8.5.2](https://www.w3.org/TR/exi/#informedFragGrammars)). Built only
    /// for fragments, so documents allocate nothing extra.
    fragment: Option<Vec<FragmentEntry>>,
    value: OwnedValue,
    /// The type of the last value read, as a type index (`TypeIdx::NONE` for untyped
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

/// Shows the options and how far decoding has got, not the tables or buffers.
impl<R: ByteSource, G: GrammarTables> core::fmt::Debug for Decoder<R, G> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Decoder")
            .field("options", &self.options)
            .field("bit_position", &self.bit_position())
            .field("depth", &self.frames.len())
            .finish_non_exhaustive()
    }
}

impl<R: ByteSource, G: GrammarTables> Decoder<R, G> {
    /// Reads the EXI header ([EXI 5](https://www.w3.org/TR/exi/#header)). Options
    /// found in the stream win over `out_of_band`. If the stream has none,
    /// `out_of_band` must be given. The options' `schemaId` must name `tables`
    /// (see [`ExiError::SchemaIdMismatch`] for the rule).
    ///
    /// Uses the safe [`DecodeLimits::default`]. Call [`Decoder::with_limits`] to
    /// pick others.
    ///
    /// # Errors
    ///
    /// Fails when the header is bad or cut short ([`ExiError::Header`],
    /// [`ExiError::Truncated`]), when the stream carries no options and `out_of_band`
    /// is `None`, when the options ask for something this codec does not do
    /// ([`ExiError::Unsupported`]), or when the stream's `schemaId` does not name
    /// `tables` ([`ExiError::SchemaIdMismatch`]).
    pub fn new(input: R, tables: G, out_of_band: Option<ExiOptions>) -> ExiResult<Self> {
        Self::with_limits(input, tables, out_of_band, DecodeLimits::default())
    }

    /// Like [`Decoder::new`], with the given limits on what the stream may ask
    /// for. See [`DecodeLimits`].
    ///
    /// # Errors
    ///
    /// The same as [`Decoder::new`].
    ///
    /// # Examples
    ///
    /// Decodes with tighter limits than the defaults, for input from a network.
    ///
    /// ```
    /// use rexi_core::grammar::options_grammar::OPTIONS_GRAMMARS;
    /// use rexi_core::{DecodeLimits, Decoder, Encoder, ExiOptions, HeaderConfig};
    ///
    /// # const EXI: &str = "http://www.w3.org/2009/exi";
    /// # let mut enc = Encoder::new(Vec::new(), &OPTIONS_GRAMMARS, ExiOptions::strict(), HeaderConfig::default())?;
    /// # enc.start_document()?;
    /// # enc.start_element(EXI, "header")?;
    /// # enc.end_element()?;
    /// # enc.end_document()?;
    /// # let bytes = enc.finish()?;
    /// let limits = DecodeLimits::default()
    ///     .with_max_depth(Some(16))
    ///     .with_max_string_bytes(Some(64 * 1024));
    /// let mut dec = Decoder::with_limits(&bytes[..], &OPTIONS_GRAMMARS, Some(ExiOptions::strict()), limits)?;
    /// let mut events = 0;
    /// while dec.next_event()?.is_some() {
    ///     events += 1;
    /// }
    /// assert_eq!(events, 4); // SD, SE(header), EE, ED
    /// # Ok::<(), rexi_core::ExiError>(())
    /// ```
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
            out_of_band.ok_or_else(|| {
                ExiError::Header("stream carries no options and none were given out of band")
            })?
        };
        options.validate()?;
        header::check_schema_id(&options.schema_id, tables.schema_id())?;
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
        let fragment = options.fragment.then(|| fragment_entries(&tables));
        Ok(Self {
            inp: reader,
            tables,
            options,
            header: HeaderInfo::default(),
            strings,
            frames: Vec::new(),
            cursor: Cursor::Document,
            fragment,
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

    /// Number of bits read so far, header included (if there was one).
    pub fn bit_position(&self) -> u64 {
        self.inp.bit_position()
    }

    /// Gives back the bit reader, for example to read what follows the EXI body.
    pub(crate) fn into_reader(self) -> BitReader<R> {
        self.inp
    }

    /// Returns the next event, or `None` after `EndDocument`.
    ///
    /// Inlined, so the event is built where the caller uses it instead of being
    /// copied out through memory.
    ///
    /// # Errors
    ///
    /// Fails when the stream does not follow the grammar ([`ExiError::Grammar`]),
    /// holds a value its datatype does not allow ([`ExiError::Value`]), ends too
    /// soon ([`ExiError::Truncated`]), needs a feature this codec does not do
    /// ([`ExiError::Unsupported`]), or goes over one of the [`DecodeLimits`]
    /// ([`ExiError::LimitExceeded`]).
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
            Step::At(q) => Event::Attribute(qn(q), self.value.to_value()),
            Step::XsiType(q) => Event::XsiType(qn(q)),
            Step::XsiNil(b) => Event::XsiNil(b),
            Step::Ch => Event::Characters(self.value.to_value()),
            Step::Done => return None,
        })
    }

    /// Reads one event. `SD` and `ED` take no bits (each is the only choice where
    /// it occurs); everything else starts with an event code.
    ///
    /// Inlined into its one caller, [`Decoder::next_event`]. The result is too big
    /// to come back in registers, so without inlining it goes through memory on
    /// every event, which costs about 5 % of decode time.
    #[inline]
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
            if let Some(entries) = &self.fragment {
                return self.step_fragment_content(entries.len());
            }
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
                self.last_type = Some(TypeIdx::NONE);
                self.read_value_in_place(DatatypeKind::Untyped, context)?;
                self.top_mut()?.rule = prod.next;
                Ok(Step::Ch)
            }
            Terminal::Ee => {
                self.frames.pop();
                // A fragment goes back to FragmentContent for the next element.
                if self.frames.is_empty() && self.fragment.is_none() {
                    self.cursor = Cursor::DocEnd;
                }
                Ok(Step::Ee)
            }
            #[cfg(feature = "wildcard")]
            Terminal::SeAny | Terminal::SeNs(_) => self.wildcard_element(prod),
            #[cfg(feature = "wildcard")]
            Terminal::AtAny | Terminal::AtNs(_) => self.wildcard_attribute(prod),
            #[cfg(not(feature = "wildcard"))]
            Terminal::SeAny | Terminal::SeNs(_) | Terminal::AtAny | Terminal::AtNs(_) => {
                Err(ExiError::Unsupported(Feature::Wildcard))
            }
            // AT(*) with an untyped value appears only in non-strict grammars.
            Terminal::AtAnyUntyped => Err(ExiError::Unsupported(Feature::Wildcard)),
            other => Err(ExiError::Grammar {
                expected: "a strict schema-informed production".into(),
                got: format!("{other:?}"),
            }),
        }
    }

    /// Reads the name after a wildcard production: the whole QName after `SE(*)` /
    /// `AT(*)`, only the local name after `SE(uri:*)` / `AT(uri:*)`
    /// ([EXI 7.1.7](https://www.w3.org/TR/exi/#encodingQName)).
    #[cfg(feature = "wildcard")]
    fn read_wildcard_name(&mut self, terminal: Terminal) -> ExiResult<QNameId> {
        match terminal {
            Terminal::SeNs(u) | Terminal::AtNs(u) => {
                let l = self
                    .strings
                    .read_local_name(&self.tables, &mut self.inp, u)?;
                Ok(QNameId::new(u, l))
            }
            _ => self.strings.read_qname(&self.tables, &mut self.inp),
        }
    }

    /// `SE(*)` or `SE(uri:*)` ([EXI 8.5.4.1.6](https://www.w3.org/TR/exi/#elementTerms)):
    /// the name, then the element's global declaration's grammar, as EXIficient
    /// does. A name with no global declaration would need a built-in grammar.
    #[cfg(feature = "wildcard")]
    fn wildcard_element(&mut self, prod: crate::grammar::ir::Production) -> ExiResult<Step> {
        let qname = self.read_wildcard_name(prod.terminal)?;
        let Some(elem) = self.tables.find_global_element(qname) else {
            return Err(ExiError::Unsupported(Feature::BuiltInElementGrammar));
        };
        self.top_mut()?.rule = prod.next;
        self.push_element(elem)?;
        Ok(Step::Se(qname))
    }

    /// `AT(*)` or `AT(uri:*)` ([EXI 8.5.4.1.6](https://www.w3.org/TR/exi/#elementTerms)):
    /// the name, then the value in the type of the attribute's global declaration,
    /// or as a string when there is none, as EXIficient does.
    #[cfg(feature = "wildcard")]
    fn wildcard_attribute(&mut self, prod: crate::grammar::ir::Production) -> ExiResult<Step> {
        let frame = self.top()?;
        let qname = self.read_wildcard_name(prod.terminal)?;
        let global = self
            .tables
            .global_attributes()
            .iter()
            .find(|a| a.qname == qname)
            .map(|a| a.type_idx);
        let kind = global.map_or(DatatypeKind::Untyped, |t| self.tables.datatype(t));
        self.last_type = Some(global.unwrap_or(TypeIdx::NONE));
        self.read_value_in_place(kind, qname)?;
        self.top_mut()?.rule = frame.after_attribute_wildcard(prod.next);
        Ok(Step::At(qname))
    }

    /// Reads the second-level code after an escape, then the `xsi:type` QName
    /// ([EXI 7.1.7](https://www.w3.org/TR/exi/#encodingQName)) or the `xsi:nil`
    /// Boolean, and switches grammars the same way as the encoder does.
    fn step_escape(&mut self, frame: Frame) -> ExiResult<Step> {
        let code2 = self.inp.read_nbit_unsigned(frame.second_level_width())? as u32;
        if Some(code2) == frame.xsi_type_code() {
            #[cfg(feature = "lexical-values")]
            let qname = if self.options.preserve.lexical_values {
                self.read_xsi_type_text()?
            } else {
                self.strings.read_qname(&self.tables, &mut self.inp)?
            };
            #[cfg(not(feature = "lexical-values"))]
            let qname = self.strings.read_qname(&self.tables, &mut self.inp)?;
            let Some(type_idx) = self.tables.find_type(qname) else {
                // No type of that name: keep the grammar, as the encoder does.
                return Ok(Step::XsiType(qname));
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
            #[cfg(feature = "lexical-values")]
            let nil = if self.options.preserve.lexical_values {
                let text = self.read_text(XSI_NIL, Some(datatype::lexical_values::NIL_SET))?;
                let nil = datatype::boolean::parse_boolean(&text);
                self.text = text;
                nil.ok_or_else(|| ExiError::value("xsi:nil", "not a boolean"))?
            } else {
                self.inp.read_bool()?
            };
            #[cfg(not(feature = "lexical-values"))]
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

    /// Reads an `xsi:type` written as text (Preserve.lexicalValues; see
    /// `Encoder::xsi_type_lexical_name`): a name with no prefix, in no namespace.
    #[cfg(feature = "lexical-values")]
    fn read_xsi_type_text(&mut self) -> ExiResult<QNameId> {
        let text = self.read_text(XSI_TYPE, None)?;
        let found = if text.contains(':') {
            // A prefix, and no way to know its namespace in strict mode.
            Err(ExiError::Unsupported(Feature::PreservePrefixes))
        } else {
            self.strings
                .qname_id(&self.tables, "", &text)
                .ok_or_else(|| {
                    ExiError::value("xsi:type", format!("{text} is not a name the schema uses"))
                })
        };
        self.text = text;
        found
    }

    /// Reads one value of `kind` into `self.value`. A string goes straight into
    /// the string already there, keeping its capacity, with no copy of
    /// the value afterwards.
    fn read_value_in_place(&mut self, kind: DatatypeKind, context: QNameId) -> ExiResult<()> {
        #[cfg(feature = "lexical-values")]
        if self.options.preserve.lexical_values {
            let v = self.read_value(kind, context)?;
            self.keep_buffer(v);
            return Ok(());
        }
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
        #[cfg(feature = "lexical-values")]
        if self.options.preserve.lexical_values {
            let alphabet = datatype::lexical_values::alphabet(kind, &self.tables);
            return self.read_text(context, alphabet).map(OwnedValue::String);
        }
        match kind {
            // EXI 7.1.10.1. With no alphabet (a pattern that could not be read),
            // refused before the string table is touched.
            DatatypeKind::RestrictedString { alphabet, .. } => {
                let alphabet = alphabet
                    .ok_or_else(|| ExiError::Unsupported(Feature::RestrictedCharacterSet))?;
                let mut buf = match core::mem::replace(&mut self.value, OwnedValue::Boolean(false))
                {
                    OwnedValue::String(s) => s,
                    _ => core::mem::take(&mut self.text),
                };
                self.strings.read_value_restricted(
                    &mut self.inp,
                    context,
                    &mut buf,
                    self.tables.string(alphabet),
                )?;
                self.count_string_bytes(buf.len())?;
                Ok(OwnedValue::String(buf))
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

    /// Reads a String value under `context`, with the restricted character set
    /// `alphabet` if there is one, into the spare string buffer: the reverse of
    /// `Encoder::write_lexical_value`.
    #[cfg(feature = "lexical-values")]
    fn read_text(&mut self, context: QNameId, alphabet: Option<&str>) -> ExiResult<String> {
        let mut buf = match core::mem::replace(&mut self.value, OwnedValue::Boolean(false)) {
            OwnedValue::String(s) => s,
            _ => core::mem::take(&mut self.text),
        };
        match alphabet {
            Some(a) => self
                .strings
                .read_value_restricted(&mut self.inp, context, &mut buf, a)?,
            None => self.strings.read_value(&mut self.inp, context, &mut buf)?,
        }
        self.count_string_bytes(buf.len())?;
        Ok(buf)
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
    /// Reads one code of a fragment's top level:
    /// `FragmentContent : SE(F_0) ... SE(F_n-1) SE(*) ED`
    /// ([EXI 8.5.2](https://www.w3.org/TR/exi/#informedFragGrammars)), so codes
    /// `0..n` start an element, `n` is `SE(*)` and `n + 1` is `ED`.
    #[cold]
    fn step_fragment_content(&mut self, n: usize) -> ExiResult<Step> {
        let width = ceil_log2(n + 2);
        let code = self.inp.read_nbit_unsigned(width)? as usize;
        if code == n + 1 {
            self.cursor = Cursor::Done;
            return Ok(Step::Ed);
        }
        if code == n {
            return Err(ExiError::Unsupported(Feature::BuiltInElementGrammar));
        }
        let elem = match self.fragment.as_ref().and_then(|f| f.get(code)) {
            Some(e) => e.elem,
            None => {
                return Err(ExiError::Grammar {
                    expected: format!("fragment event code <= {}", n + 1),
                    got: format!("{code} at bit {}", self.inp.bit_position()),
                });
            }
        };
        if elem.is_none() {
            return Err(ExiError::Unsupported(Feature::RelaxedElementGrammar));
        }
        let qname = self.push_element(elem)?;
        Ok(Step::Se(qname))
    }

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
            .ok_or_else(|| ExiError::Internal("empty grammar stack"))
    }

    fn top_mut(&mut self) -> ExiResult<&mut Frame> {
        self.frames
            .last_mut()
            .ok_or_else(|| ExiError::Internal("empty grammar stack"))
    }

    /// The datatype of the value carried by the most recent `Attribute` or
    /// `Characters` event.
    pub fn last_datatype(&self) -> Option<DatatypeKind> {
        self.last_type.map(|t| self.tables.datatype(t))
    }

    /// Formats a value in the standard text form (the XML Schema "canonical lexical"
    /// form) of `kind`.
    ///
    /// # Errors
    ///
    /// Fails with [`ExiError::Value`] when `value` is not a value of `kind`.
    pub fn format(&self, kind: DatatypeKind, value: &crate::value::Value<'_>) -> ExiResult<String> {
        lexical::format_value(kind, value, &self.tables)
    }

    /// Formats the value of the most recent `Attribute`/`Characters` event, using
    /// its datatype (untyped if there was none yet).
    ///
    /// # Errors
    ///
    /// Fails with [`ExiError::Value`] when `value` is not a value of that datatype.
    pub fn format_last(&self, value: &crate::value::Value<'_>) -> ExiResult<String> {
        let kind = self.last_datatype().unwrap_or(DatatypeKind::Untyped);
        lexical::format_value(kind, value, &self.tables)
    }
}
