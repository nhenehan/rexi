//! EXI proto-grammars ([EXI 8.5.4.1](https://www.w3.org/TR/exi/#protoGrammars)).
//!
//! A grammar is a list of rules ("non-terminals"). Each rule is a list of
//! "productions". A production says: "on this event (the terminal), go to that
//! rule". A proto-grammar is a first draft of a grammar. Unlike the final form, a
//! production may be a bare jump to another rule with no event at all.
//! [`normalize`] removes those later
//! ([EXI 8.5.4.2](https://www.w3.org/TR/exi/#normalizedGrammars)).
//!
//! Here grammars are stored in vectors: a rule is referred to by its index, and
//! rule 0 is where the grammar starts. This module also has the functions that
//! build the proto-grammar of each schema part: element terms, wildcards,
//! particles, model groups, attribute uses and type grammars. Each one follows a
//! subsection of 8.5.4.1, linked on the function.
//!
//! Most of these grammars end with an `EE` production (end element). To join
//! grammars one after another, the `EE` of the first is replaced by a jump to the
//! start of the second ([`Proto::concat`]).

pub mod emit;
pub mod event_codes;
pub mod normalize;
pub mod string_table;

use rexi_core::grammar::Terminal;
use rexi_core::{ElemIdx, QNameId, TypeIdx, UriId};

/// A production of a proto-grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PProd {
    /// `LHS : Terminal NonTerminal`: on this event, go to rule `next`. `order` is
    /// the place in the schema of the particle that made an `SE` (start element)
    /// terminal. Event-code assignment uses it.
    Term {
        /// The event.
        terminal: Terminal,
        /// Index of the rule to go to after the event.
        next: usize,
        /// Place in the schema of the particle that made an `SE` terminal.
        order: u32,
    },
    /// `LHS : EE`: the element may end here.
    Ee,
    /// `LHS : NonTerminal`: a jump to another rule with no event. Only found in
    /// proto-grammars.
    NonTerm(usize),
}

/// One rule (non-terminal) of a proto-grammar: its list of productions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PRule {
    /// The productions of this rule. After event-code assignment, a production's
    /// place in this list is its event code.
    pub prods: Vec<PProd>,
}

/// A proto-grammar. Rule 0 is the start non-terminal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Proto {
    /// The rules; an index into this list names a rule.
    pub rules: Vec<PRule>,
}

impl Proto {
    /// The grammar that only ends: `G_0 : EE`.
    pub fn ee() -> Self {
        Proto {
            rules: vec![PRule {
                prods: vec![PProd::Ee],
            }],
        }
    }

    /// Adds the rules of `other` to the end, and returns the new index of its
    /// rule 0. Every rule index inside the copied rules is shifted by that amount,
    /// so they still point at the right rules.
    pub fn append(&mut self, other: &Proto) -> usize {
        let offset = self.rules.len();
        for r in &other.rules {
            let prods = r
                .prods
                .iter()
                .map(|p| match *p {
                    PProd::Term {
                        terminal,
                        next,
                        order,
                    } => PProd::Term {
                        terminal,
                        next: next + offset,
                        order,
                    },
                    PProd::Ee => PProd::Ee,
                    PProd::NonTerm(k) => PProd::NonTerm(k + offset),
                })
                .collect();
            self.rules.push(PRule { prods });
        }
        offset
    }

    /// Replaces every `EE` production with `NonTerm(target)`.
    pub fn replace_ee(&mut self, target: usize) {
        for r in &mut self.rules {
            for p in &mut r.prods {
                if *p == PProd::Ee {
                    *p = PProd::NonTerm(target);
                }
            }
        }
    }

    /// Grammar concatenation `self ⊕ right`
    /// ([EXI 8.5.4.1.1](https://www.w3.org/TR/exi/#grammarConcatOperator)): every
    /// `EE` of `self` becomes a jump to the start of `right`, so `right` follows
    /// wherever `self` could end.
    pub fn concat(mut self, right: &Proto) -> Proto {
        let offset = self.rules.len();
        self.replace_ee(offset);
        self.append(right);
        self
    }

    /// Many grammars joined in a row, `g_0 ⊕ g_1 ⊕ ...`: the same result as
    /// joining them one by one with [`Proto::concat`], but quicker. After a join,
    /// only the grammar added last still has `EE`s, so only its rules are scanned;
    /// the time then grows with the total size, not with its square. An empty
    /// list gives the grammar that only ends.
    pub fn concat_all(parts: impl IntoIterator<Item = Proto>) -> Proto {
        let mut it = parts.into_iter();
        let Some(mut acc) = it.next() else {
            return Proto::ee();
        };
        // The first rule of the grammar added last.
        let mut tail = 0;
        for g in it {
            let offset = acc.rules.len();
            for r in &mut acc.rules[tail..] {
                for p in &mut r.prods {
                    if *p == PProd::Ee {
                        *p = PProd::NonTerm(offset);
                    }
                }
            }
            acc.append(&g);
            tail = offset;
        }
        acc
    }

    fn rule_has_ee(&self, rule: usize) -> bool {
        self.rules[rule].prods.contains(&PProd::Ee)
    }
}

/// The grammar of an element term
/// ([EXI 8.5.4.1.6](https://www.w3.org/TR/exi/#elementTerms)):
/// `SE(qname_0) | ... | SE(qname_n-1)`, then `EE`. `members` is the element and its
/// substitution group (the elements that may appear in its place), sorted by
/// local name, then by URI.
pub fn element_term(members: &[ElemIdx], order: u32) -> Proto {
    let prods = members
        .iter()
        .map(|&e| PProd::Term {
            terminal: Terminal::Se(e),
            next: 1,
            order,
        })
        .collect();
    Proto {
        rules: vec![
            PRule { prods },
            PRule {
                prods: vec![PProd::Ee],
            },
        ],
    }
}

/// The grammar of a wildcard term
/// ([EXI 8.5.4.1.7](https://www.w3.org/TR/exi/#wildcardTerms)): `SE(*)`, or one
/// `SE(uri:*)` for each namespace, then `EE`.
pub fn wildcard_term(terminals: &[Terminal], order: u32) -> Proto {
    let prods = terminals
        .iter()
        .map(|&t| PProd::Term {
            terminal: t,
            next: 1,
            order,
        })
        .collect();
    Proto {
        rules: vec![
            PRule { prods },
            PRule {
                prods: vec![PProd::Ee],
            },
        ],
    }
}

/// The `SE(*)` or `SE(uri:*)` terminals for a wildcard's namespace constraint.
/// `None` means any namespace.
pub fn wildcard_terminals(uris: Option<&[UriId]>) -> Vec<Terminal> {
    match uris {
        None => vec![Terminal::SeAny],
        Some(list) => list.iter().map(|&u| Terminal::SeNs(u)).collect(),
    }
}

/// The grammar of a particle
/// ([EXI 8.5.4.1.5](https://www.w3.org/TR/exi/#particles)): copies of the term
/// grammar joined one after another, to allow `min` to `max` occurrences.
///
/// - With a fixed `max`, there are `max` copies. Every copy after the first `min`
///   also gets an `EE` at its start, so the content may stop there. `max` of 0
///   gives the grammar that only ends.
/// - With `max` unbounded, there are `min` copies, then one more copy whose `EE`s
///   jump back to its own start (a loop). That last copy also gets an `EE` at its
///   start, so the loop can stop.
pub fn particle(term: &Proto, min: u32, max: Option<u32>) -> Proto {
    match max {
        Some(max) => {
            if max == 0 {
                return Proto::ee();
            }
            Proto::concat_all((0..max).map(|i| {
                let mut g = term.clone();
                if i >= min && !g.rule_has_ee(0) {
                    g.rules[0].prods.push(PProd::Ee);
                }
                g
            }))
        }
        None => {
            let mut last = term.clone();
            last.replace_ee(0);
            if !last.rule_has_ee(0) {
                last.rules[0].prods.push(PProd::Ee);
            }
            Proto::concat_all((0..min).map(|_| term.clone()).chain(std::iter::once(last)))
        }
    }
}

/// The grammar of a sequence group
/// ([EXI 8.5.4.1.8.1](https://www.w3.org/TR/exi/#sequenceGroupTerms)): the
/// particle grammars joined in order. An empty sequence only ends.
pub fn sequence(particles: Vec<Proto>) -> Proto {
    Proto::concat_all(particles)
}

/// The grammar of a choice group
/// ([EXI 8.5.4.1.8.2](https://www.w3.org/TR/exi/#choiceGroupTerms)): a new start
/// rule with a jump to the start of each particle grammar. An empty choice only
/// ends.
pub fn choice(particles: Vec<Proto>) -> Proto {
    if particles.is_empty() {
        return Proto::ee();
    }
    let mut g = Proto {
        rules: vec![PRule::default()],
    };
    for p in &particles {
        let offset = g.append(p);
        g.rules[0].prods.push(PProd::NonTerm(offset));
    }
    g
}

/// The grammar of an `all` group
/// ([EXI 8.5.4.1.8.3](https://www.w3.org/TR/exi/#allGroupTerms)): a start rule
/// that may end, or jump to any particle. Every particle's `EE` loops back to the
/// group's rule 0, so the particles may come in any order.
pub fn all(particles: Vec<Proto>) -> Proto {
    let mut g = Proto {
        rules: vec![PRule {
            prods: vec![PProd::Ee],
        }],
    };
    for p in &particles {
        let offset = g.append(p);
        for r in &mut g.rules[offset..] {
            for prod in &mut r.prods {
                if *prod == PProd::Ee {
                    *prod = PProd::NonTerm(0);
                }
            }
        }
        g.rules[0].prods.push(PProd::NonTerm(offset));
    }
    g
}

/// The grammar of an attribute use
/// ([EXI 8.5.4.1.4](https://www.w3.org/TR/exi/#attributeUses)): `AT(qname)`, then
/// `EE`. When the attribute is optional, the start rule also has `EE`, so the
/// attribute can be skipped.
pub fn attribute_use(qname: QNameId, type_idx: TypeIdx, required: bool) -> Proto {
    let mut prods = vec![PProd::Term {
        terminal: Terminal::At(qname, type_idx),
        next: 1,
        order: 0,
    }];
    if !required {
        prods.push(PProd::Ee);
    }
    Proto {
        rules: vec![
            PRule { prods },
            PRule {
                prods: vec![PProd::Ee],
            },
        ],
    }
}

/// The content grammar of a simple type
/// ([EXI 8.5.4.1.3.1](https://www.w3.org/TR/exi/#simpleTypeGrammars)): one typed
/// `CH` (character data) event, then `EE`.
pub fn simple_content(type_idx: TypeIdx) -> Proto {
    Proto {
        rules: vec![
            PRule {
                prods: vec![PProd::Term {
                    terminal: Terminal::Ch(type_idx),
                    next: 1,
                    order: 0,
                }],
            },
            PRule {
                prods: vec![PProd::Ee],
            },
        ],
    }
}

/// Builds `Type_i` and `TypeEmpty_i` for a complex type
/// ([EXI 8.5.4.1.3.2](https://www.w3.org/TR/exi/#complexTypeGrammars)) from the
/// attribute use grammars (already sorted), the attribute wildcard terminals, and
/// the content grammar. Returns `(Type_i, TypeEmpty_i)`.
///
/// - With a wildcard, one more attribute grammar `G_n-1` (two rules, both `EE`)
///   is added. Then every attribute grammar gets the wildcard's `AT(*)` or
///   `AT(uri:*)` productions on its start rule, each looping back to that same
///   rule, so wildcard attributes may come between declared ones.
/// - With no attribute uses and no wildcard, the grammar that only ends is used.
/// - When the type is mixed, every rule of the content gets an untyped `CH`
///   production that stays on the same rule, so text may appear anywhere.
/// - `Type_i` is the attribute grammars followed by the content. `TypeEmpty_i` is
///   the attribute grammars followed by an empty content (used with `xsi:nil`).
pub fn complex_type(
    attribute_uses: &[Proto],
    wildcard: &[Terminal],
    content: &Proto,
    mixed: bool,
) -> (Proto, Proto) {
    let mut uses: Vec<Proto> = attribute_uses.to_vec();
    if !wildcard.is_empty() {
        uses.push(Proto {
            rules: vec![
                PRule {
                    prods: vec![PProd::Ee],
                },
                PRule {
                    prods: vec![PProd::Ee],
                },
            ],
        });
        for g in &mut uses {
            for &t in wildcard {
                g.rules[0].prods.push(PProd::Term {
                    terminal: t,
                    next: 0,
                    order: 0,
                });
            }
        }
    }
    if uses.is_empty() {
        uses.push(Proto::ee());
    }
    let mut content = content.clone();
    if mixed {
        for (j, r) in content.rules.iter_mut().enumerate() {
            r.prods.push(PProd::Term {
                terminal: Terminal::ChUntyped,
                next: j,
                order: 0,
            });
        }
    }
    let type_grammar = uses.iter().fold(None, |acc: Option<Proto>, g| match acc {
        None => Some(g.clone()),
        Some(a) => Some(a.concat(g)),
    });
    let type_grammar = type_grammar.unwrap_or_else(Proto::ee);
    let empty = type_grammar.clone().concat(&Proto::ee());
    let full = type_grammar.concat(&content);
    (full, empty)
}
