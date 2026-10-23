//! Writes an [`Item`] tree to an [`Encoder`]. Every name and every value's datatype
//! is looked up in the grammar. These are the mapping rules described in the
//! crate docs:
//!
//! - a field is an attribute if the current grammar rule has `AT(qname)` for its
//!   name. Attributes are sent in grammar order, whatever the struct field order;
//! - element and attribute namespaces come from the matching production, so only
//!   names that could match more than one need a `{uri}local` rename;
//! - an enum variant that carries data is an `xs:choice` branch
//!   ([EXI 8.5.4.1.8.2](https://www.w3.org/TR/exi/#choiceGroupTerms)). The element
//!   is named after the variant, not the field;
//! - `$text` / `$value` holding a single value is the element's character content.
//!
//! A grammar rule is the list of productions allowed at one point in the stream.
//! The encoder tracks the current rule, and this module reads it with
//! `current_productions` to decide what to write next
//! ([EXI 8.1](https://www.w3.org/TR/exi/#grammarNotation),
//! [EXI 8.5.4](https://www.w3.org/TR/exi/#informedElemGrammars)).

use crate::prelude::*;
use rexi_core::io::ByteSink;

use rexi_core::grammar::Terminal;
use rexi_core::ids::{NONE, QNameId};
use rexi_core::{
    DatatypeKind, Decimal, Encoder, Float, GrammarTables, Int, OwnedValue, TypeResolver, Uint,
    Value,
};

use crate::error::{Error, Result};
use crate::item::{Item, Scalar};

/// Field names that carry character content.
pub(crate) const TEXT_FIELDS: [&str; 2] = ["$text", "$value"];
/// Field names that carry an `xsi:type` cast (`#[serde(tag = "@xsi:type")]`).
pub(crate) const XSI_TYPE_FIELDS: [&str; 2] = ["@xsi:type", "xsi:type"];
/// Field names that carry `xsi:nil`.
pub(crate) const XSI_NIL_FIELDS: [&str; 2] = ["@xsi:nil", "xsi:nil"];

/// A field or type name: `local`, `@local` or `{uri}local`. `uri` is `None` when
/// no namespace was given, which matches a name in any namespace.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Name<'a> {
    pub uri: Option<&'a str>,
    pub local: &'a str,
}

/// Splits a field or type name into its parts. A leading `@` is dropped.
pub(crate) fn parse_name(s: &str) -> Name<'_> {
    let s = s.strip_prefix('@').unwrap_or(s);
    if let Some(rest) = s.strip_prefix('{')
        && let Some((uri, local)) = rest.split_once('}')
    {
        return Name {
            uri: Some(uri),
            local,
        };
    }
    Name {
        uri: None,
        local: s,
    }
}

/// True if `name` names the qname `q`: the local names are equal, and so are the
/// URIs when `name` gives one.
fn name_matches(t: &impl GrammarTables, name: Name<'_>, q: QNameId) -> bool {
    t.local_name_str(q) == Some(name.local) && name.uri.is_none_or(|u| t.uri_str(q.uri) == Some(u))
}

fn qname_strings(t: &impl GrammarTables, q: QNameId) -> (String, String) {
    (
        t.uri_str(q.uri).unwrap_or_default().to_string(),
        t.local_name_str(q).unwrap_or_default().to_string(),
    )
}

/// A value ready to send: typed where the conversion is exact, otherwise as text.
/// The codec parses text with the grammar's datatype, the same as for XML input.
enum Prepared {
    Lexical(String),
    Typed(OwnedValue),
}

impl Prepared {
    fn as_value(&self) -> Value<'_> {
        match self {
            Prepared::Lexical(s) => Value::Lexical(s),
            Prepared::Typed(v) => v.as_value(),
        }
    }
}

/// Picks the typed form of a single value when it matches the grammar's datatype
/// `kind` exactly ([EXI 7.1](https://www.w3.org/TR/exi/#encodingDatatypes)):
///
/// - `bool` for Boolean;
/// - `i64` for Integer, n-bit Integer and Decimal; `u64` for the same when it fits
///   in `i64`, and for Unsigned Integer and Decimal always; a non-negative `i64`
///   for Unsigned Integer;
/// - `f32`/`f64` for Decimal, and `f32`/`f64`/`i64`/`u64` for Float, by way of
///   their text form, when that text parses;
/// - bytes for base64Binary and hexBinary.
///
/// Anything else is sent as text.
fn prepare_scalar(s: &Scalar, kind: DatatypeKind) -> Prepared {
    use DatatypeKind as K;
    use Scalar as S;
    let typed = |v| Prepared::Typed(v);
    match (kind, s) {
        (K::Boolean | K::BooleanPattern, S::Bool(b)) => typed(OwnedValue::Boolean(*b)),
        (K::Integer | K::NBit { .. } | K::Decimal, S::I64(v)) => {
            typed(OwnedValue::Integer(Int::Small(*v)))
        }
        (K::Integer | K::NBit { .. }, S::U64(v)) if i64::try_from(*v).is_ok() => {
            typed(OwnedValue::Integer(Int::Small(*v as i64)))
        }
        (K::UnsignedInteger | K::Decimal, S::U64(v)) => {
            typed(OwnedValue::Unsigned(Uint::Small(*v)))
        }
        (K::UnsignedInteger, S::I64(v)) if *v >= 0 => {
            typed(OwnedValue::Unsigned(Uint::Small(*v as u64)))
        }
        (K::Decimal, S::F32(_) | S::F64(_)) => match Decimal::from_lexical(&s.lexical()) {
            Some(d) => typed(OwnedValue::Decimal(d)),
            None => Prepared::Lexical(s.lexical()),
        },
        (K::Float, S::F32(_) | S::F64(_) | S::I64(_) | S::U64(_)) => {
            match Float::from_lexical(&s.lexical()) {
                Some(f) => typed(OwnedValue::Float(f)),
                None => Prepared::Lexical(s.lexical()),
            }
        }
        (K::Base64Binary | K::HexBinary, S::Bytes(b)) => typed(OwnedValue::Binary(b.clone())),
        _ => Prepared::Lexical(s.lexical()),
    }
}

/// Prepares an item as one value of datatype `kind`. `()` is the empty string. A
/// sequence is binary data when the datatype is binary and every item is a byte
/// (`u8` values widened to `u64`). Otherwise a sequence of single values is an
/// `xs:list`, joined with single spaces
/// ([EXI 7.1.11](https://www.w3.org/TR/exi/#encodingList)). Structures can not be
/// values. `what` names the place for error messages.
fn prepare(item: &Item, kind: DatatypeKind, what: &str) -> Result<Prepared> {
    match item {
        Item::Scalar(s) => Ok(prepare_scalar(s, kind)),
        Item::Unit => Ok(Prepared::Lexical(String::new())),
        Item::Seq(items) => {
            if matches!(kind, DatatypeKind::Base64Binary | DatatypeKind::HexBinary) {
                let bytes: Option<Vec<u8>> = items
                    .iter()
                    .map(|i| match i {
                        Item::Scalar(Scalar::U64(v)) => u8::try_from(*v).ok(),
                        _ => None,
                    })
                    .collect();
                if let Some(b) = bytes {
                    return Ok(Prepared::Typed(OwnedValue::Binary(b)));
                }
            }
            let mut parts = Vec::with_capacity(items.len());
            for i in items {
                match i {
                    Item::Scalar(s) => parts.push(s.lexical()),
                    _ => {
                        return Err(Error::msg(format!(
                            "{what}: only a sequence of simple values can be a list value"
                        )));
                    }
                }
            }
            Ok(Prepared::Lexical(parts.join(" ")))
        }
        Item::Struct { .. } | Item::Variant { .. } | Item::None => Err(Error::msg(format!(
            "{what}: expected a simple value, found a structure"
        ))),
    }
}

/// Describes the terminals (allowed events) of the current rule, for error
/// messages.
fn expected<W: ByteSink, G: GrammarTables>(enc: &Encoder<W, G>) -> String {
    let t = enc.tables();
    let parts: Vec<String> = enc
        .current_productions()
        .iter()
        .map(|p| match p.terminal {
            Terminal::Se(e) => t.element(e).map_or_else(
                || "SE(?)".to_string(),
                |d| format!("element {}", t.local_name_str(d.qname).unwrap_or("?")),
            ),
            Terminal::At(q, _) => format!("attribute {}", t.local_name_str(q).unwrap_or("?")),
            Terminal::Ch(_) | Terminal::ChUntyped => "character content".to_string(),
            Terminal::Ee => "end of element".to_string(),
            other => format!("{other:?}"),
        })
        .collect();
    parts.join(", ")
}

/// Writes a whole document: `SD`, the root element, `ED`. The root name is `root`
/// if given, else the struct's or variant's name. A top-level value that is not a
/// struct or variant needs `root`.
pub(crate) fn write_document<W: ByteSink, G: GrammarTables>(
    enc: &mut Encoder<W, G>,
    item: &Item,
    root: Option<&str>,
) -> Result<()> {
    let (name, content) = match item {
        Item::Struct { name, .. } => (root.unwrap_or(name), item),
        Item::Variant { name, inner } => (root.unwrap_or(name), inner.as_ref()),
        _ => match root {
            Some(r) => (r, item),
            None => {
                return Err(Error::msg(
                    "the top-level value must be a struct (named after the root element), \
                     or pass the root element name",
                ));
            }
        },
    };
    let (uri, local) = resolve_root(enc.tables(), parse_name(name))?;
    enc.start_document()?;
    write_element(enc, &uri, &local, content)?;
    enc.end_document()?;
    Ok(())
}

/// Finds the one global element that `name` names, as `(uri, local)`. Tables with
/// no global elements would need the built-in element grammar
/// ([EXI 8.4.3](https://www.w3.org/TR/exi/#builtinElemGrammars)), which is not
/// supported. No match, or more than one, is an error.
fn resolve_root(t: &impl GrammarTables, name: Name<'_>) -> Result<(String, String)> {
    if t.global_elements().is_empty() {
        return Err(Error::Exi(rexi_core::ExiError::Unsupported(
            rexi_core::Feature::BuiltInElementGrammar,
        )));
    }
    let matches: Vec<QNameId> = t
        .global_elements()
        .iter()
        .filter_map(|&e| t.element(e))
        .map(|d| d.qname)
        .filter(|&q| name_matches(t, name, q))
        .collect();
    match matches.as_slice() {
        [q] => Ok(qname_strings(t, *q)),
        [] => {
            let names: Vec<String> = t
                .global_elements()
                .iter()
                .filter_map(|&e| t.element(e))
                .map(|d| {
                    let (u, l) = qname_strings(t, d.qname);
                    format!("{{{u}}}{l}")
                })
                .collect();
            Err(Error::msg(format!(
                "no global element named {}; the schema declares {}",
                name.local,
                names.join(", ")
            )))
        }
        _ => Err(Error::msg(format!(
            "global element name {} is ambiguous; rename the root as {{uri}}{}",
            name.local, name.local
        ))),
    }
}

/// `SE`, the content, then `EE`.
fn write_element<W: ByteSink, G: GrammarTables>(
    enc: &mut Encoder<W, G>,
    uri: &str,
    local: &str,
    item: &Item,
) -> Result<()> {
    enc.start_element(uri, local)?;
    write_content(enc, local, item)?;
    enc.end_element()?;
    Ok(())
}

/// Writes an element's content. `ctx` is the element's local name, for error
/// messages.
fn write_content<W: ByteSink, G: GrammarTables>(
    enc: &mut Encoder<W, G>,
    ctx: &str,
    item: &Item,
) -> Result<()> {
    match item {
        Item::Unit | Item::None => Ok(()),
        Item::Scalar(_) | Item::Seq(_) => write_chars(enc, ctx, item),
        Item::Variant { name, inner } => write_child(enc, name, inner),
        Item::Struct { fields, .. } => write_struct(enc, ctx, fields),
    }
}

/// Writes character content (`CH`). The datatype comes from the first `CH`
/// production of the current rule; an untyped `CH` uses the `Untyped` kind. It is
/// an error if the rule has no `CH` production.
fn write_chars<W: ByteSink, G: GrammarTables>(
    enc: &mut Encoder<W, G>,
    ctx: &str,
    item: &Item,
) -> Result<()> {
    let kind = enc
        .current_productions()
        .iter()
        .find_map(|p| match p.terminal {
            Terminal::Ch(t) => Some(enc.tables().datatype(t)),
            Terminal::ChUntyped => Some(DatatypeKind::Untyped),
            _ => None,
        });
    let Some(kind) = kind else {
        return Err(Error::msg(format!(
            "{ctx}: character content not allowed here; expected {}",
            expected(enc)
        )));
    };
    let value = prepare(item, kind, ctx)?;
    enc.characters(&value.as_value())?;
    Ok(())
}

/// Writes a struct's fields in this order: `xsi:type`, `xsi:nil`, attributes,
/// character content, then child elements. `used` marks fields already written;
/// `None` fields start out marked, so they are skipped.
fn write_struct<W: ByteSink, G: GrammarTables>(
    enc: &mut Encoder<W, G>,
    ctx: &str,
    fields: &[(String, Item)],
) -> Result<()> {
    let mut used: Vec<bool> = fields
        .iter()
        .map(|(_, v)| matches!(v, Item::None))
        .collect();

    // xsi:type, then xsi:nil: both must come right after the start tag.
    for (i, (key, value)) in fields.iter().enumerate() {
        if used[i] || !XSI_TYPE_FIELDS.contains(&key.as_str()) {
            continue;
        }
        used[i] = true;
        let Item::Scalar(Scalar::Str(type_name)) = value else {
            return Err(Error::msg(format!("{ctx}: xsi:type must be a type name")));
        };
        let (uri, local) = resolve_type(enc.tables(), parse_name(type_name))?;
        enc.xsi_type(&uri, &local)?;
    }
    for (i, (key, value)) in fields.iter().enumerate() {
        if used[i] || !XSI_NIL_FIELDS.contains(&key.as_str()) {
            continue;
        }
        used[i] = true;
        match value {
            Item::Scalar(Scalar::Bool(nil)) => enc.xsi_nil(*nil)?,
            _ => return Err(Error::msg(format!("{ctx}: xsi:nil must be a bool"))),
        }
    }

    // Attributes, in the order the grammar's productions list them. Each pass
    // looks at the current rule's `AT` productions in order and takes the first
    // one that an unused field names. Writing it moves the grammar to the next
    // rule, and the loop asks again. It stops when no `AT` production matches.
    loop {
        let t = enc.tables();
        let pick = enc.current_productions().iter().find_map(|p| {
            let Terminal::At(q, ty) = p.terminal else {
                return None;
            };
            fields
                .iter()
                .enumerate()
                .find(|(i, (k, _))| {
                    !used[*i]
                        && !TEXT_FIELDS.contains(&k.as_str())
                        && name_matches(t, parse_name(k), q)
                })
                .map(|(i, _)| (i, q, ty))
        });
        let Some((i, q, ty)) = pick else { break };
        used[i] = true;
        let (uri, local) = qname_strings(t, q);
        let kind = t.datatype(ty);
        let value = prepare(&fields[i].1, kind, &format!("{ctx}/@{local}"))?;
        enc.attribute(&uri, &local, &value.as_value())?;
    }

    // Character content: a `$text` or `$value` field that holds a single value
    // or `()`. A `$value` field holding a structure (such as a choice) is left for
    // the child-element step.
    for (i, (key, value)) in fields.iter().enumerate() {
        if used[i] || !TEXT_FIELDS.contains(&key.as_str()) {
            continue;
        }
        if matches!(value, Item::Scalar(_) | Item::Unit) {
            used[i] = true;
            write_chars(enc, ctx, value)?;
        }
    }

    // Child elements, in field order.
    for (i, (key, value)) in fields.iter().enumerate() {
        if used[i] {
            continue;
        }
        write_child(enc, key, value)?;
    }
    Ok(())
}

/// Writes the field `field` as child elements. `None` writes nothing. A variant is
/// written as an element named after the variant. A sequence is one element when
/// the child's datatype is a list or binary, and otherwise one element per item.
fn write_child<W: ByteSink, G: GrammarTables>(
    enc: &mut Encoder<W, G>,
    field: &str,
    item: &Item,
) -> Result<()> {
    let (name, content) = match item {
        Item::None => return Ok(()),
        Item::Variant { name, inner } => (name.as_str(), inner.as_ref()),
        Item::Seq(items) => {
            // Either one element holding an `xs:list` (or binary) value, or the
            // element repeated once per item.
            if let Some((_, kind)) = find_child(enc, field)
                && matches!(
                    kind,
                    Some(
                        DatatypeKind::List { .. }
                            | DatatypeKind::Base64Binary
                            | DatatypeKind::HexBinary
                    )
                )
            {
                (field, item)
            } else {
                for it in items {
                    write_child(enc, field, it)?;
                }
                return Ok(());
            }
        }
        other => (field, other),
    };
    let Some((q, _)) = find_child(enc, name) else {
        return Err(Error::msg(format!(
            "no element {name} allowed here; expected {}",
            expected(enc)
        )));
    };
    let (uri, local) = qname_strings(enc.tables(), q);
    write_element(enc, &uri, &local, content)
}

/// The first `SE` production of the current rule that matches `name`, with the
/// datatype of the element's type (`None` for complex types).
fn find_child<W: ByteSink, G: GrammarTables>(
    enc: &Encoder<W, G>,
    name: &str,
) -> Option<(QNameId, Option<DatatypeKind>)> {
    let parsed = parse_name(name);
    let t = enc.tables();
    enc.current_productions()
        .iter()
        .find_map(|p| match p.terminal {
            Terminal::Se(e) => t
                .element(e)
                .filter(|d| name_matches(t, parsed, d.qname))
                .map(|d| (d.qname, t.type_decl(d.type_idx).and_then(|ty| ty.datatype))),
            _ => None,
        })
}

/// Finds the one named schema type that `name` names, for `xsi:type`. Anonymous
/// types (their qname URI is the `NONE` marker value) are skipped. No match, or more than
/// one, is an error.
fn resolve_type(t: &impl GrammarTables, name: Name<'_>) -> Result<(String, String)> {
    let matches: Vec<QNameId> = t
        .types()
        .iter()
        .filter(|d| d.qname.uri != NONE)
        .map(|d| d.qname)
        .filter(|&q| name_matches(t, name, q))
        .collect();
    match matches.as_slice() {
        [q] => Ok(qname_strings(t, *q)),
        [] => Err(Error::msg(format!(
            "xsi:type: no type named {}",
            name.local
        ))),
        _ => Err(Error::msg(format!(
            "xsi:type: type name {} is ambiguous; use {{uri}}{}",
            name.local, name.local
        ))),
    }
}
