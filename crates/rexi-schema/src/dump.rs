//! Text dumps of grammar tables.
//!
//! [`dump`] copies the layout of the EXIficient grammar dumps that the tests use as
//! the expected answer (the "oracle"). It prints the string-table section first,
//! then the grammars, numbered by a depth-first walk that starts from the document
//! grammar.
//!
//! EXIficient's grammar objects do not line up one-to-one with rexi's tables:
//!
//! - It keeps one first-start-tag grammar for each element declaration. This is a
//!   copy of the type grammar when the element is nillable or its type is
//!   typecastable (can be changed with `xsi:type`); otherwise elements share the
//!   type's grammar.
//! - It has one shared `EE` grammar used after all simple content.
//! - It keeps an "element content" copy of each type's content rule: the rule
//!   reached once all attributes are done.
//!
//! The walk here makes up those objects on the fly (as `Node` values), so that the
//! numbers match EXIficient's in the common cases.
//!
//! The output of [`dump`] looks like this:
//!
//! ```text
//! # rexi grammar dump
//! schema-informed true
//! schema-id <id or <none>>
//! uri-contexts <number of URIs>
//! qname-contexts <number of local names>
//!
//! # string table (...)
//! uri <id> "<uri>" prefixes=["<prefix>",...]
//!   local <id> "<name>" [type-grammar=<grammar>] [global-element=<grammar>]
//!
//! # grammars
//! document-grammar <grammar>
//! fragment-grammar <grammar>
//! grammar <id> type=<kind> ... (see `Dumper::header`)
//!   <event code>: <event> [grammar=<grammar of the element>] -> <next grammar>
//! ```
//!
//! [`canonical_dump`] is a plain listing of the IR itself that always gives the same
//! text for the same schema.

use std::collections::HashMap;

use rexi_core::grammar::{GrammarTables, Production, Terminal};
use rexi_core::{DatatypeKind, DateTimeKind, ElemIdx, NONE, QNameId, RuleIdx, TypeIdx, WhiteSpace};

/// One grammar in the dump. Most are made up to match EXIficient's objects; only
/// `Rule` is a rule of rexi's tables as is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Node {
    /// The document grammar: `SD`, then `DocContent`
    /// ([EXI 8.5.1](https://www.w3.org/TR/exi/#informedDocGrammars)).
    Document,
    /// `SE` of each global element, then `SE(*)`; all go to `DocEnd`.
    DocContent,
    /// `ED` (end of document).
    DocEnd,
    /// The empty grammar reached after `EE` or `ED`.
    End,
    /// The one shared grammar after simple content: just `EE`.
    SimpleEnd,
    /// An element's own copy of its type's start rule.
    ElemStart(ElemIdx),
    /// A type's start rule (`Type_i,0`).
    TypeStart(TypeIdx),
    /// EXIficient's "element content" copy of a type's content start rule.
    Content(TypeIdx),
    /// The content start rule when it is reached again from inside the content
    /// (loops).
    ContentOrig(TypeIdx),
    /// Any other rule of a type grammar.
    Rule(RuleIdx),
    /// The fragment grammar: `SD`, then `FragmentContent`
    /// ([EXI 8.5.2](https://www.w3.org/TR/exi/#informedFragGrammars)).
    Fragment,
    /// `SE` of every element name (once each), then `SE(*)`, all looping back to
    /// itself, then `ED`.
    FragmentContent,
}

/// One production as the dump prints it: the event text, the grammar of the
/// element it starts (for `SE`), and the next grammar.
struct DProd {
    label: String,
    elem_grammar: Option<Node>,
    next: Node,
}

/// What `analyse` found out about one rule.
#[derive(Debug, Clone, Copy)]
struct RuleInfo {
    /// The type whose grammar the rule belongs to.
    type_idx: TypeIdx,
    /// Can be reached from the start rule through `AT(qname)` productions, so it
    /// is still in the start tag (attributes may come).
    start_tag: bool,
}

struct Dumper<'t, T: GrammarTables> {
    t: &'t T,
    rule_info: HashMap<RuleIdx, RuleInfo>,
    /// For each type: (start rule, content start rule). `(NONE, NONE)` when the
    /// type's grammar has no rules.
    bounds: Vec<(RuleIdx, RuleIdx)>,
    /// The grammar number given to each node.
    ids: HashMap<Node, usize>,
    /// Nodes in the order they were numbered.
    order: Vec<Node>,
}

fn is_attribute(term: Terminal) -> bool {
    matches!(term, Terminal::At(..) | Terminal::AtNs(_) | Terminal::AtAny)
}

/// Formats grammar tables like the EXIficient oracle dumps. See the module docs for
/// the layout.
pub fn dump(t: &impl GrammarTables) -> String {
    let mut d = Dumper {
        t,
        rule_info: HashMap::new(),
        bounds: Vec::new(),
        ids: HashMap::new(),
        order: Vec::new(),
    };
    d.analyse();
    d.number();
    d.render()
}

impl<T: GrammarTables> Dumper<'_, T> {
    /// For each type, finds the start rule and the content start rule, and marks
    /// which rules are still in the start tag.
    ///
    /// The "attribute chain" is every rule reached from the start rule through
    /// `AT(qname)` productions (a breadth-first walk). The content start rule is the
    /// first rule in that chain with no `AT(qname)` production. If there is none,
    /// it is the start rule.
    fn analyse(&mut self) {
        for (ti, td) in self.t.types().iter().enumerate() {
            let Some(r0) = self.t.grammar_start_rule(td.grammar) else {
                self.bounds.push((NONE, NONE));
                continue;
            };
            let grammar = self.t.grammars()[td.grammar as usize];
            for r in grammar.rules.range() {
                self.rule_info.insert(
                    r as RuleIdx,
                    RuleInfo {
                        type_idx: ti as TypeIdx,
                        start_tag: false,
                    },
                );
            }
            // Attribute chain: rules reached from r0 through AT(qname) productions.
            // `chain` doubles as the queue of the breadth-first walk.
            let mut chain = vec![r0];
            let mut i = 0;
            while i < chain.len() {
                let r = chain[i];
                i += 1;
                for p in self.t.rule_productions(r) {
                    if let Terminal::At(..) = p.terminal
                        && p.next != NONE
                        && !chain.contains(&p.next)
                    {
                        chain.push(p.next);
                    }
                }
            }
            let content_start = chain
                .iter()
                .copied()
                .find(|&r| {
                    !self
                        .t
                        .rule_productions(r)
                        .iter()
                        .any(|p| matches!(p.terminal, Terminal::At(..)))
                })
                .unwrap_or(r0);
            for &r in &chain {
                if let Some(info) = self.rule_info.get_mut(&r) {
                    info.start_tag = true;
                }
            }
            self.bounds.push((r0, content_start));
        }
    }

    /// The grammar node of element `e`: its own copy of the start rule when it is
    /// nillable or its type is typecastable (as EXIficient does), otherwise its
    /// type's shared start rule.
    fn elem_node(&self, e: ElemIdx) -> Node {
        let decl = self.t.elements()[e as usize];
        let castable = self
            .t
            .types()
            .get(decl.type_idx as usize)
            .is_some_and(|d| d.typecastable);
        if castable || decl.nillable {
            Node::ElemStart(e)
        } else {
            Node::TypeStart(decl.type_idx)
        }
    }

    fn qname_text(&self, q: QNameId) -> String {
        format!(
            "{{{}}}{}",
            self.t.uri_str(q.uri).unwrap_or(""),
            self.t.local_name_str(q).unwrap_or("")
        )
    }

    fn type_name(&self, ti: TypeIdx) -> String {
        match self.t.types().get(ti as usize) {
            Some(td) if !td.is_anonymous() => self.qname_text(td.qname),
            _ => "{}<anonymous>".to_string(),
        }
    }

    /// The datatype of type `ti`, printed as EXIficient prints it, such as
    /// `[STRING schema-type={ns}name id=exi_string ws=preserve]`.
    ///
    /// For n-bit integers, `upper` is `min + 2^bits - 1`. For enumerations, `bits`
    /// is the number of bits needed for an index into the values (at least one
    /// value is assumed).
    fn datatype_text(&self, ti: TypeIdx) -> String {
        let st = self.type_name(ti);
        let kind = self
            .t
            .types()
            .get(ti as usize)
            .and_then(|d| d.datatype)
            .unwrap_or(DatatypeKind::Untyped);
        match kind {
            DatatypeKind::String { .. } | DatatypeKind::Untyped => {
                let ws = match kind {
                    DatatypeKind::String { whitespace } => whitespace,
                    _ => WhiteSpace::Preserve,
                };
                format!(
                    "[STRING schema-type={st} id=exi_string ws={}]",
                    whitespace_name(ws)
                )
            }
            // Printed as a plain string, the same as JExi's dump.
            DatatypeKind::RestrictedString { whitespace } => format!(
                "[STRING schema-type={st} id=exi_string ws={}]",
                whitespace_name(whitespace)
            ),
            DatatypeKind::Boolean => {
                format!("[BOOLEAN schema-type={st} id=exi_boolean ws=collapse]")
            }
            DatatypeKind::BooleanPattern => {
                format!("[BOOLEAN_FACET schema-type={st} id=exi_boolean ws=collapse]")
            }
            DatatypeKind::Integer => {
                format!("[INTEGER schema-type={st} id=exi_integer ws=collapse]")
            }
            DatatypeKind::UnsignedInteger => {
                format!("[UNSIGNED_INTEGER schema-type={st} id=exi_integer ws=collapse]")
            }
            DatatypeKind::NBit { min, bits } => {
                let upper = (min as i128) + (1i128 << bits) - 1;
                format!(
                    "[NBIT_UNSIGNED_INTEGER schema-type={st} id=exi_integer lower={min} upper={upper} bits={bits} ws=collapse]"
                )
            }
            DatatypeKind::Decimal => {
                format!("[DECIMAL schema-type={st} id=exi_decimal ws=collapse]")
            }
            DatatypeKind::Float => format!("[FLOAT schema-type={st} id=exi_double ws=collapse]"),
            DatatypeKind::DateTime(k) => {
                format!(
                    "[DATETIME schema-type={st} id=exi_{} ws=collapse]",
                    datetime_name(k)
                )
            }
            DatatypeKind::Base64Binary => {
                format!("[BINARY_BASE64 schema-type={st} id=exi_base64Binary ws=collapse]")
            }
            DatatypeKind::HexBinary => {
                format!("[BINARY_HEX schema-type={st} id=exi_hexBinary ws=collapse]")
            }
            DatatypeKind::QName => format!("[QNAME schema-type={st} id=exi_qname ws=collapse]"),
            DatatypeKind::Enumeration { set, item } => {
                let values: Vec<&str> = (0..self
                    .t
                    .enum_sets()
                    .get(set as usize)
                    .map_or(0, |s| s.len as usize))
                    .filter_map(|i| {
                        let span = self.t.enum_sets()[set as usize];
                        self.t
                            .enum_values()
                            .get(span.start as usize + i)
                            .map(|&s| self.t.string(s))
                    })
                    .collect();
                let n = values.len().max(1);
                let bits = usize::BITS - (n - 1).leading_zeros();
                format!(
                    "[ENUMERATION schema-type={st} id={} bits={bits} values=[{}] ws=collapse]",
                    self.datatype_id(item),
                    values.join("|")
                )
            }
            DatatypeKind::List { item } => format!(
                "[LIST schema-type={st} id={} item={} ws=collapse]",
                self.datatype_id(item),
                self.datatype_text(item)
            ),
            // A kind added to rexi-core later, not known to this dump yet.
            other => format!("[{} schema-type={st}]", other.name()),
        }
    }

    /// EXIficient's id for the datatype of type `ti` (such as `exi_integer`).
    /// Enumerations and lists use the id of their item type.
    fn datatype_id(&self, ti: TypeIdx) -> &'static str {
        let kind = self
            .t
            .types()
            .get(ti as usize)
            .and_then(|d| d.datatype)
            .unwrap_or(DatatypeKind::Untyped);
        match kind {
            DatatypeKind::Boolean | DatatypeKind::BooleanPattern => "exi_boolean",
            DatatypeKind::Integer | DatatypeKind::UnsignedInteger | DatatypeKind::NBit { .. } => {
                "exi_integer"
            }
            DatatypeKind::Decimal => "exi_decimal",
            DatatypeKind::Float => "exi_double",
            DatatypeKind::DateTime(k) => match k {
                DateTimeKind::DateTime => "exi_dateTime",
                DateTimeKind::Date => "exi_date",
                DateTimeKind::Time => "exi_time",
                DateTimeKind::GYear => "exi_gYear",
                DateTimeKind::GYearMonth => "exi_gYearMonth",
                DateTimeKind::GMonth => "exi_gMonth",
                DateTimeKind::GMonthDay => "exi_gMonthDay",
                DateTimeKind::GDay => "exi_gDay",
            },
            DatatypeKind::Base64Binary => "exi_base64Binary",
            DatatypeKind::HexBinary => "exi_hexBinary",
            DatatypeKind::QName => "exi_qname",
            DatatypeKind::Enumeration { item, .. } | DatatypeKind::List { item } => {
                self.datatype_id(item)
            }
            _ => "exi_string",
        }
    }

    /// The text of an event: `SE {ns}name`, `AT {ns}name [datatype]`, `SE *`,
    /// `CH [datatype]`, and so on.
    fn terminal_label(&self, term: Terminal) -> String {
        match term {
            Terminal::Sd => "SD".into(),
            Terminal::Ed => "ED".into(),
            Terminal::Ee => "EE".into(),
            Terminal::Se(e) => {
                let q = self.t.elements()[e as usize].qname;
                format!("SE {}", self.qname_text(q))
            }
            Terminal::SeNs(u) => format!("SE {{{}}}*", self.t.uri_str(u).unwrap_or("")),
            Terminal::SeAny => "SE *".into(),
            Terminal::At(q, ti) => format!("AT {} {}", self.qname_text(q), self.datatype_text(ti)),
            Terminal::AtNs(u) => format!("AT {{{}}}*", self.t.uri_str(u).unwrap_or("")),
            Terminal::AtAny => "AT *".into(),
            Terminal::Ch(ti) => format!("CH {}", self.datatype_text(ti)),
            Terminal::ChUntyped => "CH *".into(),
            other => format!("{other:?}"),
        }
    }

    /// Maps the productions of rule `r` (belonging to type `ti`) onto dump nodes.
    ///
    /// `self_node` is the node being printed. When `content_node` is true, the
    /// node is an "element content" copy, so attribute productions are left out.
    ///
    /// The next node of each production is chosen like this, in order:
    ///
    /// - no next rule (`EE`): `End`;
    /// - after typed `CH`: the shared `SimpleEnd`;
    /// - an attribute that goes back to the start rule: `self_node` itself;
    /// - an attribute that goes to the content start rule: that rule as is;
    /// - anything else that goes to the start or content start rule:
    ///   `ContentOrig`;
    /// - otherwise: the rule as is.
    fn map_rule(&self, r: RuleIdx, ti: TypeIdx, self_node: Node, content_node: bool) -> Vec<DProd> {
        let (r0, r2) = self
            .bounds
            .get(ti as usize)
            .copied()
            .unwrap_or((NONE, NONE));
        let mut out = Vec::new();
        for p in self.t.rule_productions(r) {
            let attr = is_attribute(p.terminal);
            if content_node && attr {
                continue;
            }
            let elem_grammar = match p.terminal {
                Terminal::Se(e) => Some(self.elem_node(e)),
                _ => None,
            };
            let next = if p.next == NONE {
                Node::End
            } else if matches!(p.terminal, Terminal::Ch(_)) {
                Node::SimpleEnd
            } else if p.next == r0 && attr {
                self_node
            } else if p.next == r2 && attr {
                Node::Rule(r2)
            } else if p.next == r2 || p.next == r0 {
                Node::ContentOrig(ti)
            } else {
                Node::Rule(p.next)
            };
            out.push(DProd {
                label: self.terminal_label(p.terminal),
                elem_grammar,
                next,
            });
        }
        out
    }

    /// The productions to print for a node.
    fn productions(&self, node: Node) -> Vec<DProd> {
        match node {
            Node::Document => vec![DProd {
                label: "SD".into(),
                elem_grammar: None,
                next: Node::DocContent,
            }],
            Node::DocContent => {
                let mut v: Vec<DProd> = self
                    .t
                    .global_elements()
                    .iter()
                    .map(|&e| DProd {
                        label: self.terminal_label(Terminal::Se(e)),
                        elem_grammar: Some(self.elem_node(e)),
                        next: Node::DocEnd,
                    })
                    .collect();
                v.push(DProd {
                    label: "SE *".into(),
                    elem_grammar: None,
                    next: Node::DocEnd,
                });
                v
            }
            Node::DocEnd => vec![DProd {
                label: "ED".into(),
                elem_grammar: None,
                next: Node::End,
            }],
            Node::End => Vec::new(),
            Node::SimpleEnd => vec![DProd {
                label: "EE".into(),
                elem_grammar: None,
                next: Node::End,
            }],
            Node::ElemStart(e) => {
                let ti = self.t.elements()[e as usize].type_idx;
                let (r0, _) = self.bounds[ti as usize];
                self.map_rule(r0, ti, node, false)
            }
            Node::TypeStart(ti) => {
                let (r0, _) = self.bounds[ti as usize];
                self.map_rule(r0, ti, node, false)
            }
            Node::Content(ti) | Node::ContentOrig(ti) => {
                let (_, r2) = self.bounds[ti as usize];
                self.map_rule(r2, ti, node, true)
            }
            Node::Rule(r) => match self.rule_info.get(&r) {
                Some(info) => self.map_rule(r, info.type_idx, node, false),
                None => Vec::new(),
            },
            Node::Fragment => vec![DProd {
                label: "SD".into(),
                elem_grammar: None,
                next: Node::FragmentContent,
            }],
            Node::FragmentContent => {
                // Every element, global or local, sorted by local name, then by
                // URI. Each name is listed once, even if several declarations
                // share it.
                let mut elems: Vec<ElemIdx> = (0..self.t.elements().len() as ElemIdx).collect();
                elems.sort_by(|&a, &b| {
                    let (qa, qb) = (
                        self.t.elements()[a as usize].qname,
                        self.t.elements()[b as usize].qname,
                    );
                    self.t
                        .local_name_str(qa)
                        .cmp(&self.t.local_name_str(qb))
                        .then_with(|| self.t.uri_str(qa.uri).cmp(&self.t.uri_str(qb.uri)))
                });
                let mut v = Vec::new();
                let mut seen: Vec<QNameId> = Vec::new();
                for e in elems {
                    let q = self.t.elements()[e as usize].qname;
                    if seen.contains(&q) {
                        continue;
                    }
                    seen.push(q);
                    v.push(DProd {
                        label: self.terminal_label(Terminal::Se(e)),
                        elem_grammar: Some(self.elem_node(e)),
                        next: Node::FragmentContent,
                    });
                }
                v.push(DProd {
                    label: "SE *".into(),
                    elem_grammar: None,
                    next: Node::FragmentContent,
                });
                v.push(DProd {
                    label: "ED".into(),
                    elem_grammar: None,
                    next: Node::End,
                });
                v
            }
        }
    }

    /// The "element content" node linked from a start-tag node: the start rules
    /// of elements and types, and rules still in the start tag. `None` for any
    /// other node.
    fn element_content(&self, node: Node) -> Option<Node> {
        match node {
            Node::ElemStart(e) => Some(Node::Content(self.t.elements()[e as usize].type_idx)),
            Node::TypeStart(ti) => Some(Node::Content(ti)),
            Node::Rule(r) => {
                let info = self.rule_info.get(&r)?;
                info.start_tag.then_some(Node::Content(info.type_idx))
            }
            _ => None,
        }
    }

    /// Numbers `node` and everything reached from it, depth first. A node gets its
    /// number before its children. For each production, the element's grammar is
    /// visited first, then the next grammar. The element content node comes last.
    fn visit(&mut self, node: Node) {
        if self.ids.contains_key(&node) {
            return;
        }
        self.ids.insert(node, self.order.len());
        self.order.push(node);
        for p in self.productions(node) {
            if let Some(g) = p.elem_grammar {
                self.visit(g);
            }
            self.visit(p.next);
        }
        if let Some(ec) = self.element_content(node) {
            self.visit(ec);
        }
    }

    /// Numbers all nodes: everything reached from the document grammar first, then
    /// the start rule of every named type not yet reached, then the fragment
    /// grammar.
    fn number(&mut self) {
        self.visit(Node::Document);
        // Named types in string-table order (uri, then local name).
        let mut named: Vec<(QNameId, TypeIdx)> = self
            .t
            .types()
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.is_anonymous())
            .map(|(i, d)| (d.qname, i as TypeIdx))
            .collect();
        named.sort();
        for (_, ti) in named {
            self.visit(Node::TypeStart(ti));
        }
        self.visit(Node::Fragment);
    }

    /// The number of a node. `usize::MAX` if it was never numbered.
    fn id(&self, node: Node) -> usize {
        self.ids.get(&node).copied().unwrap_or(usize::MAX)
    }

    /// The header of a `grammar <id>` line, as EXIficient prints it: the grammar
    /// kind, then fields such as `nillable`, `typecastable`, `declared-attributes`
    /// (the count of `AT {...}` productions), `has-EE` (whether there is an `EE`
    /// production) and `element-content` (number of the element content node, or 0).
    fn header(&self, node: Node, prods: &[DProd]) -> String {
        let has_ee = prods.iter().any(|p| p.label == "EE");
        let declared = prods.iter().filter(|p| p.label.starts_with("AT {")).count();
        let ec = self.element_content(node).map(|n| self.id(n));
        match node {
            Node::Document => "type=DOCUMENT declared-attributes=0 label=\"Document\" has-EE=false".into(),
            Node::DocContent => {
                "type=SCHEMA_INFORMED_DOC_CONTENT declared-attributes=0 label=\"DocContent\" has-EE=false".into()
            }
            Node::DocEnd => "type=DOC_END declared-attributes=0 label=\"DocEnd\" has-EE=false".into(),
            Node::End => "type=SCHEMA_INFORMED_ELEMENT_CONTENT declared-attributes=0 label=\"<END>\" has-EE=false".into(),
            Node::Fragment => "type=FRAGMENT declared-attributes=0 label=\"Fragment\" has-EE=false".into(),
            Node::FragmentContent => {
                "type=SCHEMA_INFORMED_FRAGMENT_CONTENT declared-attributes=0 label=\"FragmentContent\" has-EE=false".into()
            }
            Node::ElemStart(e) => {
                let decl = self.t.elements()[e as usize];
                let castable = self.t.types().get(decl.type_idx as usize).is_some_and(|d| d.typecastable);
                format!(
                    "type=SCHEMA_INFORMED_FIRST_START_TAG_CONTENT nillable={} typecastable={} declared-attributes={declared} has-EE={has_ee} element-content={}",
                    decl.nillable,
                    castable,
                    ec.unwrap_or(0)
                )
            }
            Node::TypeStart(_) => format!(
                "type=SCHEMA_INFORMED_FIRST_START_TAG_CONTENT nillable=false typecastable=false declared-attributes={declared} has-EE={has_ee} element-content={}",
                ec.unwrap_or(0)
            ),
            Node::Rule(_) if ec.is_some() => format!(
                "type=SCHEMA_INFORMED_START_TAG_CONTENT declared-attributes={declared} has-EE={has_ee} element-content={}",
                ec.unwrap_or(0)
            ),
            _ => format!("type=SCHEMA_INFORMED_ELEMENT_CONTENT declared-attributes=0 has-EE={has_ee}"),
        }
    }

    /// Writes the dump text. See the module docs for the layout.
    fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# rexi grammar dump\n");
        out.push_str("schema-informed true\n");
        match self.t.schema_id() {
            Some(id) => out.push_str(&format!("schema-id {id}\n")),
            None => out.push_str("schema-id <none>\n"),
        }
        out.push_str(&format!("uri-contexts {}\n", self.t.uris().len()));
        out.push_str(&format!("qname-contexts {}\n", self.t.local_names().len()));
        out.push_str(
            "\n# string table (Appendix D initial uri / prefix / local-name partitions)\n",
        );
        for (ui, u) in self.t.uris().iter().enumerate() {
            let prefixes: Vec<String> = u
                .prefixes
                .range()
                .filter_map(|i| {
                    self.t
                        .prefixes()
                        .get(i)
                        .map(|&s| format!("\"{}\"", self.t.string(s)))
                })
                .collect();
            out.push_str(&format!(
                "uri {ui} \"{}\" prefixes=[{}]\n",
                self.t.string(u.uri),
                prefixes.join(",")
            ));
            for (li, i) in u.local_names.range().enumerate() {
                let Some(&s) = self.t.local_names().get(i) else {
                    continue;
                };
                let q = QNameId::new(ui as u32, li as u32);
                let mut line = format!("  local {li} \"{}\"", self.t.string(s));
                if let Some(ti) = self.t.types().iter().position(|d| d.qname == q) {
                    line.push_str(&format!(
                        " type-grammar={}",
                        self.id(Node::TypeStart(ti as TypeIdx))
                    ));
                }
                if let Some(e) = self.t.find_global_element(q) {
                    line.push_str(&format!(" global-element={}", self.id(self.elem_node(e))));
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
        out.push_str("\n# grammars\n");
        out.push_str(&format!("document-grammar {}\n", self.id(Node::Document)));
        out.push_str(&format!("fragment-grammar {}\n", self.id(Node::Fragment)));
        for (id, &node) in self.order.iter().enumerate() {
            let prods = self.productions(node);
            out.push_str(&format!("grammar {id} {}\n", self.header(node, &prods)));
            for (i, p) in prods.iter().enumerate() {
                let g = p
                    .elem_grammar
                    .map(|n| format!(" grammar={}", self.id(n)))
                    .unwrap_or_default();
                out.push_str(&format!("  {i}: {}{g} -> {}\n", p.label, self.id(p.next)));
            }
        }
        out
    }
}

fn whitespace_name(ws: WhiteSpace) -> &'static str {
    match ws {
        WhiteSpace::Preserve => "preserve",
        WhiteSpace::Replace => "replace",
        WhiteSpace::Collapse => "collapse",
    }
}

fn datetime_name(k: DateTimeKind) -> &'static str {
    match k {
        DateTimeKind::DateTime => "dateTime",
        DateTimeKind::Date => "date",
        DateTimeKind::Time => "time",
        DateTimeKind::GYear => "gYear",
        DateTimeKind::GYearMonth => "gYearMonth",
        DateTimeKind::GMonth => "gMonth",
        DateTimeKind::GMonthDay => "gMonthDay",
        DateTimeKind::GDay => "gDay",
    }
}

/// A plain listing of the IR: the element table, the list of global elements, then
/// every type with its two grammars (`grammar` and `empty`). Rules are numbered
/// from 0 inside each grammar, and `END` means no next rule. It always gives the
/// same text for the same schema.
pub fn canonical_dump(t: &impl GrammarTables) -> String {
    let d = Dumper {
        t,
        rule_info: HashMap::new(),
        bounds: Vec::new(),
        ids: HashMap::new(),
        order: Vec::new(),
    };
    let mut out = String::new();
    for (ei, e) in t.elements().iter().enumerate() {
        out.push_str(&format!(
            "element {ei} {} type={} nillable={} global={}\n",
            d.qname_text(e.qname),
            e.type_idx,
            e.nillable,
            e.global
        ));
    }
    out.push_str(&format!(
        "global-elements [{}]\n",
        t.global_elements()
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(",")
    ));
    for (ti, td) in t.types().iter().enumerate() {
        out.push_str(&format!(
            "type {ti} {} typecastable={} datatype={}\n",
            d.type_name(ti as TypeIdx),
            td.typecastable,
            td.datatype.map_or("none".to_string(), |k| format!("{k:?}"))
        ));
        for (label, g) in [("grammar", td.grammar), ("empty", td.empty_grammar)] {
            let Some(grammar) = t.grammars().get(g as usize) else {
                continue;
            };
            let start = grammar.rules.start;
            for r in grammar.rules.range() {
                let prods: Vec<String> = t
                    .rule_productions(r as RuleIdx)
                    .iter()
                    .map(|p: &Production| {
                        let next = if p.next == NONE {
                            "END".to_string()
                        } else {
                            (p.next - start).to_string()
                        };
                        format!("{} -> {next}", d.terminal_label(p.terminal))
                    })
                    .collect();
                out.push_str(&format!(
                    "  {label} rule {}: {}\n",
                    r as u32 - start,
                    prods.join(" | ")
                ));
            }
        }
    }
    out
}
