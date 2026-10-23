//! Grammar tables: the static form of EXI grammars, which needs no allocation
//! ([EXI 8](https://www.w3.org/TR/exi/#grammars)).
//!
//! Everything is a flat slice of plain data indexed by `u32`. So a schema compiled
//! ahead of time can be written out as a `static GrammarSet<'static>` (the
//! gSOAP-style path), while the same layout can be built at run time as a
//! [`super::GrammarSetBuf`]. The codec is generic over [`GrammarTables`], which both
//! implement.
//!
//! The layout follows exip's `EXIGrammar` / `GrammarRule` / `Production` tables.
//! It adds the element → type → grammar step of the EXIGrammar exchange format, so
//! one type grammar is shared by every element of that type.
//!
//! The string table's starting partitions
//! ([EXI Appendix D](https://www.w3.org/TR/exi/#initialStringValues)) are part of
//! the tables: `uris` lists the URI partition in order, each entry with its prefix
//! and local-name partitions. So the compact identifiers in [`QNameId`] point at
//! those starting entries.

use crate::datatype::{DatatypeKind, TypeResolver};
use crate::ids::{ElemIdx, EnumSetIdx, GrammarIdx, NONE, QNameId, RuleIdx, StrIdx, TypeIdx, UriId};

/// A contiguous range inside one of the flat tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    /// First index of the range.
    pub start: u32,
    /// Number of entries in the range.
    pub len: u32,
}

impl Span {
    /// The range `start..start + len`.
    pub const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    /// The range as a `usize` range, for slicing.
    pub fn range(&self) -> core::ops::Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }
}

/// One URI partition with its prefix and local-name partitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UriEntry {
    /// The URI text.
    pub uri: StrIdx,
    /// Range in `GrammarTables::prefixes`.
    pub prefixes: Span,
    /// Range in `GrammarTables::local_names`, sorted by code point.
    pub local_names: Span,
}

/// The terminal symbol of a production: the event that the production matches
/// ([EXI 8.1](https://www.w3.org/TR/exi/#grammarNotation); event types in
/// [EXI 4](https://www.w3.org/TR/exi/#streams)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Terminal {
    /// `SD`: start of the document.
    Sd,
    /// `ED`: end of the document.
    Ed,
    /// `EE`: end of the element.
    Ee,
    /// `SE(qname)` for a declared element.
    Se(ElemIdx),
    /// `SE(uri:*)`.
    SeNs(UriId),
    /// `SE(*)`.
    SeAny,
    /// `AT(qname)` with the attribute's simple type.
    At(QNameId, TypeIdx),
    /// `AT(uri:*)`.
    AtNs(UriId),
    /// `AT(*)`.
    AtAny,
    /// `CH` typed by a simple type (use `TypeDecl::datatype`), or untyped mixed
    /// content.
    Ch(TypeIdx),
    /// `CH [untyped]` (mixed content).
    ChUntyped,
    // The rest of the terminals only show up in built-in grammars and in non-strict
    // schema-informed grammars. They are listed so the tables can describe those
    // grammars. The strict codec never writes them.
    // (In strict mode, xsi:type and xsi:nil are not stored as productions. They are
    // second-level codes that the runtime adds, see `runtime::Frame`.)
    /// `AT(xsi:type)`.
    XsiType,
    /// `AT(xsi:nil)`.
    XsiNil,
    /// `AT(*)` with an untyped value.
    AtAnyUntyped,
    /// `NS`: namespace declaration.
    Ns,
    /// `SC`: self-contained element.
    Sc,
    /// `CM`: comment.
    Cm,
    /// `PI`: processing instruction.
    Pi,
    /// `DT`: DOCTYPE.
    Dt,
    /// `ER`: entity reference.
    Er,
}

/// A production: a terminal plus the rule to go on with after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Production {
    /// The event this production matches.
    pub terminal: Terminal,
    /// Next rule (absolute index), or [`NONE`] after `EE` / `ED`.
    pub next: RuleIdx,
}

impl Production {
    /// A production that matches `terminal` and then goes on with rule `next`.
    pub const fn new(terminal: Terminal, next: RuleIdx) -> Self {
        Self { terminal, next }
    }
}

/// A grammar rule (a non-terminal with its right-hand sides, that is, the
/// productions it can be replaced by).
///
/// Productions are stored in event-code order, so the position of a production is
/// its first-level event code
/// ([EXI 8.5.4.3](https://www.w3.org/TR/exi/#eventCodeAssignment)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rule {
    /// Range in `GrammarTables::productions`, in event-code order.
    pub productions: Span,
}

/// A grammar: its rules, the first of which is the start rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Grammar {
    /// Range in `GrammarTables::rules`; `rules.start` is the start rule.
    pub rules: Span,
}

/// An element declaration (global or local).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ElementDecl {
    /// The element's name.
    pub qname: QNameId,
    /// The element's type.
    pub type_idx: TypeIdx,
    /// Whether `xsi:nil="true"` is allowed on the element.
    pub nillable: bool,
    /// Whether the element is declared at the top level of the schema.
    pub global: bool,
}

/// A type definition, simple or complex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeDecl {
    /// Name, or `QNameId { uri: NONE, .. }` for anonymous types.
    pub qname: QNameId,
    /// Grammar for the type's content (`Type_i`,
    /// [EXI 8.5.4.1.3](https://www.w3.org/TR/exi/#typeGrammars)).
    pub grammar: GrammarIdx,
    /// Grammar used when `xsi:nil="true"` (`TypeEmpty_i`): attributes only, then `EE`.
    pub empty_grammar: GrammarIdx,
    /// For simple types, how values are encoded. `None` for complex types.
    pub datatype: Option<DatatypeKind>,
    /// Whether the type has named subtypes or is a union. In strict mode only such
    /// types allow `AT(xsi:type)`
    /// ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).
    pub typecastable: bool,
}

impl TypeDecl {
    /// Whether the type has no name.
    pub fn is_anonymous(&self) -> bool {
        self.qname.uri == NONE
    }
}

/// A global attribute declaration, used to give a type to `AT(*)` in non-strict
/// grammars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobalAttribute {
    /// The attribute's name.
    pub qname: QNameId,
    /// The attribute's simple type.
    pub type_idx: TypeIdx,
}

/// Grammar tables as borrowed slices: the shape a generated `static` has.
#[derive(Debug, Clone, Copy)]
pub struct GrammarSet<'a> {
    /// String pool referenced by [`StrIdx`].
    pub strings: &'a [&'a str],
    /// URI partition in starting order (`""`, xml, xsi, xsd, then schema namespaces).
    pub uris: &'a [UriEntry],
    /// Prefix partitions; each [`UriEntry`] owns a range.
    pub prefixes: &'a [StrIdx],
    /// Local-name partitions; each [`UriEntry`] owns a range.
    pub local_names: &'a [StrIdx],
    /// Every production; each [`Rule`] owns a range.
    pub productions: &'a [Production],
    /// Every rule; each [`Grammar`] owns a range.
    pub rules: &'a [Rule],
    /// Every grammar; each [`TypeDecl`] points at one or two.
    pub grammars: &'a [Grammar],
    /// Every element declaration, global or local.
    pub elements: &'a [ElementDecl],
    /// Global elements sorted by local-name then URI: the `DocContent` productions
    /// ([EXI 8.5.1](https://www.w3.org/TR/exi/#informedDocGrammars)).
    pub global_elements: &'a [ElemIdx],
    /// Global attribute declarations.
    pub global_attributes: &'a [GlobalAttribute],
    /// Every type definition.
    pub types: &'a [TypeDecl],
    /// Each enumeration set is a range in `enum_values`.
    pub enum_sets: &'a [Span],
    /// The values of every enumeration, as strings.
    pub enum_values: &'a [StrIdx],
    /// The schema id that streams using these tables carry, if any
    /// ([EXI 5.4](https://www.w3.org/TR/exi/#options)).
    pub schema_id: Option<&'a str>,
}

impl GrammarSet<'static> {
    /// Tables with nothing in them: for string tables that hold only strings
    /// added at run time.
    pub const EMPTY: GrammarSet<'static> = GrammarSet {
        strings: &[],
        uris: &[],
        prefixes: &[],
        local_names: &[],
        productions: &[],
        rules: &[],
        grammars: &[],
        elements: &[],
        global_elements: &[],
        global_attributes: &[],
        types: &[],
        enum_sets: &[],
        enum_values: &[],
        schema_id: None,
    };
}

/// Read access to grammar tables, implemented by [`GrammarSet`] and
/// [`super::GrammarSetBuf`].
pub trait GrammarTables {
    /// String `i` of the string pool (empty if out of range).
    fn string(&self, i: StrIdx) -> &str;
    /// See [`GrammarSet::uris`].
    fn uris(&self) -> &[UriEntry];
    /// See [`GrammarSet::prefixes`].
    fn prefixes(&self) -> &[StrIdx];
    /// See [`GrammarSet::local_names`].
    fn local_names(&self) -> &[StrIdx];
    /// See [`GrammarSet::productions`].
    fn productions(&self) -> &[Production];
    /// See [`GrammarSet::rules`].
    fn rules(&self) -> &[Rule];
    /// See [`GrammarSet::grammars`].
    fn grammars(&self) -> &[Grammar];
    /// See [`GrammarSet::elements`].
    fn elements(&self) -> &[ElementDecl];
    /// See [`GrammarSet::global_elements`].
    fn global_elements(&self) -> &[ElemIdx];
    /// See [`GrammarSet::global_attributes`].
    fn global_attributes(&self) -> &[GlobalAttribute];
    /// See [`GrammarSet::types`].
    fn types(&self) -> &[TypeDecl];
    /// See [`GrammarSet::enum_sets`].
    fn enum_sets(&self) -> &[Span];
    /// See [`GrammarSet::enum_values`].
    fn enum_values(&self) -> &[StrIdx];
    /// See [`GrammarSet::schema_id`].
    fn schema_id(&self) -> Option<&str>;

    /// The text of URI `uri`, or `None` if there is no such partition.
    fn uri_str(&self, uri: UriId) -> Option<&str> {
        self.uris().get(uri as usize).map(|u| self.string(u.uri))
    }

    /// The local-name text of `qname`, or `None` if it is not in the starting
    /// partitions.
    fn local_name_str(&self, qname: QNameId) -> Option<&str> {
        let entry = self.uris().get(qname.uri as usize)?;
        let idx = entry.local_names.range().nth(qname.local as usize)?;
        self.local_names().get(idx).map(|&s| self.string(s))
    }

    /// The productions of `rule` (empty if out of range).
    fn rule_productions(&self, rule: RuleIdx) -> &[Production] {
        match self.rules().get(rule as usize) {
            Some(r) => self.productions().get(r.productions.range()).unwrap_or(&[]),
            None => &[],
        }
    }

    /// The start rule of `grammar`.
    fn grammar_start_rule(&self, grammar: GrammarIdx) -> Option<RuleIdx> {
        self.grammars().get(grammar as usize).map(|g| g.rules.start)
    }

    /// Element declaration `e`.
    fn element(&self, e: ElemIdx) -> Option<ElementDecl> {
        self.elements().get(e as usize).copied()
    }

    /// Type definition `t`.
    fn type_decl(&self, t: TypeIdx) -> Option<TypeDecl> {
        self.types().get(t as usize).copied()
    }

    /// The named type `qname`, by linear search.
    fn find_type(&self, qname: QNameId) -> Option<TypeIdx> {
        self.types()
            .iter()
            .position(|t| t.qname == qname)
            .map(|i| i as TypeIdx)
    }

    /// The global element `qname`, by linear search.
    fn find_global_element(&self, qname: QNameId) -> Option<ElemIdx> {
        self.global_elements().iter().copied().find(|&e| {
            self.elements()
                .get(e as usize)
                .is_some_and(|d| d.qname == qname)
        })
    }

    /// Looks up a (uri, local-name) pair in the starting partitions and returns the
    /// compact identifier of that declared qname. Local names are found by binary
    /// search (each partition is sorted by code point,
    /// [EXI D.3](https://www.w3.org/TR/exi/#initialLocalNames)), with a linear scan
    /// as a fallback for hand-built tables that are not sorted.
    fn find_qname(&self, uri: &str, local: &str) -> Option<QNameId> {
        let uri_id = self.uris().iter().position(|u| self.string(u.uri) == uri)? as UriId;
        let entry = self.uris().get(uri_id as usize)?;
        let names = self.local_names().get(entry.local_names.range())?;
        let local_id = match names.binary_search_by(|&s| self.string(s).cmp(local)) {
            // Step back to the first of any equal neighbors, the one the linear scan
            // would find.
            Ok(mut i) => {
                while i > 0 && names.get(i - 1).is_some_and(|&s| self.string(s) == local) {
                    i -= 1;
                }
                i
            }
            Err(_) => names.iter().position(|&s| self.string(s) == local)?,
        } as u32;
        Some(QNameId::new(uri_id, local_id))
    }
}

impl GrammarTables for GrammarSet<'_> {
    fn string(&self, i: StrIdx) -> &str {
        self.strings.get(i as usize).copied().unwrap_or("")
    }
    fn uris(&self) -> &[UriEntry] {
        self.uris
    }
    fn prefixes(&self) -> &[StrIdx] {
        self.prefixes
    }
    fn local_names(&self) -> &[StrIdx] {
        self.local_names
    }
    fn productions(&self) -> &[Production] {
        self.productions
    }
    fn rules(&self) -> &[Rule] {
        self.rules
    }
    fn grammars(&self) -> &[Grammar] {
        self.grammars
    }
    fn elements(&self) -> &[ElementDecl] {
        self.elements
    }
    fn global_elements(&self) -> &[ElemIdx] {
        self.global_elements
    }
    fn global_attributes(&self) -> &[GlobalAttribute] {
        self.global_attributes
    }
    fn types(&self) -> &[TypeDecl] {
        self.types
    }
    fn enum_sets(&self) -> &[Span] {
        self.enum_sets
    }
    fn enum_values(&self) -> &[StrIdx] {
        self.enum_values
    }
    fn schema_id(&self) -> Option<&str> {
        self.schema_id
    }
}

impl<G: GrammarTables + ?Sized> TypeResolver for G {
    fn datatype(&self, t: TypeIdx) -> DatatypeKind {
        self.type_decl(t)
            .and_then(|d| d.datatype)
            .unwrap_or(DatatypeKind::Untyped)
    }

    fn enum_count(&self, set: EnumSetIdx) -> usize {
        self.enum_sets()
            .get(set as usize)
            .map_or(0, |s| s.len as usize)
    }

    fn enum_value(&self, set: EnumSetIdx, ordinal: usize) -> Option<&str> {
        let span = self.enum_sets().get(set as usize)?;
        if ordinal >= span.len as usize {
            return None;
        }
        self.enum_values()
            .get(span.start as usize + ordinal)
            .map(|&s| self.string(s))
    }
}

impl<G: GrammarTables + ?Sized> GrammarTables for &G {
    fn string(&self, i: StrIdx) -> &str {
        (**self).string(i)
    }
    fn uris(&self) -> &[UriEntry] {
        (**self).uris()
    }
    fn prefixes(&self) -> &[StrIdx] {
        (**self).prefixes()
    }
    fn local_names(&self) -> &[StrIdx] {
        (**self).local_names()
    }
    fn productions(&self) -> &[Production] {
        (**self).productions()
    }
    fn rules(&self) -> &[Rule] {
        (**self).rules()
    }
    fn grammars(&self) -> &[Grammar] {
        (**self).grammars()
    }
    fn elements(&self) -> &[ElementDecl] {
        (**self).elements()
    }
    fn global_elements(&self) -> &[ElemIdx] {
        (**self).global_elements()
    }
    fn global_attributes(&self) -> &[GlobalAttribute] {
        (**self).global_attributes()
    }
    fn types(&self) -> &[TypeDecl] {
        (**self).types()
    }
    fn enum_sets(&self) -> &[Span] {
        (**self).enum_sets()
    }
    fn enum_values(&self) -> &[StrIdx] {
        (**self).enum_values()
    }
    fn schema_id(&self) -> Option<&str> {
        (**self).schema_id()
    }
}

/// Well-known starting URI partition ids
/// ([EXI Appendix D](https://www.w3.org/TR/exi/#initialUriValues) / Table D-1).
pub mod well_known {
    use crate::ids::UriId;

    /// The empty URI (no namespace).
    pub const URI_EMPTY: UriId = 0;
    /// The `xml` namespace.
    pub const URI_XML: UriId = 1;
    /// The `xsi` namespace.
    pub const URI_XSI: UriId = 2;
    /// Present only when the grammars are schema-informed.
    pub const URI_XSD: UriId = 3;

    /// The `xml` namespace URI.
    pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
    /// The `xsi` namespace URI.
    pub const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
    /// The XML Schema namespace URI.
    pub const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema";

    /// Local names of the xml partition, in order.
    pub const XML_LOCAL_NAMES: [&str; 4] = ["base", "id", "lang", "space"];
    /// Local names of the xsi partition, in order.
    pub const XSI_LOCAL_NAMES: [&str; 2] = ["nil", "type"];
    /// xsi:nil is local-name 0 and xsi:type is local-name 1 in the xsi partition.
    pub const XSI_NIL_LOCAL: u32 = 0;
    /// Local-name id of `xsi:type` in the xsi partition.
    pub const XSI_TYPE_LOCAL: u32 = 1;

    /// The 46 XML Schema built-in type names, sorted by code point
    /// ([EXI Table D-5](https://www.w3.org/TR/exi/#initialLocalNames)).
    pub const XSD_LOCAL_NAMES: [&str; 46] = [
        "ENTITIES",
        "ENTITY",
        "ID",
        "IDREF",
        "IDREFS",
        "NCName",
        "NMTOKEN",
        "NMTOKENS",
        "NOTATION",
        "Name",
        "QName",
        "anySimpleType",
        "anyType",
        "anyURI",
        "base64Binary",
        "boolean",
        "byte",
        "date",
        "dateTime",
        "decimal",
        "double",
        "duration",
        "float",
        "gDay",
        "gMonth",
        "gMonthDay",
        "gYear",
        "gYearMonth",
        "hexBinary",
        "int",
        "integer",
        "language",
        "long",
        "negativeInteger",
        "nonNegativeInteger",
        "nonPositiveInteger",
        "normalizedString",
        "positiveInteger",
        "short",
        "string",
        "time",
        "token",
        "unsignedByte",
        "unsignedInt",
        "unsignedLong",
        "unsignedShort",
    ];
}
