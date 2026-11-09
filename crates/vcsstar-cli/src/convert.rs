//! JSON <-> CSIL CBOR, driven by the schema descriptor. Records are CBOR
//! maps with text keys, unions are `[index, value]`, bytes are base64 in
//! JSON. The server's generated codec validates the result again.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use ciborium::Value;
use serde_json::Value as Json;

use crate::descriptor::{Group, Occurrence, Resolved, Schema, Ty};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub fn json_to_cbor(schema: &Schema, ty: &Ty, json: &Json, path: &str) -> Result<Value> {
    match schema.resolve(ty) {
        Resolved::Group(g, _) => group_to_cbor(schema, g, json, path),
        Resolved::GroupChoice(gs, _) => {
            let mut last = None;
            for g in gs {
                match group_to_cbor(schema, g, json, path) {
                    Ok(v) => return Ok(v),
                    Err(e) => last = Some(e),
                }
            }
            Err(last.unwrap_or_else(|| anyhow!("{path}: no group arm matched")))
        }
        Resolved::Unknown(name) => bail!("{path}: unknown type {name}"),
        Resolved::Ty(t) => match t {
            Ty::Builtin(b) => builtin_to_cbor(b, json, path),
            Ty::Array { element, .. } => {
                let items = json
                    .as_array()
                    .ok_or_else(|| anyhow!("{path}: expected an array"))?;
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| json_to_cbor(schema, element, item, &format!("{path}[{i}]")))
                    .collect::<Result<Vec<_>>>()
                    .map(Value::Array)
            }
            Ty::Map { key, value } => {
                let obj = json
                    .as_object()
                    .ok_or_else(|| anyhow!("{path}: expected an object"))?;
                let mut out = Vec::with_capacity(obj.len());
                for (k, v) in obj {
                    let kv = json_to_cbor(
                        schema,
                        key,
                        &Json::String(k.clone()),
                        &format!("{path}.{k}"),
                    )?;
                    out.push((kv, json_to_cbor(schema, value, v, &format!("{path}.{k}"))?));
                }
                Ok(Value::Map(out))
            }
            Ty::Tuple(g) => {
                let items = json
                    .as_array()
                    .ok_or_else(|| anyhow!("{path}: expected an array (tuple)"))?;
                if items.len() != g.entries.len() {
                    bail!(
                        "{path}: tuple has {} items, expected {}",
                        items.len(),
                        g.entries.len()
                    );
                }
                items
                    .iter()
                    .zip(&g.entries)
                    .enumerate()
                    .map(|(i, (item, e))| {
                        json_to_cbor(schema, &e.value, item, &format!("{path}[{i}]"))
                    })
                    .collect::<Result<Vec<_>>>()
                    .map(Value::Array)
            }
            Ty::Choice(arms) => choice_to_cbor(schema, arms, json, path),
            Ty::Literal(lit) => Ok(lit.clone()),
            Ty::Other(o) => bail!("{path}: unsupported type kind {o}"),
            Ty::Reference(_) | Ty::Constrained(_) | Ty::Group(_) => unreachable!("resolved"),
        },
    }
}

fn group_to_cbor(schema: &Schema, g: &Group, json: &Json, path: &str) -> Result<Value> {
    let obj = json
        .as_object()
        .ok_or_else(|| anyhow!("{path}: expected an object"))?;
    let mut out = Vec::with_capacity(obj.len());
    let mut seen = std::collections::BTreeSet::new();
    for e in &g.entries {
        let Some(key) = &e.key else {
            // Unkeyed entry: `* text => any` style additional properties.
            continue;
        };
        seen.insert(key.as_str());
        match obj.get(key) {
            Some(Json::Null) | None => {
                if e.occurrence == Occurrence::Once {
                    bail!("{path}: missing required field {key}");
                }
            }
            Some(v) => out.push((
                Value::Text(key.clone()),
                json_to_cbor(schema, &e.value, v, &format!("{path}.{key}"))?,
            )),
        }
    }
    let extra: Vec<&String> = obj.keys().filter(|k| !seen.contains(k.as_str())).collect();
    if !extra.is_empty() && !g.entries.iter().any(|e| e.key.is_none()) {
        bail!("{path}: unknown field(s) {extra:?}");
    }
    Ok(Value::Map(out))
}

/// Union JSON forms accepted: `{"TypeName": value}` for a named arm,
/// `{"$variant": index, "value": v}`, or `[index, v]`.
fn choice_to_cbor(schema: &Schema, arms: &[Ty], json: &Json, path: &str) -> Result<Value> {
    if let Some(arr) = json.as_array() {
        if arr.len() == 2 {
            if let Some(idx) = arr[0].as_u64() {
                let arm = arms
                    .get(idx as usize)
                    .ok_or_else(|| anyhow!("{path}: variant index {idx} out of range"))?;
                return Ok(Value::Array(vec![
                    Value::Integer(idx.into()),
                    json_to_cbor(schema, arm, &arr[1], path)?,
                ]));
            }
        }
    }
    if let Some(obj) = json.as_object() {
        if let Some(idx) = obj.get("$variant").and_then(Json::as_u64) {
            let arm = arms
                .get(idx as usize)
                .ok_or_else(|| anyhow!("{path}: variant index {idx} out of range"))?;
            let inner = obj.get("value").unwrap_or(&Json::Null);
            return Ok(Value::Array(vec![
                Value::Integer(idx.into()),
                json_to_cbor(schema, arm, inner, path)?,
            ]));
        }
        if obj.len() == 1 {
            let (k, v) = obj.iter().next().unwrap();
            if let Some(idx) = arms
                .iter()
                .position(|a| schema.arm_name(a).as_deref() == Some(k.as_str()))
            {
                return Ok(Value::Array(vec![
                    Value::Integer((idx as u64).into()),
                    json_to_cbor(schema, &arms[idx], v, path)?,
                ]));
            }
        }
    }
    // Scalar unions (e.g. text / nil): try each arm in order.
    for (idx, arm) in arms.iter().enumerate() {
        if let Ok(v) = json_to_cbor(schema, arm, json, path) {
            return Ok(Value::Array(vec![Value::Integer((idx as u64).into()), v]));
        }
    }
    let names: Vec<String> = arms
        .iter()
        .enumerate()
        .map(|(i, a)| schema.arm_name(a).unwrap_or_else(|| format!("#{i}")))
        .collect();
    bail!("{path}: union value must be {{\"<Arm>\": ...}} with Arm in {names:?}")
}

fn builtin_to_cbor(name: &str, json: &Json, path: &str) -> Result<Value> {
    Ok(match name {
        "text" | "tstr" => Value::Text(
            json.as_str()
                .ok_or_else(|| anyhow!("{path}: expected a string"))?
                .to_string(),
        ),
        "int" => {
            let i = json
                .as_i64()
                .ok_or_else(|| anyhow!("{path}: expected an integer"))?;
            Value::Integer(i.into())
        }
        "uint" => {
            let u = json
                .as_u64()
                .ok_or_else(|| anyhow!("{path}: expected an unsigned integer"))?;
            Value::Integer(u.into())
        }
        "float" | "float16" | "float32" | "float64" => Value::Float(
            json.as_f64()
                .ok_or_else(|| anyhow!("{path}: expected a number"))?,
        ),
        "bool" => Value::Bool(
            json.as_bool()
                .ok_or_else(|| anyhow!("{path}: expected a boolean"))?,
        ),
        "bytes" | "bstr" => {
            let s = json
                .as_str()
                .ok_or_else(|| anyhow!("{path}: expected a base64 string for bytes"))?;
            Value::Bytes(
                B64.decode(s)
                    .with_context(|| format!("{path}: bytes must be base64"))?,
            )
        }
        "nil" | "null" => Value::Null,
        "any" => json_any_to_cbor(json),
        other => bail!("{path}: unsupported builtin {other}"),
    })
}

fn json_any_to_cbor(json: &Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i.into())
            } else if let Some(u) = n.as_u64() {
                Value::Integer(u.into())
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        Json::String(s) => Value::Text(s.clone()),
        Json::Array(a) => Value::Array(a.iter().map(json_any_to_cbor).collect()),
        Json::Object(o) => Value::Map(
            o.iter()
                .map(|(k, v)| (Value::Text(k.clone()), json_any_to_cbor(v)))
                .collect(),
        ),
    }
}

pub fn cbor_to_json(schema: &Schema, ty: &Ty, cbor: &Value, path: &str) -> Result<Json> {
    match schema.resolve(ty) {
        Resolved::Group(g, _) => group_to_json(schema, g, cbor, path),
        Resolved::GroupChoice(gs, _) => {
            for g in gs {
                if let Ok(v) = group_to_json(schema, g, cbor, path) {
                    return Ok(v);
                }
            }
            Ok(cbor_any_to_json(cbor))
        }
        Resolved::Unknown(_) => Ok(cbor_any_to_json(cbor)),
        Resolved::Ty(t) => match t {
            Ty::Builtin(b) => builtin_to_json(b, cbor),
            Ty::Array { element, .. } => match cbor {
                Value::Array(items) => items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| cbor_to_json(schema, element, item, &format!("{path}[{i}]")))
                    .collect::<Result<Vec<_>>>()
                    .map(Json::Array),
                other => bail!("{path}: expected array, got {}", kind(other)),
            },
            Ty::Map { value, .. } => match cbor {
                Value::Map(entries) => {
                    let mut out = serde_json::Map::new();
                    for (k, v) in entries {
                        let key = match k {
                            Value::Text(t) => t.clone(),
                            other => format!("{other:?}"),
                        };
                        out.insert(
                            key.clone(),
                            cbor_to_json(schema, value, v, &format!("{path}.{key}"))?,
                        );
                    }
                    Ok(Json::Object(out))
                }
                other => bail!("{path}: expected map, got {}", kind(other)),
            },
            Ty::Tuple(g) => match cbor {
                Value::Array(items) => items
                    .iter()
                    .zip(&g.entries)
                    .enumerate()
                    .map(|(i, (item, e))| {
                        cbor_to_json(schema, &e.value, item, &format!("{path}[{i}]"))
                    })
                    .collect::<Result<Vec<_>>>()
                    .map(Json::Array),
                other => bail!("{path}: expected tuple array, got {}", kind(other)),
            },
            Ty::Choice(arms) => match cbor {
                Value::Array(items) if items.len() == 2 => {
                    let idx = items[0]
                        .as_integer()
                        .and_then(|i| u64::try_from(i).ok())
                        .ok_or_else(|| anyhow!("{path}: bad union index"))?;
                    let arm = arms
                        .get(idx as usize)
                        .ok_or_else(|| anyhow!("{path}: union index {idx} out of range"))?;
                    let inner = cbor_to_json(schema, arm, &items[1], path)?;
                    Ok(match schema.arm_name(arm) {
                        Some(name) => serde_json::json!({ name: inner }),
                        None => serde_json::json!({ "$variant": idx, "value": inner }),
                    })
                }
                other => Ok(cbor_any_to_json(other)),
            },
            Ty::Literal(_) | Ty::Other(_) => Ok(cbor_any_to_json(cbor)),
            Ty::Reference(_) | Ty::Constrained(_) | Ty::Group(_) => unreachable!("resolved"),
        },
    }
}

fn group_to_json(schema: &Schema, g: &Group, cbor: &Value, path: &str) -> Result<Json> {
    let entries = match cbor {
        Value::Map(m) => m,
        other => bail!("{path}: expected record map, got {}", kind(other)),
    };
    let mut out = serde_json::Map::new();
    for (k, v) in entries {
        let key = match k {
            Value::Text(t) => t.clone(),
            other => format!("{other:?}"),
        };
        let json = match g
            .entries
            .iter()
            .find(|e| e.key.as_deref() == Some(key.as_str()))
        {
            Some(e) => cbor_to_json(schema, &e.value, v, &format!("{path}.{key}"))?,
            None => cbor_any_to_json(v),
        };
        out.insert(key, json);
    }
    Ok(Json::Object(out))
}

fn builtin_to_json(name: &str, cbor: &Value) -> Result<Json> {
    Ok(match (name, cbor) {
        ("bytes" | "bstr", Value::Bytes(b)) => Json::String(B64.encode(b)),
        (_, other) => cbor_any_to_json(other),
    })
}

pub fn cbor_any_to_json(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Integer(i) => {
            let i128v: i128 = (*i).into();
            if let Ok(u) = u64::try_from(i128v) {
                Json::from(u)
            } else if let Ok(s) = i64::try_from(i128v) {
                Json::from(s)
            } else {
                Json::String(i128v.to_string())
            }
        }
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Value::Text(t) => Json::String(t.clone()),
        Value::Bytes(b) => Json::String(B64.encode(b)),
        Value::Array(a) => Json::Array(a.iter().map(cbor_any_to_json).collect()),
        Value::Map(m) => Json::Object(
            m.iter()
                .map(|(k, v)| {
                    let key = match k {
                        Value::Text(t) => t.clone(),
                        other => format!("{other:?}"),
                    };
                    (key, cbor_any_to_json(v))
                })
                .collect(),
        ),
        Value::Tag(_, inner) => cbor_any_to_json(inner),
        _ => Json::Null,
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Integer(_) => "integer",
        Value::Float(_) => "float",
        Value::Text(_) => "text",
        Value::Bytes(_) => "bytes",
        Value::Array(_) => "array",
        Value::Map(_) => "map",
        Value::Tag(..) => "tag",
        _ => "other",
    }
}

pub fn encode(v: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(v, &mut out).context("cbor encode")?;
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<Value> {
    ciborium::de::from_reader(bytes).context("cbor decode")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor;

    #[test]
    fn roundtrip_create_repo_request_matches_generated_codec() {
        let schema = descriptor::load().unwrap();
        let op = schema.operation("Repos", "create").unwrap();
        let json = serde_json::json!({
            "ctx": {"request_id": "r1"},
            "org": "acme", "name": "widgets", "visibility": "public"
        });
        let cbor = json_to_cbor(&schema, &op.input, &json, "$").unwrap();
        let bytes = encode(&cbor).unwrap();
        let typed = vcsstar_types::decode_create_repo_request(&bytes)
            .expect("generated codec accepts CLI encoding");
        assert_eq!(typed.org, "acme");
        assert_eq!(typed.visibility.as_deref(), Some("public"));
        let back = cbor_to_json(
            &schema,
            &op.input,
            &decode(&vcsstar_types::encode_create_repo_request(&typed)).unwrap(),
            "$",
        )
        .unwrap();
        assert_eq!(back["name"], "widgets");
    }

    #[test]
    fn union_projection_uses_arm_names() {
        let schema = descriptor::load().unwrap();
        let ev = Ty::Reference("Event".into());
        let json = serde_json::json!({
            "lane": "acme/widgets", "seq": 1, "kind": "push", "at": 0,
            "payload": {"PushPayload": {"ref": "refs/heads/main", "before": "a", "after": "b", "forced": false, "commits": []}}
        });
        let cbor = json_to_cbor(&schema, &ev, &json, "$").unwrap();
        let typed = vcsstar_types::decode_event(&encode(&cbor).unwrap()).unwrap();
        assert!(matches!(
            typed.payload,
            vcsstar_types::EventPayload::Variant0(_)
        ));
        let back = cbor_to_json(&schema, &ev, &cbor, "$").unwrap();
        assert!(back["payload"]["PushPayload"].is_object());
    }

    #[test]
    fn missing_required_field_is_an_error() {
        let schema = descriptor::load().unwrap();
        let op = schema.operation("Repos", "create").unwrap();
        let err = json_to_cbor(
            &schema,
            &op.input,
            &serde_json::json!({"ctx": {"request_id": "r"}, "org": "a"}),
            "$",
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing required field name"));
    }
}
