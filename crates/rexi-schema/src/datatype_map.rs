//! Maps a simple type definition to a [`DatatypeKind`]: the way its values are
//! written in an EXI stream
//! ([EXI 7](https://www.w3.org/TR/exi/#encodingValues),
//! [Table 7-1 in 7.1](https://www.w3.org/TR/exi/#encodingDatatypes),
//! [7.1.5](https://www.w3.org/TR/exi/#encodingInteger),
//! [7.2](https://www.w3.org/TR/exi/#encodingEnumerations)).
//!
//! The nearest built-in type in the base chain decides the representation. Then the
//! facets that apply change it:
//!
//! - integer bounds pick n-bit, unsigned or signed integer;
//! - an enumeration facet picks the enumeration representation, unless the base is
//!   a union, a list, `QName` or `NOTATION`;
//! - a pattern on a boolean picks the two-bit form
//!   ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)).
//!
//! - patterns on a type derived from `xs:string` may give a restricted character
//!   set ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)), worked out
//!   in [`crate::charset`]. Such a type maps to
//!   [`DatatypeKind::RestrictedString`]. The schema still compiles, but the encoder
//!   and decoder refuse a value of that type, because rexi does not write that form
//!   yet. Patterns it cannot read map there too, so nothing is guessed.

use std::collections::HashMap;

use rexi_core::{DatatypeKind, TypeIdx, WhiteSpace};

use crate::builtin::{self, BUILTINS, Category};
use crate::charset::{self, CharSet};
use crate::error::{Result, SchemaError};
use crate::xsd::model::{Model, TypeId, TypeKind, Variety};

/// Maps types to datatypes and caches each answer, so each type is worked out only
/// once. It also owns the sets of enumeration values it creates.
pub struct DatatypeMapper<'m> {
    model: &'m Model,
    cache: HashMap<TypeId, Option<DatatypeKind>>,
    /// Enumeration value sets, in the order they were made. The `set` field of
    /// `DatatypeKind::Enumeration` is an index into this list.
    pub enum_sets: Vec<Vec<String>>,
}

impl<'m> DatatypeMapper<'m> {
    /// Makes a mapper for the types of `model`, with an empty cache.
    pub fn new(model: &'m Model) -> Self {
        Self {
            model,
            cache: HashMap::new(),
            enum_sets: Vec::new(),
        }
    }

    /// The datatype of a type. `None` for complex types.
    pub fn kind(&mut self, t: TypeId) -> Result<Option<DatatypeKind>> {
        if let Some(k) = self.cache.get(&t) {
            return Ok(*k);
        }
        let k = self.compute(t)?;
        self.cache.insert(t, k);
        Ok(k)
    }

    /// Works out the datatype with no cache. A list type uses the List
    /// representation ([EXI 7.1.11](https://www.w3.org/TR/exi/#encodingList)) and
    /// a union is a string with whitespace collapsed.
    fn compute(&mut self, t: TypeId) -> Result<Option<DatatypeKind>> {
        let def = self
            .model
            .types
            .get(t)
            .ok_or_else(|| SchemaError::Invalid(format!("type index {t} out of range")))?;
        let simple = match &def.kind {
            TypeKind::Simple(s) => s,
            TypeKind::Complex(_) => return Ok(None),
        };
        Ok(Some(match &simple.variety {
            Variety::List { item } => DatatypeKind::List {
                item: *item as TypeIdx,
            },
            Variety::Union { .. } => DatatypeKind::String {
                whitespace: WhiteSpace::Collapse,
            },
            Variety::Atomic => match self.effective_enumeration(t) {
                Some((enum_type, values)) => self.enumeration(enum_type, values)?,
                None => self.atomic(t)?,
            },
        }))
    }

    /// The nearest type in the base chain (starting with `t` itself) that declares
    /// enumeration facets, with its values.
    fn effective_enumeration(&self, t: TypeId) -> Option<(TypeId, Vec<String>)> {
        self.model.ancestors(t).into_iter().find_map(|a| {
            self.model.types[a]
                .simple()
                .and_then(|s| s.facets.enumeration.clone())
                .map(|v| (a, v))
        })
    }

    /// The datatype of a type whose nearest enumeration is declared on `enum_type`
    /// ([EXI 7.2](https://www.w3.org/TR/exi/#encodingEnumerations)). Values are
    /// written as an index into the list of allowed values. The spec makes an
    /// exception for lists, unions, `QName` and `NOTATION`: those keep the datatype
    /// of their base (a union becomes a string with whitespace collapsed).
    fn enumeration(&mut self, enum_type: TypeId, values: Vec<String>) -> Result<DatatypeKind> {
        // The item type is the nearest type in the base chain that is not itself an
        // enumeration.
        let mut base = self.model.types[enum_type].base;
        while let Some(b) = base {
            match self.kind(b)? {
                Some(DatatypeKind::Enumeration { .. }) => base = self.model.types[b].base,
                _ => break,
            }
        }
        let base = base.unwrap_or(builtin::ANY_SIMPLE_TYPE);
        let base_kind = self.kind(base)?.unwrap_or(DatatypeKind::String {
            whitespace: WhiteSpace::Preserve,
        });
        let base_variety = self.model.types[base].simple().map(|s| &s.variety);
        let base_builtin = self.model.closest_builtin(base);
        match base_kind {
            DatatypeKind::List { .. } => Ok(base_kind),
            _ if matches!(base_variety, Some(Variety::Union { .. })) => Ok(DatatypeKind::String {
                whitespace: WhiteSpace::Collapse,
            }),
            _ if matches!(base_builtin, Some(builtin::QNAME) | Some(builtin::NOTATION)) => {
                Ok(base_kind)
            }
            _ => {
                let set = self.enum_sets.len() as u32;
                self.enum_sets.push(values);
                Ok(DatatypeKind::Enumeration {
                    set,
                    item: base as TypeIdx,
                })
            }
        }
    }

    /// The datatype of an atomic type with no enumeration. Starts from the nearest
    /// built-in and applies the facets of the types between `t` and that built-in.
    ///
    /// - Strings: the nearest `whiteSpace` facet wins.
    /// - Integers: each `minInclusive`, `minExclusive`, `maxInclusive` or
    ///   `maxExclusive` can only narrow the range. Exclusive bounds are turned into
    ///   inclusive ones by adding or taking away 1.
    fn atomic(&self, t: TypeId) -> Result<DatatypeKind> {
        let b = self
            .model
            .closest_builtin(t)
            .unwrap_or(builtin::ANY_SIMPLE_TYPE);
        let chain = self.model.ancestors(t);
        // Types below the built-in (not counting the built-in), nearest first.
        let below: Vec<TypeId> = chain.iter().copied().take_while(|&a| a != b).collect();
        Ok(match BUILTINS[b].category {
            Category::AnyType => DatatypeKind::String {
                whitespace: WhiteSpace::Preserve,
            },
            Category::String(ws) => {
                let effective = below
                    .iter()
                    .find_map(|&a| {
                        self.model.types[a]
                            .simple()
                            .and_then(|s| s.facets.whitespace)
                    })
                    .unwrap_or(ws);
                if self.restricted_charset(&chain, &below) {
                    DatatypeKind::RestrictedString {
                        whitespace: effective,
                    }
                } else {
                    DatatypeKind::String {
                        whitespace: effective,
                    }
                }
            }
            Category::Boolean => {
                let pattern = below.iter().any(|&a| {
                    self.model.types[a]
                        .simple()
                        .is_some_and(|s| !s.facets.patterns.is_empty())
                });
                if pattern {
                    DatatypeKind::BooleanPattern
                } else {
                    DatatypeKind::Boolean
                }
            }
            Category::Decimal => DatatypeKind::Decimal,
            Category::Float => DatatypeKind::Float,
            Category::DateTime(k) => DatatypeKind::DateTime(k),
            Category::Base64Binary => DatatypeKind::Base64Binary,
            Category::HexBinary => DatatypeKind::HexBinary,
            Category::List(item) => DatatypeKind::List {
                item: item as TypeIdx,
            },
            Category::Integer { min, max } => {
                let (mut lo, mut hi) = (min, max);
                for &a in &below {
                    let Some(s) = self.model.types[a].simple() else {
                        continue;
                    };
                    if let Some(v) = &s.facets.min_inclusive {
                        lo = Some(lo.map_or(parse_int(v)?, |l| l.max(parse_int(v).unwrap_or(l))));
                    }
                    if let Some(v) = &s.facets.min_exclusive {
                        let v = parse_int(v)? + 1;
                        lo = Some(lo.map_or(v, |l| l.max(v)));
                    }
                    if let Some(v) = &s.facets.max_inclusive {
                        let v = parse_int(v)?;
                        hi = Some(hi.map_or(v, |h| h.min(v)));
                    }
                    if let Some(v) = &s.facets.max_exclusive {
                        let v = parse_int(v)? - 1;
                        hi = Some(hi.map_or(v, |h| h.min(v)));
                    }
                }
                integer_kind(lo, hi)
            }
        })
    }

    /// Whether a string type has a restricted character set
    /// ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)).
    ///
    /// - Only types whose base chain reaches the built-in `xs:string` count.
    ///   `anyURI`, `QName`, `duration` and others share the string category but are
    ///   not derived from `xs:string`.
    /// - Only the patterns of the nearest type that has its own patterns are used.
    ///   Built-in types have none, so they are never restricted.
    /// - A pattern that cannot be read counts as restricted, so the value is
    ///   refused instead of guessed.
    ///
    /// `chain` is `t` and all its base types, nearest first; `below` is the part
    /// of `chain` before the nearest built-in.
    fn restricted_charset(&self, chain: &[TypeId], below: &[TypeId]) -> bool {
        if !chain.contains(&builtin::STRING) {
            return false;
        }
        let Some(patterns) = below.iter().find_map(|&a| {
            self.model.types[a]
                .simple()
                .map(|s| &s.facets.patterns)
                .filter(|p| !p.is_empty())
        }) else {
            return false;
        };
        matches!(
            charset::analyze(patterns),
            CharSet::Restricted(_) | CharSet::Unreadable
        )
    }
}

/// Picks the integer representation
/// ([EXI 7.1.5](https://www.w3.org/TR/exi/#encodingInteger)):
///
/// - n-bit unsigned ([EXI 7.1.9](https://www.w3.org/TR/exi/#encodingBoundedUnsigned))
///   when both bounds are known, the range holds at most 4096 values and the lower
///   bound fits in an `i64`. The value
///   is written as its offset from `min` in just enough bits to hold `range - 1`
///   (0 bits when only one value is allowed);
/// - unsigned when the lower bound is 0 or more;
/// - signed otherwise.
pub fn integer_kind(lo: Option<i128>, hi: Option<i128>) -> DatatypeKind {
    if let (Some(lo), Some(hi)) = (lo, hi)
        && hi >= lo
    {
        let range = (hi - lo + 1) as u128;
        if range <= 4096
            && let Ok(min) = i64::try_from(lo)
        {
            let bits = (128 - (range - 1).leading_zeros()) as u8;
            return DatatypeKind::NBit { min, bits };
        }
    }
    if lo.is_some_and(|l| l >= 0) {
        DatatypeKind::UnsignedInteger
    } else {
        DatatypeKind::Integer
    }
}

/// Parses an integer facet value. Leading and trailing spaces and a leading `+`
/// are allowed.
fn parse_int(v: &str) -> Result<i128> {
    let s = v.trim();
    let s = s.strip_prefix('+').unwrap_or(s);
    s.parse::<i128>()
        .map_err(|_| SchemaError::Invalid(format!("integer facet value '{v}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_bounds() {
        assert_eq!(
            integer_kind(Some(-128), Some(127)),
            DatatypeKind::NBit { min: -128, bits: 8 }
        );
        assert_eq!(
            integer_kind(Some(0), Some(255)),
            DatatypeKind::NBit { min: 0, bits: 8 }
        );
        assert_eq!(
            integer_kind(Some(0), Some(65535)),
            DatatypeKind::UnsignedInteger
        );
        assert_eq!(
            integer_kind(Some(-32768), Some(32767)),
            DatatypeKind::Integer
        );
        assert_eq!(integer_kind(Some(0), None), DatatypeKind::UnsignedInteger);
        assert_eq!(integer_kind(None, Some(0)), DatatypeKind::Integer);
        assert_eq!(integer_kind(None, None), DatatypeKind::Integer);
        assert_eq!(
            integer_kind(Some(1), Some(4096)),
            DatatypeKind::NBit { min: 1, bits: 12 }
        );
        assert_eq!(
            integer_kind(Some(1), Some(4097)),
            DatatypeKind::UnsignedInteger
        );
        assert_eq!(
            integer_kind(Some(5), Some(5)),
            DatatypeKind::NBit { min: 5, bits: 0 }
        );
        assert_eq!(
            integer_kind(Some(-3), Some(3)),
            DatatypeKind::NBit { min: -3, bits: 3 }
        );
    }
}
