//! A serde [`Serializer`](serde::Serializer) that builds an [`Item`] tree. It
//! knows nothing about the schema; the `write` module matches the tree to the
//! grammar later.
//!
//! How Rust shapes map to items:
//! - integers, floats, `bool`, `char`, `&str` and bytes become an [`Item::Scalar`];
//! - `Some(x)` is just `x`, and `None` is [`Item::None`];
//! - `()` and unit structs are [`Item::Unit`];
//! - a unit enum variant is its name as a string (an enumeration value);
//! - sequences and tuples are [`Item::Seq`];
//! - structs and maps are [`Item::Struct`];
//! - newtype, tuple and struct enum variants are wrapped in [`Item::Variant`];
//! - newtype structs are the value inside.

use crate::prelude::*;
use serde::ser::{self, Serialize};

use crate::error::{Error, Result};
use crate::item::{Item, Scalar};

/// Serializes any value into an [`Item`] tree.
pub(crate) fn to_item<T: Serialize + ?Sized>(value: &T) -> Result<Item> {
    value.serialize(ItemSerializer)
}

pub(crate) struct ItemSerializer;

/// Collects the items of a sequence, tuple or tuple variant. `variant` is set for
/// a tuple variant, so the result is wrapped in [`Item::Variant`].
pub(crate) struct SeqBuilder {
    items: Vec<Item>,
    variant: Option<&'static str>,
}

/// Collects the fields of a struct, struct variant or map. `variant` is set for a
/// struct variant. `pending_key` holds a map key until its value arrives.
pub(crate) struct StructBuilder {
    name: String,
    fields: Vec<(String, Item)>,
    variant: Option<&'static str>,
    pending_key: Option<String>,
}

fn scalar(s: Scalar) -> Result<Item> {
    Ok(Item::Scalar(s))
}

impl ser::Serializer for ItemSerializer {
    type Ok = Item;
    type Error = Error;
    type SerializeSeq = SeqBuilder;
    type SerializeTuple = SeqBuilder;
    type SerializeTupleStruct = SeqBuilder;
    type SerializeTupleVariant = SeqBuilder;
    type SerializeMap = StructBuilder;
    type SerializeStruct = StructBuilder;
    type SerializeStructVariant = StructBuilder;

    fn serialize_bool(self, v: bool) -> Result<Item> {
        scalar(Scalar::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Item> {
        scalar(Scalar::I64(v.into()))
    }
    fn serialize_i16(self, v: i16) -> Result<Item> {
        scalar(Scalar::I64(v.into()))
    }
    fn serialize_i32(self, v: i32) -> Result<Item> {
        scalar(Scalar::I64(v.into()))
    }
    fn serialize_i64(self, v: i64) -> Result<Item> {
        scalar(Scalar::I64(v))
    }
    fn serialize_i128(self, v: i128) -> Result<Item> {
        scalar(Scalar::I128(v))
    }
    fn serialize_u8(self, v: u8) -> Result<Item> {
        scalar(Scalar::U64(v.into()))
    }
    fn serialize_u16(self, v: u16) -> Result<Item> {
        scalar(Scalar::U64(v.into()))
    }
    fn serialize_u32(self, v: u32) -> Result<Item> {
        scalar(Scalar::U64(v.into()))
    }
    fn serialize_u64(self, v: u64) -> Result<Item> {
        scalar(Scalar::U64(v))
    }
    fn serialize_u128(self, v: u128) -> Result<Item> {
        scalar(Scalar::U128(v))
    }
    fn serialize_f32(self, v: f32) -> Result<Item> {
        scalar(Scalar::F32(v))
    }
    fn serialize_f64(self, v: f64) -> Result<Item> {
        scalar(Scalar::F64(v))
    }
    fn serialize_char(self, v: char) -> Result<Item> {
        scalar(Scalar::Str(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Result<Item> {
        scalar(Scalar::Str(v.to_string()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Item> {
        scalar(Scalar::Bytes(v.to_vec()))
    }
    fn serialize_none(self) -> Result<Item> {
        Ok(Item::None)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Item> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<Item> {
        Ok(Item::Unit)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Item> {
        Ok(Item::Unit)
    }
    /// A unit variant is an enumeration value: its name, after any rename, is the
    /// value's text.
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Item> {
        scalar(Scalar::Str(variant.to_string()))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Item> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Item> {
        Ok(Item::Variant {
            name: variant.to_string(),
            inner: Box::new(value.serialize(self)?),
        })
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<SeqBuilder> {
        Ok(SeqBuilder {
            items: Vec::with_capacity(len.unwrap_or(0)),
            variant: None,
        })
    }
    fn serialize_tuple(self, len: usize) -> Result<SeqBuilder> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<SeqBuilder> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<SeqBuilder> {
        Ok(SeqBuilder {
            items: Vec::with_capacity(len),
            variant: Some(variant),
        })
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<StructBuilder> {
        Ok(StructBuilder {
            name: String::new(),
            fields: Vec::new(),
            variant: None,
            pending_key: None,
        })
    }
    fn serialize_struct(self, name: &'static str, len: usize) -> Result<StructBuilder> {
        Ok(StructBuilder {
            name: name.to_string(),
            fields: Vec::with_capacity(len),
            variant: None,
            pending_key: None,
        })
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<StructBuilder> {
        Ok(StructBuilder {
            name: variant.to_string(),
            fields: Vec::with_capacity(len),
            variant: Some(variant),
            pending_key: None,
        })
    }
}

impl SeqBuilder {
    fn push<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.items.push(to_item(value)?);
        Ok(())
    }

    fn done(self) -> Result<Item> {
        let seq = Item::Seq(self.items);
        Ok(match self.variant {
            Some(v) => Item::Variant {
                name: v.to_string(),
                inner: Box::new(seq),
            },
            None => seq,
        })
    }
}

impl ser::SerializeSeq for SeqBuilder {
    type Ok = Item;
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }
    fn end(self) -> Result<Item> {
        self.done()
    }
}

impl ser::SerializeTuple for SeqBuilder {
    type Ok = Item;
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }
    fn end(self) -> Result<Item> {
        self.done()
    }
}

impl ser::SerializeTupleStruct for SeqBuilder {
    type Ok = Item;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }
    fn end(self) -> Result<Item> {
        self.done()
    }
}

impl ser::SerializeTupleVariant for SeqBuilder {
    type Ok = Item;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }
    fn end(self) -> Result<Item> {
        self.done()
    }
}

impl StructBuilder {
    fn field<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) -> Result<()> {
        self.fields.push((key.to_string(), to_item(value)?));
        Ok(())
    }

    fn done(self) -> Result<Item> {
        let item = Item::Struct {
            name: self.name,
            fields: self.fields,
        };
        Ok(match self.variant {
            Some(v) => Item::Variant {
                name: v.to_string(),
                inner: Box::new(item),
            },
            None => item,
        })
    }
}

impl ser::SerializeMap for StructBuilder {
    type Ok = Item;
    type Error = Error;
    /// Map keys must be single values. They are turned into text and used as field
    /// names.
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<()> {
        match to_item(key)? {
            Item::Scalar(s) => {
                self.pending_key = Some(s.lexical());
                Ok(())
            }
            other => Err(Error::msg(format!(
                "map keys must be names (strings), got {other:?}"
            ))),
        }
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        let key = self
            .pending_key
            .take()
            .ok_or_else(|| Error::msg("map value without a key"))?;
        self.field(&key, value)
    }
    fn end(self) -> Result<Item> {
        self.done()
    }
}

impl ser::SerializeStruct for StructBuilder {
    type Ok = Item;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        self.field(key, value)
    }
    fn end(self) -> Result<Item> {
        self.done()
    }
}

impl ser::SerializeStructVariant for StructBuilder {
    type Ok = Item;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        self.field(key, value)
    }
    fn end(self) -> Result<Item> {
        self.done()
    }
}
