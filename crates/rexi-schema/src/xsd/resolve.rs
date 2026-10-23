//! Turns raw schema documents into a [`Model`].
//!
//! Three things happen here:
//!
//! - Names become indices. Each reference by QName to a type, element, attribute,
//!   group or attribute group is looked up and replaced by an index into the model.
//! - Groups (`xs:group`) and attribute groups (`xs:attributeGroup`) are copied in
//!   place wherever they are referenced, so the model has no group references.
//! - Derivations are merged. A type built by extension or restriction of a base type
//!   gets its final content and attributes worked out here, so later code never has
//!   to look at the base again.
//!
//! The rules followed are those of
//! [XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Complex_Type_Definitions).
//!
//! Types are resolved on demand. A named type is first given a placeholder slot,
//! then filled in the first time something needs it. `resolving` is a stack of the
//! types being worked on right now, so a type that (through its base chain) derives
//! from itself is caught as an error instead of looping forever.

use std::collections::{HashMap, HashSet};

use rexi_core::grammar::well_known::XSD_NS;

use super::model::*;
use super::raw::*;
use crate::builtin::{self, BUILTINS, Category};
use crate::error::{Result, SchemaError};

enum RawTypeRef<'a> {
    Simple(&'a RawSimpleType),
    Complex(&'a RawComplexType),
}

struct Resolver<'a> {
    model: Model,
    named_types: HashMap<QName, TypeId>,
    raw_types: HashMap<QName, RawTypeRef<'a>>,
    resolved: Vec<bool>,
    resolving: Vec<TypeId>,
    groups: HashMap<QName, &'a RawGroup>,
    attr_groups: HashMap<QName, &'a RawAttributeGroup>,
    raw_global_elements: HashMap<QName, &'a RawElement>,
    raw_global_attributes: HashMap<QName, &'a RawAttribute>,
    global_elements: HashMap<QName, ElemId>,
    global_attributes: HashMap<QName, AttrId>,
    /// The groups being expanded right now, outermost first. A group met again
    /// while it is on this list refers back to itself, which would never end.
    expanding_groups: Vec<QName>,
    /// Particles made so far for the type whose content is being resolved, and the
    /// most allowed (`max_grammar_size`), so group references that each expand
    /// several others cannot grow without end.
    particles: usize,
    max_particles: Option<usize>,
}

/// Resolves all loaded documents into one model, where one type's content may
/// expand to at most `max_grammar_size` particles once group references are
/// replaced (`None`: no limit).
///
/// The order is: built-in types, then register every named component of every
/// document, then resolve global elements, global attributes and finally every
/// named type. Last, the list of namespaces is built and sorted.
pub fn resolve_with(docs: &[RawDocument], max_grammar_size: Option<usize>) -> Result<Model> {
    let mut r = Resolver {
        model: Model::default(),
        named_types: HashMap::new(),
        raw_types: HashMap::new(),
        resolved: Vec::new(),
        resolving: Vec::new(),
        groups: HashMap::new(),
        attr_groups: HashMap::new(),
        raw_global_elements: HashMap::new(),
        raw_global_attributes: HashMap::new(),
        global_elements: HashMap::new(),
        global_attributes: HashMap::new(),
        expanding_groups: Vec::new(),
        particles: 0,
        max_particles: max_grammar_size,
    };
    r.install_builtins();
    r.register(docs)?;

    // Resolve global components in document order, then every named type.
    let global_elements: Vec<QName> = docs
        .iter()
        .flat_map(|d| {
            d.elements
                .iter()
                .filter_map(|e| e.name.as_ref().map(|n| QName::new(&d.target_ns, n)))
        })
        .collect();
    for q in global_elements {
        r.global_element(&q)?;
    }
    let global_attributes: Vec<QName> = docs
        .iter()
        .flat_map(|d| {
            d.attributes
                .iter()
                .filter_map(|a| a.name.as_ref().map(|n| QName::new(&d.target_ns, n)))
        })
        .collect();
    for q in global_attributes {
        r.global_attribute(&q)?;
    }
    let named: Vec<TypeId> = (builtin::COUNT..r.model.types.len()).collect();
    for id in named {
        r.resolve_named(id)?;
    }

    for d in docs {
        if !r.model.namespaces.contains(&d.target_ns) {
            r.model.namespaces.push(d.target_ns.clone());
        }
    }
    // Spec 8.5.4.1.3.2 (https://www.w3.org/TR/exi/#complexTypeGrammars): each URI in
    // the namespace list of an attribute wildcard is added to the URI partition of
    // the string table. The same is done for the URIs of xs:any wildcards
    // (https://www.w3.org/TR/exi/#wildcardTerms); those were collected while
    // particles were resolved. The wildcard stored on each complex type already
    // includes those from attribute groups and from the base type.
    let wildcard_uris: Vec<String> = r
        .model
        .types
        .iter()
        .filter_map(|t| match &t.kind {
            TypeKind::Complex(c) => match &c.any_attribute {
                Some(NsConstraint::List(list)) => Some(list.clone()),
                _ => None,
            },
            TypeKind::Simple(_) => None,
        })
        .flatten()
        .collect();
    for uri in wildcard_uris {
        if !r.model.namespaces.contains(&uri) {
            r.model.namespaces.push(uri);
        }
    }
    r.model.namespaces.sort();
    check_list_items(&r.model)?;
    Ok(r.model)
}

/// A list's item type must be a simple type that is not itself a list
/// ([XML Schema 1.0 Part 2](https://www.w3.org/TR/xmlschema-2/#cos-list-of-atomic)).
/// Item types are only given an index while types are resolved, so this is
/// checked once every type is known. It also catches a list whose item type is
/// the list itself, which a decoder would otherwise follow without end.
fn check_list_items(model: &Model) -> Result<()> {
    for t in &model.types {
        let TypeKind::Simple(SimpleType {
            variety: Variety::List { item },
            ..
        }) = &t.kind
        else {
            continue;
        };
        let item_ok = matches!(
            &model.types[*item].kind,
            TypeKind::Simple(s) if !matches!(s.variety, Variety::List { .. })
        );
        if !item_ok {
            let name = t
                .name
                .as_ref()
                .map_or("<anonymous>".to_string(), |n| n.to_string());
            return Err(SchemaError::Invalid(format!(
                "list type {name} has an item type that is a list or not simple"
            )));
        }
    }
    Ok(())
}

impl<'a> Resolver<'a> {
    /// Adds the built-in XML Schema types first, at the indices of `BUILTINS`.
    ///
    /// `xs:anyType` is a mixed complex type whose content is a sequence holding one
    /// `xs:any` wildcard (any namespace, zero or more times), and it allows any
    /// attribute. Built-in list types (such as `xs:NMTOKENS`) get the list variety;
    /// all other built-ins are atomic.
    fn install_builtins(&mut self) {
        for (i, b) in BUILTINS.iter().enumerate() {
            let name = QName::new(XSD_NS, b.name);
            let kind = match b.category {
                Category::AnyType => TypeKind::Complex(ComplexType {
                    mixed: true,
                    content: Content::Particle(Particle {
                        min: 1,
                        max: Some(1),
                        term: Term::Group {
                            compositor: Compositor::Sequence,
                            particles: vec![Particle {
                                min: 0,
                                max: None,
                                term: Term::Any(NsConstraint::Any),
                            }],
                        },
                    }),
                    attributes: Vec::new(),
                    any_attribute: Some(NsConstraint::Any),
                }),
                Category::List(item) => TypeKind::Simple(SimpleType {
                    variety: Variety::List { item },
                    facets: Facets::default(),
                    builtin: Some(i),
                }),
                _ => TypeKind::Simple(SimpleType {
                    variety: Variety::Atomic,
                    facets: Facets::default(),
                    builtin: Some(i),
                }),
            };
            self.named_types.insert(name.clone(), i);
            self.model.types.push(TypeDef {
                name: Some(name),
                base: b.base,
                kind,
            });
            self.resolved.push(true);
        }
    }

    /// Records every named type, group, attribute group, global element and global
    /// attribute of every document by its QName. Nothing is resolved yet. A name
    /// declared twice is an error.
    fn register(&mut self, docs: &'a [RawDocument]) -> Result<()> {
        for d in docs {
            for ct in &d.complex_types {
                if let Some(n) = &ct.name {
                    self.register_type(QName::new(&d.target_ns, n), RawTypeRef::Complex(ct))?;
                }
            }
            for st in &d.simple_types {
                if let Some(n) = &st.name {
                    self.register_type(QName::new(&d.target_ns, n), RawTypeRef::Simple(st))?;
                }
            }
            for g in &d.groups {
                let q = QName::new(&d.target_ns, &g.name);
                if self.groups.insert(q.clone(), g).is_some() {
                    return Err(SchemaError::Invalid(format!("duplicate xs:group {q}")));
                }
            }
            for g in &d.attribute_groups {
                let q = QName::new(&d.target_ns, &g.name);
                if self.attr_groups.insert(q.clone(), g).is_some() {
                    return Err(SchemaError::Invalid(format!(
                        "duplicate xs:attributeGroup {q}"
                    )));
                }
            }
            for e in &d.elements {
                if let Some(n) = &e.name {
                    let q = QName::new(&d.target_ns, n);
                    if self.raw_global_elements.insert(q.clone(), e).is_some() {
                        return Err(SchemaError::Invalid(format!(
                            "duplicate global element {q}"
                        )));
                    }
                }
            }
            for a in &d.attributes {
                if let Some(n) = &a.name {
                    let q = QName::new(&d.target_ns, n);
                    if self.raw_global_attributes.insert(q.clone(), a).is_some() {
                        return Err(SchemaError::Invalid(format!(
                            "duplicate global attribute {q}"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Gives a named type its index now, with a placeholder definition that is
    /// marked as not resolved. `resolve_named` fills it in later.
    fn register_type(&mut self, q: QName, raw: RawTypeRef<'a>) -> Result<()> {
        if self.named_types.contains_key(&q) {
            return Err(SchemaError::Invalid(format!("duplicate type {q}")));
        }
        let id = self.model.types.len();
        self.model.types.push(TypeDef {
            name: Some(q.clone()),
            base: None,
            kind: TypeKind::Simple(SimpleType {
                variety: Variety::Atomic,
                facets: Facets::default(),
                builtin: None,
            }),
        });
        self.resolved.push(false);
        self.named_types.insert(q.clone(), id);
        self.raw_types.insert(q, raw);
        Ok(())
    }

    /// The id of a named type, without resolving its content.
    fn type_id(&self, q: &QName) -> Result<TypeId> {
        self.named_types
            .get(q)
            .copied()
            .ok_or_else(|| SchemaError::Unresolved {
                kind: "type",
                qname: q.to_string(),
            })
    }

    /// The id of a named type, with its content resolved first. This is needed for
    /// base types, since a derived type copies parts of its base.
    fn resolved_type(&mut self, q: &QName) -> Result<TypeId> {
        let id = self.type_id(q)?;
        self.resolve_named(id)?;
        Ok(id)
    }

    /// Fills in the placeholder of a named type, if that has not been done yet.
    /// Fails if the type is already on the `resolving` stack, which means its
    /// derivation goes around in a circle.
    fn resolve_named(&mut self, id: TypeId) -> Result<()> {
        if self.resolved[id] {
            return Ok(());
        }
        let name = self.model.types[id]
            .name
            .clone()
            .unwrap_or(QName::new("", ""));
        if self.resolving.contains(&id) {
            return Err(SchemaError::Invalid(format!(
                "circular derivation of type {name}"
            )));
        }
        self.resolving.push(id);
        // A named type is resolved once; a loop back to it is caught by
        // `resolving`. So the groups being expanded outside do not count inside it.
        let outer = std::mem::take(&mut self.expanding_groups);
        let def = match self.raw_types.get(&name) {
            Some(RawTypeRef::Simple(st)) => self.resolve_simple(st, Some(name.clone())),
            Some(RawTypeRef::Complex(ct)) => self.resolve_complex(ct, Some(name.clone())),
            None => Err(SchemaError::Unresolved {
                kind: "type",
                qname: name.to_string(),
            }),
        };
        self.expanding_groups = outer;
        let def = def?;
        self.model.types[id] = def;
        self.resolved[id] = true;
        self.resolving.pop();
        Ok(())
    }

    /// Adds a type with no name to the end of the type list and returns its index.
    fn push_anonymous(&mut self, def: TypeDef) -> TypeId {
        self.model.types.push(def);
        self.resolved.push(true);
        self.model.types.len() - 1
    }

    fn anonymous_simple(&mut self, raw: &RawSimpleType) -> Result<TypeId> {
        let def = self.resolve_simple(raw, None)?;
        Ok(self.push_anonymous(def))
    }

    fn anonymous_type(&mut self, raw: &RawType) -> Result<TypeId> {
        let def = match raw {
            RawType::Simple(st) => self.resolve_simple(st, None)?,
            RawType::Complex(ct) => self.resolve_complex(ct, None)?,
        };
        Ok(self.push_anonymous(def))
    }

    /// Builds a simple type
    /// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Simple_Type_Definitions)).
    ///
    /// - A restriction keeps the variety (atomic, list or union) of its base and
    ///   adds its own facets. The base must be simple.
    /// - A list or union has `xs:anySimpleType` as its base and no facets of its own.
    ///
    /// List item types and union member types are only given an index here; they
    /// are not resolved first, since nothing is copied from them.
    fn resolve_simple(&mut self, raw: &RawSimpleType, name: Option<QName>) -> Result<TypeDef> {
        let (base, variety, facets) = match &raw.variety {
            RawVariety::Restriction {
                base,
                inline,
                facets,
            } => {
                let base_id = match (base, inline) {
                    (Some(b), _) => self.resolved_type(b)?,
                    (None, Some(inl)) => self.anonymous_simple(inl)?,
                    (None, None) => {
                        return Err(SchemaError::Invalid("restriction without base".into()));
                    }
                };
                let variety = match &self.model.types[base_id].kind {
                    TypeKind::Simple(s) => s.variety.clone(),
                    TypeKind::Complex(_) => {
                        return Err(SchemaError::Invalid(format!(
                            "simple type {} restricts complex type",
                            name.as_ref()
                                .map_or("<anonymous>".to_string(), |n| n.to_string())
                        )));
                    }
                };
                (base_id, variety, collect_facets(facets))
            }
            RawVariety::List { item, inline } => {
                let item_id = match (item, inline) {
                    (Some(i), _) => self.type_id(i)?,
                    (None, Some(inl)) => self.anonymous_simple(inl)?,
                    (None, None) => return Err(SchemaError::Invalid("list without item".into())),
                };
                (
                    builtin::ANY_SIMPLE_TYPE,
                    Variety::List { item: item_id },
                    Facets::default(),
                )
            }
            RawVariety::Union { members, inline } => {
                let mut ids = Vec::new();
                for m in members {
                    ids.push(self.type_id(m)?);
                }
                for inl in inline {
                    ids.push(self.anonymous_simple(inl)?);
                }
                (
                    builtin::ANY_SIMPLE_TYPE,
                    Variety::Union { members: ids },
                    Facets::default(),
                )
            }
        };
        Ok(TypeDef {
            name,
            base: Some(base),
            kind: TypeKind::Simple(SimpleType {
                variety,
                facets,
                builtin: None,
            }),
        })
    }

    /// The resolved base type of an `xs:complexContent` derivation, with a copy of
    /// its definition. The base must be a complex type.
    fn base_complex(&mut self, base: &QName) -> Result<(TypeId, ComplexType)> {
        let id = self.resolved_type(base)?;
        match &self.model.types[id].kind {
            TypeKind::Complex(c) => Ok((id, c.clone())),
            TypeKind::Simple(_) => Err(SchemaError::Invalid(format!(
                "complexContent derivation from simple type {base}"
            ))),
        }
    }

    /// Builds a complex type, merging in its base type
    /// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Complex_Type_Definitions)).
    ///
    /// By kind of derivation:
    ///
    /// - **None** (content model written directly): the base is `xs:anyType`, but
    ///   nothing is inherited from it, not even its attribute wildcard.
    /// - **complexContent extension**: the content is the base's particle followed
    ///   by the new particle, as a sequence. The type is mixed if either is mixed.
    ///   Attributes and attribute wildcards of both are merged.
    /// - **complexContent restriction**: the content is the new particle only.
    ///   Attribute uses start from the base's and are then changed by the new ones.
    ///   The base's attribute wildcard is not kept.
    /// - **simpleContent extension**: the content is the base's simple type (the base
    ///   itself if it is simple). Attributes and wildcards of both are merged.
    /// - **simpleContent restriction**: like extension, but an inline simple type
    ///   replaces the base's simple type, and any facets create a new anonymous
    ///   simple type derived from it. The base's attribute wildcard is not kept.
    ///
    /// With no particle, the content is empty, unless the type is mixed: then it
    /// is an empty sequence, so text is still allowed.
    fn resolve_complex(&mut self, raw: &RawComplexType, name: Option<QName>) -> Result<TypeDef> {
        let mut mixed = raw.mixed;
        let own_attrs = self.attr_items(&raw.attributes, raw.any_attribute.clone())?;
        let (base, content, attributes, any_attribute) = match &raw.derivation {
            RawDerivation::None { particle } => {
                let content = match particle {
                    Some(p) => Content::Particle(self.content_particle(p)?),
                    None if mixed => Content::Particle(empty_sequence()),
                    None => Content::Empty,
                };
                let (attrs, any) =
                    merge_attributes(&self.model, Vec::new(), own_attrs.0, None, own_attrs.1);
                (builtin::ANY_TYPE, content, attrs, any)
            }
            RawDerivation::ComplexExtension { base, particle } => {
                let (base_id, bc) = self.base_complex(base)?;
                mixed |= bc.mixed;
                let own = particle
                    .as_ref()
                    .map(|p| self.content_particle(p))
                    .transpose()?;
                let content = match (bc.content, own) {
                    (Content::Particle(bp), Some(p)) => Content::Particle(Particle {
                        min: 1,
                        max: Some(1),
                        term: Term::Group {
                            compositor: Compositor::Sequence,
                            particles: vec![bp, p],
                        },
                    }),
                    (Content::Particle(bp), None) => Content::Particle(bp),
                    (Content::Empty, Some(p)) => Content::Particle(p),
                    (Content::Empty, None) if mixed => Content::Particle(empty_sequence()),
                    (Content::Empty, None) => Content::Empty,
                    (Content::Simple(t), None) => Content::Simple(t),
                    (Content::Simple(_), Some(_)) => {
                        return Err(SchemaError::Invalid(format!(
                            "extension of simple-content type {base} adds a content model"
                        )));
                    }
                };
                let (attrs, any) = merge_attributes(
                    &self.model,
                    bc.attributes,
                    own_attrs.0,
                    bc.any_attribute,
                    own_attrs.1,
                );
                (base_id, content, attrs, any)
            }
            RawDerivation::ComplexRestriction { base, particle } => {
                let (base_id, bc) = self.base_complex(base)?;
                let content = match particle {
                    Some(p) => Content::Particle(self.content_particle(p)?),
                    None if mixed => Content::Particle(empty_sequence()),
                    None => Content::Empty,
                };
                let (attrs, any) =
                    merge_attributes(&self.model, bc.attributes, own_attrs.0, None, own_attrs.1);
                (base_id, content, attrs, any)
            }
            RawDerivation::SimpleExtension { base } => {
                let base_id = self.resolved_type(base)?;
                let (content, base_attrs, base_any) = match &self.model.types[base_id].kind {
                    TypeKind::Simple(_) => (Content::Simple(base_id), Vec::new(), None),
                    TypeKind::Complex(c) => match &c.content {
                        Content::Simple(t) => (
                            Content::Simple(*t),
                            c.attributes.clone(),
                            c.any_attribute.clone(),
                        ),
                        _ => {
                            return Err(SchemaError::Invalid(format!(
                                "simpleContent extension of {base} which has no simple content"
                            )));
                        }
                    },
                };
                let (attrs, any) =
                    merge_attributes(&self.model, base_attrs, own_attrs.0, base_any, own_attrs.1);
                (base_id, content, attrs, any)
            }
            RawDerivation::SimpleRestriction {
                base,
                facets,
                inline,
            } => {
                let base_id = self.resolved_type(base)?;
                let (simple, base_attrs) = match &self.model.types[base_id].kind {
                    TypeKind::Simple(_) => (base_id, Vec::new()),
                    TypeKind::Complex(c) => match &c.content {
                        Content::Simple(t) => (*t, c.attributes.clone()),
                        _ => {
                            return Err(SchemaError::Invalid(format!(
                                "simpleContent restriction of {base} which has no simple content"
                            )));
                        }
                    },
                };
                let simple = match inline {
                    Some(inl) => self.anonymous_simple(inl)?,
                    None => simple,
                };
                let simple = if facets.is_empty() {
                    simple
                } else {
                    let variety = self.model.types[simple]
                        .simple()
                        .map(|s| s.variety.clone())
                        .unwrap_or(Variety::Atomic);
                    self.push_anonymous(TypeDef {
                        name: None,
                        base: Some(simple),
                        kind: TypeKind::Simple(SimpleType {
                            variety,
                            facets: collect_facets(facets),
                            builtin: None,
                        }),
                    })
                };
                let (attrs, any) =
                    merge_attributes(&self.model, base_attrs, own_attrs.0, None, own_attrs.1);
                (base_id, Content::Simple(simple), attrs, any)
            }
        };
        Ok(TypeDef {
            name,
            base: Some(base),
            kind: TypeKind::Complex(ComplexType {
                mixed,
                content,
                attributes,
                any_attribute,
            }),
        })
    }

    /// Expands attribute items, copying in the contents of each attribute group that
    /// is referenced. `None` in the result marks a prohibited attribute.
    ///
    /// The wildcard (`xs:anyAttribute`) is the type's own if it has one. If not,
    /// the first one found in a referenced attribute group is used. An attribute
    /// group referenced twice is copied only once; this also stops groups that
    /// refer to each other from looping forever.
    #[allow(clippy::type_complexity)]
    fn attr_items(
        &mut self,
        items: &[RawAttrItem],
        any: Option<NsConstraint>,
    ) -> Result<(Vec<(QName, Option<AttributeUse>)>, Option<NsConstraint>)> {
        let mut out = Vec::new();
        let mut any = any;
        let mut visited = HashSet::new();
        self.attr_items_into(items, &mut out, &mut any, &mut visited)?;
        Ok((out, any))
    }

    fn attr_items_into(
        &mut self,
        items: &[RawAttrItem],
        out: &mut Vec<(QName, Option<AttributeUse>)>,
        any: &mut Option<NsConstraint>,
        visited: &mut HashSet<QName>,
    ) -> Result<()> {
        for item in items {
            match item {
                RawAttrItem::Attribute(a) => {
                    let (qname, attr) = self.attribute(a)?;
                    let use_ = match a.use_ {
                        AttrUse::Prohibited => None,
                        AttrUse::Required => Some(AttributeUse {
                            attr,
                            required: true,
                        }),
                        AttrUse::Optional => Some(AttributeUse {
                            attr,
                            required: false,
                        }),
                    };
                    out.push((qname, use_));
                }
                RawAttrItem::GroupRef(q) => {
                    if !visited.insert(q.clone()) {
                        continue;
                    }
                    let group =
                        *self
                            .attr_groups
                            .get(q)
                            .ok_or_else(|| SchemaError::Unresolved {
                                kind: "attributeGroup",
                                qname: q.to_string(),
                            })?;
                    if group.any_attribute.is_some() && any.is_none() {
                        *any = group.any_attribute.clone();
                    }
                    self.attr_items_into(&group.attributes, out, any, visited)?;
                }
            }
        }
        Ok(())
    }

    /// Resolves one attribute declaration or reference to its `AttrId`. A local
    /// declaration adds a new attribute each time. With no type given, the type is
    /// `xs:anySimpleType`.
    fn attribute(&mut self, a: &RawAttribute) -> Result<(QName, AttrId)> {
        if let Some(r) = &a.reference {
            let id = self.global_attribute(r)?;
            return Ok((r.clone(), id));
        }
        let name = a.name.clone().unwrap_or_default();
        let qname = QName::new(&a.ns, &name);
        let type_id = match (&a.type_name, &a.inline_type) {
            (Some(t), _) => self.type_id(t)?,
            (None, Some(inl)) => self.anonymous_simple(inl)?,
            (None, None) => builtin::ANY_SIMPLE_TYPE,
        };
        self.model.attributes.push(AttributeDef {
            qname: qname.clone(),
            type_id,
            global: false,
        });
        Ok((qname, self.model.attributes.len() - 1))
    }

    /// The id of a global attribute, creating it on first use. The entry is added
    /// and its id recorded before its type is looked up (the same order as
    /// `global_element`). The type starts as `xs:anySimpleType` and is replaced
    /// once known.
    fn global_attribute(&mut self, q: &QName) -> Result<AttrId> {
        if let Some(id) = self.global_attributes.get(q) {
            return Ok(*id);
        }
        let raw = *self
            .raw_global_attributes
            .get(q)
            .ok_or_else(|| SchemaError::Unresolved {
                kind: "attribute",
                qname: q.to_string(),
            })?;
        let id = self.model.attributes.len();
        self.model.attributes.push(AttributeDef {
            qname: q.clone(),
            type_id: builtin::ANY_SIMPLE_TYPE,
            global: true,
        });
        self.global_attributes.insert(q.clone(), id);
        let type_id = match (&raw.type_name, &raw.inline_type) {
            (Some(t), _) => self.type_id(t)?,
            (None, Some(inl)) => self.anonymous_simple(inl)?,
            (None, None) => builtin::ANY_SIMPLE_TYPE,
        };
        self.model.attributes[id].type_id = type_id;
        Ok(id)
    }

    /// The id of a global element, creating it on first use.
    ///
    /// The entry is added and its id recorded before its type is resolved. An
    /// element's type can contain a reference back to the same element (for example
    /// a tree of nested `section` elements). Since the id is already known, that
    /// reference finds it instead of recursing forever. The type starts as
    /// `xs:anyType` and is replaced once known.
    fn global_element(&mut self, q: &QName) -> Result<ElemId> {
        if let Some(id) = self.global_elements.get(q) {
            return Ok(*id);
        }
        let raw = *self
            .raw_global_elements
            .get(q)
            .ok_or_else(|| SchemaError::Unresolved {
                kind: "element",
                qname: q.to_string(),
            })?;
        let id = self.model.elements.len();
        self.model.elements.push(ElementDef {
            qname: q.clone(),
            type_id: builtin::ANY_TYPE,
            nillable: raw.nillable,
            global: true,
            substitution_group: raw.substitution_group.clone(),
            is_abstract: raw.is_abstract,
        });
        self.global_elements.insert(q.clone(), id);
        // The element is listed before its type is resolved, so a group that
        // reaches it again stops there. That kind of loop is fine, so the groups
        // being expanded outside do not count inside its type.
        let outer = std::mem::take(&mut self.expanding_groups);
        let type_id = self.element_type(raw);
        self.expanding_groups = outer;
        self.model.elements[id].type_id = type_id?;
        Ok(id)
    }

    /// The type of an element: the named type, else the inline type, else
    /// `xs:anyType`.
    fn element_type(&mut self, raw: &RawElement) -> Result<TypeId> {
        match (&raw.type_name, &raw.inline_type) {
            (Some(t), _) => self.type_id(t),
            (None, Some(inl)) => self.anonymous_type(inl),
            (None, None) => Ok(builtin::ANY_TYPE),
        }
    }

    /// The particle of one type's content. It gets its own size count, which is
    /// put back afterwards: an anonymous type met inside another type's content is
    /// resolved in the middle of it.
    fn content_particle(&mut self, raw: &RawParticle) -> Result<Particle> {
        let outer = std::mem::replace(&mut self.particles, 0);
        let result = self.particle(raw);
        self.particles = outer;
        result
    }

    /// Resolves a particle
    /// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Model_Groups)).
    ///
    /// - An element reference points to the global element. A local element
    ///   declaration adds a new, non-global element each time.
    /// - A wildcard with a namespace list adds those URIs to the model's namespaces,
    ///   which later fill the URI partition of the string table
    ///   ([EXI 8.5.4.1.7](https://www.w3.org/TR/exi/#wildcardTerms)).
    /// - A group reference is replaced by a copy of the group's content, using the
    ///   `minOccurs` and `maxOccurs` of the reference.
    fn particle(&mut self, raw: &RawParticle) -> Result<Particle> {
        self.particles += 1;
        if let Some(max) = self.max_particles
            && self.particles > max
        {
            return Err(SchemaError::TooLarge { max });
        }
        Ok(match raw {
            RawParticle::Element(e) => {
                let elem = match &e.reference {
                    Some(r) => self.global_element(r)?,
                    None => {
                        let qname = QName::new(&e.ns, e.name.as_deref().unwrap_or_default());
                        let type_id = self.element_type(e)?;
                        self.model.elements.push(ElementDef {
                            qname,
                            type_id,
                            nillable: e.nillable,
                            global: false,
                            substitution_group: None,
                            is_abstract: e.is_abstract,
                        });
                        self.model.elements.len() - 1
                    }
                };
                Particle {
                    min: e.min,
                    max: e.max,
                    term: Term::Element(elem),
                }
            }
            RawParticle::Any { ns, min, max } => {
                if let NsConstraint::List(list) = ns {
                    for uri in list {
                        if !self.model.namespaces.contains(uri) {
                            self.model.namespaces.push(uri.clone());
                        }
                    }
                }
                Particle {
                    min: *min,
                    max: *max,
                    term: Term::Any(ns.clone()),
                }
            }
            RawParticle::Group {
                compositor,
                particles,
                min,
                max,
            } => {
                let mut ps = Vec::new();
                for p in particles {
                    ps.push(self.particle(p)?);
                }
                Particle {
                    min: *min,
                    max: *max,
                    term: Term::Group {
                        compositor: *compositor,
                        particles: ps,
                    },
                }
            }
            RawParticle::GroupRef { name, min, max } => {
                let group = *self
                    .groups
                    .get(name)
                    .ok_or_else(|| SchemaError::Unresolved {
                        kind: "group",
                        qname: name.to_string(),
                    })?;
                // A group that contains itself, directly or through other groups,
                // has no finite content, so XML Schema forbids it
                // (<https://www.w3.org/TR/xmlschema-1/#coss-modelGroupDef>).
                if self.expanding_groups.contains(name) {
                    return Err(SchemaError::Invalid(format!(
                        "circular group reference to {name}"
                    )));
                }
                self.expanding_groups.push(name.clone());
                let inner = self.particle(&group.particle);
                self.expanding_groups.pop();
                let inner = inner?;
                Particle {
                    min: *min,
                    max: *max,
                    term: inner.term,
                }
            }
        })
    }
}

/// A sequence with nothing in it, occurring once. Used as the content of mixed
/// types with no particle, so that the type still has a particle that allows text.
fn empty_sequence() -> Particle {
    Particle {
        min: 1,
        max: Some(1),
        term: Term::Group {
            compositor: Compositor::Sequence,
            particles: Vec::new(),
        },
    }
}

/// Turns a list of facets into a [`Facets`] value. All enumeration values and all
/// patterns are kept; for other facets, the last one wins. Facets in `Facet::Other` are dropped.
fn collect_facets(facets: &[Facet]) -> Facets {
    let mut out = Facets::default();
    for f in facets {
        match f {
            Facet::Enumeration(v) => out.enumeration.get_or_insert_with(Vec::new).push(v.clone()),
            Facet::MinInclusive(v) => out.min_inclusive = Some(v.clone()),
            Facet::MaxInclusive(v) => out.max_inclusive = Some(v.clone()),
            Facet::MinExclusive(v) => out.min_exclusive = Some(v.clone()),
            Facet::MaxExclusive(v) => out.max_exclusive = Some(v.clone()),
            Facet::Pattern(v) => out.patterns.push(v.clone()),
            Facet::WhiteSpace(ws) => out.whitespace = Some(*ws),
            Facet::Other(_, _) => {}
        }
    }
    out
}

/// Merges a derived type's attributes into its base type's.
///
/// Start from the base attribute uses. For each of the derived type's own uses,
/// matched by QName: a new one is added, one with the same name replaces the base
/// one, and a prohibited one removes it.
///
/// Wildcards are merged like this: if either is `##any`, the result is `##any`. If
/// only one exists, it is used. If both are lists, the result is both lists joined
/// (no duplicates). In any other case, the derived type's own wildcard is used.
fn merge_attributes(
    model: &Model,
    base: Vec<AttributeUse>,
    own: Vec<(QName, Option<AttributeUse>)>,
    base_any: Option<NsConstraint>,
    own_any: Option<NsConstraint>,
) -> (Vec<AttributeUse>, Option<NsConstraint>) {
    let mut out = base;
    for (qname, use_) in own {
        let pos = out
            .iter()
            .position(|b| model.attributes[b.attr].qname == qname);
        match (use_, pos) {
            (Some(u), Some(p)) => out[p] = u,
            (Some(u), None) => out.push(u),
            (None, Some(p)) => {
                out.remove(p);
            }
            (None, None) => {}
        }
    }
    let any = match (own_any, base_any) {
        (Some(NsConstraint::Any), _) | (_, Some(NsConstraint::Any)) => Some(NsConstraint::Any),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (Some(NsConstraint::List(mut a)), Some(NsConstraint::List(b))) => {
            for uri in b {
                if !a.contains(&uri) {
                    a.push(uri);
                }
            }
            Some(NsConstraint::List(a))
        }
        (Some(a), Some(_)) => Some(a),
        (None, None) => None,
    };
    (out, any)
}
