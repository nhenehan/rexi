//! Event code assignment
//! ([EXI 8.5.4.3](https://www.w3.org/TR/exi/#eventCodeAssignment)). The event code
//! is the number written in the stream to say which production was used. This
//! module sorts the productions of every rule so that the place of a production in
//! its rule is its event code.

use rexi_core::grammar::Terminal;
use rexi_core::{QNameId, UriId};

use super::{PProd, Proto};

/// Looks up the strings behind qname and URI ids. Needed to sort `AT` productions
/// by name.
pub trait QNameNames {
    /// The local name behind a qname id.
    fn local_name(&self, q: QNameId) -> &str;
    /// The URI behind a URI id.
    fn uri(&self, u: UriId) -> &str;
}

/// Sorts the productions of each rule into event-code order.
///
/// The order the spec requires is:
///
/// 1. `AT(qname)`, sorted by local name, then by URI;
/// 2. `AT(uri:*)`, sorted by URI;
/// 3. `AT(*)`;
/// 4. `SE(qname)`, in schema order;
/// 5. `SE(uri:*)`, in schema order;
/// 6. `SE(*)`;
/// 7. `EE`;
/// 8. `CH`.
///
/// Any other terminal comes after these, then bare non-terminals (which should
/// not be left after normalizing).
pub fn assign(proto: &mut Proto, names: &impl QNameNames) {
    for rule in &mut proto.rules {
        rule.prods.sort_by_cached_key(|p| key(p, names));
    }
}

fn key(p: &PProd, names: &impl QNameNames) -> (u8, String, String, u32) {
    match *p {
        PProd::Term {
            terminal, order, ..
        } => match terminal {
            Terminal::At(q, _) => (
                0,
                names.local_name(q).to_string(),
                names.uri(q.uri).to_string(),
                0,
            ),
            Terminal::AtNs(u) => (1, names.uri(u).to_string(), String::new(), 0),
            Terminal::AtAny => (2, String::new(), String::new(), 0),
            Terminal::Se(_) => (3, String::new(), String::new(), order),
            Terminal::SeNs(_) => (4, String::new(), String::new(), order),
            Terminal::SeAny => (5, String::new(), String::new(), 0),
            Terminal::Ee => (6, String::new(), String::new(), 0),
            Terminal::Ch(_) | Terminal::ChUntyped => (7, String::new(), String::new(), 0),
            _ => (8, String::new(), String::new(), order),
        },
        PProd::Ee => (6, String::new(), String::new(), 0),
        PProd::NonTerm(_) => (9, String::new(), String::new(), 0),
    }
}
