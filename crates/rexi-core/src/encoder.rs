//! The SAX-like EXI encoder: push events in, bits come out.
//!
//! Events must arrive in grammar order: attributes sorted by local-name then URI
//! ([EXI 8.5.4.3](https://www.w3.org/TR/exi/#eventCodeAssignment)), `xsi:type`
//! before `xsi:nil`, and both right after `StartElement`
//! ([EXI 6](https://www.w3.org/TR/exi/#encodingEvents)).
//!
//! For each event, the encoder finds the matching production in the current rule
//! of the innermost open element. It writes that production's position as the
//! event code, writes the event's content (a value or a qname), then moves to the
//! production's next rule ([EXI 6](https://www.w3.org/TR/exi/#encodingEvents)). An
//! `SE` also pushes a new frame for the child element, and an `EE` pops one. If no
//! production matches, the event is refused. If only a wildcard (`SE(*)`, `AT(*)`)
//! would match, the event is refused as an unsupported feature, since this codec
//! has no built-in or non-strict grammars.
//!
//! Character data is held in a buffer until the next structural event. That way
//! text that arrives in pieces is joined, and text that is only whitespace can be
//! dropped where EXIficient drops it: in element-only content, and, where the
//! character data is untyped (mixed content), everywhere except right between a
//! start tag and its end tag, unless `xml:space="preserve"` is in effect (see
//! `flush_chars`).

use crate::io::ByteSink;
use crate::prelude::*;

use crate::datatype::unsigned::write_unsigned;
use crate::datatype::{self, DatatypeKind, TypeResolver, lexical};
use crate::error::{ExiError, ExiResult, Feature};
use crate::event::Event;
use crate::grammar::ir::{GrammarTables, Production, Terminal};
use crate::grammar::runtime::{Cursor, Frame, describe_rule, find_production};
use crate::header;
use crate::ids::QNameId;
use crate::io::{BitWriter, ceil_log2};
use crate::options::{ExiOptions, HeaderConfig};
use crate::string_table::StringTable;
use crate::value::Value;

/// One event code that was written, as recorded with the `trace` feature.
#[cfg(feature = "trace")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEntry {
    /// Bit position of the stream where the event code starts.
    pub bit_offset: u64,
    /// The event, as its spec abbreviation (`SE`, `AT`, `CH`, ...).
    pub what: String,
    /// Width of the event code in bits (0 when the rule has only one production).
    pub width: u8,
    /// The event code that was written.
    pub code: u64,
}

/// Encodes a stream of [`Event`]s into EXI, using the grammar tables `G` and
/// writing to the byte sink `W`.
pub struct Encoder<W: ByteSink, G: GrammarTables> {
    out: BitWriter<W>,
    tables: G,
    options: ExiOptions,
    strings: StringTable,
    frames: Vec<Frame>,
    cursor: Cursor,
    pending_chars: String,
    has_pending_chars: bool,
    /// No `CH` or `EE` since the last start tag (its attributes do not count).
    after_start_tag: bool,
    #[cfg(feature = "trace")]
    trace: Vec<TraceEntry>,
}

impl<W: ByteSink, G: GrammarTables> Encoder<W, G> {
    /// Creates an encoder and writes the EXI header
    /// ([EXI 5](https://www.w3.org/TR/exi/#header)) right away.
    pub fn new(out: W, tables: G, options: ExiOptions, header: HeaderConfig) -> ExiResult<Self> {
        options.validate()?;
        let mut writer = BitWriter::new(out);
        header::write_header_bits(&mut writer, header)?;
        if header.include_options {
            writer = header::write_options_document(writer, &options)?;
        }
        if options.byte_aligned() {
            writer.set_byte_aligned(true)?;
        }
        Self::for_body(writer, tables, options)
    }

    /// Creates an encoder that writes a bare EXI body (no header) into `writer`.
    ///
    /// Used for the options document, and for callers that write the header
    /// themselves.
    #[doc(hidden)]
    pub fn for_body(writer: BitWriter<W>, tables: G, options: ExiOptions) -> ExiResult<Self> {
        options.validate()?;
        let strings = init_string_table(&tables, &options);
        Ok(Self {
            out: writer,
            tables,
            options,
            strings,
            frames: Vec::new(),
            cursor: Cursor::Document,
            pending_chars: String::new(),
            has_pending_chars: false,
            after_start_tag: false,
            #[cfg(feature = "trace")]
            trace: Vec::new(),
        })
    }

    /// The options the stream is encoded with.
    pub fn options(&self) -> &ExiOptions {
        &self.options
    }

    /// Number of bits written so far, header included (if there was one).
    pub fn bit_position(&self) -> u64 {
        self.out.bit_position()
    }

    /// The grammar tables this encoder was built with.
    pub fn tables(&self) -> &G {
        &self.tables
    }

    /// Productions of the innermost open element's current rule, in event-code
    /// order; empty at document level. Front-ends use this to resolve names and
    /// value datatypes against the grammar before pushing an event. Buffered
    /// character data (from `Characters` with a string value) does not show up here
    /// until the next non-character event.
    pub fn current_productions(&self) -> &[Production] {
        match self.frames.last() {
            Some(f) => self.tables.rule_productions(f.rule),
            None => &[],
        }
    }

    /// Every event code written so far (only with the `trace` feature).
    #[cfg(feature = "trace")]
    pub fn trace(&self) -> &[TraceEntry] {
        &self.trace
    }

    /// Finishes the stream: pads the last byte and returns the inner writer. Fails
    /// if `EndDocument` has not been pushed yet.
    pub fn finish(self) -> ExiResult<W> {
        if self.cursor != Cursor::Done {
            return Err(ExiError::Grammar {
                expected: "ED before finish".into(),
                got: "incomplete document".into(),
            });
        }
        self.out.finish()
    }

    /// Returns the bit writer without finishing (for putting a body inside a larger
    /// stream).
    pub fn into_writer(self) -> BitWriter<W> {
        self.out
    }

    /// Encodes one event. Events that strict mode does not allow (namespace
    /// declarations, comments, PIs, DOCTYPE, entity references, self-contained) are
    /// refused as unsupported features.
    pub fn push(&mut self, ev: &Event<'_>) -> ExiResult<()> {
        match ev {
            Event::StartDocument => self.start_document(),
            Event::EndDocument => self.end_document(),
            Event::StartElement(q) => self.start_element(q.uri, q.local),
            Event::EndElement => self.end_element(),
            Event::Attribute(q, v) => self.attribute(q.uri, q.local, v),
            Event::XsiType(q) => self.xsi_type(q.uri, q.local),
            Event::XsiNil(b) => self.xsi_nil(*b),
            Event::Characters(v) => self.characters(v),
            Event::NamespaceDecl { .. } => Err(ExiError::Unsupported(Feature::PreservePrefixes)),
            Event::Comment(_) => Err(ExiError::Unsupported(Feature::PreserveComments)),
            Event::ProcessingInstruction { .. } => Err(ExiError::Unsupported(Feature::PreservePis)),
            Event::DocType { .. } => Err(ExiError::Unsupported(Feature::PreserveDtd)),
            Event::EntityReference(_) => Err(ExiError::Unsupported(Feature::EntityReference)),
            Event::SelfContained => Err(ExiError::Unsupported(Feature::SelfContained)),
        }
    }

    /// Starts the document (`SD`). Must be the first event.
    pub fn start_document(&mut self) -> ExiResult<()> {
        if self.cursor != Cursor::Document {
            return Err(self.mismatch("SD"));
        }
        // Document : SD DocContent. There is only one production, so its event code
        // takes zero bits (EXI 6.2).
        self.record("SD", 0, 0);
        self.cursor = Cursor::DocContent;
        Ok(())
    }

    /// Ends the document (`ED`) after the root element has ended. Call
    /// [`Encoder::finish`] afterwards to write the last partial byte.
    pub fn end_document(&mut self) -> ExiResult<()> {
        self.flush_chars(false)?;
        if self.cursor != Cursor::DocEnd {
            return Err(self.mismatch("ED"));
        }
        self.record("ED", 0, 0);
        self.cursor = Cursor::Done;
        Ok(())
    }

    /// Starts element `{uri}local` (`SE`). The name is looked up in the tables; a name
    /// the schema does not allow at this point is refused.
    pub fn start_element(&mut self, uri: &str, local: &str) -> ExiResult<()> {
        let qname = self.tables.find_qname(uri, local);
        self.start_element_resolved(qname, Some((uri, local)))
    }

    /// [`Encoder::start_element`] for a qname already looked up in this encoder's
    /// tables (for example a generated `LN_*` constant), so no lookup is needed.
    pub fn start_element_id(&mut self, qname: QNameId) -> ExiResult<()> {
        self.start_element_resolved(Some(qname), None)
    }

    /// The text of a qname, for error messages: the names given, or else the names
    /// from the tables.
    fn qname_text(&self, qname: Option<QNameId>, names: Option<(&str, &str)>) -> String {
        let (uri, local) = names.unwrap_or_else(|| {
            qname.map_or(("", "?"), |q| {
                (
                    self.tables.uri_str(q.uri).unwrap_or(""),
                    self.tables.local_name_str(q).unwrap_or("?"),
                )
            })
        });
        crate::event::QName::new(uri, local).to_string()
    }

    /// Writes the event code for `SE(qname)` and opens the child element. At
    /// document level the code comes from `DocContent`; inside an element it is the
    /// position of the matching `SE` production in the current rule.
    fn start_element_resolved(
        &mut self,
        qname: Option<QNameId>,
        names: Option<(&str, &str)>,
    ) -> ExiResult<()> {
        self.flush_chars(false)?;
        if self.frames.is_empty() {
            if self.cursor != Cursor::DocContent {
                return Err(self.mismatch("SE"));
            }
            // DocContent : SE(G_0) ... SE(G_n-1) SE(*)  (EXI 8.5.1)
            // The root's code is its place among the sorted global elements. A root
            // that is not a global element would need SE(*) and a built-in grammar.
            let globals = self.tables.global_elements();
            let width = ceil_log2(globals.len() + 1);
            let Some(qname) = qname else {
                return Err(ExiError::Unsupported(Feature::BuiltInElementGrammar));
            };
            let pos = globals
                .iter()
                .position(|&e| self.tables.element(e).is_some_and(|d| d.qname == qname));
            let Some(pos) = pos else {
                return Err(ExiError::Unsupported(Feature::BuiltInElementGrammar));
            };
            let elem = globals[pos];
            self.emit_code("SE(root)", pos as u64, width)?;
            self.push_element(elem, qname)?;
            return Ok(());
        }
        let frame = self.top()?;
        let found = qname.and_then(|q| {
            find_production(&self.tables, frame.rule, |t| match t {
                Terminal::Se(e) => self.tables.element(*e).is_some_and(|d| d.qname == q),
                _ => false,
            })
        });
        match found {
            Some((idx, prod)) => {
                let width = frame.first_level_width(&self.tables);
                self.emit_code("SE", idx as u64, width)?;
                self.top_mut()?.rule = prod.next;
                if let Terminal::Se(e) = prod.terminal
                    && let Some(q) = qname
                {
                    self.push_element(e, q)?;
                }
                Ok(())
            }
            None => {
                let wildcard = find_production(&self.tables, frame.rule, |t| {
                    matches!(t, Terminal::SeAny | Terminal::SeNs(_))
                });
                if wildcard.is_some() {
                    Err(ExiError::Unsupported(Feature::Wildcard))
                } else {
                    Err(self.mismatch(&format!("SE({})", self.qname_text(qname, names))))
                }
            }
        }
    }

    /// Encodes an attribute: the `AT(qname)` event code, then its value in the
    /// attribute's datatype. An `xml:space` attribute is also tracked for the
    /// whitespace rules in `flush_chars`.
    pub fn attribute(&mut self, uri: &str, local: &str, value: &Value<'_>) -> ExiResult<()> {
        let qname = self.tables.find_qname(uri, local);
        let xml_space = uri == crate::grammar::ir::well_known::XML_NS && local == "space";
        self.attribute_resolved(qname, Some((uri, local)), xml_space, value)
    }

    /// [`Encoder::attribute`] for a qname already looked up in this encoder's
    /// tables (for example a generated `LN_*` constant), so no lookup is needed.
    pub fn attribute_id(&mut self, qname: QNameId, value: &Value<'_>) -> ExiResult<()> {
        let xml_space = qname.uri == crate::string_table::XML_NS_URI_ID
            && self.tables.local_name_str(qname) == Some("space")
            && self.tables.uri_str(qname.uri) == Some(crate::grammar::ir::well_known::XML_NS);
        self.attribute_resolved(Some(qname), None, xml_space, value)
    }

    fn attribute_resolved(
        &mut self,
        qname: Option<QNameId>,
        names: Option<(&str, &str)>,
        xml_space: bool,
        value: &Value<'_>,
    ) -> ExiResult<()> {
        self.flush_chars(false)?;
        let frame = self.top()?;
        let found = qname.and_then(|q| {
            find_production(
                &self.tables,
                frame.rule,
                |t| matches!(t, Terminal::At(aq, _) if *aq == q),
            )
        });
        let Some((idx, prod)) = found else {
            let wildcard = find_production(&self.tables, frame.rule, |t| {
                matches!(t, Terminal::AtAny | Terminal::AtNs(_))
            });
            return if wildcard.is_some() {
                Err(ExiError::Unsupported(Feature::Wildcard))
            } else {
                Err(self.mismatch(&format!("AT({})", self.qname_text(qname, names))))
            };
        };
        let Terminal::At(q, type_idx) = prod.terminal else {
            return Err(ExiError::Internal(
                "attribute production without AT terminal",
            ));
        };
        let width = frame.first_level_width(&self.tables);
        self.emit_code("AT", idx as u64, width)?;
        let kind = self.tables.datatype(type_idx);
        self.write_value(kind, value, q)?;
        let top = self.top_mut()?;
        top.rule = prod.next;
        if xml_space {
            top.space_preserve = match value {
                Value::Lexical(v) | Value::String(v) => Some(v.trim() == "preserve"),
                _ => None,
            };
        }
        Ok(())
    }

    /// Encodes `xsi:type`: the escape code, the second-level code, then the type
    /// name as a QName ([EXI 7.1.7](https://www.w3.org/TR/exi/#encodingQName)). The
    /// element then goes on with the named type's grammar
    /// ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).
    pub fn xsi_type(&mut self, uri: &str, local: &str) -> ExiResult<()> {
        self.flush_chars(false)?;
        let frame = self.top()?;
        let Some(code2) = frame.xsi_type_code() else {
            return Err(self.mismatch("AT(xsi:type)"));
        };
        let Some(escape) = frame.escape_code(&self.tables) else {
            return Err(self.mismatch("AT(xsi:type)"));
        };
        self.emit_code("escape", escape, frame.first_level_width(&self.tables))?;
        self.emit_code("xsi:type", u64::from(code2), frame.second_level_width())?;
        let qname = self
            .strings
            .write_qname(&self.tables, &mut self.out, uri, local)?;
        let Some(type_idx) = self.tables.find_type(qname) else {
            return Err(ExiError::Schema(format!(
                "xsi:type names unknown type {}",
                crate::event::QName::new(uri, local)
            )));
        };
        let Some(decl) = self.tables.type_decl(type_idx) else {
            return Err(ExiError::Internal("dangling type index"));
        };
        let Some(start) = self.tables.grammar_start_rule(decl.grammar) else {
            return Err(ExiError::Internal("dangling grammar index"));
        };
        // Does the same as EXIficient: the target type's own first start-tag grammar
        // is used. EXIficient builds every such type grammar with typecastable=false
        // and nillable=false, so no second-level escape is left after a cast (its
        // grammar dumps list all type grammars that way). Its grammar builder refuses
        // a type grammar with either flag ("Type grammar is nillable or typable") and
        // sets them only on element grammars, copying a named type's grammar when
        // the element needs them:
        // https://github.com/EXIficient/exificient-grammars/blob/9baa044237f649a65ae4190aed75081c75c86a16/src/main/java/com/siemens/ct/exi/grammars/XSDGrammarsBuilder.java#L1474-L1499
        let top = self.top_mut()?;
        top.rule = start;
        top.start_rule = start;
        top.type_idx = type_idx;
        top.typecastable = false;
        top.nillable = false;
        Ok(())
    }

    /// Encodes `xsi:nil`: the escape code, the second-level code, then the value as
    /// a Boolean ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)). When it is
    /// true, the element switches to its type's `TypeEmpty` grammar, which allows
    /// only attributes and then `EE`
    /// ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).
    pub fn xsi_nil(&mut self, nil: bool) -> ExiResult<()> {
        self.flush_chars(false)?;
        let frame = self.top()?;
        let Some(code2) = frame.xsi_nil_code() else {
            return Err(self.mismatch("AT(xsi:nil)"));
        };
        let Some(escape) = frame.escape_code(&self.tables) else {
            return Err(self.mismatch("AT(xsi:nil)"));
        };
        self.emit_code("escape", escape, frame.first_level_width(&self.tables))?;
        self.emit_code("xsi:nil", u64::from(code2), frame.second_level_width())?;
        self.out.write_bool(nil)?;
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
        Ok(())
    }

    /// Adds character data. Text values are held in a buffer (see `flush_chars`);
    /// typed values are written right away.
    pub fn characters(&mut self, value: &Value<'_>) -> ExiResult<()> {
        match value {
            Value::Lexical(s) | Value::String(s) => {
                self.pending_chars.push_str(s);
                self.has_pending_chars = true;
                Ok(())
            }
            other => {
                self.flush_chars(false)?;
                self.encode_chars(other)
            }
        }
    }

    /// Encodes `EE` and closes the innermost element. Closing the root moves the
    /// document to `DocEnd`, where only `ED` may follow.
    pub fn end_element(&mut self) -> ExiResult<()> {
        self.flush_chars(true)?;
        let frame = self.top()?;
        // Empty simple content (`<a></a>`, an empty list) is an empty `CH`: simple
        // type grammars have no `EE` before their `CH` (EXI 8.5.4.1.3.1,
        // https://www.w3.org/TR/exi/#simpleTypeGrammars), and
        // EXIficient encodes the empty value the same way ("Should only happen in
        // STRICT mode" / "Special case: SAX does not inform about empty (\"\") CH
        // events"): https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/coder/AbstractEXIBodyEncoder.java#L609-L625
        if !frame.nil
            && find_production(&self.tables, frame.rule, |t| *t == Terminal::Ee).is_none()
            && find_production(&self.tables, frame.rule, |t| matches!(t, Terminal::Ch(_))).is_some()
        {
            self.encode_chars(&Value::Lexical(""))?;
        }
        let frame = self.top()?;
        let Some((idx, _)) = find_production(&self.tables, frame.rule, |t| *t == Terminal::Ee)
        else {
            return Err(self.mismatch("EE"));
        };
        let width = frame.first_level_width(&self.tables);
        self.emit_code("EE", idx as u64, width)?;
        self.frames.pop();
        self.after_start_tag = false;
        if self.frames.is_empty() {
            self.cursor = Cursor::DocEnd;
        }
        Ok(())
    }

    /// Encodes the buffered text before the next structural event (that event is
    /// `EE` when `before_end_tag` is true).
    ///
    /// Whitespace-only text (XML whitespace: space, tab, CR, LF) is dropped where
    /// EXIficient drops it (`checkPendingCharacters`):
    /// - where the current rule has no `CH` (element-only content);
    /// - where the `CH` with event code 0 is not typed (mixed content, or a `CH` that
    ///   is not the first production), unless the text sits right between a start
    ///   tag and its end tag or `xml:space="preserve"` is in effect.
    ///
    /// Typed character data keeps it and follows its datatype's whitespace facet.
    ///
    /// The spec leaves this to the encoder (it encodes whatever character items the
    /// application passes in). EXIficient 1.0.7 takes the rule from an answer by the
    /// EXI Working Group. The branch is the one for a rule whose production 0 is not
    /// a `CH`
    /// ([`AbstractEXIBodyEncoder.java#L1490-L1566`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/coder/AbstractEXIBodyEncoder.java#L1490-L1566),
    /// `getDatatypeWhiteSpace` at [`#L1352-L1364`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/coder/AbstractEXIBodyEncoder.java#L1352-L1364)):
    ///
    /// ```text
    /// // https://lists.w3.org/Archives/Public/public-exi/2015Oct/0008.html
    /// // If it is schema-less:
    /// // - Simple data (data between s+e) are all preserved.
    /// // - For complex data (data between s+s, e+s, e+e), it
    /// // is same as schema-informed case.
    /// ...
    /// // For complex data (data between s+s, e+s, e+e),
    /// // whitespaces nodes (i.e.
    /// // strings that consist solely of whitespaces) are
    /// // removed
    /// ```
    ///
    /// `xml:space` is tracked as EXIficient does: set by the attribute
    /// ([`#L1148-L1158`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/coder/AbstractEXIBodyEncoder.java#L1148-L1158)) and
    /// restored from the enclosing elements at `EE`
    /// ([`#L650-L666`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/coder/AbstractEXIBodyEncoder.java#L650-L666), "make sure to
    /// adapt xml:space behavior").
    fn flush_chars(&mut self, before_end_tag: bool) -> ExiResult<()> {
        if !self.has_pending_chars {
            return Ok(());
        }
        let text = core::mem::take(&mut self.pending_chars);
        self.has_pending_chars = false;
        if text.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r')) {
            let frame = self.top()?;
            let has_ch = find_production(&self.tables, frame.rule, |t| {
                matches!(t, Terminal::Ch(_) | Terminal::ChUntyped)
            })
            .is_some();
            if !has_ch {
                return Ok(());
            }
            let typed_first = matches!(
                self.tables
                    .rule_productions(frame.rule)
                    .first()
                    .map(|p| p.terminal),
                Some(Terminal::Ch(_))
            );
            let simple_data = self.after_start_tag && before_end_tag;
            if !typed_first && !simple_data && !self.space_preserved() {
                return Ok(());
            }
        }
        self.encode_chars(&Value::Lexical(&text))
    }

    /// Whether `xml:space="preserve"` is set on the current element or on the
    /// nearest ancestor that has `xml:space`.
    fn space_preserved(&self) -> bool {
        self.frames.iter().rev().find_map(|f| f.space_preserve) == Some(true)
    }

    /// Writes a `CH` event code and its value, typed by the `CH` production's
    /// datatype (or untyped).
    fn encode_chars(&mut self, value: &Value<'_>) -> ExiResult<()> {
        let frame = self.top()?;
        let found = find_production(&self.tables, frame.rule, |t| {
            matches!(t, Terminal::Ch(_) | Terminal::ChUntyped)
        });
        let Some((idx, prod)) = found else {
            return Err(self.mismatch("CH"));
        };
        let width = frame.first_level_width(&self.tables);
        self.emit_code("CH", idx as u64, width)?;
        let kind = match prod.terminal {
            Terminal::Ch(t) => self.tables.datatype(t),
            _ => DatatypeKind::Untyped,
        };
        self.write_value(kind, value, frame.context)?;
        self.top_mut()?.rule = prod.next;
        self.after_start_tag = false;
        Ok(())
    }

    /// Writes one value in the datatype `kind`
    /// ([EXI 7](https://www.w3.org/TR/exi/#encodingValues)). String-like values go
    /// through the string table under `context` (the attribute or element qname).
    /// A list is its item count as an Unsigned Integer, then each item
    /// ([EXI 7.1.11](https://www.w3.org/TR/exi/#encodingList)). Other values go to
    /// the scalar encoders in `datatype`.
    fn write_value(
        &mut self,
        kind: DatatypeKind,
        value: &Value<'_>,
        context: QNameId,
    ) -> ExiResult<()> {
        match kind {
            // Refused before the string table is touched: see
            // `DatatypeKind::RestrictedString` (EXI 7.1.10.1).
            DatatypeKind::RestrictedString { .. } => {
                Err(ExiError::Unsupported(Feature::RestrictedCharacterSet))
            }
            k if k.is_string_like() => {
                let ws = match k {
                    DatatypeKind::String { whitespace } => whitespace,
                    _ => crate::datatype::WhiteSpace::Preserve,
                };
                match value {
                    Value::Lexical(s) => {
                        let s = lexical::apply_whitespace(ws, s);
                        self.strings.write_value(&mut self.out, context, &s)
                    }
                    Value::String(s) => self.strings.write_value(&mut self.out, context, s),
                    other => {
                        let s = lexical::format_value(k, other, &self.tables)?;
                        self.strings.write_value(&mut self.out, context, &s)
                    }
                }
            }
            DatatypeKind::List { item } => {
                let item_kind = self.tables.datatype(item);
                match value {
                    Value::List(items) => {
                        write_unsigned(&mut self.out, items.len() as u64)?;
                        for it in items {
                            self.write_value(item_kind, it, context)?;
                        }
                        Ok(())
                    }
                    Value::Lexical(s) | Value::String(s) => {
                        // Split on XML spaces only, not on every Unicode space.
                        let items: Vec<&str> = s
                            .split(lexical::is_xml_space)
                            .filter(|item| !item.is_empty())
                            .collect();
                        write_unsigned(&mut self.out, items.len() as u64)?;
                        for it in items {
                            self.write_value(item_kind, &Value::Lexical(it), context)?;
                        }
                        Ok(())
                    }
                    _ => Err(ExiError::value("List", "expected a list or lexical value")),
                }
            }
            DatatypeKind::QName => Err(ExiError::Internal("QName values are encoded by the codec")),
            _ => datatype::write_scalar(&mut self.out, kind, value, &self.tables),
        }
    }

    /// Opens a frame for element `elem`, starting at the first rule of its type's
    /// grammar.
    fn push_element(&mut self, elem: crate::ids::ElemIdx, qname: QNameId) -> ExiResult<()> {
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
            context: qname,
            elem,
            type_idx: decl.type_idx,
            nillable: decl.nillable,
            typecastable: ty.typecastable,
            nil: false,
            space_preserve: None,
        });
        self.after_start_tag = true;
        Ok(())
    }

    fn top(&self) -> ExiResult<Frame> {
        self.frames
            .last()
            .copied()
            .ok_or_else(|| ExiError::Grammar {
                expected: "an open element".into(),
                got: "document level".into(),
            })
    }

    fn top_mut(&mut self) -> ExiResult<&mut Frame> {
        self.frames
            .last_mut()
            .ok_or(ExiError::Internal("empty grammar stack"))
    }

    /// Builds the error for an event that no production allows here, listing what
    /// would have been allowed.
    fn mismatch(&self, got: &str) -> ExiError {
        let expected = match (self.cursor, self.frames.last()) {
            (Cursor::Document, _) => "SD".to_string(),
            (Cursor::DocContent, None) => "SE(root element)".to_string(),
            (Cursor::DocEnd, _) => "ED".to_string(),
            (Cursor::Done, _) => "nothing (document complete)".to_string(),
            (_, Some(f)) => describe_rule(&self.tables, f.rule, Some(f)),
        };
        ExiError::Grammar {
            expected,
            got: got.to_string(),
        }
    }

    /// Writes one part of an event code as a `width`-bit unsigned integer
    /// ([EXI 6.2](https://www.w3.org/TR/exi/#encodingEventCodes),
    /// [EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned)).
    fn emit_code(&mut self, what: &str, code: u64, width: u8) -> ExiResult<()> {
        self.record(what, code, width);
        self.out.write_nbit_unsigned(code, width)
    }

    #[cfg_attr(not(feature = "trace"), allow(unused_variables))]
    fn record(&mut self, what: &str, code: u64, width: u8) {
        #[cfg(feature = "trace")]
        self.trace.push(TraceEntry {
            bit_offset: self.out.bit_position(),
            what: what.to_string(),
            width,
            code,
        });
    }
}

/// Builds the starting string table
/// ([EXI Appendix D](https://www.w3.org/TR/exi/#initialStringValues)) from the
/// grammar tables, with the `valueMaxLength` and `valuePartitionCapacity` limits.
#[doc(hidden)]
pub fn init_string_table(tables: &impl GrammarTables, options: &ExiOptions) -> StringTable {
    StringTable::from_grammar(
        tables,
        options.value_max_length,
        options.value_partition_capacity,
    )
}
