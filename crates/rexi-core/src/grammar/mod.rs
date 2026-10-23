//! EXI grammars ([EXI 8](https://www.w3.org/TR/exi/#grammars)): how the tables are
//! stored, and the code that walks them while encoding and decoding.
//!
//! A grammar is a set of rules. Each rule lists the events that may come next at one
//! point in the stream. Each choice in a rule is a production: an event (the
//! terminal) plus the rule to move to after it.

pub mod ir;
pub mod options_grammar;
pub mod owned;
#[doc(hidden)]
pub mod runtime;

pub use ir::{
    ElementDecl, GlobalAttribute, Grammar, GrammarSet, GrammarTables, Production, Rule, Span,
    Terminal, TypeDecl, UriEntry, well_known,
};
pub use owned::GrammarSetBuf;
