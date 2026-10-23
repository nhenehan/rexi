//! The resolved schema model. Every reference from one part of the schema to another
//! is now an index into one of the vectors in [`Model`], not a name.

pub use super::raw::{Compositor, NsConstraint, QName};
use rexi_core::WhiteSpace;

/// Index of a type in [`Model::types`].
pub type TypeId = usize;
/// Index of an element declaration in [`Model::elements`].
pub type ElemId = usize;
/// Index of an attribute declaration in [`Model::attributes`].
pub type AttrId = usize;

/// The facets of a simple type that matter for EXI. A facet is a rule that narrows a
/// type, such as a list of allowed values or a minimum value
/// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Simple_Type_Definitions)).
/// Pattern texts are kept so the datatype mapper can work out whether a string type
/// has a restricted character set
/// ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facets {
    /// The allowed values (`xs:enumeration`), in schema order.
    pub enumeration: Option<Vec<String>>,
    /// `xs:minInclusive`, as written.
    pub min_inclusive: Option<String>,
    /// `xs:maxInclusive`, as written.
    pub max_inclusive: Option<String>,
    /// `xs:minExclusive`, as written.
    pub min_exclusive: Option<String>,
    /// `xs:maxExclusive`, as written.
    pub max_exclusive: Option<String>,
    /// The `xs:pattern` texts written on this type itself, in schema order. Patterns
    /// of base types are not copied here. Character references such as `&#x41;`
    /// have already been turned into characters by the XML parser.
    pub patterns: Vec<String>,
    /// `xs:whiteSpace`: how spaces, tabs and line breaks are handled.
    pub whitespace: Option<WhiteSpace>,
}

/// The three kinds of simple type
/// ([XML Schema 1.0 Part 2](https://www.w3.org/TR/xmlschema-2/#defn-variety)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Variety {
    /// One value, not a list or union.
    Atomic,
    /// A space-separated list of items of one type.
    List {
        /// The type of each item.
        item: TypeId,
    },
    /// A value of any one of several member types.
    Union {
        /// The member types, in schema order.
        members: Vec<TypeId>,
    },
}

/// A simple type: the type of text content and attribute values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleType {
    /// Whether it is atomic, a list or a union.
    pub variety: Variety,
    /// Facets declared on this type itself. To find the facets that actually apply,
    /// walk up the chain of base types.
    pub facets: Facets,
    /// Index into the built-in table when this is one of the 46 built-in types.
    pub builtin: Option<usize>,
}

/// A particle: a term (element, wildcard or group) with how many times it may occur.
/// `max` of `None` means `unbounded`
/// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Model_Groups)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Particle {
    /// `minOccurs`.
    pub min: u32,
    /// `maxOccurs`; `None` means `unbounded`.
    pub max: Option<u32>,
    /// What occurs: an element, a wildcard or a group.
    pub term: Term,
}

/// The thing a particle stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// An element declaration.
    Element(ElemId),
    /// An `xs:any` element wildcard that accepts the given namespaces.
    Any(NsConstraint),
    /// A model group (`xs:sequence`, `xs:choice` or `xs:all`), with every group
    /// reference already replaced by its content.
    Group {
        /// How the particles combine.
        compositor: Compositor,
        /// The particles inside the group, in schema order.
        particles: Vec<Particle>,
    },
}

/// What a complex type allows between its start tag and end tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// No child elements and no simple content.
    Empty,
    /// Text of the given simple type (`xs:simpleContent`).
    Simple(TypeId),
    /// Child elements, as described by the particle.
    Particle(Particle),
}

/// One attribute that a complex type allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeUse {
    /// The attribute declaration.
    pub attr: AttrId,
    /// Whether the attribute must appear (`use="required"`).
    pub required: bool,
}

/// A complex type: a type with attributes, child elements, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexType {
    /// `mixed="true"`: text may appear between the child elements.
    pub mixed: bool,
    /// What the type allows as content.
    pub content: Content,
    /// The attributes the type allows, base type's included. Prohibited
    /// attributes are already removed.
    pub attributes: Vec<AttributeUse>,
    /// The `xs:anyAttribute` wildcard, if any.
    pub any_attribute: Option<NsConstraint>,
}

/// Simple or complex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeKind {
    /// A simple type.
    Simple(SimpleType),
    /// A complex type.
    Complex(ComplexType),
}

/// One type of the schema, built-in or declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDef {
    /// `None` for anonymous types (types with no name).
    pub name: Option<QName>,
    /// The type this one is derived from (`xs:anyType` has none).
    pub base: Option<TypeId>,
    /// Simple or complex, with the details.
    pub kind: TypeKind,
}

impl TypeDef {
    /// The simple type, or `None` for a complex type.
    pub fn simple(&self) -> Option<&SimpleType> {
        match &self.kind {
            TypeKind::Simple(s) => Some(s),
            TypeKind::Complex(_) => None,
        }
    }
}

/// One element declaration, top-level or local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementDef {
    /// The element's name.
    pub qname: QName,
    /// The element's type.
    pub type_id: TypeId,
    /// `nillable="true"`: the element may carry `xsi:nil="true"` and no content.
    pub nillable: bool,
    /// Whether the element is declared at the top level of a schema document.
    pub global: bool,
    /// The `substitutionGroup=` head element, if any.
    pub substitution_group: Option<QName>,
    /// `abstract="true"`: only the members of its substitution group can appear.
    pub is_abstract: bool,
}

/// One attribute declaration, top-level or local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeDef {
    /// The attribute's name.
    pub qname: QName,
    /// The attribute's simple type.
    pub type_id: TypeId,
    /// Whether the attribute is declared at the top level of a schema document.
    pub global: bool,
}

/// A set of schema documents with every name looked up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Model {
    /// Built-in types first (in `XSD_LOCAL_NAMES` order), then named schema types,
    /// then anonymous types in the order they were found.
    pub types: Vec<TypeDef>,
    /// Every element declaration, top-level and local.
    pub elements: Vec<ElementDef>,
    /// Every attribute declaration, top-level and local.
    pub attributes: Vec<AttributeDef>,
    /// Target namespaces of all loaded documents, plus the namespaces listed in
    /// wildcards.
    pub namespaces: Vec<String>,
}

impl Model {
    /// The type itself, followed by its base type, that type's base, and so on up to
    /// `xs:anyType`. Stops early if the chain loops back on itself.
    pub fn ancestors(&self, t: TypeId) -> Vec<TypeId> {
        let mut out = Vec::new();
        let mut cur = Some(t);
        while let Some(c) = cur {
            if out.contains(&c) {
                break;
            }
            out.push(c);
            cur = self.types.get(c).and_then(|d| d.base);
        }
        out
    }

    /// The nearest built-in type in the chain of `t` and its base types (this can be
    /// `t` itself). For complex types, only `xs:anyType` counts.
    pub fn closest_builtin(&self, t: TypeId) -> Option<usize> {
        self.ancestors(t)
            .into_iter()
            .find_map(|a| match &self.types.get(a)?.kind {
                TypeKind::Simple(s) => s.builtin,
                TypeKind::Complex(_) => (a == crate::builtin::ANY_TYPE).then_some(a),
            })
    }

    /// Global elements that may appear in place of `head`, not counting `head`.
    ///
    /// This is the substitution group of `head`: every global element whose
    /// `substitutionGroup` names `head`, and then, again and again, every element
    /// that names one of those
    /// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#cElement_Declarations)).
    /// It is a depth-first walk with a work stack.
    pub fn substitution_members(&self, head: ElemId) -> Vec<ElemId> {
        let mut members = Vec::new();
        let mut frontier = vec![head];
        while let Some(h) = frontier.pop() {
            let hq = &self.elements[h].qname;
            for (i, e) in self.elements.iter().enumerate() {
                if e.global
                    && e.substitution_group.as_ref() == Some(hq)
                    && i != head
                    && !members.contains(&i)
                {
                    members.push(i);
                    frontier.push(i);
                }
            }
        }
        members
    }
}
