//! XML Schema documents as they were written, before any names are looked up.
//!
//! The loader turns each `xs:schema` document into a [`RawDocument`]. The data is
//! owned (no borrows from the source text). When one schema part (a "component",
//! such as an element, type or group) refers to another, the reference is kept as a
//! [`QName`]. [`super::resolve`](mod@super::resolve) looks these names up later.

use rexi_core::WhiteSpace;

/// A name with its namespace: a namespace URI plus a local name. `Display` prints
/// it as `{ns}local`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QName {
    /// Namespace URI; `""` means no namespace.
    pub ns: String,
    /// Local name (the part after the prefix).
    pub local: String,
}

impl QName {
    /// Makes a name from a namespace URI and a local name.
    pub fn new(ns: &str, local: &str) -> Self {
        Self {
            ns: ns.to_string(),
            local: local.to_string(),
        }
    }
}

impl std::fmt::Display for QName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{{}}}{}", self.ns, self.local)
    }
}

/// The `{namespace constraint}` of a wildcard (`xs:any` or `xs:anyAttribute`): which
/// namespaces the wildcard accepts
/// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Wildcards)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NsConstraint {
    /// `##any`.
    Any,
    /// `##other`: any namespace except the given target namespace. Names with no
    /// namespace are not accepted either.
    Other(String),
    /// A list of namespaces written out in full. `""` stands for `##local` (no
    /// namespace).
    List(Vec<String>),
}

/// The `use` of an attribute declaration: whether it must, may or must not appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrUse {
    /// `use="optional"` (the default): the attribute may appear.
    Optional,
    /// `use="required"`: the attribute must appear.
    Required,
    /// `use="prohibited"`: the attribute may not appear (used to remove an
    /// attribute in a restriction).
    Prohibited,
}

/// How the particles of a model group combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compositor {
    /// `xs:sequence`: all of them, in the written order.
    Sequence,
    /// `xs:choice`: exactly one of them.
    Choice,
    /// `xs:all`: all of them, in any order (each at most once).
    All,
}

/// A constraining facet of a simple-type restriction (`xs:enumeration`,
/// `xs:pattern`, ...). Values are kept as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Facet {
    /// `xs:enumeration`: one allowed value.
    Enumeration(String),
    /// `xs:minInclusive`: the smallest allowed value.
    MinInclusive(String),
    /// `xs:maxInclusive`: the largest allowed value.
    MaxInclusive(String),
    /// `xs:minExclusive`: values must be larger than this.
    MinExclusive(String),
    /// `xs:maxExclusive`: values must be smaller than this.
    MaxExclusive(String),
    /// `xs:pattern`: a regular expression the text must match.
    Pattern(String),
    /// `xs:whiteSpace`: how spaces, tabs and line breaks are handled.
    WhiteSpace(WhiteSpace),
    /// length, minLength, maxLength, totalDigits, fractionDigits: stored, but not
    /// used.
    Other(String, String),
}

/// An `xs:element`, either a declaration (`name=`) or a reference (`ref=`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawElement {
    /// `None` when this is a reference to another element (`ref=`).
    pub name: Option<String>,
    /// Namespace of the declared name: the target namespace, or `""` when the name
    /// is unqualified (has no namespace).
    pub ns: String,
    /// The element referred to by `ref=`, when this is a reference.
    pub reference: Option<QName>,
    /// The `type=` attribute: the name of a declared type.
    pub type_name: Option<QName>,
    /// A type declared inside the element (an anonymous type).
    pub inline_type: Option<Box<RawType>>,
    /// `minOccurs` (default 1).
    pub min: u32,
    /// `None` means `unbounded`.
    pub max: Option<u32>,
    /// `nillable="true"`: the element may carry `xsi:nil="true"` and no content.
    pub nillable: bool,
    /// The `substitutionGroup=` head element, if any.
    pub substitution_group: Option<QName>,
    /// `abstract="true"`: the element can not appear itself, only the members of
    /// its substitution group can.
    pub is_abstract: bool,
}

/// A type declared inside an element: simple or complex.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum RawType {
    /// An `xs:simpleType`.
    Simple(RawSimpleType),
    /// An `xs:complexType`.
    Complex(RawComplexType),
}

/// An `xs:attribute`, either a declaration (`name=`) or a reference (`ref=`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAttribute {
    /// The declared name; `None` when this is a reference.
    pub name: Option<String>,
    /// Namespace of the declared name: the target namespace, or `""` when the name
    /// is unqualified.
    pub ns: String,
    /// The attribute referred to by `ref=`, when this is a reference.
    pub reference: Option<QName>,
    /// The `type=` attribute: the name of a simple type.
    pub type_name: Option<QName>,
    /// A simple type declared inside the attribute.
    pub inline_type: Option<RawSimpleType>,
    /// Whether the attribute is optional, required or prohibited.
    pub use_: AttrUse,
}

/// One entry of an attribute list: an attribute or a reference to an attribute
/// group.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum RawAttrItem {
    /// An `xs:attribute`.
    Attribute(RawAttribute),
    /// An `xs:attributeGroup ref=` to the named attribute group.
    GroupRef(QName),
}

/// A particle: one entry of a content model, with how often it may occur
/// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#cParticles)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawParticle {
    /// An element declaration or reference (it carries its own min and max).
    Element(RawElement),
    /// An `xs:any` element wildcard.
    Any {
        /// The namespaces the wildcard accepts.
        ns: NsConstraint,
        /// `minOccurs`.
        min: u32,
        /// `maxOccurs`; `None` means `unbounded`.
        max: Option<u32>,
    },
    /// An `xs:sequence`, `xs:choice` or `xs:all` written in place.
    Group {
        /// How the particles combine.
        compositor: Compositor,
        /// The particles inside the group, in written order.
        particles: Vec<RawParticle>,
        /// `minOccurs`.
        min: u32,
        /// `maxOccurs`; `None` means `unbounded`.
        max: Option<u32>,
    },
    /// An `xs:group ref=` to a named model group.
    GroupRef {
        /// The name of the model group.
        name: QName,
        /// `minOccurs`.
        min: u32,
        /// `maxOccurs`; `None` means `unbounded`.
        max: Option<u32>,
    },
}

/// How a complex type gets its content: written in place, or derived from a
/// base type by extension or restriction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawDerivation {
    /// A plain content model with no `xs:extension` or `xs:restriction`. XML Schema
    /// treats this as a restriction of `xs:anyType` that is not written out.
    None {
        /// The content model, if the type has element content.
        particle: Option<RawParticle>,
    },
    /// `xs:complexContent/xs:extension`: the base type's content followed by
    /// `particle`.
    ComplexExtension {
        /// The base type.
        base: QName,
        /// The content added after the base type's content.
        particle: Option<RawParticle>,
    },
    /// `xs:complexContent/xs:restriction`: a content model that allows less than the
    /// base type's.
    ComplexRestriction {
        /// The base type.
        base: QName,
        /// The full content model of the restricted type.
        particle: Option<RawParticle>,
    },
    /// `xs:simpleContent/xs:extension`: text content of the base simple type, plus
    /// attributes.
    SimpleExtension {
        /// The base type.
        base: QName,
    },
    /// `xs:simpleContent/xs:restriction`: text content of the base type, with facets.
    SimpleRestriction {
        /// The base type.
        base: QName,
        /// The facets that narrow the base type.
        facets: Vec<Facet>,
        /// A simple type declared inside the restriction; when present, it is the
        /// type of the text instead of the base type's.
        inline: Option<RawSimpleType>,
    },
}

/// An `xs:complexType`: a type with attributes, child elements, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawComplexType {
    /// The declared name; `None` for an anonymous type inside an element.
    pub name: Option<String>,
    /// Namespace of the name (the target namespace).
    pub ns: String,
    /// `mixed="true"`: text may appear between the child elements.
    pub mixed: bool,
    /// How the content is defined.
    pub derivation: RawDerivation,
    /// The attributes and attribute-group references, in written order.
    pub attributes: Vec<RawAttrItem>,
    /// The `xs:anyAttribute` wildcard, if any.
    pub any_attribute: Option<NsConstraint>,
}

/// An `xs:simpleType`: a type for text content and attribute values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSimpleType {
    /// The declared name; `None` for an anonymous type.
    pub name: Option<String>,
    /// Namespace of the name (the target namespace).
    pub ns: String,
    /// Whether it is a restriction, a list or a union.
    pub variety: RawVariety,
}

/// The three kinds of simple type
/// ([XML Schema 1.0 Part 2](https://www.w3.org/TR/xmlschema-2/#defn-variety)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawVariety {
    /// `xs:restriction`: a base type narrowed by facets.
    Restriction {
        /// The `base=` type; `None` when the base is declared inside (`inline`).
        base: Option<QName>,
        /// A base type declared inside the restriction.
        inline: Option<Box<RawSimpleType>>,
        /// The facets that narrow the base type.
        facets: Vec<Facet>,
    },
    /// `xs:list`: a space-separated list of items of one type.
    List {
        /// The `itemType=`; `None` when the item type is declared inside (`inline`).
        item: Option<QName>,
        /// An item type declared inside the list.
        inline: Option<Box<RawSimpleType>>,
    },
    /// `xs:union`: a value of any one of the member types.
    Union {
        /// The `memberTypes=` names, in order.
        members: Vec<QName>,
        /// Member types declared inside the union, in order (after `members`).
        inline: Vec<RawSimpleType>,
    },
}

/// A named model group (`xs:group name=`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawGroup {
    /// The group's name.
    pub name: String,
    /// Namespace of the name (the target namespace).
    pub ns: String,
    /// The group's content: one sequence, choice or all.
    pub particle: RawParticle,
}

/// A named attribute group (`xs:attributeGroup name=`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAttributeGroup {
    /// The group's name.
    pub name: String,
    /// Namespace of the name (the target namespace).
    pub ns: String,
    /// The attributes and nested group references, in written order.
    pub attributes: Vec<RawAttrItem>,
    /// The `xs:anyAttribute` wildcard, if any.
    pub any_attribute: Option<NsConstraint>,
}

/// One parsed `xs:schema` document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawDocument {
    /// The `targetNamespace=` of the document; `""` when it has none.
    pub target_ns: String,
    /// `elementFormDefault="qualified"`: local elements get the target namespace.
    pub element_qualified: bool,
    /// `attributeFormDefault="qualified"`: local attributes get the target namespace.
    pub attribute_qualified: bool,
    /// Top-level element declarations.
    pub elements: Vec<RawElement>,
    /// Top-level attribute declarations.
    pub attributes: Vec<RawAttribute>,
    /// Top-level named complex types.
    pub complex_types: Vec<RawComplexType>,
    /// Top-level named simple types.
    pub simple_types: Vec<RawSimpleType>,
    /// Named model groups.
    pub groups: Vec<RawGroup>,
    /// Named attribute groups.
    pub attribute_groups: Vec<RawAttributeGroup>,
}
