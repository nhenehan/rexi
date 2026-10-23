//! Turns a proto-grammar into a normalized grammar
//! ([EXI 8.5.4.2](https://www.w3.org/TR/exi/#normalizedGrammars)). There are three
//! steps:
//!
//! 1. Remove productions with no terminal (no event): each bare jump to another
//!    rule is replaced by that rule's productions
//!    ([8.5.4.2.1](https://www.w3.org/TR/exi/#eliminatingProductions)).
//! 2. Merge productions that have the same terminal. If they go to different rules,
//!    the merged production goes to a new "union" rule that has the productions of
//!    all of them ([8.5.4.2.2](https://www.w3.org/TR/exi/#eliminatingSymbols)).
//! 3. Drop the rules that can no longer be reached from the start rule.
//!
//! After this, a decoder that sees an event always knows the one rule to go to.

use std::collections::HashMap;

use super::{PProd, PRule, Proto};
use crate::error::{Result, SchemaError};

/// Normalizes a proto-grammar into a grammar where every production has exactly
/// one terminal. Rule 0 is the start rule, and only rules that can be reached are
/// kept.
#[cfg(test)]
pub fn normalize(proto: &Proto) -> Proto {
    match normalize_with(proto, None) {
        Ok(g) => g,
        // With no limit there is nothing to go over.
        Err(_) => Proto::ee(),
    }
}

/// How many productions normalizing may make, for each rule `max_grammar_size`
/// allows.
pub const PRODUCTIONS_PER_RULE: usize = 100;

/// [`normalize`], stopping with [`SchemaError::TooLarge`] once more than
/// `max_grammar_size` times [`PRODUCTIONS_PER_RULE`] productions have been made
/// (`None`: no limit).
///
/// The work can grow with the square of the grammar's size: in a chain of n
/// optional particles, each rule can skip to every later one, so expanding the
/// jumps makes about n * n / 2 productions. `maxOccurs="20000"` is only 40,000
/// rules but would need about 200 million productions, so the count is checked
/// as the productions are made.
pub fn normalize_with(proto: &Proto, max_grammar_size: Option<usize>) -> Result<Proto> {
    let n = proto.rules.len();
    if n == 0 {
        return Ok(Proto::ee());
    }
    let mut made = 0usize;
    let mut count = |k: usize| -> Result<()> {
        made = made.saturating_add(k);
        match max_grammar_size {
            Some(max) if made > max.saturating_mul(PRODUCTIONS_PER_RULE) => {
                Err(SchemaError::TooLarge { max })
            }
            _ => Ok(()),
        }
    };
    // Step 1 (8.5.4.2.1): replace each bare non-terminal with its productions.
    let mut rules: Vec<Vec<PProd>> = Vec::with_capacity(n);
    for j in 0..n {
        let prods = expand(proto, j);
        count(prods.len())?;
        rules.push(prods);
    }

    // Step 2 (8.5.4.2.2): merge duplicate terminals through union non-terminals.
    //
    // `members[r]` is the set of original rules that rule `r` stands for: just
    // `{r}` for an original rule, or several for a union rule. `unions` maps each
    // set to its union rule, so the same union is made only once. Since a set
    // always holds original rules (never union rules), a union of unions is
    // flattened to one set.
    //
    // The loop also visits union rules added while it runs (`rules` grows), so
    // duplicates inside a new union rule get merged as well. When two productions
    // merge, the smaller `order` (earlier in the schema) is kept. Duplicate `EE`s
    // are kept only once.
    let mut members: Vec<RuleSet> = (0..n).map(|i| RuleSet::one(n, i)).collect();
    let mut unions: HashMap<RuleSet, usize> = HashMap::new();
    // A union rule gets its productions only when the loop reaches it. By then
    // every original rule (they all come first) is merged, so the union copies a
    // few merged productions per member instead of the raw ones. Copying the raw
    // ones made schemas like `maxOccurs="100"` take seconds (found by fuzzing).
    let mut filled: Vec<bool> = vec![true; n];
    let mut i = 0;
    while i < rules.len() {
        if !filled[i] {
            let prods: Vec<PProd> = members[i]
                .iter()
                .flat_map(|m| rules[m].iter().copied())
                .collect();
            count(prods.len())?;
            rules[i] = prods;
            filled[i] = true;
        }
        // Group the productions by terminal, in the order each terminal first
        // appears. `targets[k]` collects the original rules that `out[k]` may go to.
        let mut out: Vec<PProd> = Vec::with_capacity(rules[i].len());
        let mut targets: Vec<RuleSet> = Vec::with_capacity(rules[i].len());
        for &p in &rules[i] {
            match p {
                PProd::Ee => {
                    if !out.contains(&PProd::Ee) {
                        out.push(PProd::Ee);
                        targets.push(RuleSet::empty(n));
                    }
                }
                PProd::Term {
                    terminal,
                    next,
                    order,
                } => {
                    let existing = out.iter().position(
                        |q| matches!(q, PProd::Term { terminal: t, .. } if *t == terminal),
                    );
                    match existing {
                        Some(pos) => {
                            if let PProd::Term { order: o2, .. } = &mut out[pos] {
                                *o2 = (*o2).min(order);
                            }
                            targets[pos].union_with(&members[next]);
                        }
                        None => {
                            out.push(p);
                            targets.push(members[next].clone());
                        }
                    }
                }
                PProd::NonTerm(_) => {}
            }
        }
        // Each terminal goes to the one rule that stands for all its targets.
        for (p, set) in out.iter_mut().zip(targets) {
            if let PProd::Term { next, .. } = p
                && set.has_two_or_more()
            {
                *next = union_rule(set, &mut rules, &mut members, &mut unions, &mut filled);
            }
        }
        rules[i] = out;
        i += 1;
    }

    // Step 3: keep only rules that can be reached from rule 0. This is a
    // breadth-first walk (`order` is used as the queue), and rules are given new
    // numbers in the order they are visited.
    let mut map: HashMap<usize, usize> = HashMap::new();
    let mut order: Vec<usize> = Vec::new();
    let mut stack = vec![0usize];
    map.insert(0, 0);
    order.push(0);
    let mut queue_pos = 0;
    while queue_pos < order.len() {
        let r = order[queue_pos];
        queue_pos += 1;
        for p in &rules[r] {
            if let PProd::Term { next, .. } = p
                && !map.contains_key(next)
            {
                map.insert(*next, order.len());
                order.push(*next);
            }
        }
    }
    stack.clear();
    let out_rules = order
        .iter()
        .map(|&r| PRule {
            prods: rules[r]
                .iter()
                .map(|p| match *p {
                    PProd::Term {
                        terminal,
                        next,
                        order,
                    } => PProd::Term {
                        terminal,
                        next: map.get(&next).copied().unwrap_or(0),
                        order,
                    },
                    other => other,
                })
                .collect(),
        })
        .collect();
    Ok(Proto { rules: out_rules })
}

/// The productions of rule `j`, with every bare non-terminal replaced by that
/// rule's productions, again and again
/// ([8.5.4.2.1](https://www.w3.org/TR/exi/#eliminatingProductions)). Each
/// non-terminal is expanded at most once, which also stops loops of jumps.
fn expand(proto: &Proto, j: usize) -> Vec<PProd> {
    let mut out = Vec::new();
    // One flag per rule, not a list to search: a chain of n optional particles
    // visits n rules from each of n rules.
    let mut visited = vec![false; proto.rules.len()];
    visited[j] = true;
    // A depth-first walk with its own stack of (rule, next production to look at),
    // not recursion: a long chain of jumps would otherwise use up the call stack.
    // The order of the output is the same as a recursive walk.
    let mut stack = vec![(j, 0usize)];
    while let Some((k, i)) = stack.pop() {
        let Some(p) = proto.rules[k].prods.get(i) else {
            continue;
        };
        stack.push((k, i + 1));
        match p {
            PProd::NonTerm(m) => {
                if !visited[*m] {
                    visited[*m] = true;
                    stack.push((*m, 0));
                }
            }
            other => out.push(*other),
        }
    }
    out
}

/// The rule that stands for the union of the original rules in `set` (two or
/// more). Makes it if it does not exist yet; its productions are filled in when
/// the loop of [`normalize`] reaches it.
fn union_rule(
    set: RuleSet,
    rules: &mut Vec<Vec<PProd>>,
    members: &mut Vec<RuleSet>,
    unions: &mut HashMap<RuleSet, usize>,
    filled: &mut Vec<bool>,
) -> usize {
    if let Some(&r) = unions.get(&set) {
        return r;
    }
    let idx = rules.len();
    rules.push(Vec::new());
    filled.push(false);
    members.push(set.clone());
    unions.insert(set, idx);
    idx
}

/// A set of original rules, one bit per rule. Union rules stand for such sets, and
/// a long chain of optional particles makes many large ones, so joining two sets
/// must be cheap: here it is a few word-wide ORs. Iteration is in ascending order.
#[derive(Clone, PartialEq, Eq, Hash)]
struct RuleSet(Vec<u64>);

impl RuleSet {
    fn empty(n: usize) -> Self {
        RuleSet(vec![0; n.div_ceil(64)])
    }

    fn one(n: usize, rule: usize) -> Self {
        let mut set = Self::empty(n);
        set.0[rule / 64] |= 1 << (rule % 64);
        set
    }

    fn union_with(&mut self, other: &RuleSet) {
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a |= b;
        }
    }

    fn has_two_or_more(&self) -> bool {
        self.0.iter().map(|w| w.count_ones()).sum::<u32>() > 1
    }

    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(k, &word)| {
            (0..64)
                .filter(move |b| word & (1 << b) != 0)
                .map(move |b| k * 64 + b)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexi_core::grammar::Terminal;

    fn se(e: u32, next: usize) -> PProd {
        PProd::Term {
            terminal: Terminal::Se(e),
            next,
            order: e,
        }
    }

    #[test]
    fn inlines_nonterminals_and_merges_duplicates() {
        // 0: NonTerm(1) | NonTerm(2); 1: SE(a) 3; 2: SE(a) 4 | EE; 3: EE; 4: SE(b) 3
        let proto = Proto {
            rules: vec![
                PRule {
                    prods: vec![PProd::NonTerm(1), PProd::NonTerm(2)],
                },
                PRule {
                    prods: vec![se(0, 3)],
                },
                PRule {
                    prods: vec![se(0, 4), PProd::Ee],
                },
                PRule {
                    prods: vec![PProd::Ee],
                },
                PRule {
                    prods: vec![se(1, 3)],
                },
            ],
        };
        let n = normalize(&proto);
        assert_eq!(n.rules[0].prods.len(), 2);
        assert!(matches!(
            n.rules[0].prods[0],
            PProd::Term {
                terminal: Terminal::Se(0),
                ..
            }
        ));
        assert_eq!(n.rules[0].prods[1], PProd::Ee);
        // The union of 3 and 4 accepts EE and SE(b).
        let PProd::Term { next, .. } = n.rules[0].prods[0] else {
            panic!()
        };
        let union = &n.rules[next].prods;
        assert!(union.contains(&PProd::Ee));
        assert!(union.iter().any(|p| matches!(
            p,
            PProd::Term {
                terminal: Terminal::Se(1),
                ..
            }
        )));
    }
}
