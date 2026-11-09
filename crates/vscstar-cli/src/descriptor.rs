//! Reader for the csilgen schema descriptor (`v1alpha1`). The CLI drives
//! its generic command layer from this: every service and operation in the
//! contract appears here, so the CLI can never lag the server.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use ciborium::Value;

pub const DESCRIPTOR: &[u8] = include_bytes!("../../vscstar-types/vscstar.csil-schema.cbor");

#[derive(Debug, Clone)]
pub struct Schema {
    pub version: String,
    pub digest: Vec<u8>,
    pub rules: BTreeMap<String, Rule>,
    pub services: Vec<Service>,
}

#[derive(Debug, Clone)]
pub enum Rule {
    Type(Ty),
    Group(Group),
    GroupChoice(Vec<Group>),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Ty {
    Builtin(String),
    Reference(String),
    Array {
        element: Box<Ty>,
        occurrence: Occurrence,
    },
    Map {
        key: Box<Ty>,
        value: Box<Ty>,
    },
    Group(Group),
    Tuple(Group),
    Choice(Vec<Ty>),
    Literal(Value),
    Constrained(Box<Ty>),
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occurrence {
    Once,
    Optional,
    ZeroOrMore,
    OneOrMore,
    Other,
}

#[derive(Debug, Clone, Default)]
pub struct Group {
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: Option<String>,
    pub value: Ty,
    pub occurrence: Occurrence,
}

#[derive(Debug, Clone)]
pub struct Service {
    pub name: String,
    pub wire_id: Option<u64>,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub name: String,
    pub wire_id: Option<u64>,
    pub input: Ty,
    pub output: Ty,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Unidirectional,
    Bidirectional,
    Reverse,
}

pub fn load() -> Result<Schema> {
    parse(DESCRIPTOR)
}

pub fn parse(bytes: &[u8]) -> Result<Schema> {
    let root: Value = ciborium::de::from_reader(bytes).context("descriptor is not CBOR")?;
    let format = get(&root, "format").and_then(Value::as_text).unwrap_or("");
    if format != "csil-schema" {
        bail!("descriptor format is {format:?}, expected csil-schema");
    }
    let version = get(&root, "version")
        .and_then(Value::as_text)
        .unwrap_or("")
        .to_string();
    if version != "v1alpha1" {
        bail!("descriptor version {version} is not supported");
    }
    let digest = get(&root, "digest")
        .and_then(Value::as_bytes)
        .cloned()
        .unwrap_or_default();
    let body = get(&root, "body").ok_or_else(|| anyhow!("descriptor has no body"))?;
    let mut rules = BTreeMap::new();
    for r in get(body, "rules")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("no rules"))?
    {
        let name = text(get(r, "name"))?;
        let def = get(r, "definition").ok_or_else(|| anyhow!("rule {name} has no definition"))?;
        let (variant, data) = enum_parts(def)?;
        let rule = match variant.as_str() {
            "Type" => Rule::Type(parse_ty(data.ok_or_else(|| anyhow!("Type without data"))?)?),
            "Group" => Rule::Group(parse_group(
                data.ok_or_else(|| anyhow!("Group without data"))?,
            )?),
            "GroupChoice" => Rule::GroupChoice(
                data.and_then(Value::as_array)
                    .ok_or_else(|| anyhow!("GroupChoice without data"))?
                    .iter()
                    .map(parse_group)
                    .collect::<Result<_>>()?,
            ),
            other => bail!("unknown rule definition {other}"),
        };
        rules.insert(name, rule);
    }
    let mut services = Vec::new();
    for s in get(body, "services")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("no services"))?
    {
        let name = text(get(s, "name"))?;
        let wire_id = get(s, "wire_id").and_then(as_u64);
        let mut operations = Vec::new();
        for o in get(s, "operations")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("service {name} has no operations"))?
        {
            let direction = match get(o, "direction")
                .and_then(Value::as_text)
                .unwrap_or("Unidirectional")
            {
                "Bidirectional" => Direction::Bidirectional,
                "Reverse" => Direction::Reverse,
                _ => Direction::Unidirectional,
            };
            operations.push(Operation {
                name: text(get(o, "name"))?,
                wire_id: get(o, "wire_id").and_then(as_u64),
                input: parse_ty(get(o, "input").ok_or_else(|| anyhow!("op without input"))?)?,
                output: parse_ty(get(o, "output").ok_or_else(|| anyhow!("op without output"))?)?,
                direction,
            });
        }
        services.push(Service {
            name,
            wire_id,
            operations,
        });
    }
    Ok(Schema {
        version,
        digest,
        rules,
        services,
    })
}

fn parse_ty(v: &Value) -> Result<Ty> {
    let (variant, data) = enum_parts(v)?;
    Ok(match variant.as_str() {
        "Builtin" => Ty::Builtin(text(data)?),
        "Reference" => Ty::Reference(text(data)?),
        "Socket" | "Plug" => Ty::Reference(text(data)?),
        "Array" => {
            let d = data.ok_or_else(|| anyhow!("Array without data"))?;
            Ty::Array {
                element: Box::new(parse_ty(
                    get(d, "element").ok_or_else(|| anyhow!("Array without element"))?,
                )?),
                occurrence: parse_occurrence(get(d, "occurrence")),
            }
        }
        "Map" => {
            let d = data.ok_or_else(|| anyhow!("Map without data"))?;
            Ty::Map {
                key: Box::new(parse_ty(
                    get(d, "key").ok_or_else(|| anyhow!("Map without key"))?,
                )?),
                value: Box::new(parse_ty(
                    get(d, "value").ok_or_else(|| anyhow!("Map without value"))?,
                )?),
            }
        }
        "Group" => Ty::Group(parse_group(
            data.ok_or_else(|| anyhow!("Group without data"))?,
        )?),
        "Tuple" => Ty::Tuple(parse_group(
            data.ok_or_else(|| anyhow!("Tuple without data"))?,
        )?),
        "Choice" => Ty::Choice(
            data.and_then(Value::as_array)
                .ok_or_else(|| anyhow!("Choice without arms"))?
                .iter()
                .map(parse_ty)
                .collect::<Result<_>>()?,
        ),
        "Literal" => Ty::Literal(parse_literal(
            data.ok_or_else(|| anyhow!("Literal without data"))?,
        )?),
        "Constrained" => {
            let d = data.ok_or_else(|| anyhow!("Constrained without data"))?;
            Ty::Constrained(Box::new(parse_ty(
                get(d, "base").ok_or_else(|| anyhow!("Constrained without base"))?,
            )?))
        }
        other => Ty::Other(other.to_string()),
    })
}

fn parse_literal(v: &Value) -> Result<Value> {
    let (variant, data) = enum_parts(v)?;
    Ok(match (variant.as_str(), data) {
        ("Null", _) => Value::Null,
        ("Array", Some(Value::Array(items))) => {
            Value::Array(items.iter().map(parse_literal).collect::<Result<_>>()?)
        }
        (_, Some(d)) => d.clone(),
        (_, None) => Value::Null,
    })
}

fn parse_group(v: &Value) -> Result<Group> {
    let mut entries = Vec::new();
    for e in get(v, "entries")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("group without entries"))?
    {
        let key = match get(e, "key") {
            None | Some(Value::Null) => None,
            Some(k) => {
                let (variant, data) = enum_parts(k)?;
                match (variant.as_str(), data) {
                    ("Bare", Some(Value::Text(t))) => Some(t.clone()),
                    ("Literal", Some(l)) => match parse_literal(l)? {
                        Value::Text(t) => Some(t),
                        other => Some(format!("{other:?}")),
                    },
                    _ => None,
                }
            }
        };
        entries.push(Entry {
            key,
            value: parse_ty(get(e, "value").ok_or_else(|| anyhow!("entry without value"))?)?,
            occurrence: parse_occurrence(get(e, "occurrence")),
        });
    }
    Ok(Group { entries })
}

fn parse_occurrence(v: Option<&Value>) -> Occurrence {
    match v {
        None | Some(Value::Null) => Occurrence::Once,
        Some(Value::Text(t)) => match t.as_str() {
            "Optional" => Occurrence::Optional,
            "ZeroOrMore" => Occurrence::ZeroOrMore,
            "OneOrMore" => Occurrence::OneOrMore,
            _ => Occurrence::Other,
        },
        Some(_) => Occurrence::Other,
    }
}

/// An enum with data is a one-entry map `{Variant: data}`; without data it is text.
fn enum_parts(v: &Value) -> Result<(String, Option<&Value>)> {
    match v {
        Value::Text(t) => Ok((t.clone(), None)),
        Value::Map(m) if m.len() == 1 => {
            let (k, d) = &m[0];
            Ok((
                k.as_text()
                    .ok_or_else(|| anyhow!("enum key is not text"))?
                    .to_string(),
                Some(d),
            ))
        }
        other => bail!("expected enum encoding, found {other:?}"),
    }
}

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_map()?
        .iter()
        .find(|(k, _)| k.as_text() == Some(key))
        .map(|(_, v)| v)
}

fn text(v: Option<&Value>) -> Result<String> {
    v.and_then(Value::as_text)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("expected text"))
}

fn as_u64(v: &Value) -> Option<u64> {
    v.as_integer().and_then(|i| u64::try_from(i).ok())
}

impl Schema {
    pub fn service(&self, name: &str) -> Option<&Service> {
        self.services.iter().find(|s| s.name == name)
    }

    pub fn operation(&self, service: &str, op: &str) -> Option<&Operation> {
        self.service(service)?
            .operations
            .iter()
            .find(|o| o.name == op)
    }

    /// Follow references until a concrete type or group is reached.
    pub fn resolve<'a>(&'a self, ty: &'a Ty) -> Resolved<'a> {
        let mut cur = ty;
        let mut last_name: Option<&'a str> = None;
        for _ in 0..32 {
            match cur {
                Ty::Reference(name) => match self.rules.get(name) {
                    Some(Rule::Type(t)) => {
                        last_name = Some(name);
                        cur = t;
                    }
                    Some(Rule::Group(g)) => return Resolved::Group(g, Some(name)),
                    Some(Rule::GroupChoice(gs)) => return Resolved::GroupChoice(gs, Some(name)),
                    None => return Resolved::Unknown(name),
                },
                Ty::Constrained(inner) => cur = inner,
                Ty::Group(g) => return Resolved::Group(g, last_name),
                other => return Resolved::Ty(other),
            }
        }
        Resolved::Unknown("reference cycle")
    }

    /// Name of the type an arm refers to, for union JSON projection.
    pub fn arm_name(&self, ty: &Ty) -> Option<String> {
        match ty {
            Ty::Reference(n) => Some(n.clone()),
            _ => None,
        }
    }
}

pub enum Resolved<'a> {
    Ty(&'a Ty),
    Group(&'a Group, Option<&'a str>),
    GroupChoice(&'a [Group], Option<&'a str>),
    Unknown(&'a str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_descriptor_parses() {
        let s = load().expect("descriptor parses");
        assert_eq!(s.digest.len(), 32);
        assert!(s.services.len() >= 9);
        let op = s.operation("Repos", "create").expect("Repos/create exists");
        assert_eq!(op.direction, Direction::Unidirectional);
        match s.resolve(&op.input) {
            Resolved::Group(g, Some("CreateRepoRequest")) => {
                assert_eq!(g.entries[0].key.as_deref(), Some("ctx"));
            }
            _ => panic!("CreateRepoRequest should resolve to a group"),
        }
    }
}
