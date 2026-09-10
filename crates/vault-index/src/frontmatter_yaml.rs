//! YAML 解析辅助类型，保持原有标量与映射解释。

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum YamlValue {
    Null,
    Bool(bool),
    Number(String),
    Text(String),
    Sequence(Vec<Self>),
    Mapping(BTreeMap<String, Self>),
}

impl<'de> Deserialize<'de> for YamlValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(YamlValueVisitor)
    }
}

struct YamlValueVisitor;

impl<'de> Visitor<'de> for YamlValueVisitor {
    type Value = YamlValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a YAML value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(YamlValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(YamlValue::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        YamlValue::deserialize(deserializer)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(YamlValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(YamlValue::Number(value.to_string()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(YamlValue::Number(value.to_string()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(YamlValue::Number(value.to_string()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(YamlValue::Text(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(YamlValue::Text(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(YamlValue::Sequence(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some((key, value)) = map.next_entry()? {
            values.insert(key, value);
        }
        Ok(YamlValue::Mapping(values))
    }
}
