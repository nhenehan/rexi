//! Reads `xs:schema` documents with roxmltree. It also follows `xs:import` and
//! `xs:include`, finding each `schemaLocation` relative to the file that names it.
//!
//! Each document becomes one [`RawDocument`]. Names are not looked up here; that is
//! done later by `super::resolve`. See
//! [XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/) for the elements read.

use std::io::Read;
use std::path::{Path, PathBuf};

use rexi_core::WhiteSpace;
use rexi_core::grammar::ir::well_known;
use roxmltree::{Document, Node};

use super::raw::*;
use crate::error::{Result, SchemaError};
use crate::load_options::{SchemaLoadOptions, SchemaRoot};

/// The XML Schema namespace URI.
pub const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema";

/// Loads a set of schema documents. Each import or include is followed only once,
/// even if several documents name it. Which files an import or include may read is
/// set by [`SchemaLoadOptions`].
#[derive(Debug, Default)]
pub struct Loader {
    /// The documents loaded so far, in the order they were read.
    pub docs: Vec<RawDocument>,
    loaded: Vec<PathBuf>,
    options: SchemaLoadOptions,
    /// The folders includes and imports may read under. Each is kept twice: as
    /// written (made absolute, `..` removed) and as its real path (symbolic links
    /// followed), so either form of a location can be matched. Empty with
    /// [`SchemaRoot::Anywhere`], and while no main schema has been seen.
    roots: Vec<PathBuf>,
}

struct Ctx {
    target_ns: String,
    element_qualified: bool,
    attribute_qualified: bool,
}

impl Loader {
    /// Makes a loader with nothing loaded and the given rules. A
    /// [`SchemaRoot::Folder`] must exist.
    pub fn with_options(options: SchemaLoadOptions) -> Result<Self> {
        let mut loader = Self {
            options,
            ..Self::default()
        };
        if let SchemaRoot::Folder(dir) = loader.options.root.clone() {
            loader.add_root(&dir)?;
        }
        Ok(loader)
    }

    /// Notes the folder of a main schema (a file the caller named, or the
    /// `base_dir` of a schema in memory) as a place includes may read from, when
    /// the rule is [`SchemaRoot::MainSchemaFolders`]. Call it for every main file
    /// before loading any, so an include in one may reach the folder of another.
    pub fn add_main_folder(&mut self, dir: &Path) -> Result<()> {
        if self.options.root == SchemaRoot::MainSchemaFolders {
            self.add_root(dir)?;
        }
        Ok(())
    }

    fn add_root(&mut self, dir: &Path) -> Result<()> {
        let io = |e: std::io::Error| SchemaError::Io {
            path: dir.to_path_buf(),
            message: e.to_string(),
        };
        for form in [
            lexical_absolute(dir).map_err(io)?,
            dir.canonicalize().map_err(io)?,
        ] {
            if !self.roots.contains(&form) {
                self.roots.push(form);
            }
        }
        Ok(())
    }

    fn inside_root(&self, path: &Path) -> bool {
        self.options.root == SchemaRoot::Anywhere
            || self.roots.iter().any(|root| path.starts_with(root))
    }

    /// Loads a main schema file: one the caller named, so no root rule applies to
    /// it (the size and regular-file rules still do).
    pub fn load_main_file(&mut self, path: &Path) -> Result<()> {
        self.load_file(path, None, None)
    }

    /// Follows a `schemaLocation` from a document whose folder is `base_dir`,
    /// after checking the load rules.
    fn load_location(
        &mut self,
        base_dir: Option<&Path>,
        location: &str,
        expected_ns: Option<&str>,
        chameleon_ns: Option<&str>,
    ) -> Result<()> {
        let refused = |reason: &str| SchemaError::Refused {
            location: location.to_string(),
            reason: reason.to_string(),
        };
        // Nothing is fetched over a network. "C:" style drive letters are one
        // letter, so a scheme of two or more letters is a URL.
        if let Some((scheme, _)) = location.split_once(':')
            && scheme.len() > 1
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        {
            return Err(refused("URLs are not fetched"));
        }
        let not_inside = || {
            refused(match self.options.root {
                SchemaRoot::MainSchemaFolders if self.roots.is_empty() => {
                    "a schema in memory with no base folder may not include or import files"
                }
                SchemaRoot::MainSchemaFolders => {
                    "outside the main schema's folder (set a schema root to allow it)"
                }
                _ => "outside the schema root folder",
            })
        };
        let path = resolve_location(base_dir, location);
        // First check the path as written, without touching the disk. A location
        // outside the root is refused the same way whether or not the file
        // exists, so the error never tells which files are there.
        let written = lexical_absolute(&path).map_err(|e| SchemaError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        if !self.inside_root(&written) {
            return Err(not_inside());
        }
        // Then follow symbolic links, which may lead out of the root.
        let real = path.canonicalize().map_err(|e| SchemaError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        if !self.inside_root(&real) {
            return Err(not_inside());
        }
        self.load_file(&real, expected_ns, chameleon_ns)
    }

    /// Loads `path`. When an `xs:import` asked for a namespace (`expected_ns`), the
    /// document's target namespace must match it.
    ///
    /// `chameleon_ns` is the namespace of the document that does the `xs:include`.
    /// An included document with no target namespace takes on this namespace. XML
    /// Schema calls this a "chameleon" include.
    ///
    /// Files are compared by their full real path, so a file reached twice is read
    /// only once.
    fn load_file(
        &mut self,
        path: &Path,
        expected_ns: Option<&str>,
        chameleon_ns: Option<&str>,
    ) -> Result<()> {
        let canonical = path.canonicalize().map_err(|e| SchemaError::Io {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        if self.loaded.contains(&canonical) {
            return Ok(());
        }
        self.loaded.push(canonical.clone());
        let text = read_schema_file(&canonical, self.options.max_file_bytes)?;
        let base_dir = canonical.parent().map(Path::to_path_buf);
        self.load_str_inner(
            &text,
            base_dir.as_deref(),
            &canonical.display().to_string(),
            expected_ns,
            chameleon_ns,
        )
    }

    /// Loads a document held in memory. `base_dir` is used to find schema locations
    /// given as relative paths, and is the main schema's folder for the load rules.
    pub fn load_str(&mut self, text: &str, base_dir: Option<&Path>) -> Result<()> {
        if let Some(dir) = base_dir {
            self.add_main_folder(dir)?;
        }
        self.load_str_inner(text, base_dir, "<string>", None, None)
    }

    fn load_str_inner(
        &mut self,
        text: &str,
        base_dir: Option<&Path>,
        source: &str,
        expected_ns: Option<&str>,
        chameleon_ns: Option<&str>,
    ) -> Result<()> {
        let doc = Document::parse(text).map_err(|e| SchemaError::Xml {
            source: source.to_string(),
            message: e.to_string(),
        })?;
        let root = doc.root_element();
        if !is_xs(root, "schema") {
            return Err(SchemaError::Invalid(format!(
                "{source}: root element is not xs:schema"
            )));
        }
        let mut target_ns = root.attribute("targetNamespace").unwrap_or("").to_string();
        if target_ns.is_empty()
            && let Some(ns) = chameleon_ns
        {
            target_ns = ns.to_string();
        }
        if let Some(expected) = expected_ns
            && expected != target_ns
        {
            return Err(SchemaError::Invalid(format!(
                "{source}: imported for namespace '{expected}' but declares '{target_ns}'"
            )));
        }
        let ctx = Ctx {
            target_ns,
            element_qualified: root.attribute("elementFormDefault") == Some("qualified"),
            attribute_qualified: root.attribute("attributeFormDefault") == Some("qualified"),
        };
        let mut raw = RawDocument {
            target_ns: ctx.target_ns.clone(),
            element_qualified: ctx.element_qualified,
            attribute_qualified: ctx.attribute_qualified,
            ..Default::default()
        };
        // Reserve this document's place in the list now, before following imports.
        // That way a document always comes before the documents it imports.
        let slot = self.docs.len();
        self.docs.push(RawDocument::default());

        for child in root.children().filter(|n| n.is_element()) {
            if child.tag_name().namespace() != Some(XSD_NS) {
                continue;
            }
            match child.tag_name().name() {
                "annotation" => {}
                "import" => {
                    let ns = child.attribute("namespace").unwrap_or("");
                    if let Some(location) = child.attribute("schemaLocation") {
                        self.load_location(base_dir, location, Some(ns), None)?;
                    }
                }
                "include" => {
                    let location = child.attribute("schemaLocation").ok_or_else(|| {
                        SchemaError::Invalid(format!("{source}: xs:include without schemaLocation"))
                    })?;
                    self.load_location(base_dir, location, None, Some(&ctx.target_ns))?;
                }
                "redefine" | "override" | "notation" => {
                    return Err(SchemaError::Unsupported(format!(
                        "xs:{}",
                        child.tag_name().name()
                    )));
                }
                "element" => raw.elements.push(parse_element(child, &ctx, true)?),
                "attribute" => raw.attributes.push(parse_attribute(child, &ctx, true)?),
                "complexType" => raw.complex_types.push(parse_complex_type(child, &ctx)?),
                "simpleType" => raw.simple_types.push(parse_simple_type(child, &ctx)?),
                "group" => {
                    let name = required(child, "name")?.to_string();
                    let particle = child
                        .children()
                        .filter(|n| n.is_element())
                        .find_map(|n| parse_particle(n, &ctx).transpose())
                        .transpose()?
                        .ok_or_else(|| {
                            SchemaError::Invalid(format!("xs:group {name} has no model group"))
                        })?;
                    raw.groups.push(RawGroup {
                        name,
                        ns: ctx.target_ns.clone(),
                        particle,
                    });
                }
                "attributeGroup" => {
                    let name = required(child, "name")?.to_string();
                    let (attributes, any_attribute) = parse_attribute_items(child, &ctx)?;
                    raw.attribute_groups.push(RawAttributeGroup {
                        name,
                        ns: ctx.target_ns.clone(),
                        attributes,
                        any_attribute,
                    });
                }
                other => {
                    return Err(SchemaError::Unsupported(format!(
                        "xs:{other} at schema level"
                    )));
                }
            }
        }
        self.docs[slot] = raw;
        Ok(())
    }
}

/// Reads a schema file as text, after checking that it is a regular file (not a
/// pipe or a device, which could block or never end) and not over `max` bytes. The
/// read itself also stops after `max` bytes, because some files (under `/proc`, for
/// example) report a size of 0.
fn read_schema_file(path: &Path, max: Option<u64>) -> Result<String> {
    let refused = |reason: String| SchemaError::Refused {
        location: path.display().to_string(),
        reason,
    };
    let io = |e: std::io::Error| SchemaError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    };
    let meta = std::fs::metadata(path).map_err(io)?;
    if !meta.is_file() {
        return Err(refused("not a regular file".to_string()));
    }
    let too_big = |max: u64| refused(format!("bigger than {max} bytes"));
    if let Some(max) = max
        && meta.len() > max
    {
        return Err(too_big(max));
    }
    let file = std::fs::File::open(path).map_err(io)?;
    let mut text = String::new();
    file.take(max.map_or(u64::MAX, |m| m.saturating_add(1)))
        .read_to_string(&mut text)
        .map_err(io)?;
    if let Some(max) = max
        && text.len() as u64 > max
    {
        return Err(too_big(max));
    }
    Ok(text)
}

/// `path` made absolute with `.` and `..` worked out by text alone: no file is
/// looked at and symbolic links are not followed.
fn lexical_absolute(path: &Path) -> std::io::Result<PathBuf> {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in std::path::absolute(path)?.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

fn resolve_location(base_dir: Option<&Path>, location: &str) -> PathBuf {
    let p = Path::new(location);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match base_dir {
        Some(dir) => dir.join(p),
        None => p.to_path_buf(),
    }
}

fn is_xs(node: Node, name: &str) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(XSD_NS)
        && node.tag_name().name() == name
}

fn xs_name<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    if node.is_element() && node.tag_name().namespace() == Some(XSD_NS) {
        Some(node.tag_name().name())
    } else {
        None
    }
}

fn required<'a>(node: Node<'a, '_>, attr: &str) -> Result<&'a str> {
    node.attribute(attr).ok_or_else(|| {
        SchemaError::Invalid(format!(
            "xs:{} is missing the {attr} attribute",
            node.tag_name().name()
        ))
    })
}

/// Turns a QName written as text (`prefix:local` or just `local`) into a
/// namespace and local name, using the namespace declarations in scope at `node`.
/// With no prefix, the default namespace is used (or no namespace if there is none).
fn resolve_qname(node: Node, value: &str) -> Result<QName> {
    let value = value.trim();
    match value.split_once(':') {
        // The `xml` prefix is always bound, by definition (Namespaces in XML,
        // section 3, https://www.w3.org/TR/xml-names/). roxmltree does not report
        // it, whether it is declared or not.
        Some(("xml", local)) => Ok(QName::new(well_known::XML_NS, local)),
        Some((prefix, local)) => {
            let ns = node.lookup_namespace_uri(Some(prefix)).ok_or_else(|| {
                SchemaError::Invalid(format!(
                    "undeclared namespace prefix '{prefix}' in '{value}'"
                ))
            })?;
            Ok(QName::new(ns, local))
        }
        None => Ok(QName::new(
            node.lookup_namespace_uri(None).unwrap_or(""),
            value,
        )),
    }
}

/// Reads `minOccurs` and `maxOccurs`. Both default to 1. `maxOccurs="unbounded"`
/// gives `None`. It is an error if the max is less than the min.
fn occurs(node: Node) -> Result<(u32, Option<u32>)> {
    let min = match node.attribute("minOccurs") {
        Some(v) => v
            .trim()
            .parse::<u32>()
            .map_err(|_| SchemaError::Invalid(format!("bad minOccurs '{v}'")))?,
        None => 1,
    };
    let max = match node.attribute("maxOccurs") {
        Some("unbounded") => None,
        Some(v) => Some(
            v.trim()
                .parse::<u32>()
                .map_err(|_| SchemaError::Invalid(format!("bad maxOccurs '{v}'")))?,
        ),
        None => Some(1),
    };
    if let Some(m) = max
        && m < min
    {
        return Err(SchemaError::Invalid(format!(
            "maxOccurs {m} < minOccurs {min}"
        )));
    }
    Ok((min, max))
}

/// An XML Schema boolean attribute: `true` and `1` count as true; anything else,
/// or no attribute, counts as false.
fn bool_attr(node: Node, name: &str) -> bool {
    matches!(
        node.attribute(name).map(str::trim),
        Some("true") | Some("1")
    )
}

/// Reads the `namespace` attribute of `xs:any` or `xs:anyAttribute` (default
/// `##any`). In a list, `##targetNamespace` becomes this document's target
/// namespace and `##local` becomes `""`
/// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Wildcards)).
fn parse_ns_constraint(node: Node, ctx: &Ctx) -> NsConstraint {
    match node
        .attribute("namespace")
        .map(str::trim)
        .unwrap_or("##any")
    {
        "##any" => NsConstraint::Any,
        "##other" => NsConstraint::Other(ctx.target_ns.clone()),
        list => NsConstraint::List(
            list.split_whitespace()
                .map(|t| match t {
                    "##targetNamespace" => ctx.target_ns.clone(),
                    "##local" => String::new(),
                    uri => uri.to_string(),
                })
                .collect(),
        ),
    }
}

/// Reads an `xs:element`. `global` is true for a top-level declaration.
///
/// - A global element always occurs exactly once and is in the target namespace.
/// - A local element is in the target namespace only if `form="qualified"`, or if
///   there is no `form` and the schema has `elementFormDefault="qualified"`.
///   Otherwise it has no namespace.
/// - `substitutionGroup` is kept only on global elements.
fn parse_element(node: Node, ctx: &Ctx, global: bool) -> Result<RawElement> {
    let (min, max) = if global { (1, Some(1)) } else { occurs(node)? };
    let reference = node
        .attribute("ref")
        .map(|r| resolve_qname(node, r))
        .transpose()?;
    let name = node.attribute("name").map(str::to_string);
    if reference.is_none() && name.is_none() {
        return Err(SchemaError::Invalid(
            "xs:element without name or ref".into(),
        ));
    }
    let ns = if global {
        ctx.target_ns.clone()
    } else {
        match node.attribute("form") {
            Some("qualified") => ctx.target_ns.clone(),
            Some(_) => String::new(),
            None if ctx.element_qualified => ctx.target_ns.clone(),
            None => String::new(),
        }
    };
    let type_name = node
        .attribute("type")
        .map(|t| resolve_qname(node, t))
        .transpose()?;
    let mut inline_type = None;
    for child in node.children().filter(|n| n.is_element()) {
        match xs_name(child) {
            Some("complexType") => {
                inline_type = Some(Box::new(RawType::Complex(parse_complex_type(child, ctx)?)));
            }
            Some("simpleType") => {
                inline_type = Some(Box::new(RawType::Simple(parse_simple_type(child, ctx)?)));
            }
            _ => {}
        }
    }
    let substitution_group = node
        .attribute("substitutionGroup")
        .map(|s| resolve_qname(node, s))
        .transpose()?;
    Ok(RawElement {
        name,
        ns,
        reference,
        type_name,
        inline_type,
        min,
        max,
        nillable: bool_attr(node, "nillable"),
        substitution_group: if global { substitution_group } else { None },
        is_abstract: bool_attr(node, "abstract"),
    })
}

/// Reads an `xs:attribute`. The namespace rules are the same as for elements, but
/// use `attributeFormDefault`. `use` defaults to optional.
fn parse_attribute(node: Node, ctx: &Ctx, global: bool) -> Result<RawAttribute> {
    let reference = node
        .attribute("ref")
        .map(|r| resolve_qname(node, r))
        .transpose()?;
    let name = node.attribute("name").map(str::to_string);
    if reference.is_none() && name.is_none() {
        return Err(SchemaError::Invalid(
            "xs:attribute without name or ref".into(),
        ));
    }
    let ns = if global {
        ctx.target_ns.clone()
    } else {
        match node.attribute("form") {
            Some("qualified") => ctx.target_ns.clone(),
            Some(_) => String::new(),
            None if ctx.attribute_qualified => ctx.target_ns.clone(),
            None => String::new(),
        }
    };
    let type_name = node
        .attribute("type")
        .map(|t| resolve_qname(node, t))
        .transpose()?;
    let inline_type = node
        .children()
        .find(|n| is_xs(*n, "simpleType"))
        .map(|n| parse_simple_type(n, ctx))
        .transpose()?;
    let use_ = match node.attribute("use").map(str::trim) {
        Some("required") => AttrUse::Required,
        Some("prohibited") => AttrUse::Prohibited,
        _ => AttrUse::Optional,
    };
    Ok(RawAttribute {
        name,
        ns,
        reference,
        type_name,
        inline_type,
        use_,
    })
}

/// Collects the `xs:attribute`, `xs:attributeGroup ref` and `xs:anyAttribute`
/// children of `node`.
fn parse_attribute_items(
    node: Node,
    ctx: &Ctx,
) -> Result<(Vec<RawAttrItem>, Option<NsConstraint>)> {
    let mut items = Vec::new();
    let mut any = None;
    for child in node.children().filter(|n| n.is_element()) {
        match xs_name(child) {
            Some("attribute") => {
                items.push(RawAttrItem::Attribute(parse_attribute(child, ctx, false)?))
            }
            Some("attributeGroup") => {
                let r = required(child, "ref")?;
                items.push(RawAttrItem::GroupRef(resolve_qname(child, r)?));
            }
            Some("anyAttribute") => any = Some(parse_ns_constraint(child, ctx)),
            _ => {}
        }
    }
    Ok((items, any))
}

/// Reads one particle: `xs:element`, `xs:any`, a model group (`xs:sequence`,
/// `xs:choice`, `xs:all`, read with recursion) or an `xs:group ref`. Returns `None`
/// for any other child, such as `xs:annotation`.
fn parse_particle(node: Node, ctx: &Ctx) -> Result<Option<RawParticle>> {
    Ok(match xs_name(node) {
        Some("element") => Some(RawParticle::Element(parse_element(node, ctx, false)?)),
        Some("any") => {
            let (min, max) = occurs(node)?;
            Some(RawParticle::Any {
                ns: parse_ns_constraint(node, ctx),
                min,
                max,
            })
        }
        Some(c @ ("sequence" | "choice" | "all")) => {
            let compositor = match c {
                "sequence" => Compositor::Sequence,
                "choice" => Compositor::Choice,
                _ => Compositor::All,
            };
            let (min, max) = occurs(node)?;
            let mut particles = Vec::new();
            for child in node.children().filter(|n| n.is_element()) {
                if let Some(p) = parse_particle(child, ctx)? {
                    particles.push(p);
                }
            }
            Some(RawParticle::Group {
                compositor,
                particles,
                min,
                max,
            })
        }
        Some("group") => {
            let r = required(node, "ref")?;
            let (min, max) = occurs(node)?;
            Some(RawParticle::GroupRef {
                name: resolve_qname(node, r)?,
                min,
                max,
            })
        }
        _ => None,
    })
}

/// The first child of `node` that is a particle, if any.
fn first_particle(node: Node, ctx: &Ctx) -> Result<Option<RawParticle>> {
    for child in node.children().filter(|n| n.is_element()) {
        if let Some(p) = parse_particle(child, ctx)? {
            return Ok(Some(p));
        }
    }
    Ok(None)
}

/// Reads an `xs:complexType`
/// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Complex_Type_Definitions)).
///
/// The type can take one of three shapes:
///
/// - `xs:complexContent` with `xs:extension` or `xs:restriction` of a base type.
///   `mixed` on `xs:complexContent` is added to `mixed` on the type.
/// - `xs:simpleContent` with `xs:extension` or `xs:restriction` of a base type.
///   A restriction may carry facets and an inline `xs:simpleType`.
/// - A model group or `xs:group ref` directly inside the type
///   ([`RawDerivation::None`]).
///
/// With none of these there is no particle (`RawDerivation::None` with `None`).
/// With `xs:complexContent` or `xs:simpleContent`, the attributes inside the
/// extension or restriction replace any found directly on the type.
fn parse_complex_type(node: Node, ctx: &Ctx) -> Result<RawComplexType> {
    let name = node.attribute("name").map(str::to_string);
    let mut mixed = bool_attr(node, "mixed");
    let mut derivation = None;
    let (mut attributes, mut any_attribute) = parse_attribute_items(node, ctx)?;
    for child in node.children().filter(|n| n.is_element()) {
        match xs_name(child) {
            Some("complexContent") => {
                mixed |= bool_attr(child, "mixed");
                for d in child.children().filter(|n| n.is_element()) {
                    let base = match xs_name(d) {
                        Some("extension" | "restriction") => {
                            resolve_qname(d, required(d, "base")?)?
                        }
                        _ => continue,
                    };
                    let particle = first_particle(d, ctx)?;
                    let (attrs, any) = parse_attribute_items(d, ctx)?;
                    attributes = attrs;
                    any_attribute = any;
                    derivation = Some(if xs_name(d) == Some("extension") {
                        RawDerivation::ComplexExtension { base, particle }
                    } else {
                        RawDerivation::ComplexRestriction { base, particle }
                    });
                }
            }
            Some("simpleContent") => {
                for d in child.children().filter(|n| n.is_element()) {
                    let base = match xs_name(d) {
                        Some("extension" | "restriction") => {
                            resolve_qname(d, required(d, "base")?)?
                        }
                        _ => continue,
                    };
                    let (attrs, any) = parse_attribute_items(d, ctx)?;
                    attributes = attrs;
                    any_attribute = any;
                    derivation = Some(if xs_name(d) == Some("extension") {
                        RawDerivation::SimpleExtension { base }
                    } else {
                        let facets = parse_facets(d)?;
                        let inline = d
                            .children()
                            .find(|n| is_xs(*n, "simpleType"))
                            .map(|n| parse_simple_type(n, ctx))
                            .transpose()?;
                        RawDerivation::SimpleRestriction {
                            base,
                            facets,
                            inline,
                        }
                    });
                }
            }
            Some("sequence" | "choice" | "all" | "group") => {
                derivation = Some(RawDerivation::None {
                    particle: parse_particle(child, ctx)?,
                });
            }
            _ => {}
        }
    }
    Ok(RawComplexType {
        name,
        ns: ctx.target_ns.clone(),
        mixed,
        derivation: derivation.unwrap_or(RawDerivation::None { particle: None }),
        attributes,
        any_attribute,
    })
}

/// Reads the facet children of an `xs:restriction`, skipping `xs:simpleType` and
/// `xs:annotation`.
fn parse_facets(node: Node) -> Result<Vec<Facet>> {
    let mut facets = Vec::new();
    for child in node.children().filter(|n| n.is_element()) {
        let Some(name) = xs_name(child) else { continue };
        if matches!(name, "simpleType" | "annotation") {
            continue;
        }
        let value = child.attribute("value").unwrap_or("").to_string();
        facets.push(match name {
            "enumeration" => Facet::Enumeration(value),
            "minInclusive" => Facet::MinInclusive(value),
            "maxInclusive" => Facet::MaxInclusive(value),
            "minExclusive" => Facet::MinExclusive(value),
            "maxExclusive" => Facet::MaxExclusive(value),
            "pattern" => Facet::Pattern(value),
            "whiteSpace" => Facet::WhiteSpace(match value.trim() {
                "preserve" => WhiteSpace::Preserve,
                "replace" => WhiteSpace::Replace,
                "collapse" => WhiteSpace::Collapse,
                other => {
                    return Err(SchemaError::Invalid(format!(
                        "bad whiteSpace facet '{other}'"
                    )));
                }
            }),
            other => Facet::Other(other.to_string(), value),
        });
    }
    Ok(facets)
}

/// Reads an `xs:simpleType`: a restriction, list or union
/// ([XML Schema 1.0 Part 1](https://www.w3.org/TR/xmlschema-1/#Simple_Type_Definitions)).
/// The base or item type may be named by an attribute or given inline as a child
/// `xs:simpleType`.
fn parse_simple_type(node: Node, ctx: &Ctx) -> Result<RawSimpleType> {
    let name = node.attribute("name").map(str::to_string);
    let mut variety = None;
    for child in node.children().filter(|n| n.is_element()) {
        match xs_name(child) {
            Some("restriction") => {
                let base = child
                    .attribute("base")
                    .map(|b| resolve_qname(child, b))
                    .transpose()?;
                let inline = child
                    .children()
                    .find(|n| is_xs(*n, "simpleType"))
                    .map(|n| parse_simple_type(n, ctx).map(Box::new))
                    .transpose()?;
                if base.is_none() && inline.is_none() {
                    return Err(SchemaError::Invalid("xs:restriction without base".into()));
                }
                variety = Some(RawVariety::Restriction {
                    base,
                    inline,
                    facets: parse_facets(child)?,
                });
            }
            Some("list") => {
                let item = child
                    .attribute("itemType")
                    .map(|b| resolve_qname(child, b))
                    .transpose()?;
                let inline = child
                    .children()
                    .find(|n| is_xs(*n, "simpleType"))
                    .map(|n| parse_simple_type(n, ctx).map(Box::new))
                    .transpose()?;
                if item.is_none() && inline.is_none() {
                    return Err(SchemaError::Invalid("xs:list without item type".into()));
                }
                variety = Some(RawVariety::List { item, inline });
            }
            Some("union") => {
                let members = child
                    .attribute("memberTypes")
                    .unwrap_or("")
                    .split_whitespace()
                    .map(|m| resolve_qname(child, m))
                    .collect::<Result<Vec<_>>>()?;
                let inline = child
                    .children()
                    .filter(|n| is_xs(*n, "simpleType"))
                    .map(|n| parse_simple_type(n, ctx))
                    .collect::<Result<Vec<_>>>()?;
                variety = Some(RawVariety::Union { members, inline });
            }
            _ => {}
        }
    }
    let variety = variety.ok_or_else(|| {
        SchemaError::Invalid(format!(
            "xs:simpleType {} has no restriction, list or union",
            name.as_deref().unwrap_or("<anonymous>")
        ))
    })?;
    Ok(RawSimpleType {
        name,
        ns: ctx.target_ns.clone(),
        variety,
    })
}
