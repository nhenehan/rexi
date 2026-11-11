//! The grammar for the EXI Options document
//! ([EXI 5.4](https://www.w3.org/TR/exi/#options),
//! [Appendix C](https://www.w3.org/TR/exi/#optionsSchema)), built by hand as a
//! static table. It is the codec's first user of the static tables (see
//! [`crate::grammar::ir`]), and the proof that the runtime handles a real strict
//! grammar (optional sequences, a choice, wildcards and a nillable element).
//!
//! Every type is `typecastable = false`, the same as EXIficient's built-in header
//! grammar. So `blockSize`/`valueMaxLength` content has no `xsi:type` escape bit.
//! `schemaId` is nillable. The exi local-name partition lists the 39 element and
//! simple-type names declared by the Appendix C schema, as EXIficient does.
//!
//! EXIficient's header grammar is written by hand and never sets typecastable. The
//! only flag it sets is `schemaId`'s nillable (`g33.setNillable(true)`):
//! [`AbstractEXIHeader.java#L91-L1109`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/coder/AbstractEXIHeader.java#L91-L1109),
//! [`#L1057`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/coder/AbstractEXIHeader.java#L1057). The spec would make, for example,
//! `xs:unsignedInt` (which has built-in sub-types) typecastable
//! ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).
//!
//! How the tables below fit together: `STRINGS` is the string pool; `URIS` lists
//! the five URI partitions (`""`, xml, xsi, xsd, exi) with ranges into `PREFIXES`
//! and `LOCAL_NAMES`; `ELEMENTS` point to `TYPES`, each type points to a grammar
//! in `GRAMMARS`, a grammar is a range of `RULES`, and a rule is a range of
//! `PRODUCTIONS` in event-code order.

use crate::datatype::{DatatypeKind, WhiteSpace};
use crate::grammar::ir::{
    ElementDecl, Grammar, GrammarSet, Production, Rule, Span, Terminal, TypeDecl, UriEntry,
};
use crate::ids::{ElemIdx, GrammarIdx, LocalId, QNameId, RuleIdx, StrIdx, TypeIdx, UriId};

/// URI partition id of `http://www.w3.org/2009/exi` in [`OPTIONS_GRAMMARS`].
pub const EXI_URI: UriId = UriId(4);

// Local-name ids (compact identifiers) of the options elements in the exi
// partition. The partition is sorted by code point, so each id is the name's place
// in that sorted list.
/// Local-name id of `<header>`.
pub const LN_HEADER: LocalId = LocalId(20);
/// Local-name id of `<lesscommon>`.
pub const LN_LESSCOMMON: LocalId = LocalId(25);
/// Local-name id of `<uncommon>`.
pub const LN_UNCOMMON: LocalId = LocalId(36);
/// Local-name id of `<alignment>`.
pub const LN_ALIGNMENT: LocalId = LocalId(0);
/// Local-name id of `<byte>`.
pub const LN_BYTE: LocalId = LocalId(4);
/// Local-name id of `<pre-compress>`.
pub const LN_PRE_COMPRESS: LocalId = LocalId(28);
/// Local-name id of `<selfContained>`.
pub const LN_SELFCONTAINED: LocalId = LocalId(32);
/// Local-name id of `<valueMaxLength>`.
pub const LN_VALUEMAXLENGTH: LocalId = LocalId(37);
/// Local-name id of `<valuePartitionCapacity>`.
pub const LN_VALUEPARTITIONCAPACITY: LocalId = LocalId(38);
/// Local-name id of `<datatypeRepresentationMap>`.
pub const LN_DATATYPEREPRESENTATIONMAP: LocalId = LocalId(8);
/// Local-name id of `<preserve>`.
pub const LN_PRESERVE: LocalId = LocalId(30);
/// Local-name id of `<dtd>`.
pub const LN_DTD: LocalId = LocalId(13);
/// Local-name id of `<prefixes>`.
pub const LN_PREFIXES: LocalId = LocalId(29);
/// Local-name id of `<lexicalValues>`.
pub const LN_LEXICALVALUES: LocalId = LocalId(26);
/// Local-name id of `<comments>`.
pub const LN_COMMENTS: LocalId = LocalId(5);
/// Local-name id of `<pis>`.
pub const LN_PIS: LocalId = LocalId(27);
/// Local-name id of `<blockSize>`.
pub const LN_BLOCKSIZE: LocalId = LocalId(2);
/// Local-name id of `<common>`.
pub const LN_COMMON: LocalId = LocalId(6);
/// Local-name id of `<compression>`.
pub const LN_COMPRESSION: LocalId = LocalId(7);
/// Local-name id of `<fragment>`.
pub const LN_FRAGMENT: LocalId = LocalId(14);
/// Local-name id of `<schemaId>`.
pub const LN_SCHEMAID: LocalId = LocalId(31);
/// Local-name id of `<strict>`.
pub const LN_STRICT: LocalId = LocalId(33);

static STRINGS: &[&str] = &[
    "",
    "http://www.w3.org/XML/1998/namespace",
    "http://www.w3.org/2001/XMLSchema-instance",
    "http://www.w3.org/2001/XMLSchema",
    "http://www.w3.org/2009/exi",
    "xml",
    "xsi",
    "base",
    "id",
    "lang",
    "space",
    "nil",
    "type",
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
    "alignment",
    "blockSize",
    "comments",
    "common",
    "compression",
    "datatypeRepresentationMap",
    "dtd",
    "fragment",
    "header",
    "ieeeBinary32",
    "ieeeBinary64",
    "lesscommon",
    "lexicalValues",
    "pis",
    "pre-compress",
    "prefixes",
    "preserve",
    "schemaId",
    "selfContained",
    "strict",
    "uncommon",
    "valueMaxLength",
    "valuePartitionCapacity",
];

static URIS: &[UriEntry] = &[
    UriEntry {
        uri: StrIdx(0),
        prefixes: Span::new(0, 1),
        local_names: Span::new(0, 0),
    },
    UriEntry {
        uri: StrIdx(1),
        prefixes: Span::new(1, 1),
        local_names: Span::new(0, 4),
    },
    UriEntry {
        uri: StrIdx(2),
        prefixes: Span::new(2, 1),
        local_names: Span::new(4, 2),
    },
    UriEntry {
        uri: StrIdx(3),
        prefixes: Span::new(3, 0),
        local_names: Span::new(6, 46),
    },
    UriEntry {
        uri: StrIdx(4),
        prefixes: Span::new(3, 0),
        local_names: Span::new(52, 39),
    },
];

static PREFIXES: &[StrIdx] = &[StrIdx(0), StrIdx(5), StrIdx(6)];

static LOCAL_NAMES: &[StrIdx] = &[
    StrIdx(7),
    StrIdx(8),
    StrIdx(9),
    StrIdx(10),
    StrIdx(11),
    StrIdx(12),
    StrIdx(13),
    StrIdx(14),
    StrIdx(15),
    StrIdx(16),
    StrIdx(17),
    StrIdx(18),
    StrIdx(19),
    StrIdx(20),
    StrIdx(21),
    StrIdx(22),
    StrIdx(23),
    StrIdx(24),
    StrIdx(25),
    StrIdx(26),
    StrIdx(27),
    StrIdx(28),
    StrIdx(29),
    StrIdx(30),
    StrIdx(31),
    StrIdx(32),
    StrIdx(33),
    StrIdx(34),
    StrIdx(35),
    StrIdx(36),
    StrIdx(37),
    StrIdx(38),
    StrIdx(39),
    StrIdx(40),
    StrIdx(41),
    StrIdx(42),
    StrIdx(43),
    StrIdx(44),
    StrIdx(45),
    StrIdx(46),
    StrIdx(47),
    StrIdx(48),
    StrIdx(49),
    StrIdx(50),
    StrIdx(51),
    StrIdx(52),
    StrIdx(53),
    StrIdx(54),
    StrIdx(55),
    StrIdx(56),
    StrIdx(57),
    StrIdx(58),
    StrIdx(59),
    StrIdx(27),
    StrIdx(60),
    StrIdx(28),
    StrIdx(29),
    StrIdx(61),
    StrIdx(62),
    StrIdx(63),
    StrIdx(64),
    StrIdx(30),
    StrIdx(31),
    StrIdx(32),
    StrIdx(33),
    StrIdx(65),
    StrIdx(66),
    StrIdx(36),
    StrIdx(37),
    StrIdx(38),
    StrIdx(39),
    StrIdx(40),
    StrIdx(67),
    StrIdx(41),
    StrIdx(68),
    StrIdx(69),
    StrIdx(43),
    StrIdx(70),
    StrIdx(71),
    StrIdx(72),
    StrIdx(73),
    StrIdx(74),
    StrIdx(75),
    StrIdx(76),
    StrIdx(77),
    StrIdx(78),
    StrIdx(52),
    StrIdx(53),
    StrIdx(79),
    StrIdx(80),
    StrIdx(81),
];

static PRODUCTIONS: &[Production] = &[
    Production::new(Terminal::Se(ElemIdx(1)), RuleIdx(1)),
    Production::new(Terminal::Se(ElemIdx(17)), RuleIdx(2)),
    Production::new(Terminal::Se(ElemIdx(21)), RuleIdx(3)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(17)), RuleIdx(2)),
    Production::new(Terminal::Se(ElemIdx(21)), RuleIdx(3)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(21)), RuleIdx(3)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(2)), RuleIdx(5)),
    Production::new(Terminal::Se(ElemIdx(10)), RuleIdx(6)),
    Production::new(Terminal::Se(ElemIdx(16)), RuleIdx(7)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(10)), RuleIdx(6)),
    Production::new(Terminal::Se(ElemIdx(16)), RuleIdx(7)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(16)), RuleIdx(7)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(3)), RuleIdx(9)),
    Production::new(Terminal::Se(ElemIdx(6)), RuleIdx(10)),
    Production::new(Terminal::Se(ElemIdx(7)), RuleIdx(11)),
    Production::new(Terminal::Se(ElemIdx(8)), RuleIdx(12)),
    Production::new(Terminal::Se(ElemIdx(9)), RuleIdx(13)),
    Production::new(Terminal::SeAny, RuleIdx(8)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(6)), RuleIdx(10)),
    Production::new(Terminal::Se(ElemIdx(7)), RuleIdx(11)),
    Production::new(Terminal::Se(ElemIdx(8)), RuleIdx(12)),
    Production::new(Terminal::Se(ElemIdx(9)), RuleIdx(13)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(7)), RuleIdx(11)),
    Production::new(Terminal::Se(ElemIdx(8)), RuleIdx(12)),
    Production::new(Terminal::Se(ElemIdx(9)), RuleIdx(13)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(8)), RuleIdx(12)),
    Production::new(Terminal::Se(ElemIdx(9)), RuleIdx(13)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(9)), RuleIdx(13)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(9)), RuleIdx(13)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(4)), RuleIdx(15)),
    Production::new(Terminal::Se(ElemIdx(5)), RuleIdx(15)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Ch(TypeIdx(5)), RuleIdx(18)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::SeAny, RuleIdx(20)),
    Production::new(Terminal::SeAny, RuleIdx(21)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(11)), RuleIdx(23)),
    Production::new(Terminal::Se(ElemIdx(12)), RuleIdx(24)),
    Production::new(Terminal::Se(ElemIdx(13)), RuleIdx(25)),
    Production::new(Terminal::Se(ElemIdx(14)), RuleIdx(26)),
    Production::new(Terminal::Se(ElemIdx(15)), RuleIdx(27)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(12)), RuleIdx(24)),
    Production::new(Terminal::Se(ElemIdx(13)), RuleIdx(25)),
    Production::new(Terminal::Se(ElemIdx(14)), RuleIdx(26)),
    Production::new(Terminal::Se(ElemIdx(15)), RuleIdx(27)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(13)), RuleIdx(25)),
    Production::new(Terminal::Se(ElemIdx(14)), RuleIdx(26)),
    Production::new(Terminal::Se(ElemIdx(15)), RuleIdx(27)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(14)), RuleIdx(26)),
    Production::new(Terminal::Se(ElemIdx(15)), RuleIdx(27)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(15)), RuleIdx(27)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(18)), RuleIdx(29)),
    Production::new(Terminal::Se(ElemIdx(19)), RuleIdx(30)),
    Production::new(Terminal::Se(ElemIdx(20)), RuleIdx(31)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(19)), RuleIdx(30)),
    Production::new(Terminal::Se(ElemIdx(20)), RuleIdx(31)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Se(ElemIdx(20)), RuleIdx(31)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Ee, RuleIdx::NONE),
    Production::new(Terminal::Ch(TypeIdx(9)), RuleIdx(33)),
    Production::new(Terminal::Ee, RuleIdx::NONE),
];

static RULES: &[Rule] = &[
    Rule {
        productions: Span::new(0, 4),
    },
    Rule {
        productions: Span::new(4, 3),
    },
    Rule {
        productions: Span::new(7, 2),
    },
    Rule {
        productions: Span::new(9, 1),
    },
    Rule {
        productions: Span::new(10, 4),
    },
    Rule {
        productions: Span::new(14, 3),
    },
    Rule {
        productions: Span::new(17, 2),
    },
    Rule {
        productions: Span::new(19, 1),
    },
    Rule {
        productions: Span::new(20, 7),
    },
    Rule {
        productions: Span::new(27, 5),
    },
    Rule {
        productions: Span::new(32, 4),
    },
    Rule {
        productions: Span::new(36, 3),
    },
    Rule {
        productions: Span::new(39, 2),
    },
    Rule {
        productions: Span::new(41, 2),
    },
    Rule {
        productions: Span::new(43, 2),
    },
    Rule {
        productions: Span::new(45, 1),
    },
    Rule {
        productions: Span::new(46, 1),
    },
    Rule {
        productions: Span::new(47, 1),
    },
    Rule {
        productions: Span::new(48, 1),
    },
    Rule {
        productions: Span::new(49, 1),
    },
    Rule {
        productions: Span::new(50, 1),
    },
    Rule {
        productions: Span::new(51, 1),
    },
    Rule {
        productions: Span::new(52, 6),
    },
    Rule {
        productions: Span::new(58, 5),
    },
    Rule {
        productions: Span::new(63, 4),
    },
    Rule {
        productions: Span::new(67, 3),
    },
    Rule {
        productions: Span::new(70, 2),
    },
    Rule {
        productions: Span::new(72, 1),
    },
    Rule {
        productions: Span::new(73, 4),
    },
    Rule {
        productions: Span::new(77, 3),
    },
    Rule {
        productions: Span::new(80, 2),
    },
    Rule {
        productions: Span::new(82, 1),
    },
    Rule {
        productions: Span::new(83, 1),
    },
    Rule {
        productions: Span::new(84, 1),
    },
];

static GRAMMARS: &[Grammar] = &[
    Grammar {
        rules: Span::new(0, 4),
    },
    Grammar {
        rules: Span::new(4, 4),
    },
    Grammar {
        rules: Span::new(8, 6),
    },
    Grammar {
        rules: Span::new(14, 2),
    },
    Grammar {
        rules: Span::new(16, 1),
    },
    Grammar {
        rules: Span::new(17, 2),
    },
    Grammar {
        rules: Span::new(19, 3),
    },
    Grammar {
        rules: Span::new(22, 6),
    },
    Grammar {
        rules: Span::new(28, 4),
    },
    Grammar {
        rules: Span::new(32, 2),
    },
];

static ELEMENTS: &[ElementDecl] = &[
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(20)),
        type_idx: TypeIdx(0),
        nillable: false,
        global: true,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(25)),
        type_idx: TypeIdx(1),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(36)),
        type_idx: TypeIdx(2),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(0)),
        type_idx: TypeIdx(3),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(4)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(28)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(32)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(37)),
        type_idx: TypeIdx(5),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(38)),
        type_idx: TypeIdx(5),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(8)),
        type_idx: TypeIdx(6),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(30)),
        type_idx: TypeIdx(7),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(13)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(29)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(26)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(5)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(27)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(2)),
        type_idx: TypeIdx(5),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(6)),
        type_idx: TypeIdx(8),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(7)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(14)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(31)),
        type_idx: TypeIdx(9),
        nillable: true,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(UriId(4), LocalId(33)),
        type_idx: TypeIdx(4),
        nillable: false,
        global: false,
    },
];

static TYPES: &[TypeDecl] = &[
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(0),
        empty_grammar: GrammarIdx(4),
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(1),
        empty_grammar: GrammarIdx(4),
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(2),
        empty_grammar: GrammarIdx(4),
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(3),
        empty_grammar: GrammarIdx(4),
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(4),
        empty_grammar: GrammarIdx(4),
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(5),
        empty_grammar: GrammarIdx(4),
        datatype: Some(DatatypeKind::UnsignedInteger),
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(6),
        empty_grammar: GrammarIdx(4),
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(7),
        empty_grammar: GrammarIdx(4),
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(8),
        empty_grammar: GrammarIdx(4),
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(UriId::NONE, LocalId(0)),
        grammar: GrammarIdx(9),
        empty_grammar: GrammarIdx(4),
        datatype: Some(DatatypeKind::String {
            whitespace: WhiteSpace::Preserve,
        }),
        typecastable: false,
    },
];

// Only `header` is a global element, so the document grammar has one SE(G_0).
static GLOBAL_ELEMENTS: &[ElemIdx] = &[ElemIdx(0)];

/// The full grammar set for encoding and decoding EXI Options documents.
pub static OPTIONS_GRAMMARS: GrammarSet<'static> = GrammarSet {
    strings: STRINGS,
    uris: URIS,
    prefixes: PREFIXES,
    local_names: LOCAL_NAMES,
    productions: PRODUCTIONS,
    rules: RULES,
    grammars: GRAMMARS,
    elements: ELEMENTS,
    global_elements: GLOBAL_ELEMENTS,
    global_attributes: &[],
    types: TYPES,
    enum_sets: &[],
    enum_values: &[],
    schema_id: Some("http://www.w3.org/2009/exi"),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::ir::GrammarTables;

    #[test]
    fn table_is_consistent() {
        let g = &OPTIONS_GRAMMARS;
        assert_eq!(g.uris().len(), 5);
        assert_eq!(
            g.local_name_str(QNameId::new(EXI_URI, LN_HEADER)),
            Some("header")
        );
        assert_eq!(
            g.local_name_str(QNameId::new(EXI_URI, LN_SCHEMAID)),
            Some("schemaId")
        );
        assert_eq!(
            g.find_qname("http://www.w3.org/2009/exi", "strict"),
            Some(QNameId::new(EXI_URI, LN_STRICT))
        );
        assert_eq!(
            g.find_qname("http://www.w3.org/2001/XMLSchema", "unsignedShort"),
            Some(QNameId::new(UriId(3), LocalId(45)))
        );
        for p in g.productions() {
            match p.terminal {
                Terminal::Ee => assert_eq!(p.next, RuleIdx::NONE),
                _ => assert!(p.next.index() < g.rules().len()),
            }
            if let Terminal::Se(e) = p.terminal {
                assert!(e.index() < g.elements().len());
            }
        }
        for e in g.elements() {
            assert!(e.type_idx.index() < g.types().len());
            assert!(g.local_name_str(e.qname).map(str::len).unwrap_or(0) > 0);
        }
        let root = g.find_global_element(QNameId::new(EXI_URI, LN_HEADER));
        assert_eq!(root, Some(ElemIdx(0)));
    }
}
