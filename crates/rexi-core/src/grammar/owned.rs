//! Grammar tables on the heap, built at run time (by `rexi-schema`), with the same
//! layout as [`GrammarSet`].

use super::ir::{
    ElementDecl, GlobalAttribute, Grammar, GrammarSet, GrammarTables, Production, Rule, Span,
    TypeDecl, UriEntry,
};
use crate::ids::{ElemIdx, StrIdx};
use crate::prelude::*;

/// Owned grammar tables. Each field holds the same data as the field of the same
/// name in [`GrammarSet`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrammarSetBuf {
    /// String pool. See [`GrammarSet`] for the meaning.
    pub strings: Vec<String>,
    /// URI partitions. See [`GrammarSet`] for the meaning.
    pub uris: Vec<UriEntry>,
    /// Prefix partitions. See [`GrammarSet`] for the meaning.
    pub prefixes: Vec<StrIdx>,
    /// Local-name partitions. See [`GrammarSet`] for the meaning.
    pub local_names: Vec<StrIdx>,
    /// Productions. See [`GrammarSet`] for the meaning.
    pub productions: Vec<Production>,
    /// Rules. See [`GrammarSet`] for the meaning.
    pub rules: Vec<Rule>,
    /// Grammars. See [`GrammarSet`] for the meaning.
    pub grammars: Vec<Grammar>,
    /// Element declarations. See [`GrammarSet`] for the meaning.
    pub elements: Vec<ElementDecl>,
    /// Global elements (the `DocContent` productions). See [`GrammarSet`] for the meaning.
    pub global_elements: Vec<ElemIdx>,
    /// Global attribute declarations. See [`GrammarSet`] for the meaning.
    pub global_attributes: Vec<GlobalAttribute>,
    /// Type definitions. See [`GrammarSet`] for the meaning.
    pub types: Vec<TypeDecl>,
    /// Enumeration sets. See [`GrammarSet`] for the meaning.
    pub enum_sets: Vec<Span>,
    /// Enumeration values. See [`GrammarSet`] for the meaning.
    pub enum_values: Vec<StrIdx>,
    /// Schema id, if any. See [`GrammarSet`] for the meaning.
    pub schema_id: Option<String>,
}

impl GrammarSetBuf {
    /// Interns a string (adds it to the pool only if it is not there yet) and
    /// returns its index. The lookup is a linear scan.
    pub fn intern(&mut self, s: &str) -> StrIdx {
        if let Some(i) = self.strings.iter().position(|x| x == s) {
            return StrIdx::new(i as u32);
        }
        self.strings.push(s.to_string());
        StrIdx::new((self.strings.len() - 1) as u32)
    }

    /// Gives the tables a `schemaId`, which a decoder checks the stream's against
    /// (see [`ExiError::SchemaIdMismatch`](crate::ExiError::SchemaIdMismatch)).
    ///
    /// # Examples
    ///
    /// ```
    /// use rexi_core::grammar::options_grammar::OPTIONS_GRAMMARS;
    /// use rexi_core::{GrammarSetBuf, GrammarTables};
    ///
    /// let tables = GrammarSetBuf::from_set(&OPTIONS_GRAMMARS).with_schema_id("urn:example:v1");
    /// assert_eq!(tables.schema_id(), Some("urn:example:v1"));
    /// ```
    pub fn with_schema_id(mut self, id: impl Into<String>) -> Self {
        self.schema_id = Some(id.into());
        self
    }

    /// Copies borrowed tables into an owned buffer.
    pub fn from_set(set: &GrammarSet<'_>) -> Self {
        Self {
            strings: set.strings.iter().map(|s| (*s).to_string()).collect(),
            uris: set.uris.to_vec(),
            prefixes: set.prefixes.to_vec(),
            local_names: set.local_names.to_vec(),
            productions: set.productions.to_vec(),
            rules: set.rules.to_vec(),
            grammars: set.grammars.to_vec(),
            elements: set.elements.to_vec(),
            global_elements: set.global_elements.to_vec(),
            global_attributes: set.global_attributes.to_vec(),
            types: set.types.to_vec(),
            enum_sets: set.enum_sets.to_vec(),
            enum_values: set.enum_values.to_vec(),
            schema_id: set.schema_id.map(str::to_string),
        }
    }
}

impl GrammarTables for GrammarSetBuf {
    fn string(&self, i: StrIdx) -> &str {
        self.strings.get(i.index()).map_or("", String::as_str)
    }
    fn uris(&self) -> &[UriEntry] {
        &self.uris
    }
    fn prefixes(&self) -> &[StrIdx] {
        &self.prefixes
    }
    fn local_names(&self) -> &[StrIdx] {
        &self.local_names
    }
    fn productions(&self) -> &[Production] {
        &self.productions
    }
    fn rules(&self) -> &[Rule] {
        &self.rules
    }
    fn grammars(&self) -> &[Grammar] {
        &self.grammars
    }
    fn elements(&self) -> &[ElementDecl] {
        &self.elements
    }
    fn global_elements(&self) -> &[ElemIdx] {
        &self.global_elements
    }
    fn global_attributes(&self) -> &[GlobalAttribute] {
        &self.global_attributes
    }
    fn types(&self) -> &[TypeDecl] {
        &self.types
    }
    fn enum_sets(&self) -> &[Span] {
        &self.enum_sets
    }
    fn enum_values(&self) -> &[StrIdx] {
        &self.enum_values
    }
    fn schema_id(&self) -> Option<&str> {
        self.schema_id.as_deref()
    }
}
