//! Bounds JSON frames, wire integers and schema closing.
use super::{
    ControlError, MAX_COLLECTION, MAX_DEPTH, MAX_FRAME_BYTES, MAX_NODES, MAX_RESPONSE_BYTES,
    MAX_SAFE_INTEGER, MAX_TEXT_BYTES,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::io::Write;

struct Capped {
    bytes: Vec<u8>,
    max: usize,
}
struct JsonSeed<'a> {
    depth: usize,
    nodes: &'a mut usize,
    text: &'a mut usize,
}

// The caller already owns Value; this caps additional processing, not its creation.
pub(crate) fn budget(value: &Value) -> std::result::Result<(), ControlError> {
    let mut stack = vec![(value, 0)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > MAX_NODES || depth > MAX_DEPTH {
            return Err(ControlError::Limit);
        }
        match value {
            Value::Array(a) => {
                if a.len() > MAX_COLLECTION || nodes + stack.len() + a.len() > MAX_NODES {
                    return Err(ControlError::Limit);
                }
                stack.extend(a.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(o) => {
                if o.len() > MAX_COLLECTION || nodes + stack.len() + o.len() > MAX_NODES {
                    return Err(ControlError::Limit);
                }
                for (k, v) in o {
                    bytes = bytes.saturating_add(k.len());
                    stack.push((v, depth + 1));
                }
            }
            Value::Number(n)
                if n.as_i64()
                    .is_some_and(|v| !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&v))
                    || n.as_u64().is_some_and(|v| v > MAX_SAFE_INTEGER as u64)
                    || n.as_f64()
                        .is_some_and(|v| v.abs() > MAX_SAFE_INTEGER as f64) =>
            {
                return Err(ControlError::InvalidParams);
            }
            Value::String(s) => bytes = bytes.saturating_add(s.len()),
            _ => {}
        }
        if bytes > MAX_TEXT_BYTES {
            return Err(ControlError::Limit);
        }
    }
    Ok(())
}

/// Validate the actual wire representation of a candidate, including generated
/// IDs and arithmetic-derived ticks, while the edit is still reversible.
pub(crate) fn validate_wire<T: Serialize + ?Sized>(
    value: &T,
) -> std::result::Result<(), ControlError> {
    let bytes = bounded_json(value, MAX_RESPONSE_BYTES - 4096)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| ControlError::InvalidParams)?;
    check_integers(&value)
}

fn check_integers(value: &Value) -> std::result::Result<(), ControlError> {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::Number(n)
                if n.as_i64()
                    .is_some_and(|v| !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&v))
                    || n.as_u64().is_some_and(|v| v > MAX_SAFE_INTEGER as u64) =>
            {
                return Err(ControlError::InvalidParams);
            }
            Value::Array(values) => stack.extend(values),
            Value::Object(values) => stack.extend(values.values()),
            _ => {}
        }
    }
    Ok(())
}

/// Close fixed objects but retain typed dictionaries and local generated refs.
/// The wire integer policy is applied identically to every nested schema.
pub(crate) fn close_schema(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if object.contains_key("properties") {
            object.insert("additionalProperties".into(), Value::Bool(false));
        }
        let integer = object.get("type").is_some_and(|t| {
            t == "integer"
                || t.as_array()
                    .is_some_and(|a| a.iter().any(|x| x == "integer"))
        });
        if integer {
            let min = object
                .get("minimum")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN)
                .max(-MAX_SAFE_INTEGER);
            let max = object
                .get("maximum")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX)
                .min(MAX_SAFE_INTEGER as u64);
            object.insert("minimum".into(), json!(min));
            object.insert("maximum".into(), json!(max));
        }
        for v in object.values_mut() {
            close_schema(v);
        }
    } else if let Some(array) = value.as_array_mut() {
        for v in array {
            close_schema(v);
        }
    }
}

impl Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.max {
            return Err(std::io::Error::other("control byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn bounded_json<T: Serialize + ?Sized>(
    value: &T,
    max: usize,
) -> std::result::Result<Vec<u8>, ControlError> {
    let mut writer = Capped {
        bytes: Vec::new(),
        max: max.min(MAX_RESPONSE_BYTES),
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| ControlError::Limit)?;
    Ok(writer.bytes)
}

pub fn parse_frame(bytes: &[u8]) -> std::result::Result<Value, ControlError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(ControlError::Limit);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde::de::DeserializeSeed::deserialize(
        JsonSeed {
            depth: 0,
            nodes: &mut 0,
            text: &mut 0,
        },
        &mut deserializer,
    )
    .map_err(|_| ControlError::InvalidParams)?;
    deserializer
        .end()
        .map_err(|_| ControlError::InvalidParams)?;
    Ok(value)
}
impl<'de> serde::de::DeserializeSeed<'de> for JsonSeed<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Value, D::Error> {
        if self.depth > MAX_DEPTH || *self.nodes >= MAX_NODES {
            return Err(serde::de::Error::custom("control limit"));
        }
        *self.nodes += 1;
        d.deserialize_any(self)
    }
}
impl<'de> serde::de::Visitor<'de> for JsonSeed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Value, E> {
        if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&v) {
            return Err(E::custom("integer range"));
        }
        Ok(json!(v))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Value, E> {
        if v > MAX_SAFE_INTEGER as u64 {
            return Err(E::custom("integer range"));
        }
        Ok(json!(v))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Value, E> {
        if !v.is_finite() || v.abs() > MAX_SAFE_INTEGER as f64 {
            return Err(E::custom("number range"));
        }
        Ok(json!(v))
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Value, E> {
        *self.text = self.text.saturating_add(v.len());
        if *self.text > MAX_TEXT_BYTES {
            return Err(E::custom("text limit"));
        }
        Ok(Value::String(v.into()))
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut a: A,
    ) -> std::result::Result<Value, A::Error> {
        let mut values = serde_json::Map::new();
        while let Some(key) = a.next_key::<String>()? {
            *self.text = self.text.saturating_add(key.len());
            if values.len() >= MAX_COLLECTION
                || *self.text > MAX_TEXT_BYTES
                || values.contains_key(&key)
            {
                return Err(serde::de::Error::custom("object limit or duplicate key"));
            }
            let value = a.next_value_seed(JsonSeed {
                depth: self.depth + 1,
                nodes: self.nodes,
                text: self.text,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(
        self,
        mut a: A,
    ) -> std::result::Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(v) = a.next_element_seed(JsonSeed {
            depth: self.depth + 1,
            nodes: self.nodes,
            text: self.text,
        })? {
            if values.len() >= MAX_COLLECTION {
                return Err(serde::de::Error::custom("array limit"));
            }
            values.push(v);
        }
        Ok(Value::Array(values))
    }
}
