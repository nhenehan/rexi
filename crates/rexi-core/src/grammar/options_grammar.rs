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
use crate::ids::{NONE, QNameId};

/// URI partition id of `http://www.w3.org/2009/exi` in [`OPTIONS_GRAMMARS`].
pub const EXI_URI: u32 = 4;

// Local-name ids (compact identifiers) of the options elements in the exi
// partition. The partition is sorted by code point, so each id is the name's place
// in that sorted list.
/// Local-name id of `<header>`.
pub const LN_HEADER: u32 = 20;
/// Local-name id of `<lesscommon>`.
pub const LN_LESSCOMMON: u32 = 25;
/// Local-name id of `<uncommon>`.
pub const LN_UNCOMMON: u32 = 36;
/// Local-name id of `<alignment>`.
pub const LN_ALIGNMENT: u32 = 0;
/// Local-name id of `<byte>`.
pub const LN_BYTE: u32 = 4;
/// Local-name id of `<pre-compress>`.
pub const LN_PRE_COMPRESS: u32 = 28;
/// Local-name id of `<selfContained>`.
pub const LN_SELFCONTAINED: u32 = 32;
/// Local-name id of `<valueMaxLength>`.
pub const LN_VALUEMAXLENGTH: u32 = 37;
/// Local-name id of `<valuePartitionCapacity>`.
pub const LN_VALUEPARTITIONCAPACITY: u32 = 38;
/// Local-name id of `<datatypeRepresentationMap>`.
pub const LN_DATATYPEREPRESENTATIONMAP: u32 = 8;
/// Local-name id of `<preserve>`.
pub const LN_PRESERVE: u32 = 30;
/// Local-name id of `<dtd>`.
pub const LN_DTD: u32 = 13;
/// Local-name id of `<prefixes>`.
pub const LN_PREFIXES: u32 = 29;
/// Local-name id of `<lexicalValues>`.
pub const LN_LEXICALVALUES: u32 = 26;
/// Local-name id of `<comments>`.
pub const LN_COMMENTS: u32 = 5;
/// Local-name id of `<pis>`.
pub const LN_PIS: u32 = 27;
/// Local-name id of `<blockSize>`.
pub const LN_BLOCKSIZE: u32 = 2;
/// Local-name id of `<common>`.
pub const LN_COMMON: u32 = 6;
/// Local-name id of `<compression>`.
pub const LN_COMPRESSION: u32 = 7;
/// Local-name id of `<fragment>`.
pub const LN_FRAGMENT: u32 = 14;
/// Local-name id of `<schemaId>`.
pub const LN_SCHEMAID: u32 = 31;
/// Local-name id of `<strict>`.
pub const LN_STRICT: u32 = 33;

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
        uri: 0,
        prefixes: Span::new(0, 1),
        local_names: Span::new(0, 0),
    },
    UriEntry {
        uri: 1,
        prefixes: Span::new(1, 1),
        local_names: Span::new(0, 4),
    },
    UriEntry {
        uri: 2,
        prefixes: Span::new(2, 1),
        local_names: Span::new(4, 2),
    },
    UriEntry {
        uri: 3,
        prefixes: Span::new(3, 0),
        local_names: Span::new(6, 46),
    },
    UriEntry {
        uri: 4,
        prefixes: Span::new(3, 0),
        local_names: Span::new(52, 39),
    },
];

static PREFIXES: &[u32] = &[0, 5, 6];

static LOCAL_NAMES: &[u32] = &[
    7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54,
    55, 56, 57, 58, 59, 27, 60, 28, 29, 61, 62, 63, 64, 30, 31, 32, 33, 65, 66, 36, 37, 38, 39, 40,
    67, 41, 68, 69, 43, 70, 71, 72, 73, 74, 75, 76, 77, 78, 52, 53, 79, 80, 81,
];

static PRODUCTIONS: &[Production] = &[
    Production::new(Terminal::Se(1), 1),
    Production::new(Terminal::Se(17), 2),
    Production::new(Terminal::Se(21), 3),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(17), 2),
    Production::new(Terminal::Se(21), 3),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(21), 3),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(2), 5),
    Production::new(Terminal::Se(10), 6),
    Production::new(Terminal::Se(16), 7),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(10), 6),
    Production::new(Terminal::Se(16), 7),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(16), 7),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(3), 9),
    Production::new(Terminal::Se(6), 10),
    Production::new(Terminal::Se(7), 11),
    Production::new(Terminal::Se(8), 12),
    Production::new(Terminal::Se(9), 13),
    Production::new(Terminal::SeAny, 8),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(6), 10),
    Production::new(Terminal::Se(7), 11),
    Production::new(Terminal::Se(8), 12),
    Production::new(Terminal::Se(9), 13),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(7), 11),
    Production::new(Terminal::Se(8), 12),
    Production::new(Terminal::Se(9), 13),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(8), 12),
    Production::new(Terminal::Se(9), 13),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(9), 13),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(9), 13),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(4), 15),
    Production::new(Terminal::Se(5), 15),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Ch(5), 18),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::SeAny, 20),
    Production::new(Terminal::SeAny, 21),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(11), 23),
    Production::new(Terminal::Se(12), 24),
    Production::new(Terminal::Se(13), 25),
    Production::new(Terminal::Se(14), 26),
    Production::new(Terminal::Se(15), 27),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(12), 24),
    Production::new(Terminal::Se(13), 25),
    Production::new(Terminal::Se(14), 26),
    Production::new(Terminal::Se(15), 27),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(13), 25),
    Production::new(Terminal::Se(14), 26),
    Production::new(Terminal::Se(15), 27),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(14), 26),
    Production::new(Terminal::Se(15), 27),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(15), 27),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(18), 29),
    Production::new(Terminal::Se(19), 30),
    Production::new(Terminal::Se(20), 31),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(19), 30),
    Production::new(Terminal::Se(20), 31),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Se(20), 31),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Ee, NONE),
    Production::new(Terminal::Ch(9), 33),
    Production::new(Terminal::Ee, NONE),
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
        qname: QNameId::new(4, 20),
        type_idx: 0,
        nillable: false,
        global: true,
    },
    ElementDecl {
        qname: QNameId::new(4, 25),
        type_idx: 1,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 36),
        type_idx: 2,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 0),
        type_idx: 3,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 4),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 28),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 32),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 37),
        type_idx: 5,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 38),
        type_idx: 5,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 8),
        type_idx: 6,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 30),
        type_idx: 7,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 13),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 29),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 26),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 5),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 27),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 2),
        type_idx: 5,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 6),
        type_idx: 8,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 7),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 14),
        type_idx: 4,
        nillable: false,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 31),
        type_idx: 9,
        nillable: true,
        global: false,
    },
    ElementDecl {
        qname: QNameId::new(4, 33),
        type_idx: 4,
        nillable: false,
        global: false,
    },
];

static TYPES: &[TypeDecl] = &[
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 0,
        empty_grammar: 4,
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 1,
        empty_grammar: 4,
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 2,
        empty_grammar: 4,
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 3,
        empty_grammar: 4,
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 4,
        empty_grammar: 4,
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 5,
        empty_grammar: 4,
        datatype: Some(DatatypeKind::UnsignedInteger),
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 6,
        empty_grammar: 4,
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 7,
        empty_grammar: 4,
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 8,
        empty_grammar: 4,
        datatype: None,
        typecastable: false,
    },
    TypeDecl {
        qname: QNameId::new(NONE, 0),
        grammar: 9,
        empty_grammar: 4,
        datatype: Some(DatatypeKind::String {
            whitespace: WhiteSpace::Preserve,
        }),
        typecastable: false,
    },
];

// Only `header` is a global element, so the document grammar has one SE(G_0).
static GLOBAL_ELEMENTS: &[u32] = &[0];

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
            Some(QNameId::new(3, 45))
        );
        for p in g.productions() {
            match p.terminal {
                Terminal::Ee => assert_eq!(p.next, NONE),
                _ => assert!((p.next as usize) < g.rules().len()),
            }
            if let Terminal::Se(e) = p.terminal {
                assert!((e as usize) < g.elements().len());
            }
        }
        for e in g.elements() {
            assert!((e.type_idx as usize) < g.types().len());
            assert!(g.local_name_str(e.qname).map(str::len).unwrap_or(0) > 0);
        }
        let root = g.find_global_element(QNameId::new(EXI_URI, LN_HEADER));
        assert_eq!(root, Some(0));
    }
}
