//! serde integration: convert any `Serialize` Rust value into a Lua [`Value`],
//! and any Lua [`Value`] into a `Deserialize` Rust type.
//!
//! Mirrors `mlua`'s `LuaSerdeExt` so a project migrating from `mlua` finds the
//! same `to_value` / `from_value` shape. This is a pure additive layer over the
//! existing [`Value`] model and the public `Table`/`LuaString` API — no VM, GC,
//! or `unsafe` involvement.
//!
//! Conventions (see the crate docs and the embedding spec):
//! - structs / maps become string-or-mixed-keyed tables; sequences and tuples
//!   become `1..n` array tables.
//! - Rust strings and byte strings both become Lua strings (Lua strings are
//!   bytes); on the way back, `&str`/`String` require valid UTF-8 while
//!   `&[u8]`/`Vec<u8>` accept any bytes.
//! - `None` / unit become `nil`; `nil` becomes `None` / unit.
//! - enums are externally tagged, matching `mlua`: a unit variant serializes to
//!   its name as a string; any other variant to a single-key table
//!   `{ "Variant" = payload }`.
//! - host integers cross the version number-model seam through
//!   [`crate::LossyIntPolicy`], identical to every other host→Lua integer.

use std::fmt::Display;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};
use serde::ser::{
    self, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant, Serializer,
};

use crate::{lower_host_int, Error, LoweredInt, Lua, LuaError, Result, Table, Value};

/// The canonical "null" marker. Lua tables cannot store `nil` as a value, so a
/// `None` / unit / JSON `null` in a *nested* position would silently delete the
/// table slot (corrupting sequences and dropping map keys). Instead those
/// serialize to this sentinel — a light-userdata at the address of a private
/// static, recognized on the way back. Mirrors `mlua`'s `Lua::null`.
static NULL_SENTINEL: u8 = 0;

/// The null sentinel as a [`Value`].
fn null_value() -> Value {
    Value::LightUserData(&NULL_SENTINEL as *const u8 as *mut core::ffi::c_void)
}

/// Whether `value` is the null sentinel from [`null_value`].
fn is_null(value: &Value) -> bool {
    matches!(value, Value::LightUserData(p) if std::ptr::eq(*p as *const u8, &NULL_SENTINEL))
}

/// Extract a host `i64` from an integer or an exactly-integral, in-range finite
/// float — the float case is how 5.1/5.2 (float-only) store integers. The range
/// guard avoids the saturating `as i64` cast (e.g. `1e20 as i64` would silently
/// become `i64::MAX`).
fn as_i64(value: &Value) -> Result<i64> {
    const MIN_F: f64 = -9_223_372_036_854_775_808.0;
    const MAX_PLUS_1_F: f64 = 9_223_372_036_854_775_808.0;
    match value {
        Value::Integer(i) => Ok(*i),
        Value::Number(n) if n.is_finite() && n.fract() == 0.0 => {
            if *n >= MIN_F && *n < MAX_PLUS_1_F {
                Ok(*n as i64)
            } else {
                Err(serde_error(format_args!(
                    "Lua number {n} is out of range for a 64-bit integer"
                )))
            }
        }
        other => Err(serde_error(format_args!(
            "expected an integer, found Lua {}",
            value_type_name(other)
        ))),
    }
}

/// The registry key under which each instance caches its serde array-marker
/// metatable.
const ARRAY_METATABLE_KEY: &str = "__omnilua_serde_array_metatable";

/// The per-instance array-marker metatable: an empty table (no metamethods) used
/// only as an identity tag so a Lua table that came from a serde sequence is
/// recognized as an array even when empty — otherwise an empty array and an
/// empty map are indistinguishable. Mirrors `mlua`'s `array_metatable`.
fn array_metatable(lua: &Lua) -> Result<Table> {
    let existing: Value = lua.named_registry_value(ARRAY_METATABLE_KEY)?;
    if let Value::Table(t) = existing {
        return Ok(t);
    }
    let mt = lua.create_table()?;
    lua.set_named_registry_value(ARRAY_METATABLE_KEY, &mt)?;
    Ok(mt)
}

/// Tag `table` as a serde array (give it the array-marker metatable).
fn mark_as_array(lua: &Lua, table: &Table) -> Result<()> {
    let mt = array_metatable(lua)?;
    table.set_metatable(Some(&mt))
}

/// Whether `table` carries the array-marker metatable.
fn is_marked_array(lua: &Lua, table: &Table) -> Result<bool> {
    match table.get_metatable()? {
        Some(mt) => Ok(mt.to_pointer()? == array_metatable(lua)?.to_pointer()?),
        None => Ok(false),
    }
}

impl ser::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        LuaError::runtime(format_args!("{msg}")).into()
    }
}

impl de::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        LuaError::runtime(format_args!("{msg}")).into()
    }
}

/// serde conversions between Rust types and Lua [`Value`]s, available on [`Lua`]
/// when the `serde` feature is enabled. The naming mirrors `mlua`'s
/// `LuaSerdeExt`.
pub trait LuaSerdeExt {
    /// Serialize any [`Serialize`] value into a Lua [`Value`] owned by this
    /// instance.
    fn to_value<T>(&self, value: &T) -> Result<Value>
    where
        T: Serialize + ?Sized;

    /// Deserialize a Lua [`Value`] into any owned [`DeserializeOwned`] type.
    fn from_value<T>(&self, value: Value) -> Result<T>
    where
        T: DeserializeOwned;

    /// The null sentinel (see the module docs): a non-`nil` marker for JSON
    /// `null` / `None` so it can be stored in a table slot. Deserializing it
    /// yields `None` / unit / `null` again.
    fn null(&self) -> Value;

    /// The array-marker metatable. Tables produced from serde sequences carry it
    /// so they round-trip as arrays even when empty; set it on a table yourself
    /// to force `from_value` to treat it as a sequence.
    fn array_metatable(&self) -> Result<Table>;
}

impl LuaSerdeExt for Lua {
    fn to_value<T>(&self, value: &T) -> Result<Value>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(LuaSerializer { lua: self })
    }

    fn from_value<T>(&self, value: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        T::deserialize(LuaDeserializer { lua: self, value })
    }

    fn null(&self) -> Value {
        null_value()
    }

    fn array_metatable(&self) -> Result<Table> {
        array_metatable(self)
    }
}

/// A serde [`Serializer`] producing Lua [`Value`]s in `lua`.
struct LuaSerializer<'a> {
    lua: &'a Lua,
}

impl<'a> LuaSerializer<'a> {
    /// Lower a host `i64` to the instance's number model (integer on 5.3+, float
    /// on 5.1/5.2, honoring the lossy-int policy) — the single host→Lua integer
    /// seam shared with the rest of the API.
    fn integer(&self, i: i64) -> Result<Value> {
        Ok(
            match lower_host_int(self.lua.version(), self.lua.lossy_int_policy(), i)? {
                LoweredInt::Int(i) => Value::Integer(i),
                LoweredInt::Float(f) => Value::Number(f),
            },
        )
    }

    /// Build a Lua string from raw bytes.
    fn string(&self, bytes: &[u8]) -> Result<Value> {
        Ok(Value::String(self.lua.create_string(bytes)?))
    }
}

impl<'a> Serializer for LuaSerializer<'a> {
    type Ok = Value;
    type Error = Error;

    type SerializeSeq = SeqSerializer<'a>;
    type SerializeTuple = SeqSerializer<'a>;
    type SerializeTupleStruct = SeqSerializer<'a>;
    type SerializeTupleVariant = TupleVariantSerializer<'a>;
    type SerializeMap = MapSerializer<'a>;
    type SerializeStruct = StructSerializer<'a>;
    type SerializeStructVariant = StructVariantSerializer<'a>;

    fn serialize_bool(self, v: bool) -> Result<Value> {
        Ok(Value::Boolean(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Value> {
        self.integer(i64::from(v))
    }

    fn serialize_i16(self, v: i16) -> Result<Value> {
        self.integer(i64::from(v))
    }

    fn serialize_i32(self, v: i32) -> Result<Value> {
        self.integer(i64::from(v))
    }

    fn serialize_i64(self, v: i64) -> Result<Value> {
        self.integer(v)
    }

    fn serialize_u8(self, v: u8) -> Result<Value> {
        self.integer(i64::from(v))
    }

    fn serialize_u16(self, v: u16) -> Result<Value> {
        self.integer(i64::from(v))
    }

    fn serialize_u32(self, v: u32) -> Result<Value> {
        self.integer(i64::from(v))
    }

    /// A `u64` above `i64::MAX` has no exact Lua integer representation (Lua
    /// integers are signed 64-bit), so rather than silently widen it to a lossy
    /// float, reject it — the caller can opt into a lossless encoding (e.g. a
    /// string) on their own type.
    fn serialize_u64(self, v: u64) -> Result<Value> {
        match i64::try_from(v) {
            Ok(i) => self.integer(i),
            Err(_) => Err(serde_error(format_args!(
                "u64 value {v} exceeds the i64 range Lua can represent exactly"
            ))),
        }
    }

    fn serialize_f32(self, v: f32) -> Result<Value> {
        Ok(Value::Number(f64::from(v)))
    }

    fn serialize_f64(self, v: f64) -> Result<Value> {
        Ok(Value::Number(v))
    }

    fn serialize_char(self, v: char) -> Result<Value> {
        let mut buf = [0u8; 4];
        let encoded = v.encode_utf8(&mut buf);
        self.string(encoded.as_bytes())
    }

    fn serialize_str(self, v: &str) -> Result<Value> {
        self.string(v.as_bytes())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Value> {
        self.string(v)
    }

    /// `None` serializes to the null sentinel, not `nil`, so it survives in a
    /// nested table slot (a `nil` value would delete the slot).
    fn serialize_none(self) -> Result<Value> {
        Ok(null_value())
    }

    fn serialize_some<T>(self, value: &T) -> Result<Value>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value> {
        Ok(null_value())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value> {
        Ok(null_value())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Value> {
        self.string(variant.as_bytes())
    }

    fn serialize_newtype_struct<T>(self, _name: &'static str, value: &T) -> Result<Value>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value>
    where
        T: Serialize + ?Sized,
    {
        let inner = value.serialize(LuaSerializer { lua: self.lua })?;
        let outer = self.lua.create_table()?;
        outer.raw_set(variant, inner)?;
        Ok(Value::Table(outer))
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<SeqSerializer<'a>> {
        let table = self.lua.create_table()?;
        mark_as_array(self.lua, &table)?;
        Ok(SeqSerializer {
            lua: self.lua,
            table,
            idx: 1,
        })
    }

    fn serialize_tuple(self, _len: usize) -> Result<SeqSerializer<'a>> {
        self.serialize_seq(None)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<SeqSerializer<'a>> {
        self.serialize_seq(None)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<TupleVariantSerializer<'a>> {
        let table = self.lua.create_table()?;
        mark_as_array(self.lua, &table)?;
        Ok(TupleVariantSerializer {
            lua: self.lua,
            variant,
            table,
            idx: 1,
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<MapSerializer<'a>> {
        Ok(MapSerializer {
            lua: self.lua,
            table: self.lua.create_table()?,
            pending_key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<StructSerializer<'a>> {
        Ok(StructSerializer {
            lua: self.lua,
            table: self.lua.create_table()?,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<StructVariantSerializer<'a>> {
        Ok(StructVariantSerializer {
            lua: self.lua,
            variant,
            table: self.lua.create_table()?,
        })
    }
}

/// Builds an array table for sequences, tuples, and tuple structs.
struct SeqSerializer<'a> {
    lua: &'a Lua,
    table: Table,
    idx: i64,
}

impl<'a> SeqSerializer<'a> {
    fn push<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let v = value.serialize(LuaSerializer { lua: self.lua })?;
        self.table.raw_set(self.idx, v)?;
        self.idx += 1;
        Ok(())
    }
}

impl<'a> SerializeSeq for SeqSerializer<'a> {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        self.push(value)
    }

    fn end(self) -> Result<Value> {
        Ok(Value::Table(self.table))
    }
}

impl<'a> SerializeTuple for SeqSerializer<'a> {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        self.push(value)
    }

    fn end(self) -> Result<Value> {
        Ok(Value::Table(self.table))
    }
}

impl<'a> SerializeTupleStruct for SeqSerializer<'a> {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        self.push(value)
    }

    fn end(self) -> Result<Value> {
        Ok(Value::Table(self.table))
    }
}

/// Builds the inner array of a tuple enum variant and wraps it as
/// `{ "Variant" = { ... } }`.
struct TupleVariantSerializer<'a> {
    lua: &'a Lua,
    variant: &'static str,
    table: Table,
    idx: i64,
}

impl<'a> SerializeTupleVariant for TupleVariantSerializer<'a> {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let v = value.serialize(LuaSerializer { lua: self.lua })?;
        self.table.raw_set(self.idx, v)?;
        self.idx += 1;
        Ok(())
    }

    fn end(self) -> Result<Value> {
        let outer = self.lua.create_table()?;
        outer.raw_set(self.variant, Value::Table(self.table))?;
        Ok(Value::Table(outer))
    }
}

/// Builds a table from a map, holding each key until its value arrives.
struct MapSerializer<'a> {
    lua: &'a Lua,
    table: Table,
    pending_key: Option<Value>,
}

impl<'a> SerializeMap for MapSerializer<'a> {
    type Ok = Value;
    type Error = Error;

    fn serialize_key<T>(&mut self, key: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        self.pending_key = Some(key.serialize(LuaSerializer { lua: self.lua })?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let key = self
            .pending_key
            .take()
            .ok_or_else(|| serde_error("serialize_value called before serialize_key"))?;
        if matches!(key, Value::Nil) {
            return Err(serde_error("map key serialized to nil, which Lua cannot store"));
        }
        let v = value.serialize(LuaSerializer { lua: self.lua })?;
        self.table.raw_set(key, v)?;
        Ok(())
    }

    fn end(self) -> Result<Value> {
        Ok(Value::Table(self.table))
    }
}

/// Builds a string-keyed table from a struct.
struct StructSerializer<'a> {
    lua: &'a Lua,
    table: Table,
}

impl<'a> SerializeStruct for StructSerializer<'a> {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let v = value.serialize(LuaSerializer { lua: self.lua })?;
        self.table.raw_set(key, v)?;
        Ok(())
    }

    fn end(self) -> Result<Value> {
        Ok(Value::Table(self.table))
    }
}

/// Builds the inner struct of a struct enum variant and wraps it as
/// `{ "Variant" = { ... } }`.
struct StructVariantSerializer<'a> {
    lua: &'a Lua,
    variant: &'static str,
    table: Table,
}

impl<'a> SerializeStructVariant for StructVariantSerializer<'a> {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let v = value.serialize(LuaSerializer { lua: self.lua })?;
        self.table.raw_set(key, v)?;
        Ok(())
    }

    fn end(self) -> Result<Value> {
        let outer = self.lua.create_table()?;
        outer.raw_set(self.variant, Value::Table(self.table))?;
        Ok(Value::Table(outer))
    }
}

/// Construct an embedding [`Error`] carrying a serde diagnostic message.
fn serde_error(msg: impl Display) -> Error {
    LuaError::runtime(format_args!("{msg}")).into()
}

/// A serde [`Deserializer`] reading a Lua [`Value`]. Owns its value and never
/// borrows from a `'de` input, so it deserializes only owned types.
struct LuaDeserializer<'a> {
    lua: &'a Lua,
    value: Value,
}

/// Whether `pairs` forms a dense `1..=n` array (integer or integral-float keys,
/// no gaps, no duplicates, no other keys). An empty table is *not* a sequence —
/// it is treated as an empty map, the only consistent default given Lua cannot
/// distinguish an empty array from an empty map.
fn sequence_len(pairs: &[(Value, Value)]) -> Option<u64> {
    let n = pairs.len() as u64;
    if n == 0 {
        return None;
    }
    let mut seen = vec![false; pairs.len()];
    for (key, _) in pairs {
        let idx = match key {
            Value::Integer(i) if *i >= 1 => *i as u64,
            Value::Number(f) if f.fract() == 0.0 && *f >= 1.0 && *f <= n as f64 => *f as u64,
            _ => return None,
        };
        if idx < 1 || idx > n {
            return None;
        }
        let slot = (idx - 1) as usize;
        if seen[slot] {
            return None;
        }
        seen[slot] = true;
    }
    Some(n)
}

/// Read a Lua array table's values in `1..=len` order.
fn table_seq_values(table: &Table) -> Result<Vec<Value>> {
    let len = table.len()?;
    let mut out = Vec::with_capacity(len as usize);
    for idx in 1..=len {
        out.push(table.raw_get::<i64, Value>(idx as i64)?);
    }
    Ok(out)
}

impl<'a, 'de> Deserializer<'de> for LuaDeserializer<'a> {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        if is_null(&self.value) {
            return visitor.visit_unit();
        }
        match self.value {
            Value::Nil => visitor.visit_unit(),
            Value::Boolean(b) => visitor.visit_bool(b),
            Value::Integer(i) => visitor.visit_i64(i),
            Value::Number(n) => visitor.visit_f64(n),
            Value::String(s) => {
                let bytes = s.as_bytes()?;
                match std::str::from_utf8(&bytes) {
                    Ok(text) => visitor.visit_str(text),
                    Err(_) => visitor.visit_bytes(&bytes),
                }
            }
            Value::Table(t) => {
                if is_marked_array(self.lua, &t)? {
                    return visitor.visit_seq(SeqAccessor {
                        lua: self.lua,
                        items: table_seq_values(&t)?.into_iter(),
                    });
                }
                let pairs = t.raw_pairs()?;
                match sequence_len(&pairs) {
                    Some(_) => visitor.visit_seq(SeqAccessor {
                        lua: self.lua,
                        items: table_seq_values(&t)?.into_iter(),
                    }),
                    None => visitor.visit_map(MapAccessor {
                        lua: self.lua,
                        pairs: pairs.into_iter(),
                        pending_value: None,
                    }),
                }
            }
            other => Err(serde_error(format_args!(
                "cannot deserialize Lua {} into a serde value",
                value_type_name(&other)
            ))),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        if matches!(self.value, Value::Nil) || is_null(&self.value) {
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        if matches!(self.value, Value::Nil) || is_null(&self.value) {
            visitor.visit_unit()
        } else {
            Err(serde_error(format_args!(
                "expected nil or the null sentinel for unit, found Lua {}",
                value_type_name(&self.value)
            )))
        }
    }

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(as_i64(&self.value)?)
    }

    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(as_i64(&self.value)?)
    }

    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(as_i64(&self.value)?)
    }

    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(as_i64(&self.value)?)
    }

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(as_i64(&self.value)?)
    }

    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(as_i64(&self.value)?)
    }

    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(as_i64(&self.value)?)
    }

    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(as_i64(&self.value)?)
    }

    fn deserialize_unit_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::String(s) => visitor.visit_bytes(&s.as_bytes()?),
            other => Err(serde_error(format_args!(
                "expected string for bytes, found Lua {}",
                value_type_name(&other)
            ))),
        }
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::String(s) => visitor.visit_byte_buf(s.as_bytes()?),
            other => Err(serde_error(format_args!(
                "expected string for byte buffer, found Lua {}",
                value_type_name(&other)
            ))),
        }
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::Table(t) => visitor.visit_seq(SeqAccessor {
                lua: self.lua,
                items: table_seq_values(&t)?.into_iter(),
            }),
            other => Err(serde_error(format_args!(
                "expected table for sequence, found Lua {}",
                value_type_name(&other)
            ))),
        }
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::Table(t) => visitor.visit_map(MapAccessor {
                lua: self.lua,
                pairs: t.raw_pairs()?.into_iter(),
                pending_value: None,
            }),
            other => Err(serde_error(format_args!(
                "expected table for map, found Lua {}",
                value_type_name(&other)
            ))),
        }
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::String(_) => visitor.visit_enum(EnumAccessor {
                lua: self.lua,
                variant: self.value,
                payload: None,
            }),
            Value::Table(t) => {
                let pairs = t.raw_pairs()?;
                if pairs.len() != 1 {
                    return Err(serde_error(
                        "expected a single-key table for an externally tagged enum variant",
                    ));
                }
                let (variant, payload) = pairs
                    .into_iter()
                    .next()
                    .ok_or_else(|| serde_error("enum variant table is empty"))?;
                visitor.visit_enum(EnumAccessor {
                    lua: self.lua,
                    variant,
                    payload: Some(payload),
                })
            }
            other => Err(serde_error(format_args!(
                "expected string or single-key table for enum, found Lua {}",
                value_type_name(&other)
            ))),
        }
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i128 u128 f32 f64 char str string identifier
    }
}

/// Yields a Lua array table's elements to a serde sequence visitor.
struct SeqAccessor<'a> {
    lua: &'a Lua,
    items: std::vec::IntoIter<Value>,
}

impl<'a, 'de> SeqAccess<'de> for SeqAccessor<'a> {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: DeserializeSeed<'de>,
    {
        match self.items.next() {
            Some(value) => seed
                .deserialize(LuaDeserializer {
                    lua: self.lua,
                    value,
                })
                .map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }
}

/// Yields a Lua table's key/value pairs to a serde map visitor.
struct MapAccessor<'a> {
    lua: &'a Lua,
    pairs: std::vec::IntoIter<(Value, Value)>,
    pending_value: Option<Value>,
}

impl<'a, 'de> MapAccess<'de> for MapAccessor<'a> {
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
    where
        K: DeserializeSeed<'de>,
    {
        match self.pairs.next() {
            Some((key, value)) => {
                self.pending_value = Some(value);
                seed.deserialize(LuaDeserializer {
                    lua: self.lua,
                    value: key,
                })
                .map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
    where
        V: DeserializeSeed<'de>,
    {
        let value = self
            .pending_value
            .take()
            .ok_or_else(|| serde_error("next_value_seed called before next_key_seed"))?;
        seed.deserialize(LuaDeserializer {
            lua: self.lua,
            value,
        })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.pairs.len())
    }
}

/// Drives an externally tagged enum: the variant name, then its payload.
struct EnumAccessor<'a> {
    lua: &'a Lua,
    variant: Value,
    payload: Option<Value>,
}

impl<'a, 'de> EnumAccess<'de> for EnumAccessor<'a> {
    type Error = Error;
    type Variant = VariantAccessor<'a>;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant)>
    where
        V: DeserializeSeed<'de>,
    {
        let variant = seed.deserialize(LuaDeserializer {
            lua: self.lua,
            value: self.variant,
        })?;
        Ok((
            variant,
            VariantAccessor {
                lua: self.lua,
                payload: self.payload,
            },
        ))
    }
}

/// The payload side of an externally tagged enum variant.
struct VariantAccessor<'a> {
    lua: &'a Lua,
    payload: Option<Value>,
}

impl<'a> VariantAccessor<'a> {
    /// Take the variant payload or report that one was required.
    fn take_payload(self) -> Result<Value> {
        self.payload
            .ok_or_else(|| serde_error("expected a payload for this enum variant"))
    }
}

impl<'a, 'de> VariantAccess<'de> for VariantAccessor<'a> {
    type Error = Error;

    fn unit_variant(self) -> Result<()> {
        match self.payload {
            None => Ok(()),
            Some(_) => Err(serde_error("expected a unit variant, found a payload")),
        }
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value>
    where
        T: DeserializeSeed<'de>,
    {
        let lua = self.lua;
        let value = self.take_payload()?;
        seed.deserialize(LuaDeserializer { lua, value })
    }

    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let lua = self.lua;
        let value = self.take_payload()?;
        LuaDeserializer { lua, value }.deserialize_seq(visitor)
    }

    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let lua = self.lua;
        let value = self.take_payload()?;
        LuaDeserializer { lua, value }.deserialize_map(visitor)
    }
}

/// A human-readable Lua type name for diagnostics.
fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Nil => "nil",
        Value::Boolean(_) => "boolean",
        Value::Integer(_) | Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Table(_) => "table",
        Value::Function(_) => "function",
        Value::UserData(_) => "userdata",
        Value::LightUserData(_) => "light userdata",
        Value::Thread(_) => "thread",
    }
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (no C analog — Rust-native serde integration)
//   target_crate:  omnilua
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         serde Serializer/Deserializer over Value; mirrors mlua's
//                  LuaSerdeExt. Pure additive layer on the public Value/Table
//                  API; integer lowering reuses lower_host_int. Feature-gated
//                  behind `serde`.
// ──────────────────────────────────────────────────────────────────────────
