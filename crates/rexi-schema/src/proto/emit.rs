//! Builds every type grammar from the resolved model and packs the result into the
//! flat tables of a [`GrammarSetBuf`].
//!
//! For each type, the proto-grammars `Type_i` and `TypeEmpty_i` are built
//! ([EXI 8.5.4.1.3](https://www.w3.org/TR/exi/#typeGrammars)), then normalized
//! ([EXI 8.5.4.2](https://www.w3.org/TR/exi/#normalizedGrammars)), then their
//! productions are sorted into event-code order
//! ([EXI 8.5.4.3](https://www.w3.org/TR/exi/#eventCodeAssignment)). The tables also
//! get the string table, element declarations, global elements and attributes, and
//! enumeration value sets.

use std::collections::HashMap;

use rexi_core::grammar::{
    ElementDecl, GlobalAttribute, Grammar, GrammarSetBuf, Production, Rule, Span, Terminal,
    TypeDecl,
};
use rexi_core::{ElemIdx, GrammarIdx, NONE, QNameId, TypeIdx};

use super::string_table::StringTable;
use super::{
    PProd, Proto, all, attribute_use, choice, complex_type, element_term, particle, sequence,
    simple_content, wildcard_term, wildcard_terminals,
};
use crate::datatype_map::DatatypeMapper;
use crate::error::{Result, SchemaError};
use crate::xsd::model::{
    Compositor, Content, ElemId, Model, NsConstraint, Particle, Term, TypeId, TypeKind, Variety,
};

/// [`emit`], where one type's content grammar may have at most
/// `max_grammar_size` rules (`None`: no limit).
pub fn emit_with(model: &Model, max_grammar_size: Option<usize>) -> Result<GrammarSetBuf> {
    let table = StringTable::build(model);
    let mut buf = GrammarSetBuf::default();
    table.write_into(&mut buf);

    // Element declarations: one entry for each distinct (qname, type, nillable).
    // Local elements that match in all three share one entry. The entry is global if
    // any of the elements that share it is global. `elem_ir[i]` is the entry for
    // model element `i`.
    let mut elem_keys: HashMap<(QNameId, TypeIdx, bool), ElemIdx> = HashMap::new();
    let mut elem_ir: Vec<ElemIdx> = Vec::with_capacity(model.elements.len());
    for e in &model.elements {
        let qname = qname_of(&table, &e.qname.ns, &e.qname.local)?;
        let key = (qname, e.type_id as TypeIdx, e.nillable);
        let idx = match elem_keys.get(&key) {
            Some(&i) => {
                buf.elements[i as usize].global |= e.global;
                i
            }
            None => {
                let i = buf.elements.len() as ElemIdx;
                buf.elements.push(ElementDecl {
                    qname,
                    type_idx: e.type_id as TypeIdx,
                    nillable: e.nillable,
                    global: e.global,
                });
                elem_keys.insert(key, i);
                i
            }
        };
        elem_ir.push(idx);
    }

    // Global elements in the order used by DocContent (the document grammar,
    // https://www.w3.org/TR/exi/#informedDocGrammars): by local name, then by URI.
    let mut globals: Vec<ElemIdx> = buf
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| e.global)
        .map(|(i, _)| i as ElemIdx)
        .collect();
    globals.sort_by(|&a, &b| {
        let (qa, qb) = (
            buf.elements[a as usize].qname,
            buf.elements[b as usize].qname,
        );
        table
            .local_name(qa)
            .cmp(table.local_name(qb))
            .then_with(|| table.uri_str(qa.uri).cmp(table.uri_str(qb.uri)))
    });
    buf.global_elements = globals;

    for a in &model.attributes {
        if a.global {
            buf.global_attributes.push(GlobalAttribute {
                qname: qname_of(&table, &a.qname.ns, &a.qname.local)?,
                type_idx: a.type_id as TypeIdx,
            });
        }
    }

    // Typecastable: a type that has a direct named subtype, or is a union (the same
    // as EXIficient). In strict mode, only the grammars of these types get the
    // AT(xsi:type) production (https://www.w3.org/TR/exi/#addingProductionsStrict).
    // EXIficient looks through every type of the schema model, the XSD built-ins
    // included, for one whose base is this type ("has named sub-types"; "is a simple
    // type definition of which {variety} is union"). So xs:string, xs:int or
    // xs:decimal are typecastable through their built-in subtypes, and xs:double or
    // xs:boolean are not:
    // https://github.com/EXIficient/exificient-grammars/blob/9baa044237f649a65ae4190aed75081c75c86a16/src/main/java/com/siemens/ct/exi/grammars/XSDGrammarsBuilder.java#L1558-L1582
    let mut has_named_subtype = vec![false; model.types.len()];
    for t in &model.types {
        if t.name.is_some()
            && let Some(b) = t.base
        {
            has_named_subtype[b] = true;
        }
    }

    let mut mapper = DatatypeMapper::new(model);
    let mut builder = Builder {
        model,
        table: &table,
        elem_ir: &elem_ir,
        order: 0,
        max_rules: max_grammar_size,
    };
    for (t, def) in model.types.iter().enumerate() {
        let (full, empty) = builder.type_protos(t)?;
        let grammar = push_grammar(&mut buf, &table, full, max_grammar_size)?;
        let empty_grammar = push_grammar(&mut buf, &table, empty, max_grammar_size)?;
        let qname = match &def.name {
            Some(q) => qname_of(&table, &q.ns, &q.local)?,
            None => QNameId::new(NONE, NONE),
        };
        let is_union =
            matches!(&def.kind, TypeKind::Simple(s) if matches!(s.variety, Variety::Union { .. }));
        buf.types.push(TypeDecl {
            qname,
            grammar,
            empty_grammar,
            datatype: mapper.kind(t)?,
            typecastable: has_named_subtype[t] || is_union,
        });
    }

    for set in &mapper.enum_sets {
        let start = buf.enum_values.len() as u32;
        for v in set {
            let s = buf.intern(v);
            buf.enum_values.push(s);
        }
        buf.enum_sets.push(Span::new(start, set.len() as u32));
    }
    Ok(buf)
}

fn qname_of(table: &StringTable, ns: &str, local: &str) -> Result<QNameId> {
    table.qname(ns, local).ok_or_else(|| {
        SchemaError::Invalid(format!(
            "name {{{ns}}}{local} missing from the string table"
        ))
    })
}

/// Normalizes a grammar, assigns its event codes and adds it to the tables.
/// Rule numbers become absolute indices into `buf.rules`. `EE` has no next rule
/// (`NONE`). A bare non-terminal should not be left after normalizing; if one is,
/// it is written as `EE`.
fn push_grammar(
    buf: &mut GrammarSetBuf,
    table: &StringTable,
    proto: Proto,
    max_grammar_size: Option<usize>,
) -> Result<GrammarIdx> {
    let mut g = super::normalize::normalize_with(&proto, max_grammar_size)?;
    super::event_codes::assign(&mut g, table);
    let rules_start = buf.rules.len() as u32;
    for rule in &g.rules {
        let prods_start = buf.productions.len() as u32;
        for p in &rule.prods {
            buf.productions.push(match *p {
                PProd::Term { terminal, next, .. } => {
                    Production::new(terminal, rules_start + next as u32)
                }
                PProd::Ee => Production::new(Terminal::Ee, NONE),
                PProd::NonTerm(_) => Production::new(Terminal::Ee, NONE),
            });
        }
        buf.rules.push(Rule {
            productions: Span::new(prods_start, rule.prods.len() as u32),
        });
    }
    buf.grammars.push(Grammar {
        rules: Span::new(rules_start, g.rules.len() as u32),
    });
    Ok((buf.grammars.len() - 1) as GrammarIdx)
}

struct Builder<'a> {
    model: &'a Model,
    table: &'a StringTable,
    elem_ir: &'a [ElemIdx],
    /// Counter that gives each element and wildcard particle of the type being
    /// built its place in schema order (a depth-first walk, starting at 1).
    order: u32,
    /// The most rules one content grammar may have (`None`: no limit).
    max_rules: Option<usize>,
}

impl Builder<'_> {
    /// The `Type_i` and `TypeEmpty_i` proto-grammars of type `t`
    /// ([EXI 8.5.4.1.3](https://www.w3.org/TR/exi/#typeGrammars)).
    ///
    /// A simple type gets `CH [typed] EE` and an empty grammar that only ends. For a
    /// complex type, the attribute uses are sorted by local name, then by URI. An
    /// attribute wildcard of `##any` or `##other` becomes `AT(*)`; a list becomes
    /// one `AT(uri:*)` for each URI.
    fn type_protos(&mut self, t: TypeId) -> Result<(Proto, Proto)> {
        let def = &self.model.types[t];
        let ct = match &def.kind {
            TypeKind::Simple(_) => return Ok((simple_content(t as TypeIdx), Proto::ee())),
            TypeKind::Complex(c) => c,
        };
        let mut uses: Vec<(String, String, Proto)> = Vec::new();
        for u in &ct.attributes {
            let a = &self.model.attributes[u.attr];
            let qname = qname_of(self.table, &a.qname.ns, &a.qname.local)?;
            uses.push((
                a.qname.local.clone(),
                a.qname.ns.clone(),
                attribute_use(qname, a.type_id as TypeIdx, u.required),
            ));
        }
        uses.sort_by(|x, y| x.0.cmp(&y.0).then_with(|| x.1.cmp(&y.1)));
        let uses: Vec<Proto> = uses.into_iter().map(|u| u.2).collect();
        let wildcard: Vec<Terminal> = match &ct.any_attribute {
            None => Vec::new(),
            Some(NsConstraint::Any) | Some(NsConstraint::Other(_)) => vec![Terminal::AtAny],
            Some(NsConstraint::List(list)) => list
                .iter()
                .map(|u| self.uri_id(u).map(Terminal::AtNs))
                .collect::<Result<Vec<_>>>()?,
        };
        self.order = 0;
        let content = match &ct.content {
            Content::Empty => Proto::ee(),
            Content::Simple(s) => simple_content(*s as TypeIdx),
            Content::Particle(p) => self.particle(p)?,
        };
        Ok(complex_type(&uses, &wildcard, &content, ct.mixed))
    }

    fn uri_id(&self, uri: &str) -> Result<u32> {
        self.table.uri_id(uri).ok_or_else(|| {
            SchemaError::Invalid(format!("namespace '{uri}' missing from the string table"))
        })
    }

    fn next_order(&mut self) -> u32 {
        self.order += 1;
        self.order
    }

    /// The proto-grammar of a particle and, with recursion, of everything inside
    /// it. Element and wildcard terms take the next schema-order number as they are
    /// met. A wildcard of `##any` or `##other` becomes `SE(*)`.
    fn particle(&mut self, p: &Particle) -> Result<Proto> {
        let term = match &p.term {
            Term::Element(e) => {
                let order = self.next_order();
                element_term(&self.members(*e), order)
            }
            Term::Any(ns) => {
                let order = self.next_order();
                let uris = match ns {
                    NsConstraint::Any | NsConstraint::Other(_) => None,
                    NsConstraint::List(list) => Some(
                        list.iter()
                            .map(|u| self.uri_id(u))
                            .collect::<Result<Vec<_>>>()?,
                    ),
                };
                wildcard_term(&wildcard_terminals(uris.as_deref()), order)
            }
            Term::Group {
                compositor,
                particles,
            } => {
                let mut ps = Vec::with_capacity(particles.len());
                for q in particles {
                    ps.push(self.particle(q)?);
                }
                self.check_size(ps.iter().map(|g| g.rules.len()).sum())?;
                match compositor {
                    Compositor::Sequence => sequence(ps),
                    Compositor::Choice => choice(ps),
                    Compositor::All => all(ps),
                }
            }
        };
        // The particle is `max` copies of the term (or `min` and a loop when
        // unbounded), so check the size before making any of them.
        let copies = p.max.unwrap_or_else(|| p.min.saturating_add(1)).max(1) as usize;
        self.check_size(term.rules.len().saturating_mul(copies))?;
        Ok(particle(&term, p.min, p.max))
    }

    fn check_size(&self, rules: usize) -> Result<()> {
        match self.max_rules {
            Some(max) if rules > max => Err(SchemaError::TooLarge { max }),
            _ => Ok(()),
        }
    }

    /// The element plus its substitution group, sorted by local name, then by URI,
    /// as IR element indices. Only a global element can head a substitution group.
    /// Elements that share one IR entry are listed once.
    fn members(&self, e: ElemId) -> Vec<ElemIdx> {
        let mut ids = vec![e];
        if self.model.elements[e].global {
            ids.extend(self.model.substitution_members(e));
        }
        ids.sort_by(|&a, &b| {
            let (qa, qb) = (&self.model.elements[a].qname, &self.model.elements[b].qname);
            qa.local.cmp(&qb.local).then_with(|| qa.ns.cmp(&qb.ns))
        });
        let mut out: Vec<ElemIdx> = Vec::with_capacity(ids.len());
        for id in ids {
            let ir = self.elem_ir[id];
            if !out.contains(&ir) {
                out.push(ir);
            }
        }
        out
    }
}
