//! Grammar runtime shared by the encoder and decoder: the grammar stack (one frame
//! per open element), the current rule, and the event-code widths that strict
//! schema-informed grammars call for.
//!
//! # Event codes
//!
//! An event code says which production of the current rule was taken
//! ([EXI 6.1](https://www.w3.org/TR/exi/#eventCodes)). It has one to three parts.
//! Each part is written as an n-bit unsigned integer, where n = `ceil_log2(m)` and
//! m is the number of different values that part can have among its sibling codes.
//! A part with only one possible value takes 0 bits. With byte alignment, each part
//! takes whole bytes instead
//! ([EXI 6.2](https://www.w3.org/TR/exi/#encodingEventCodes)). In a strict
//! schema-informed grammar, almost every production has a one-part code: its
//! position in the rule.
//!
//! # Document grammar
//!
//! The document grammar ([EXI 8.5.1](https://www.w3.org/TR/exi/#informedDocGrammars))
//! is built from the global elements, not stored in the tables. `DocContent` has
//! one `SE(G_i)` per global element in sorted order, followed by `SE(*)`, so its
//! first-level width is `ceil_log2(n + 1)`.
//!
//! # Escapes for `xsi:type` and `xsi:nil`
//!
//! Second-level escapes in strict mode
//! ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)) exist only
//! in an element's first start-tag rule: `AT(xsi:type)` when the type is
//! typecastable, and `AT(xsi:nil)` when the element is nillable. They get two-part
//! codes `n.0` and `n.1`, where n is one past the last one-part code. So the first
//! part n is the "escape" value, and the second part picks which attribute.
//!
//! Both EXIficient and exip work out the first-level width as
//! `ceil_log2(productions + (escapes > 0))` on that rule and
//! `ceil_log2(productions)` everywhere else, and this runtime does the same.
//!
//! Example: a start rule with 3 productions, on a typecastable and nillable
//! element, has a 2-bit first part (`ceil_log2(4)`). Code 3 is the escape, and then
//! a 1-bit second part follows: 0 for `xsi:type`, 1 for `xsi:nil`.

use crate::grammar::ir::{GrammarTables, Production, Terminal};
use crate::ids::{ElemIdx, QNameId, RuleIdx, TypeIdx};
use crate::io::ceil_log2;
use crate::prelude::*;

/// Where the codec is at document level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    /// Before `SD`.
    Document,
    /// Inside the document, waiting for the root element (or inside it, see the
    /// stack).
    DocContent,
    /// After the root element's `EE`, expecting `ED`.
    DocEnd,
    /// After `ED`.
    Done,
}

/// One open element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Current rule.
    pub rule: RuleIdx,
    /// The start rule of the grammar this element is being evaluated with.
    pub start_rule: RuleIdx,
    /// Element qname: picks the local value partition (the per-name string list)
    /// that `CH` values are looked up in
    /// ([EXI 7.3.3](https://www.w3.org/TR/exi/#encodingOptimizedForMisses)).
    pub context: QNameId,
    /// The element declaration this frame is for.
    pub elem: ElemIdx,
    /// The type the element is evaluated with (changed by `xsi:type`).
    pub type_idx: TypeIdx,
    /// Whether `xsi:nil` is allowed on this element.
    pub nillable: bool,
    /// Whether `xsi:type` is allowed on this element.
    pub typecastable: bool,
    /// `xsi:nil="true"` was seen: the element now uses its `TypeEmpty` grammar
    /// (attributes, then `EE`).
    pub nil: bool,
    /// `xml:space` on this element: `Some(true)` for `preserve`, `Some(false)` for
    /// `default`, `None` when not set here (then it comes from the parent). Used by
    /// the encoder only.
    pub space_preserve: Option<bool>,
}

impl Frame {
    /// Number of second-level escape productions in the current rule: 0, 1 or 2.
    /// They exist only in the start rule, and not after `xsi:nil="true"`.
    pub fn escape_count(&self) -> u32 {
        if self.rule == self.start_rule && !self.nil {
            u32::from(self.typecastable) + u32::from(self.nillable)
        } else {
            0
        }
    }

    /// Width in bits of the first-level event code in the current rule. The escapes
    /// share one extra first-level value, so they add 1 to the count at most.
    pub fn first_level_width(&self, tables: &impl GrammarTables) -> u8 {
        let n = tables.rule_productions(self.rule).len() + usize::from(self.escape_count() > 0);
        ceil_log2(n)
    }

    /// Width in bits of the second-level part after an escape: 0 bits when there is
    /// one escape, 1 bit when there are two.
    pub fn second_level_width(&self) -> u8 {
        ceil_log2(self.escape_count() as usize)
    }

    /// Second-level code of `AT(xsi:type)`, if allowed here. It is always 0, since
    /// `xsi:type` comes first.
    pub fn xsi_type_code(&self) -> Option<u32> {
        (self.escape_count() > 0 && self.typecastable).then_some(0)
    }

    /// Second-level code of `AT(xsi:nil)`, if allowed here: 1 when `xsi:type` is
    /// also allowed, else 0.
    pub fn xsi_nil_code(&self) -> Option<u32> {
        (self.escape_count() > 0 && self.nillable).then_some(u32::from(self.typecastable))
    }

    /// The escape code itself (first-level), if any escape exists. It is the number
    /// of productions, one past the last normal code.
    pub fn escape_code(&self, tables: &impl GrammarTables) -> Option<u64> {
        (self.escape_count() > 0).then(|| tables.rule_productions(self.rule).len() as u64)
    }
}

/// Finds the first production in `rule` whose terminal satisfies `pred`.
pub fn find_production(
    tables: &impl GrammarTables,
    rule: RuleIdx,
    pred: impl Fn(&Terminal) -> bool,
) -> Option<(usize, Production)> {
    tables
        .rule_productions(rule)
        .iter()
        .enumerate()
        .find(|(_, p)| pred(&p.terminal))
        .map(|(i, p)| (i, *p))
}

/// Readable list of the terminals a rule accepts, for error messages.
pub fn describe_rule(tables: &impl GrammarTables, rule: RuleIdx, frame: Option<&Frame>) -> String {
    let mut parts: Vec<String> = tables
        .rule_productions(rule)
        .iter()
        .map(|p| describe_terminal(tables, &p.terminal))
        .collect();
    if let Some(f) = frame {
        if f.xsi_type_code().is_some() {
            parts.push("AT(xsi:type)".into());
        }
        if f.xsi_nil_code().is_some() {
            parts.push("AT(xsi:nil)".into());
        }
    }
    format!("one of [{}]", parts.join(", "))
}

/// Readable form of one terminal, like `SE({uri}local)` or `AT(*)`, for error
/// messages.
pub fn describe_terminal(tables: &impl GrammarTables, t: &Terminal) -> String {
    let qn = |q: QNameId| {
        let uri = tables.uri_str(q.uri).unwrap_or("?");
        let local = tables.local_name_str(q).unwrap_or("?");
        if uri.is_empty() {
            local.to_string()
        } else {
            format!("{{{uri}}}{local}")
        }
    };
    match t {
        Terminal::Sd => "SD".into(),
        Terminal::Ed => "ED".into(),
        Terminal::Ee => "EE".into(),
        Terminal::Se(e) => format!(
            "SE({})",
            tables.element(*e).map(|d| qn(d.qname)).unwrap_or_default()
        ),
        Terminal::SeNs(u) => format!("SE({}:*)", tables.uri_str(*u).unwrap_or("?")),
        Terminal::SeAny => "SE(*)".into(),
        Terminal::At(q, _) => format!("AT({})", qn(*q)),
        Terminal::AtNs(u) => format!("AT({}:*)", tables.uri_str(*u).unwrap_or("?")),
        Terminal::AtAny => "AT(*)".into(),
        Terminal::Ch(_) => "CH".into(),
        Terminal::ChUntyped => "CH[untyped]".into(),
        other => format!("{other:?}"),
    }
}
