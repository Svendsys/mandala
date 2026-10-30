// SPDX-License-Identifier: MPL-2.0

//! [`Objects`]: a deserializer
//! adapter that holds the whole file to being JSON *objects* where the
//! schema has objects.
//!
//! serde's derived `Deserialize` for a struct accepts two shapes: the
//! object a file means, and a *positional array of the field values*
//! (`[1, 0, [0, 0]]` for a node), because `serde_json` lets a struct's
//! visitor take either. `deny_unknown_fields` does not close that: an
//! array has no keys to be unknown. It is a second way to write every
//! object in the file, and the format has one way to write each thing
//! (D4). So the loader reads the text through this adapter, which hands
//! a struct's visitor only maps; a struct written as an array is an
//! `invalid type: sequence` error at the array's path.
//!
//! The adapter has to reach every nested value, so it wraps what serde
//! hands out at each step — the sequence, the map and the enum accessors
//! and the seeds they take — and is otherwise a pass-through. It changes
//! one thing: `Strict` is the visitor a struct's `deserialize_struct`
//! (or a struct variant) is given, and it refuses `visit_seq`.

use serde::de::{
    self, DeserializeSeed, Deserializer, EnumAccess, Expected, MapAccess, SeqAccess, Unexpected,
    VariantAccess, Visitor,
};
use std::fmt;

/// `D`, held to objects where a struct is expected. See the module docs.
pub(crate) struct Objects<D>(pub D);

/// Forward `deserialize_*(self, visitor)` methods through [`Through`].
macro_rules! forward_methods {
    ($($method:ident),* $(,)?) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
                self.0.$method(Through(visitor))
            }
        )*
    };
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for Objects<D> {
    type Error = D::Error;

    forward_methods!(
        deserialize_any,
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
        deserialize_option,
        deserialize_unit,
        deserialize_seq,
        deserialize_map,
        deserialize_identifier,
        deserialize_ignored_any,
    );

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_unit_struct(name, Through(visitor))
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_newtype_struct(name, Through(visitor))
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_tuple(len, Through(visitor))
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_tuple_struct(name, len, Through(visitor))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_struct(name, fields, Strict(visitor))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_enum(name, variants, Through(visitor))
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
}

/// Forward `visit_*(self, value)` methods of scalar values.
macro_rules! forward_scalars {
    ($($method:ident($ty:ty)),* $(,)?) => {
        $(
            fn $method<E: de::Error>(self, value: $ty) -> Result<V::Value, E> {
                self.0.$method(value)
            }
        )*
    };
}

/// A visitor that passes everything to `V`, wrapping whatever `V`
/// reads next so it is held to objects too.
struct Through<V>(V);

impl<'de, V: Visitor<'de>> Visitor<'de> for Through<V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(f)
    }

    forward_scalars!(
        visit_bool(bool),
        visit_i8(i8),
        visit_i16(i16),
        visit_i32(i32),
        visit_i64(i64),
        visit_i128(i128),
        visit_u8(u8),
        visit_u16(u16),
        visit_u32(u32),
        visit_u64(u64),
        visit_u128(u128),
        visit_f32(f32),
        visit_f64(f64),
        visit_char(char),
        visit_str(&str),
        visit_string(String),
        visit_bytes(&[u8]),
        visit_byte_buf(Vec<u8>),
    );

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<V::Value, E> {
        self.0.visit_borrowed_str(value)
    }

    fn visit_borrowed_bytes<E: de::Error>(self, value: &'de [u8]) -> Result<V::Value, E> {
        self.0.visit_borrowed_bytes(value)
    }

    fn visit_none<E: de::Error>(self) -> Result<V::Value, E> {
        self.0.visit_none()
    }

    fn visit_unit<E: de::Error>(self) -> Result<V::Value, E> {
        self.0.visit_unit()
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<V::Value, D::Error> {
        self.0.visit_some(Objects(deserializer))
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<V::Value, D::Error> {
        self.0.visit_newtype_struct(Objects(deserializer))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<V::Value, A::Error> {
        self.0.visit_seq(ThroughSeq(seq))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<V::Value, A::Error> {
        self.0.visit_map(ThroughMap(map))
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<V::Value, A::Error> {
        self.0.visit_enum(ThroughEnum(data))
    }
}

/// [`Through`] for a struct: the same, except that it refuses the
/// positional array.
struct Strict<V>(V);

/// An [`Expected`] that delegates to a visitor's `expecting`.
struct Expecting<'a, V>(&'a V);

impl<'de, V: Visitor<'de>> Expected for Expecting<'_, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(f)
    }
}

impl<'de, V: Visitor<'de>> Visitor<'de> for Strict<V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(f)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, _seq: A) -> Result<V::Value, A::Error> {
        Err(de::Error::invalid_type(Unexpected::Seq, &Expecting(&self.0)))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<V::Value, A::Error> {
        self.0.visit_map(ThroughMap(map))
    }
}

/// A seed whose value is read through [`Objects`].
struct ThroughSeed<S>(S);

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for ThroughSeed<S> {
    type Value = S::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<S::Value, D::Error> {
        self.0.deserialize(Objects(deserializer))
    }
}

/// A sequence whose elements are read through [`Objects`].
struct ThroughSeq<A>(A);

impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for ThroughSeq<A> {
    type Error = A::Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, A::Error> {
        self.0.next_element_seed(ThroughSeed(seed))
    }

    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}

/// A map whose keys and values are read through [`Objects`].
struct ThroughMap<A>(A);

impl<'de, A: MapAccess<'de>> MapAccess<'de> for ThroughMap<A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error> {
        self.0.next_key_seed(ThroughSeed(seed))
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.0.next_value_seed(ThroughSeed(seed))
    }

    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}

/// An enum whose variant name and payload are read through [`Objects`].
struct ThroughEnum<A>(A);

impl<'de, A: EnumAccess<'de>> EnumAccess<'de> for ThroughEnum<A> {
    type Error = A::Error;
    type Variant = ThroughVariant<A::Variant>;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self::Variant), A::Error> {
        let (value, variant) = self.0.variant_seed(ThroughSeed(seed))?;
        Ok((value, ThroughVariant(variant)))
    }
}

/// An enum variant whose payload is read through [`Objects`], a struct
/// variant as an object only.
struct ThroughVariant<A>(A);

impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for ThroughVariant<A> {
    type Error = A::Error;

    fn unit_variant(self) -> Result<(), A::Error> {
        self.0.unit_variant()
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, A::Error> {
        self.0.newtype_variant_seed(ThroughSeed(seed))
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, A::Error> {
        self.0.tuple_variant(len, Through(visitor))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, A::Error> {
        self.0.struct_variant(fields, Strict(visitor))
    }
}
