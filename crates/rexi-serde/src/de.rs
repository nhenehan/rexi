//! A serde [`Deserializer`](serde::Deserializer) over the decoded element tree.
//!
//! Struct fields are looked up by name in four places:
//! - the element's attributes, as `name`, `@name` or `{uri}name`;
//! - its child elements. A field gets all children with that name, so a `Vec<T>`
//!   field collects the repeats;
//! - `$text`, for character content;
//! - `$value`, for the children that no other field names. This is an
//!   `xs:choice` ([EXI 8.5.4.1.8.2](https://www.w3.org/TR/exi/#choiceGroupTerms)),
//!   read as an enum whose variant is the element name.
//!
//! Values come from the decoder's typed values. Strings and enumerations use the
//! value's standard text form.
//!
//! There are three deserializers: [`TextDe`] for one simple value, [`NodeDe`] for
//! one element, and `GroupDe` for all the child elements one field maps to.

use crate::prelude::*;
use serde::de::value::StrDeserializer;
use serde::de::{self, DeserializeSeed, IntoDeserializer, Visitor};

use rexi_core::{Int, OwnedValue, Uint};

use crate::error::{Error, Result};
use crate::node::{Node, Text};
use crate::write::{TEXT_FIELDS, XSI_NIL_FIELDS, XSI_TYPE_FIELDS};

fn str_de(s: &str) -> StrDeserializer<'_, Error> {
    s.into_deserializer()
}

/// Reads a float from its text form, with the XML Schema spellings `INF`, `-INF`
/// and `NaN` ([XML Schema double](https://www.w3.org/TR/xmlschema-2/#double)).
/// EXI Float and Decimal values both go through here.
fn parse_f64(t: &Text) -> Result<f64> {
    let s = t.lexical.trim();
    match s {
        "INF" => Ok(f64::INFINITY),
        "-INF" => Ok(f64::NEG_INFINITY),
        "NaN" => Ok(f64::NAN),
        _ => s
            .parse()
            .map_err(|_| Error::msg(format!("expected a number, found {s:?}"))),
    }
}

/// A simple value: an attribute value or an element's text.
pub(crate) struct TextDe(pub Text);

impl TextDe {
    /// The value as an `i64`: from the typed value when it fits, else by parsing
    /// the text. `None` if neither works.
    fn int(&self) -> Result<Option<i64>> {
        Ok(match &self.0.value {
            OwnedValue::Integer(Int::Small(i)) => Some(*i),
            OwnedValue::Unsigned(Uint::Small(u)) => i64::try_from(*u).ok(),
            _ => self.0.lexical.trim().parse::<i64>().ok(),
        })
    }

    /// Splits the value into the items of a sequence. Binary data gives one item
    /// per byte. An `xs:list` gives its typed items, each paired with its piece
    /// of the text. Anything else is split on whitespace into strings.
    fn items(&self) -> Vec<Text> {
        let pieces = self.0.lexical.split_whitespace();
        match &self.0.value {
            OwnedValue::Binary(bytes) => bytes
                .iter()
                .map(|b| Text {
                    value: OwnedValue::Unsigned(Uint::Small(u64::from(*b))),
                    lexical: b.to_string(),
                })
                .collect(),
            OwnedValue::List(items) => items
                .iter()
                .cloned()
                .zip(pieces)
                .map(|(value, lexical)| Text {
                    value,
                    lexical: lexical.to_string(),
                })
                .collect(),
            _ => pieces
                .map(|p| Text {
                    value: OwnedValue::String(p.to_string()),
                    lexical: p.to_string(),
                })
                .collect(),
        }
    }
}

// Signed integer methods: all go through `int` and `visit_i64`. Serde checks
// that the value fits the smaller type.
macro_rules! text_signed {
    ($($m:ident),*) => {$(
        fn $m<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
            match self.int()? {
                Some(i) => v.visit_i64(i),
                None => Err(Error::msg(format!("expected an integer, found {:?}", self.0.lexical))),
            }
        }
    )*};
}

// Unsigned integer methods: the typed value if it is a small integer, else the
// text parsed as `u64`.
macro_rules! text_unsigned {
    ($($m:ident),*) => {$(
        fn $m<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
            match &self.0.value {
                OwnedValue::Unsigned(Uint::Small(u)) => v.visit_u64(*u),
                OwnedValue::Integer(Int::Small(i)) => v.visit_i64(*i),
                _ => match self.0.lexical.trim().parse::<u64>() {
                    Ok(u) => v.visit_u64(u),
                    Err(_) => Err(Error::msg(format!(
                        "expected an unsigned integer, found {:?}",
                        self.0.lexical
                    ))),
                },
            }
        }
    )*};
}

impl<'de> de::Deserializer<'de> for TextDe {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match &self.0.value {
            OwnedValue::Boolean(b) => v.visit_bool(*b),
            OwnedValue::Integer(Int::Small(i)) => v.visit_i64(*i),
            OwnedValue::Unsigned(Uint::Small(u)) => v.visit_u64(*u),
            OwnedValue::Float(_) | OwnedValue::Decimal(_) => v.visit_f64(parse_f64(&self.0)?),
            OwnedValue::Binary(b) => v.visit_bytes(b),
            OwnedValue::List(_) => self.deserialize_seq(v),
            _ => v.visit_str(&self.0.lexical),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match &self.0.value {
            OwnedValue::Boolean(b) => v.visit_bool(*b),
            _ => match self.0.lexical.trim() {
                "true" | "1" => v.visit_bool(true),
                "false" | "0" => v.visit_bool(false),
                s => Err(Error::msg(format!("expected a boolean, found {s:?}"))),
            },
        }
    }

    text_signed!(
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64
    );
    text_unsigned!(
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64
    );

    fn deserialize_i128<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match self.0.lexical.trim().parse::<i128>() {
            Ok(i) => v.visit_i128(i),
            Err(_) => Err(Error::msg(format!(
                "expected an integer, found {:?}",
                self.0.lexical
            ))),
        }
    }

    fn deserialize_u128<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match self.0.lexical.trim().parse::<u128>() {
            Ok(u) => v.visit_u128(u),
            Err(_) => Err(Error::msg(format!(
                "expected an unsigned integer, found {:?}",
                self.0.lexical
            ))),
        }
    }

    fn deserialize_f32<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_f64(parse_f64(&self.0)?)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_f64(parse_f64(&self.0)?)
    }

    fn deserialize_char<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        let mut chars = self.0.lexical.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => v.visit_char(c),
            _ => Err(Error::msg(format!(
                "expected one character, found {:?}",
                self.0.lexical
            ))),
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_str(&self.0.lexical)
    }

    fn deserialize_string<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_string(self.0.lexical)
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_str(&self.0.lexical)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match &self.0.value {
            OwnedValue::Binary(b) => v.visit_bytes(b),
            _ => v.visit_bytes(self.0.lexical.as_bytes()),
        }
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        self.deserialize_bytes(v)
    }

    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_some(self)
    }

    fn deserialize_unit<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        v: V,
    ) -> Result<V::Value> {
        v.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        v: V,
    ) -> Result<V::Value> {
        v.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_seq(TextSeq(self.items().into_iter()))
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, v: V) -> Result<V::Value> {
        self.deserialize_seq(v)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        v: V,
    ) -> Result<V::Value> {
        self.deserialize_seq(v)
    }

    fn deserialize_map<V: Visitor<'de>>(self, _v: V) -> Result<V::Value> {
        Err(Error::msg(format!(
            "expected element content, found the simple value {:?}",
            self.0.lexical
        )))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        self.deserialize_map(v)
    }

    /// A unit variant named by the value's text, that is, an enumeration value
    /// ([EXI 7.2](https://www.w3.org/TR/exi/#encodingEnumerations)).
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        v.visit_enum(str_de(self.0.lexical.trim()))
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_unit()
    }
}

/// The items of a simple value, one sequence item each.
struct TextSeq(alloc::vec::IntoIter<Text>);

impl<'de> de::SeqAccess<'de> for TextSeq {
    type Error = Error;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        self.0
            .next()
            .map(|t| seed.deserialize(TextDe(t)))
            .transpose()
    }
}

/// One element, from the tree that `node::read_document` built.
pub(crate) struct NodeDe<'a>(pub &'a Node);

impl NodeDe<'_> {
    /// The element's character content as a simple value. An element with no text
    /// and no children gives the empty string. One with children but no text is
    /// an error.
    fn text(&self) -> Result<TextDe> {
        match &self.0.text {
            Some(t) => Ok(TextDe(t.clone())),
            None if self.0.children.is_empty() => Ok(TextDe(Text {
                value: OwnedValue::String(String::new()),
                lexical: String::new(),
            })),
            None => Err(Error::msg(format!(
                "element {} has element content, expected a simple value",
                self.0.local
            ))),
        }
    }
}

// Methods for simple values: read the element's text.
macro_rules! via_text {
    ($($m:ident),*) => {$(
        fn $m<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
            self.text()?.$m(v)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for NodeDe<'_> {
    type Error = Error;

    /// An element with attributes, children or `xsi:type` is a map. Otherwise a
    /// nil element is `None`, an element with text is that value, and an empty
    /// element is `()`.
    fn deserialize_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        if !self.0.is_simple() {
            v.visit_map(NodeMap::new(self.0, None))
        } else if self.0.nil {
            v.visit_none()
        } else if self.0.text.is_some() {
            self.text()?.deserialize_any(v)
        } else {
            v.visit_unit()
        }
    }

    via_text!(
        deserialize_bool,
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64,
        deserialize_i128,
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64,
        deserialize_u128,
        deserialize_f32,
        deserialize_f64,
        deserialize_char,
        deserialize_str,
        deserialize_string,
        deserialize_bytes,
        deserialize_byte_buf,
        deserialize_identifier,
        deserialize_seq
    );

    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        if self.0.nil {
            v.visit_none()
        } else {
            v.visit_some(self)
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        v: V,
    ) -> Result<V::Value> {
        v.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        v: V,
    ) -> Result<V::Value> {
        v.visit_newtype_struct(self)
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, v: V) -> Result<V::Value> {
        self.deserialize_seq(v)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        v: V,
    ) -> Result<V::Value> {
        self.deserialize_seq(v)
    }

    fn deserialize_map<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_map(NodeMap::new(self.0, None))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        v.visit_map(NodeMap::new(self.0, Some(fields)))
    }

    /// An `xs:choice` branch, when the element's name is one of the variants. Or,
    /// for a simple element, an enumeration value in the element's text.
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        if variants.contains(&self.0.local.as_str()) {
            v.visit_enum(NodeEnum(self.0))
        } else if self.0.is_simple() {
            self.text()?.deserialize_enum(name, variants, v)
        } else {
            Err(Error::msg(format!(
                "element {} is not one of the variants of {name}: {}",
                self.0.local,
                variants.join(", ")
            )))
        }
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_unit()
    }
}

/// The value of a struct field: one simple value, or a group of child elements.
enum Entry<'a> {
    Text(Text),
    Group(Vec<&'a Node>),
}

/// An element's attributes, text and children as a map keyed by field name. The
/// entries come in this order: `xsi:type`, `xsi:nil`, attributes, text, then child
/// groups.
struct NodeMap<'a> {
    entries: alloc::vec::IntoIter<(String, Entry<'a>)>,
    value: Option<Entry<'a>>,
}

impl<'a> NodeMap<'a> {
    /// Builds the entries. `fields` is the struct's field list, or `None` when
    /// the target is a map or `deserialize_any`; then plain local names are used
    /// as keys.
    ///
    /// Only the local name of `xsi:type` is kept. An `xsi:nil` entry is added only
    /// when the element is nil and the struct has an `xsi:nil` field. Text goes to
    /// `$text`, or to `$value` if there is no `$text` field.
    fn new(node: &'a Node, fields: Option<&[&str]>) -> Self {
        let has = |k: &str| fields.is_some_and(|f| f.contains(&k));
        // The key a field uses for a name: the first spelling found in `fields`
        // (`local`, `@local`, `{uri}local`, `@{uri}local`), or the local name when
        // there is no field list. `None` means no field takes it.
        let key = |prefix: &str, uri: &str, local: &str| -> Option<String> {
            let candidates = [
                local.to_string(),
                format!("{prefix}{local}"),
                format!("{{{uri}}}{local}"),
                format!("{prefix}{{{uri}}}{local}"),
            ];
            match fields {
                None => Some(local.to_string()),
                Some(_) => candidates.into_iter().find(|c| has(c)),
            }
        };
        let mut entries = Vec::new();
        let text = |s: &str| Text {
            value: OwnedValue::String(s.to_string()),
            lexical: s.to_string(),
        };

        if let Some((_, local)) = &node.xsi_type {
            let k = XSI_TYPE_FIELDS
                .iter()
                .find(|k| has(k))
                .unwrap_or(&XSI_TYPE_FIELDS[0]);
            entries.push((k.to_string(), Entry::Text(text(local))));
        }
        if node.nil
            && let Some(k) = XSI_NIL_FIELDS.iter().find(|k| has(k))
        {
            entries.push((
                k.to_string(),
                Entry::Text(Text {
                    value: OwnedValue::Boolean(true),
                    lexical: "true".into(),
                }),
            ));
        }
        for (uri, local, value) in &node.attrs {
            if let Some(k) = key("@", uri, local) {
                entries.push((k, Entry::Text(value.clone())));
            }
        }
        let mut value_taken = false;
        if let Some(t) = &node.text {
            let k = if fields.is_none() || has(TEXT_FIELDS[0]) {
                Some(TEXT_FIELDS[0])
            } else if has(TEXT_FIELDS[1]) {
                value_taken = true;
                Some(TEXT_FIELDS[1])
            } else {
                None
            };
            if let Some(k) = k {
                entries.push((k.to_string(), Entry::Text(t.clone())));
            }
        }

        // Children grouped by name (URI and local name), in the order each name
        // first appears.
        let mut groups: Vec<(&str, &str, Vec<&Node>)> = Vec::new();
        for c in &node.children {
            match groups
                .iter_mut()
                .find(|(u, l, _)| *u == c.uri && *l == c.local)
            {
                Some(g) => g.2.push(c),
                None => groups.push((&c.uri, &c.local, vec![c])),
            }
        }
        let mut leftovers: Vec<&Node> = Vec::new();
        for (uri, local, nodes) in groups {
            match key("", uri, local) {
                Some(k) => entries.push((k, Entry::Group(nodes))),
                None => leftovers.extend(nodes),
            }
        }
        if !leftovers.is_empty() {
            if has(TEXT_FIELDS[1]) && !value_taken {
                // Keep document order for a `Vec` of choices. Grouping by name
                // broke that order, so sort by each child's position.
                leftovers.sort_by_key(|n| {
                    node.children
                        .iter()
                        .position(|c| core::ptr::eq(c, *n))
                        .unwrap_or(usize::MAX)
                });
                entries.push((TEXT_FIELDS[1].to_string(), Entry::Group(leftovers)));
            } else {
                // Unknown fields: ignored, or rejected by `deny_unknown_fields`.
                // Each child becomes its own entry, keyed by local name.
                for n in leftovers {
                    entries.push((n.local.clone(), Entry::Group(vec![n])));
                }
            }
        }
        NodeMap {
            entries: entries.into_iter(),
            value: None,
        }
    }
}

impl<'de> de::MapAccess<'de> for NodeMap<'_> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        let Some((k, v)) = self.entries.next() else {
            return Ok(None);
        };
        self.value = Some(v);
        seed.deserialize(str_de(&k)).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        match self.value.take() {
            Some(Entry::Text(t)) => seed.deserialize(TextDe(t)),
            Some(Entry::Group(nodes)) => seed.deserialize(GroupDe(nodes)),
            None => Err(Error::msg("map value requested before its key")),
        }
    }
}

/// All the child elements that one field maps to. Most methods need exactly one
/// element; `Vec` fields and `Option` fields can take any number.
struct GroupDe<'a>(Vec<&'a Node>);

impl<'a> GroupDe<'a> {
    /// The one element of the group. Zero or more than one is an error.
    fn single(self) -> Result<NodeDe<'a>> {
        match self.0.as_slice() {
            [n] => Ok(NodeDe(n)),
            [first, ..] => Err(Error::msg(format!(
                "{} {} elements for a single-valued field; use a Vec",
                self.0.len(),
                first.local
            ))),
            [] => Err(Error::msg("no element for field")),
        }
    }
}

// Methods that need one element: forward to that element's `NodeDe`.
macro_rules! via_single {
    ($($m:ident),*) => {$(
        fn $m<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
            self.single()?.$m(v)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for GroupDe<'_> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        if self.0.len() == 1 {
            self.single()?.deserialize_any(v)
        } else {
            self.deserialize_seq(v)
        }
    }

    via_single!(
        deserialize_bool,
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64,
        deserialize_i128,
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64,
        deserialize_u128,
        deserialize_f32,
        deserialize_f64,
        deserialize_char,
        deserialize_str,
        deserialize_string,
        deserialize_bytes,
        deserialize_byte_buf,
        deserialize_identifier,
        deserialize_unit,
        deserialize_map
    );

    /// No element, or one nil element, is `None`.
    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match self.0.as_slice() {
            [] => v.visit_none(),
            [n] if n.nil => v.visit_none(),
            _ => v.visit_some(self),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        v: V,
    ) -> Result<V::Value> {
        self.single()?.deserialize_unit_struct(name, v)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        v: V,
    ) -> Result<V::Value> {
        v.visit_newtype_struct(self)
    }

    /// Repeated elements. Or, for one simple element holding an `xs:list` or
    /// binary value, the items of that value.
    fn deserialize_seq<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        if let [n] = self.0.as_slice()
            && n.is_simple()
            && matches!(
                n.text.as_ref().map(|t| &t.value),
                Some(OwnedValue::List(_) | OwnedValue::Binary(_))
            )
        {
            return NodeDe(n).deserialize_seq(v);
        }
        v.visit_seq(NodeSeq(self.0.into_iter()))
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, v: V) -> Result<V::Value> {
        self.deserialize_seq(v)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        v: V,
    ) -> Result<V::Value> {
        self.deserialize_seq(v)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        self.single()?.deserialize_struct(name, fields, v)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        self.single()?.deserialize_enum(name, variants, v)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_unit()
    }
}

/// The elements of a group, one sequence item each.
struct NodeSeq<'a>(alloc::vec::IntoIter<&'a Node>);

impl<'de> de::SeqAccess<'de> for NodeSeq<'_> {
    type Error = Error;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        self.0
            .next()
            .map(|n| seed.deserialize(NodeDe(n)))
            .transpose()
    }
}

/// An element as the enum variant named after it.
struct NodeEnum<'a>(&'a Node);

impl<'de, 'a> de::EnumAccess<'de> for NodeEnum<'a> {
    type Error = Error;
    type Variant = NodeDe<'a>;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, NodeDe<'a>)> {
        let v = seed.deserialize(str_de(&self.0.local))?;
        Ok((v, NodeDe(self.0)))
    }
}

impl<'de> de::VariantAccess<'de> for NodeDe<'_> {
    type Error = Error;

    fn unit_variant(self) -> Result<()> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value> {
        seed.deserialize(self)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, v: V) -> Result<V::Value> {
        de::Deserializer::deserialize_seq(self, v)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        de::Deserializer::deserialize_struct(self, "", fields, v)
    }
}
